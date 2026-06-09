use bittorrent_rs::{PeerDetails, PeerMessage, TorrentSession, TorrentStatus};
use bittorrent_rs::peer_network::PeerNetwork;
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

    let mut session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, false)
        .expect("Failed to create torrent session");
    let expected_info_hash = session.context().lock().unwrap().info_hash.clone();
    let expected_bitfield = session.context().lock().unwrap().bitfield.clone();
    let expected_info_hash_clone = expected_info_hash.clone();

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().expect("Failed to get listener address");

    let server_handle = thread::spawn(move || {
        futures::executor::block_on(async {
            let (stream, _) = listener.accept().expect("Failed to accept connection");
            let socket = std::sync::Arc::new(bittorrent_rs::peer_network::TcpSocket::new(stream));
            let net = PeerNetwork::new(socket);
            let (remote_info_hash, _) = net.read_handshake().await.expect("Failed to read handshake");
            assert_eq!(remote_info_hash, expected_info_hash_clone);

            let local_peer_id = *b"-RS0001-000000000000";
            net.write_handshake(&expected_info_hash_clone, &local_peer_id).await
                .expect("Failed to write handshake");

            let mut read_buf = vec![0u8; 1024 * 16 + 2 * 4 + 1];
            let first = net.read_message(&mut read_buf).await.expect("Failed to read first message");
            assert_eq!(first, PeerMessage::Bitfield(&expected_bitfield));

            let second = net.read_message(&mut read_buf).await.expect("Failed to read second message");
            assert_eq!(second, PeerMessage::Unchoke);

            let third = net.read_message(&mut read_buf).await.expect("Failed to read third message");
            assert_eq!(third, PeerMessage::Interested);
        });
    });

    let peer_details = PeerDetails {
        info_hash: expected_info_hash,
        peer_id: None,
        ip: "127.0.0.1".to_string(),
        port: addr.port(),
    };
    session
        .connect_and_download_peer(peer_details)
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

    let mut session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, true)
        .expect("Failed to create seeding torrent session");
    let expected_info_hash = session.context().lock().unwrap().info_hash.clone();
    let expected_info_hash_clone = expected_info_hash.clone();

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().expect("Failed to get listener address");

    let server_handle = thread::spawn(move || {
        futures::executor::block_on(async {
            let (stream, _) = listener.accept().expect("Failed to accept connection");
            let socket = std::sync::Arc::new(bittorrent_rs::peer_network::TcpSocket::new(stream));
            let net = PeerNetwork::new(socket);
            let (remote_info_hash, _) = net.read_handshake().await.expect("Failed to read handshake");
            assert_eq!(remote_info_hash, expected_info_hash_clone);

            let local_peer_id = *b"-RS0001-000000000000";
            net.write_handshake(&expected_info_hash_clone, &local_peer_id).await
                .expect("Failed to write handshake");

            let mut read_buf = vec![0u8; 1024 * 16 + 2 * 4 + 1];
            let _ = net.read_message(&mut read_buf).await.expect("Failed to read bitfield");
            let _ = net.read_message(&mut read_buf).await.expect("Failed to read unchoke");
            let _ = net.read_message(&mut read_buf).await.expect("Failed to read interested");

            net.write_message(PeerMessage::Request {
                index: 0,
                begin: 0,
                length: 16384,
            }).await
            .expect("Failed to send request");

            let response = net.read_message(&mut read_buf).await.expect("Failed to read piece response");
            match response {
                PeerMessage::Piece { index, begin, block } => {
                    assert_eq!(index, 0);
                    assert_eq!(begin, 0);
                    assert_eq!(block.len(), 16384);
                }
                other => panic!("Expected Piece response, got {:?}", other),
            }
        });
    });

    let peer_details = PeerDetails {
        info_hash: expected_info_hash,
        peer_id: None,
        ip: "127.0.0.1".to_string(),
        port: addr.port(),
    };
    session
        .connect_and_download_peer(peer_details)
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
    assert!(session.context().lock().unwrap().files_to_download.len() >= 1);
    assert!(
        session
            .context()
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

#[test]
fn test_download_piece_from_peer() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_download_piece");
    cleanup_download_path(&download_path);

    let mut session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, false)
        .expect("Failed to create torrent session");

    let expected_info_hash = session.context().lock().unwrap().info_hash.clone();
    let expected_info_hash_clone = expected_info_hash.clone();

    {
        use sha1::Digest;
        let mut new_hashes = Vec::new();
        let num_pieces = session.context().lock().unwrap().number_of_pieces;
        let piece_length = session.context().lock().unwrap().piece_length as usize;
        let file_length = 351874;
        for i in 0..num_pieces {
            let current_piece_len = if i == num_pieces - 1 {
                file_length % piece_length
            } else {
                piece_length
            };
            let data = vec![0x55u8; current_piece_len];
            let hash = sha1::Sha1::digest(&data);
            new_hashes.extend_from_slice(&hash);
        }
        session.context().lock().unwrap().pieces_info_hash = new_hashes;
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener.local_addr().expect("Failed to get listener address");

    let server_handle = thread::spawn(move || {
        futures::executor::block_on(async {
            let (stream, _) = listener.accept().expect("Failed to accept connection");
            let socket = std::sync::Arc::new(bittorrent_rs::peer_network::TcpSocket::new(stream));
            let net = PeerNetwork::new(socket);
            let (remote_info_hash, _) = net.read_handshake().await.expect("Failed to read handshake");
            assert_eq!(remote_info_hash, expected_info_hash_clone);

            let local_peer_id = *b"-RS0001-000000000000";
            net.write_handshake(&expected_info_hash_clone, &local_peer_id).await
                .expect("Failed to write handshake");

            let mut read_buf = vec![0u8; 1024 * 16 + 2 * 4 + 1];
            let _ = net.read_message(&mut read_buf).await.expect("Failed to read bitfield");
            let _ = net.read_message(&mut read_buf).await.expect("Failed to read unchoke");
            let _ = net.read_message(&mut read_buf).await.expect("Failed to read interested");

            let bitfield_data = vec![0xFF, 0xFF, 0xFC];
            net.write_message(PeerMessage::Bitfield(&bitfield_data)).await
                .expect("Failed to send bitfield");

            net.write_message(PeerMessage::Unchoke).await
                .expect("Failed to send unchoke");

            loop {
                let msg = match net.read_message(&mut read_buf).await {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Mock server read_message error: {:?}", e);
                        break;
                    }
                };
                match msg {
                    PeerMessage::Request { index, begin, length } => {
                        println!("Mock server received Request for piece {}, begin {}, length {}", index, begin, length);
                        let block = vec![0x55u8; length as usize];
                        if let Err(e) = net.write_message(PeerMessage::Piece { index, begin, block: &block }).await {
                            println!("Mock server failed to write Piece response: {:?}", e);
                            break;
                        }
                    }
                    PeerMessage::KeepAlive => {
                        println!("Mock server received KeepAlive");
                    }
                    other => {
                        println!("Mock server received other message: {:?}", other);
                    }
                }
            }
            println!("Mock server thread exiting");
        });
    });

    let peer_details = PeerDetails {
        info_hash: expected_info_hash,
        peer_id: None,
        ip: "127.0.0.1".to_string(),
        port: addr.port(),
    };

    session.start_download().expect("Failed to start download");

    session
        .connect_and_download_peer(peer_details)
        .expect("Failed to connect to peer");

    let finished = session.wait_for_download_finished(15000);
    assert!(finished, "Download did not finish in time");

    session.join_peer_workers();
    server_handle.join().expect("Server thread panicked");
    cleanup_download_path(&download_path);
}
