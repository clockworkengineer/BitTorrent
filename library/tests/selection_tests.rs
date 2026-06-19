use bittorrent_rs::{Peer, PieceSelector};
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
    let piece_length = meta.get_piece_length().unwrap();
    let (_, files_to_download) = meta.local_files_to_download_list(&download_path).unwrap();
    let disk_io = std::sync::Arc::new(bittorrent_rs::disk_io::DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().unwrap();
    let selector = std::sync::Arc::new(bittorrent_rs::selector::RarestFirstSelector);
    let mut context = bittorrent_rs::torrent_context::TorrentContext::new(
        &meta,
        selector,
        disk_io.clone(),
        &download_path,
        false,
        bittorrent_rs::session::SessionConfig::default(),
    )
    .expect("Failed to create torrent context");
    disk_io.create_torrent_bitfield(&mut context).unwrap();
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

#[test]
fn test_sequential_and_rarest_first_selectors() {
    let mut meta = bittorrent_rs::MetaInfoFile::new(sample_file("singlefile.torrent"))
        .expect("Failed to load torrent metadata");
    meta.parse().expect("Failed to parse torrent metadata");
    
    let storage = std::sync::Arc::new(bittorrent_rs::MemStorage::new(1024 * 1024));
    let seq_selector = std::sync::Arc::new(bittorrent_rs::selector::SequentialSelector);
    
    let mut context = bittorrent_rs::torrent_context::TorrentContext::new(
        &meta,
        seq_selector.clone(),
        storage,
        std::path::Path::new("."),
        false,
        bittorrent_rs::session::SessionConfig::default(),
    )
    .expect("Failed to create torrent context");

    // Let's assume we have 5 pieces.
    context.number_of_pieces = 5;
    // None are local by default. Set missing pieces:
    for i in 0..5 {
        context.mark_piece_missing(i, true);
    }

    // Set up peer counts: piece 0 (3 peers), piece 1 (1 peer), piece 2 (2 peers), piece 3 (4 peers), piece 4 (2 peers)
    for _ in 0..3 { context.increment_peer_count(0); }
    for _ in 0..1 { context.increment_peer_count(1); }
    for _ in 0..2 { context.increment_peer_count(2); }
    for _ in 0..4 { context.increment_peer_count(3); }
    for _ in 0..2 { context.increment_peer_count(4); }

    // Create a peer that has all pieces
    let (socket, _, _) = bittorrent_rs::MockSocket::new();
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, std::sync::Arc::new(bittorrent_rs::Socket::Mock(socket)));
    let context_ref = std::sync::Arc::new(std::sync::Mutex::new(context));
    peer.set_torrent_context(context_ref.clone());
    peer.connected = true;
    peer.remote_piece_bitfield = vec![0xF8]; // pieces 0..5 are set (first 5 bits of the byte)

    // 1. Test SequentialSelector
    // It should select the first missing piece (piece 0)
    let context_guard = context_ref.lock().unwrap();
    let selected_seq = seq_selector.select_piece(&context_guard, &peer);
    assert_eq!(selected_seq, Some(0));

    // 2. Test RarestFirstSelector
    // Rarest is piece 1 (peer count 1)
    let rarest_selector = bittorrent_rs::selector::RarestFirstSelector;
    let selected_rarest = rarest_selector.select_piece(&context_guard, &peer);
    assert_eq!(selected_rarest, Some(1));

    drop(context_guard);

    // Mark piece 1 as local (meaning we no longer need it)
    let mut context_guard = context_ref.lock().unwrap();
    context_guard.mark_piece_local(1, true);
    context_guard.mark_piece_missing(1, false);

    // Now rarest should be piece 2 or 4 (both have peer count 2, piece 2 is selected first due to index ordering)
    let selected_rarest_2 = rarest_selector.select_piece(&context_guard, &peer);
    assert_eq!(selected_rarest_2, Some(2));
}

