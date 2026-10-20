use bittorrent_rs::core::piece_store::PieceStore;
use bittorrent_rs::core::swarm_topology::SwarmTopology;
use bittorrent_rs::core::transfer_metrics::TransferMetrics;
use bittorrent_rs::core::metadata_bootstrap::MetadataBootstrap;
use bittorrent_rs::core::torrent_context::TorrentContext;
use bittorrent_rs::core::selector::RarestFirstSelector;
use bittorrent_rs::session::SessionConfig;
use std::sync::Arc;

#[test]
fn test_piece_store_bitfield_and_rarity() {
    let mut store = PieceStore::new(16, 16384, vec![0u8; 16 * 20]);
    assert_eq!(store.number_of_pieces, 16);
    assert_eq!(store.piece_length, 16384);
    assert!(!store.is_piece_local(0));
    assert!(!store.is_download_complete());

    // Mark pieces local
    store.mark_piece_local(0, true);
    assert!(store.is_piece_local(0));
    assert!(!store.is_piece_local(1));

    // Mark missing
    store.mark_piece_missing(1, true);
    assert!(store.is_piece_missing(1));
    store.mark_piece_missing(1, false);
    assert!(!store.is_piece_missing(1));

    // Verify pieces verifying state
    assert!(!store.is_piece_verifying(2));
    store.set_piece_verifying(2, true);
    assert!(store.is_piece_verifying(2));
    store.set_piece_verifying(2, false);
    assert!(!store.is_piece_verifying(2));

    // Mark all complete
    for i in 0..16 {
        store.mark_piece_local(i, true);
    }
    assert!(store.is_download_complete());
}

#[test]
fn test_swarm_topology_peer_scoring_and_limits() {
    let topology = SwarmTopology::new(50);
    assert_eq!(topology.connected_count(), 0);
    assert_eq!(topology.maximum_swarm_size, 50);

    // Bad peer scores
    assert_eq!(topology.record_bad_peer("192.168.1.10"), 1);
    assert_eq!(topology.record_bad_peer("192.168.1.10"), 2);
    assert!(!topology.is_bad_peer("192.168.1.10", 3));
    assert_eq!(topology.record_bad_peer("192.168.1.10"), 3);
    assert!(topology.is_bad_peer("192.168.1.10", 3));

    // Clear
    topology.clear();
    assert_eq!(topology.connected_count(), 0);
    assert!(!topology.is_bad_peer("192.168.1.10", 3));
}

#[test]
fn test_transfer_metrics_progress_and_speeds() {
    let metrics = TransferMetrics::new(1_000_000);
    assert_eq!(metrics.progress_ppm(), 0);
    assert_eq!(metrics.progress_percent(), 0.0);
    assert_eq!(metrics.bytes_left_to_download().unwrap(), 1_000_000);

    metrics.record_download(500_000);
    assert_eq!(metrics.progress_ppm(), 5000);
    assert_eq!(metrics.progress_percent(), 50.0);
    assert_eq!(metrics.bytes_left_to_download().unwrap(), 500_000);

    metrics.record_upload(123_456);
    assert_eq!(
        metrics.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed),
        123_456
    );

    // Verify speed reporting doesn't panic
    let _ = metrics.bytes_per_second();
    let _ = metrics.upload_bytes_per_second();
}

#[test]
fn test_metadata_bootstrap_assembly() {
    let mut bootstrap = MetadataBootstrap::new();
    assert!(!bootstrap.is_complete());

    let chunk1 = vec![b'a'; 16384];
    let chunk2 = vec![b'b'; 1000];
    let total_size = chunk1.len() + chunk2.len();

    bootstrap.set_metadata_size(total_size);
    assert!(!bootstrap.add_piece(0, chunk1.clone()));
    assert!(!bootstrap.is_complete());

    assert!(bootstrap.add_piece(1, chunk2.clone()));
    assert!(bootstrap.is_complete());

    let assembled = bootstrap.assemble_info_dict().expect("assembly succeeds");
    assert_eq!(assembled.len(), total_size);
    assert_eq!(&assembled[..16384], &chunk1[..]);
    assert_eq!(&assembled[16384..], &chunk2[..]);
}

#[test]
fn test_torrent_context_domain_aggregates_integration() {
    let selector = Arc::new(RarestFirstSelector);
    let temp_dir = std::env::temp_dir().join("solid_phase1_test");
    let config = SessionConfig::default();

    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0u8; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &temp_dir,
        config,
    )
    .expect("bootstrap context creation succeeds");

    // Initialize 8 pieces
    ctx.init_pieces(8, 16384);
    assert_eq!(ctx.pieces().number_of_pieces, 8);
    assert_eq!(ctx.pieces().piece_length, 16384);

    // Mark piece local via delegation
    ctx.mark_piece_local(3, true);
    assert!(ctx.is_piece_local(3));
    assert!(ctx.pieces().is_piece_local(3));

    // Missing piece
    ctx.mark_piece_missing(2, true);
    assert!(ctx.is_piece_missing(2));
    assert!(ctx.pieces().is_piece_missing(2));

    // Verifying piece
    assert!(!ctx.is_piece_verifying(4));
    ctx.mark_piece_verifying(4, true);
    assert!(ctx.is_piece_verifying(4));
    assert!(ctx.pieces().is_piece_verifying(4));

    // Metrics access
    assert_eq!(ctx.metrics().progress_ppm(), ctx.progress_ppm());
    assert_eq!(ctx.metrics().bytes_per_second(), ctx.bytes_per_second());

    // Swarm access
    assert_eq!(ctx.swarm().connected_count(), 0);
}
