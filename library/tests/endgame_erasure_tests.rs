use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::peer::Peer;
use bittorrent_rs::{
    TorrentSession, TorrentStatus,
    io_traits::{MemStorage, MockSocket, Socket},
    network::latency_profiler::{PeerLatencyProfile, SwarmLatencyProfiler},
    selector::SequentialSelector,
    session::config::SessionConfig,
    storage::erasure_staging::{AdaptiveErasureStaging, BlockArrivalOutcome},
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn create_test_context(num_pieces: usize, piece_length: u32) -> TorrentContext {
    let mut config = SessionConfig::default();
    config.enable_adaptive_endgame = true;
    config.endgame_straggler_ratio = 1.5;
    config.endgame_max_clones_per_block = 2;

    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xAA; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    ctx.init_pieces(num_pieces, piece_length);
    ctx.total_bytes_to_download = num_pieces as u64 * piece_length as u64;
    ctx.storage = Arc::new(MemStorage::new(num_pieces * piece_length as usize));
    ctx.status = TorrentStatus::Downloading;
    ctx
}

fn create_mock_peer(ip: &str, context: Arc<Mutex<TorrentContext>>) -> Peer {
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket(ip.to_string(), 6881, socket);
    peer.set_torrent_context(context);
    peer.connected = true;
    peer.peer_choking.set(); // Unchoked
    peer.remote_piece_bitfield = vec![0xFF; 10]; // Has all pieces
    peer
}

#[test]
fn test_peer_latency_profile_smoothing_and_ect() {
    let mut profile = PeerLatencyProfile::new("192.168.1.10");
    assert_eq!(profile.peer_ip, "192.168.1.10");
    assert_eq!(profile.delivered_blocks, 0);

    // Initial RTT sample: sets smoothed RTT directly
    profile.update_sample(Duration::from_millis(80), 16384);
    assert_eq!(profile.delivered_blocks, 1);
    assert_eq!(profile.smoothed_rtt, Duration::from_millis(80));
    assert_eq!(profile.rtt_variance, Duration::from_millis(40));

    // Second RTT sample: applies RFC 6298 smoothing
    profile.update_sample(Duration::from_millis(100), 16384);
    assert_eq!(profile.delivered_blocks, 2);
    assert!(profile.smoothed_rtt > Duration::from_millis(80));
    assert!(profile.smoothed_rtt < Duration::from_millis(100));

    // ECT should factor in smoothed RTT + transmission delay
    let ect = profile.estimated_completion_time(16384);
    assert!(ect >= profile.smoothed_rtt);

    // Record timeout: penalizes smoothed RTT and rolls download rate down
    let srtt_before = profile.smoothed_rtt;
    profile.record_timeout();
    assert_eq!(profile.timed_out_blocks, 1);
    assert!(profile.smoothed_rtt > srtt_before);
}

#[test]
fn test_swarm_latency_profiler_median_and_ranking() {
    let mut profiler = SwarmLatencyProfiler::new();

    // Fast peer: 20ms RTT, high delivery
    for _ in 0..5 {
        profiler.record_sample("10.0.0.1", Duration::from_millis(20), 16384);
    }

    // Medium peer: 70ms RTT
    for _ in 0..5 {
        profiler.record_sample("10.0.0.2", Duration::from_millis(70), 16384);
    }

    // Slow peer: 250ms RTT
    for _ in 0..5 {
        profiler.record_sample("10.0.0.3", Duration::from_millis(250), 16384);
    }

    let median = profiler.median_rtt();
    assert!(median >= Duration::from_millis(60) && median <= Duration::from_millis(80));

    let candidates = ["10.0.0.3", "10.0.0.1", "10.0.0.2"];
    let ranked = profiler.rank_peers_by_ect(&candidates, 16384);
    assert_eq!(ranked[0].0, "10.0.0.1");
    assert_eq!(ranked[1].0, "10.0.0.2");
    assert_eq!(ranked[2].0, "10.0.0.3");

    let summary = profiler.summary(1.8);
    assert_eq!(summary.len(), 3);
    let slow = summary.iter().find(|s| s.peer_ip == "10.0.0.3").unwrap();
    assert!(slow.is_straggler);
    let fast = summary.iter().find(|s| s.peer_ip == "10.0.0.1").unwrap();
    assert!(!fast.is_straggler);
}

#[test]
fn test_adaptive_erasure_staging_cloning_and_absorption() {
    let mut staging = AdaptiveErasureStaging::new(2, 1.8);

    // Initially can clone
    assert!(staging.can_clone_block(0, 0));

    // Stage primary request to peer A
    staging.stage_request(0, 0, "10.0.0.1", Duration::from_millis(150));
    assert!(staging.can_clone_block(0, 0));

    // Stage redundant clone to peer B (hits max_clones_per_block = 2)
    staging.stage_request(0, 0, "10.0.0.2", Duration::from_millis(80));
    assert!(!staging.can_clone_block(0, 0));

    // First arrival from peer B
    let outcome1 = staging.on_block_arrived(0, 0, "10.0.0.2");
    match outcome1 {
        BlockArrivalOutcome::CommitAndCancelOthers { peers_to_cancel } => {
            assert_eq!(peers_to_cancel, vec!["10.0.0.1"]);
        }
        _ => panic!("Expected CommitAndCancelOthers"),
    }

    // Block is now committed, no further cloning allowed
    assert!(!staging.can_clone_block(0, 0));

    // Late redundant duplicate arrival from peer A
    let outcome2 = staging.on_block_arrived(0, 0, "10.0.0.1");
    match outcome2 {
        BlockArrivalOutcome::AbsorbDuplicate { source_peer } => {
            assert_eq!(source_peer, "10.0.0.1");
        }
        _ => panic!("Expected AbsorbDuplicate"),
    }

    let metrics = staging.metrics();
    assert_eq!(metrics.total_staged_blocks, 1);
    assert_eq!(metrics.committed_blocks, 1);
    assert_eq!(metrics.absorbed_duplicate_blocks, 1);
    assert_eq!(metrics.cancelled_redundant_requests, 1);
    assert_eq!(metrics.active_staged_blocks, 0);
}

#[test]
fn test_straggler_detection_and_preemption() {
    let mut staging = AdaptiveErasureStaging::new(2, 1.2);

    // Stage request with very low ECT
    staging.stage_request(1, 0, "192.168.1.50", Duration::from_millis(10));

    // Sleep briefly so elapsed time exceeds stall_threshold (min clamped 400ms)
    std::thread::sleep(Duration::from_millis(450));

    let stalled = staging.detect_stalled_blocks();
    assert_eq!(stalled.len(), 1);
    assert_eq!(stalled[0], (1, 0, "192.168.1.50".to_string()));

    // Stalled block is now marked preempted, second detection returns empty
    let stalled_again = staging.detect_stalled_blocks();
    assert!(stalled_again.is_empty());

    // When block arrives, verify estimated stall time saved
    let outcome = staging.on_block_arrived(1, 0, "192.168.1.99");
    assert!(matches!(
        outcome,
        BlockArrivalOutcome::CommitAndCancelOthers { .. }
    ));

    let metrics = staging.metrics();
    assert_eq!(metrics.preempted_stragglers, 1);
    assert!(metrics.estimated_stall_time_saved_ms > 0);
}

#[test]
fn test_torrent_context_endgame_preemption_and_erasure() {
    // 1 piece with 2 blocks (32 KiB) -> immediately in endgame mode
    let context = Arc::new(Mutex::new(create_test_context(1, 32768)));
    assert!(context.lock().unwrap().is_endgame());

    let mut peer_slow = create_mock_peer("10.0.0.10", context.clone());
    let peer_fast = create_mock_peer("10.0.0.20", context.clone());

    // Peer slow requests block 0
    let req1 = context
        .lock()
        .unwrap()
        .next_block_request_for_peer(&peer_slow);
    assert_eq!(req1, Some((0, 0, 16384)));
    peer_slow
        .reserved_blocks
        .push((0, 0, std::time::Instant::now()));

    // Verify block 0 is staged
    {
        let ctx = context.lock().unwrap();
        let metrics = ctx.endgame_staging_metrics();
        assert_eq!(metrics.active_staged_blocks, 1);
    }

    // Simulate stall: sleep beyond stall threshold (> 525ms)
    std::thread::sleep(Duration::from_millis(600));

    // Peer fast requests next block: should preempt stalled block 0 from slow peer!
    let req2 = context
        .lock()
        .unwrap()
        .next_block_request_for_peer(&peer_fast);
    assert_eq!(req2, Some((0, 0, 16384)));

    // Fast peer delivers block 0
    let block_data = vec![0x42; 16384];
    let storage = { context.lock().unwrap().storage.clone() };

    let completed = context
        .lock()
        .unwrap()
        .process_piece_block(&*storage, 0, 0, &block_data, "10.0.0.20")
        .expect("write failed");
    assert!(!completed); // Piece not complete yet, needs block 1

    // Slow peer now delivers late duplicate block 0
    let duplicate_completed = context
        .lock()
        .unwrap()
        .process_piece_block(&*storage, 0, 0, &block_data, "10.0.0.10")
        .expect("duplicate processing failed");
    // Redundant duplicate was absorbed without disk re-write!
    assert!(!duplicate_completed);

    let metrics = context.lock().unwrap().endgame_staging_metrics();
    assert_eq!(metrics.preempted_stragglers, 1);
    assert_eq!(metrics.absorbed_duplicate_blocks, 1);
}

#[test]
fn test_session_builder_and_telemetry_apis() {
    let session = TorrentSession::builder("dummy.torrent", std::env::temp_dir())
        .enable_adaptive_endgame(true)
        .endgame_straggler_ratio(2.0)
        .endgame_max_clones_per_block(3);

    assert!(session.get_config().enable_adaptive_endgame);
    assert_eq!(session.get_config().endgame_straggler_ratio, 2.0);
    assert_eq!(session.get_config().endgame_max_clones_per_block, 3);
}
