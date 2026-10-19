use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::{
    FileDetails, FilePriority, MockSocket, Peer, PieceSelector, RarestFirstSelector,
    SequentialSelector, SessionConfig, Socket, StreamingPieceSelector, TorrentStatus,
};
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

fn create_test_context(
    files: Vec<FileDetails>,
    piece_length: u32,
    total_bytes: u64,
) -> TorrentContext {
    let config = SessionConfig::default();
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xAA; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    let num_pieces = ((total_bytes + piece_length as u64 - 1) / piece_length as u64) as usize;
    ctx.init_pieces(num_pieces, piece_length);
    ctx.total_bytes_to_download = total_bytes;
    ctx.files_to_download = files;

    // Mark pieces missing
    for p in 0..num_pieces as u32 {
        ctx.mark_piece_missing(p, true);
    }

    ctx
}

fn create_mock_peer(context: Arc<Mutex<TorrentContext>>, num_pieces: usize) -> Peer {
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket);
    peer.set_torrent_context(context);
    peer.connected = true;
    peer.remote_piece_bitfield = vec![0xFFu8; (num_pieces + 7) / 8];
    peer
}

#[test]
fn test_piece_priority_calculation_multi_file() {
    // 3 files in a 64 KB torrent with 16 KB piece size
    let files = vec![
        FileDetails {
            name: "file0.dat".to_string(),
            torrent_path: "file0.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Skip,
        },
        FileDetails {
            name: "file1.dat".to_string(),
            torrent_path: "file1.dat".to_string(),
            length: 32768,
            md5sum: None,
            offset: 16384,
            pieces_root: None,
            priority: FilePriority::High,
        },
        FileDetails {
            name: "file2.dat".to_string(),
            torrent_path: "file2.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 49152,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
    ];

    let ctx = create_test_context(files, 16384, 65536);

    // Piece 0: spans [0, 16384) -> File 0 (Skip)
    assert_eq!(ctx.get_piece_priority(0), FilePriority::Skip);
    assert!(!ctx.is_piece_wanted(0));

    // Piece 1: spans [16384, 32768) -> File 1 (High)
    assert_eq!(ctx.get_piece_priority(1), FilePriority::High);
    assert!(ctx.is_piece_wanted(1));

    // Piece 2: spans [32768, 49152) -> File 1 (High)
    assert_eq!(ctx.get_piece_priority(2), FilePriority::High);
    assert!(ctx.is_piece_wanted(2));

    // Piece 3: spans [49152, 65536) -> File 2 (Normal)
    assert_eq!(ctx.get_piece_priority(3), FilePriority::Normal);
    assert!(ctx.is_piece_wanted(3));
}

#[test]
fn test_boundary_piece_priority_sharing() {
    // Piece length: 16384 (16 KB)
    // File 0: 0..10000 (Skip)
    // File 1: 10000..32768 (Low)
    // Piece 0 spans [0, 16384), overlapping File 0 (Skip) and File 1 (Low).
    // Because File 1 is Low (wanted), Piece 0 must NOT be skipped!
    let files = vec![
        FileDetails {
            name: "skipped.dat".to_string(),
            torrent_path: "skipped.dat".to_string(),
            length: 10000,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Skip,
        },
        FileDetails {
            name: "wanted.dat".to_string(),
            torrent_path: "wanted.dat".to_string(),
            length: 22768,
            md5sum: None,
            offset: 10000,
            pieces_root: None,
            priority: FilePriority::Low,
        },
    ];

    let mut ctx = create_test_context(files, 16384, 32768);

    // Piece 0 overlaps both Skip and Low -> max priority is Low
    assert_eq!(ctx.get_piece_priority(0), FilePriority::Low);
    assert!(ctx.is_piece_wanted(0));

    // Piece 1 overlaps only File 1 (Low)
    assert_eq!(ctx.get_piece_priority(1), FilePriority::Low);
    assert!(ctx.is_piece_wanted(1));

    // Now set File 1 to Skip as well
    ctx.set_file_priority(1, FilePriority::Skip).unwrap();

    // Both files are now Skip, so Piece 0 and Piece 1 must both be Skip
    assert_eq!(ctx.get_piece_priority(0), FilePriority::Skip);
    assert!(!ctx.is_piece_wanted(0));
    assert_eq!(ctx.get_piece_priority(1), FilePriority::Skip);
    assert!(!ctx.is_piece_wanted(1));
}

#[test]
fn test_piece_selectors_skip_unwanted_pieces() {
    let files = vec![
        FileDetails {
            name: "f0.dat".to_string(),
            torrent_path: "f0.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Skip,
        },
        FileDetails {
            name: "f1.dat".to_string(),
            torrent_path: "f1.dat".to_string(),
            length: 49152,
            md5sum: None,
            offset: 16384,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
    ];

    let ctx = create_test_context(files, 16384, 65536);
    let ctx_arc = Arc::new(Mutex::new(ctx));
    let peer = create_mock_peer(ctx_arc.clone(), 4);
    let ctx_guard = ctx_arc.lock().unwrap();

    // 1. SequentialSelector: should skip piece 0 and select piece 1
    let seq_selector = SequentialSelector;
    let selected = seq_selector.select_piece(&ctx_guard, &peer);
    assert_eq!(selected, Some(1));

    // 2. StreamingPieceSelector with cursor at 0: should skip piece 0 and select piece 1
    let cursor = Arc::new(AtomicU32::new(0));
    let streaming_selector = StreamingPieceSelector::new(cursor, 4);
    let selected = streaming_selector.select_piece(&ctx_guard, &peer);
    assert_eq!(selected, Some(1));

    // 3. RarestFirstSelector: piece 0 has 1 peer (skipped), piece 1 has 2 peers, pieces 2 & 3 have 5 peers
    drop(ctx_guard);
    {
        let mut ctx_mut = ctx_arc.lock().unwrap();
        ctx_mut.increment_peer_count(0); // count = 1 (rarest, but skipped)
        for _ in 0..2 {
            ctx_mut.increment_peer_count(1); // count = 2 (rarest wanted piece)
        }
        for _ in 0..5 {
            ctx_mut.increment_peer_count(2); // count = 5
            ctx_mut.increment_peer_count(3); // count = 5
        }
    }
    let ctx_guard = ctx_arc.lock().unwrap();
    let rarest_selector = RarestFirstSelector;
    let selected = rarest_selector.select_piece(&ctx_guard, &peer);
    assert_eq!(selected, Some(1));
}

#[test]
fn test_next_block_request_for_peer_high_priority_first() {
    // 3 pieces: Piece 0 is Normal, Piece 1 is High, Piece 2 is Skip
    let files = vec![
        FileDetails {
            name: "normal.dat".to_string(),
            torrent_path: "normal.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
        FileDetails {
            name: "high.dat".to_string(),
            torrent_path: "high.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 16384,
            pieces_root: None,
            priority: FilePriority::High,
        },
        FileDetails {
            name: "skip.dat".to_string(),
            torrent_path: "skip.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 32768,
            pieces_root: None,
            priority: FilePriority::Skip,
        },
    ];

    let mut ctx = create_test_context(files, 16384, 49152);
    ctx.status = TorrentStatus::Downloading;

    let ctx_arc = Arc::new(Mutex::new(ctx));
    let peer = create_mock_peer(ctx_arc.clone(), 3);
    let mut ctx_mut = ctx_arc.lock().unwrap();

    // next_block_request_for_peer should prioritize High (Piece 1) over Normal (Piece 0)
    let request = ctx_mut.next_block_request_for_peer(&peer);
    assert!(request.is_some());
    let (piece, begin, length) = request.unwrap();
    assert_eq!(
        piece, 1,
        "Expected block request from High priority piece 1"
    );
    assert_eq!(begin, 0);
    assert_eq!(length, 16384);
}

#[test]
fn test_dynamic_set_file_priority_and_clearing() {
    let files = vec![
        FileDetails {
            name: "f0.dat".to_string(),
            torrent_path: "f0.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
        FileDetails {
            name: "f1.dat".to_string(),
            torrent_path: "f1.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 16384,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
    ];

    let mut ctx = create_test_context(files, 16384, 32768);
    assert!(ctx.is_piece_wanted(0));

    // Reserve a block for piece 0
    assert!(ctx.reserve_block_request(0, 0));

    // Now set file 0 to Skip
    ctx.set_file_priority(0, FilePriority::Skip).unwrap();
    assert_eq!(ctx.get_piece_priority(0), FilePriority::Skip);
    assert!(!ctx.is_piece_wanted(0));

    // The block reservation for piece 0 should have been cleared
    // so reserving it again succeeds if desired
    assert!(ctx.reserve_block_request(0, 0));

    // Set file 0 back to High
    ctx.set_file_priority(0, FilePriority::High).unwrap();
    assert_eq!(ctx.get_piece_priority(0), FilePriority::High);
    assert!(ctx.is_piece_wanted(0));

    // Verify out-of-bounds error
    assert!(ctx.set_file_priority(999, FilePriority::Normal).is_err());
}

#[test]
fn test_selective_download_completion_check() {
    let files = vec![
        FileDetails {
            name: "skipped.dat".to_string(),
            torrent_path: "skipped.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Skip,
        },
        FileDetails {
            name: "wanted.dat".to_string(),
            torrent_path: "wanted.dat".to_string(),
            length: 16384,
            md5sum: None,
            offset: 16384,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
    ];

    let mut ctx = create_test_context(files, 16384, 32768);
    ctx.status = TorrentStatus::Downloading;

    // Initially neither piece is local -> not complete
    assert!(!ctx.is_download_complete());

    // Mark ONLY piece 1 (the wanted piece) as local
    ctx.mark_piece_local(1, true);
    ctx.mark_piece_missing(1, false);

    // Piece 0 is still missing, but it is Skipped. Download should be complete!
    assert!(ctx.is_download_complete());

    // try_complete_download should transition to Seeding
    ctx.try_complete_download();
    assert_eq!(ctx.status, TorrentStatus::Seeding);
    assert!(ctx.download_finished.wait_one(0));
}
