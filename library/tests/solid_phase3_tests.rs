use bittorrent_rs::SessionConfig;
use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::io_traits::{MockSocket, Socket};
use bittorrent_rs::peer::Peer;
use bittorrent_rs::selector::{
    PieceSelector, RarestFirstSelector, SequentialSelector, StreamingPieceSelector,
};
use bittorrent_rs::views::{
    MockPieceAvailabilityView, PeerSnapshot, PieceAvailabilityView, RemotePeerPieceView, SwarmView,
    TransferStatsView,
};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

#[test]
fn test_sequential_selector_with_mock_view_and_raw_bitfield() {
    // 8 pieces: piece 0, 1 already local; piece 2 unwanted; piece 3 is first available missing wanted
    let mut mock = MockPieceAvailabilityView::new(8, 16384);
    mock.local_pieces[0] = true;
    mock.local_pieces[1] = true;
    mock.wanted_pieces[2] = false;

    // Remote peer has pieces 0, 1, 2, 3 (0b1111_0000)
    let remote_bitfield: &[u8] = &[0b1111_0000];

    let selector = SequentialSelector;
    let selected = selector.select_piece(&mock, &remote_bitfield);

    // Must select piece 3 without requiring TorrentContext or Peer
    assert_eq!(selected, Some(3));
}

#[test]
fn test_rarest_first_selector_with_mock_view_and_raw_bitfield() {
    // 5 pieces, all missing & wanted. Peer counts: piece 0 (5), piece 1 (2), piece 2 (1), piece 3 (8), piece 4 (3)
    let mut mock = MockPieceAvailabilityView::new(5, 16384);
    mock.peer_counts[0] = 5;
    mock.peer_counts[1] = 2;
    mock.peer_counts[2] = 1; // Rarest overall
    mock.peer_counts[3] = 8;
    mock.peer_counts[4] = 3;

    let selector = RarestFirstSelector;

    // Remote peer A has pieces 0, 1, 2 (0b1110_0000) -> should select rarest piece 2
    let remote_a: &[u8] = &[0b1110_0000];
    assert_eq!(selector.select_piece(&mock, &remote_a), Some(2));

    // Remote peer B does NOT have piece 2, but has pieces 0, 1, 4 (0b1100_1000) -> should select next rarest piece 1 (count 2)
    let remote_b: &[u8] = &[0b1100_1000];
    assert_eq!(selector.select_piece(&mock, &remote_b), Some(1));
}

#[test]
fn test_streaming_selector_with_mock_view_and_raw_bitfield() {
    let mock = MockPieceAvailabilityView::new(10, 16384);
    let cursor = Arc::new(AtomicU32::new(4));
    let selector = StreamingPieceSelector::new(cursor.clone(), 3); // Lookahead window [4..7)

    // Remote peer has all pieces
    let remote: &[u8] = &[0b1111_1111, 0b1100_0000];

    // Priority 1: cursor piece 4
    assert_eq!(selector.select_piece(&mock, &remote), Some(4));

    // Seek cursor to piece 8
    selector.set_cursor_piece(8);
    // Lookahead window is now [8..10), so piece 8 is selected first
    assert_eq!(selector.select_piece(&mock, &remote), Some(8));
}

#[test]
fn test_torrent_context_implements_swarm_view() {
    let selector = Arc::new(SequentialSelector);
    let temp_dir = std::env::temp_dir().join("solid_phase3_swarm_test");
    let config = SessionConfig::default();
    let ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xAA; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &temp_dir,
        config,
    )
    .expect("Bootstrap succeeds");

    let (mock_sock, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(mock_sock));
    let mut peer = Peer::new_with_socket("192.168.1.10".to_string(), 6881, socket);
    peer.rolling_download_rate = 102400.0;
    peer.rolling_upload_rate = 51200.0;
    peer.am_choking = false;
    peer.am_interested = true;
    let peer_arc = Arc::new(std::sync::Mutex::new(peer));

    ctx.add_peer(peer_arc);

    let swarm_view: &dyn SwarmView = &ctx;
    assert_eq!(swarm_view.connected_peers_count(), 1);

    let mut snapshots = Vec::new();
    swarm_view.for_each_peer(&mut |_ip, snapshot: &PeerSnapshot| {
        snapshots.push(snapshot.clone());
    });

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].ip, "192.168.1.10");
    assert_eq!(snapshots[0].port, 6881);
    assert!(!snapshots[0].am_choking);
    assert!(snapshots[0].am_interested);
    assert_eq!(snapshots[0].download_rate, 102400.0);
    assert_eq!(snapshots[0].upload_rate, 51200.0);
}

#[test]
fn test_torrent_context_implements_transfer_stats_view() {
    let selector = Arc::new(SequentialSelector);
    let temp_dir = std::env::temp_dir().join("solid_phase3_stats_test");
    let config = SessionConfig::default();
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xBB; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &temp_dir,
        config,
    )
    .expect("Bootstrap succeeds");
    ctx.init_pieces(4, 16384);
    ctx.total_bytes_to_download = 65536;
    ctx.transfer_metrics.set_total_bytes_to_download(65536);

    let stats_view: &dyn TransferStatsView = &ctx;
    assert_eq!(stats_view.total_downloaded(), 0);
    assert_eq!(stats_view.total_uploaded(), 0);
    assert_eq!(stats_view.progress_percent(), 0.0);

    // Verify view reads atomically updated progress
    ctx.total_bytes_downloaded
        .store(32768, std::sync::atomic::Ordering::Relaxed);
    ctx.transfer_metrics
        .total_bytes_downloaded
        .store(32768, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(stats_view.total_downloaded(), 32768);
    assert_eq!(stats_view.progress_percent(), 50.0);
}

/// Demonstrates Interface Segregation Principle:
/// Consumer function only accepts PieceAvailabilityView and knows nothing about TorrentContext,
/// sockets, tracker threads, or swarm state.
fn count_available_missing_pieces(
    view: &dyn PieceAvailabilityView,
    remote: &dyn RemotePeerPieceView,
) -> usize {
    let mut count = 0;
    for piece in 0..view.number_of_pieces() as u32 {
        if view.is_piece_wanted(piece)
            && !view.is_piece_local(piece)
            && remote.is_piece_on_remote_peer(piece)
        {
            count += 1;
        }
    }
    count
}

#[test]
fn test_interface_segregation_consumer() {
    let mut mock = MockPieceAvailabilityView::new(4, 16384);
    mock.local_pieces[0] = true; // Piece 0 local
    mock.wanted_pieces[3] = false; // Piece 3 unwanted

    // Remote peer has all 4 pieces (0b1111_0000)
    let remote: &[u8] = &[0b1111_0000];

    // Pieces 1 and 2 are wanted, missing, and remote has them -> 2 pieces
    let count = count_available_missing_pieces(&mock, &remote);
    assert_eq!(count, 2);
}
