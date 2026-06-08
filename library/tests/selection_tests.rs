use bittorrent_rs::Peer;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::thread;

fn sample_file(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}

#[test]
fn test_next_block_request_from_peer_reserves_request() {
    let download_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("selection_test");
    let _ = std::fs::remove_dir_all(&download_path);
    let mut meta = bittorrent_rs::MetaInfoFile::new(sample_file("singlefile.torrent"))
        .expect("Failed to load torrent metadata");
    meta.parse().expect("Failed to parse torrent metadata");
    let disk_io = bittorrent_rs::disk_io::DiskIO::new();
    let selector = bittorrent_rs::selector::Selector::new();
    let mut context = bittorrent_rs::torrent_context::TorrentContext::new(
        &meta,
        selector,
        &disk_io,
        &download_path,
        false,
    )
    .expect("Failed to create torrent context");
    context.status = bittorrent_rs::torrent_context::TorrentStatus::Downloading;

    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind listener");
    let addr = listener
        .local_addr()
        .expect("Failed to get listener address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("Failed to accept connection");
        let mut buffer = [0u8; 17];
        stream
            .read_exact(&mut buffer)
            .expect("Failed to read request");
        assert_eq!(u32::from_be_bytes(buffer[0..4].try_into().unwrap()), 13);
        assert_eq!(buffer[4], 6);
    });

    let stream = TcpStream::connect(addr).expect("Failed to connect to listener");
    let mut peer = Peer::new("127.0.0.1".to_string(), addr.port(), stream);
    let context_ref = std::sync::Arc::new(std::sync::Mutex::new(context));
    peer.set_torrent_context(context_ref.clone());
    peer.connected = true;
    peer.remote_piece_bitfield = vec![0xFF; peer.remote_piece_bitfield.len()];

    let mut context_guard = context_ref.lock().unwrap();
    let request = context_guard
        .next_block_request_for_peer(&peer)
        .expect("Failed to compute next block request");
    assert_eq!(peer.outstanding_requests_count, 0);

    let (_, begin, length) = request;
    futures::executor::block_on(async {
        peer.send_request(0, begin, length).await
    }).expect("Failed to send request");
    peer.outstanding_requests_count = peer.outstanding_requests_count.saturating_add(1);

    assert_eq!(peer.outstanding_requests_count, 1);
    handle.join().expect("Listener thread failed");
    let _ = std::fs::remove_dir_all(&download_path);
}
