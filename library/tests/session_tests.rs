use bittorrent_rs::{PeerDetails, PeerMessage, PeerNetwork, TorrentSession, TorrentStatus};
use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn sample_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}

fn cleanup_download_path(download_path: &PathBuf) {
    let _ = fs::remove_dir_all(download_path);
}

#[test]
fn test_send_bitfield_and_unchoke_after_handshake() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_handshake");
    cleanup_download_path(&download_path);

    let session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, false)
        .expect("Failed to create torrent session");
    let expected_info_hash = session.context.lock().unwrap().info_hash.clone();
    let expected_bitfield = session.context.lock().unwrap().bitfield.clone();

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().expect("Failed to get listener address");

    let server_handle = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("Failed to accept connection");
        let mut net = PeerNetwork::new(stream);
        let (remote_info_hash, _) = net.read_handshake().expect("Failed to read handshake");
        assert_eq!(remote_info_hash, expected_info_hash);

        let local_peer_id = *b"-RS0001-000000000000";
        net.write_handshake(&expected_info_hash, &local_peer_id)
            .expect("Failed to write handshake");

        let first = net.read_message().expect("Failed to read first message");
        assert_eq!(first, PeerMessage::Bitfield(expected_bitfield.clone()));

        let second = net.read_message().expect("Failed to read second message");
        assert_eq!(second, PeerMessage::Unchoke);

        let third = net.read_message().expect("Failed to read third message");
        assert_eq!(third, PeerMessage::Interested);
    });

    let peer_details = PeerDetails {
        info_hash: expected_info_hash,
        peer_id: None,
        ip: "127.0.0.1".to_string(),
        port: addr.port(),
    };
    session
        .connect_and_download_peer(peer_details, None)
        .expect("Failed to connect to peer");

    thread::sleep(Duration::from_secs(1));
    session.join_peer_workers();
    server_handle.join().expect("Server thread panicked");
    cleanup_download_path(&download_path);
}

#[test]
fn test_uploads_piece_when_peer_requests_block() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_upload_request");
    cleanup_download_path(&download_path);

    let session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, true)
        .expect("Failed to create seeding torrent session");
    let expected_info_hash = session.context.lock().unwrap().info_hash.clone();

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().expect("Failed to get listener address");

    let server_handle = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("Failed to accept connection");
        let mut net = PeerNetwork::new(stream);
        let (remote_info_hash, _) = net.read_handshake().expect("Failed to read handshake");
        assert_eq!(remote_info_hash, expected_info_hash);

        let local_peer_id = *b"-RS0001-000000000000";
        net.write_handshake(&expected_info_hash, &local_peer_id)
            .expect("Failed to write handshake");

        let _ = net.read_message().expect("Failed to read bitfield");
        let _ = net.read_message().expect("Failed to read unchoke");
        let _ = net.read_message().expect("Failed to read interested");

        net.write_message(PeerMessage::Request {
            index: 0,
            begin: 0,
            length: 16384,
        })
        .expect("Failed to send request");

        let response = net.read_message().expect("Failed to read piece response");
        match response {
            PeerMessage::Piece { index, begin, block } => {
                assert_eq!(index, 0);
                assert_eq!(begin, 0);
                assert_eq!(block.len(), 16384);
            }
            other => panic!("Expected Piece response, got {:?}", other),
        }
    });

    let peer_details = PeerDetails {
        info_hash: expected_info_hash,
        peer_id: None,
        ip: "127.0.0.1".to_string(),
        port: addr.port(),
    };
    session
        .connect_and_download_peer(peer_details, None)
        .expect("Failed to connect to peer");

    thread::sleep(Duration::from_secs(1));
    session.join_peer_workers();
    server_handle.join().expect("Server thread panicked");
    cleanup_download_path(&download_path);
}

#[test]
fn test_create_session_for_download() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_download");
    cleanup_download_path(&download_path);

    let mut session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, false)
        .expect("Failed to create torrent session");

    assert_eq!(session.status(), TorrentStatus::Initialised);
    assert!(session.context.lock().unwrap().files_to_download.len() >= 1);
    assert!(
        session
            .context
            .lock()
            .unwrap()
            .files_to_download
            .iter()
            .all(|f| std::path::Path::new(&f.name).exists())
    );

    session.start_download().expect("Failed to start download");
    assert_eq!(session.status(), TorrentStatus::Downloading);
    assert!(session.progress() >= 0.0);

    session.pause().expect("Failed to pause download");
    assert_eq!(session.status(), TorrentStatus::Paused);

    session.resume().expect("Failed to resume download");
    assert_eq!(session.status(), TorrentStatus::Downloading);

    session.stop().expect("Failed to stop download");
    assert_eq!(session.status(), TorrentStatus::Ended);

    cleanup_download_path(&download_path);
}

#[test]
fn test_create_session_for_seeding() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_seeding");
    cleanup_download_path(&download_path);

    let session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, true)
        .expect("Failed to create seeding torrent session");

    assert_eq!(session.status(), TorrentStatus::Seeding);
    assert_eq!(session.progress(), 100.0);
    assert!(session.validate().is_ok());

    cleanup_download_path(&download_path);
}
