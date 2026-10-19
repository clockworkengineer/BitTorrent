use bittorrent_rs::{
    io_traits::MemStorage,
    network::webseed_tracker::{MirrorHealthProfile, WebSeedMirrorTracker},
    selector::SequentialSelector,
    session::config::SessionConfig,
    TorrentSession, TorrentStatus,
};
use bittorrent_rs::internals::TorrentContext;
use std::sync::Arc;
use std::time::Duration;

fn create_test_context_with_webseeds(web_seeds: Vec<String>) -> TorrentContext {
    let mut config = SessionConfig::default();
    config.enable_webseed_health_tracker = true;
    config.webseed_failure_threshold = 3;
    config.webseed_base_cooldown_secs = 5;
    config.webseed_max_cooldown_secs = 60;
    config.webseed_timeout_secs = 10;

    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xCC; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    ctx.init_pieces(4, 16384);
    ctx.total_bytes_to_download = 4 * 16384;
    ctx.storage = Arc::new(MemStorage::new(4 * 16384));
    ctx.status = TorrentStatus::Downloading;

    for ws in web_seeds {
        ctx.add_webseed(ws);
    }

    ctx
}

#[test]
fn test_mirror_health_profile_smoothing_and_scoring() {
    let mut profile = MirrorHealthProfile::new("http://mirror-alpha.example.com");
    assert_eq!(profile.endpoint_url, "http://mirror-alpha.example.com");
    assert_eq!(profile.total_requests, 0);

    // Initial optimistic baseline score
    let initial_score = profile.health_score();
    assert!(initial_score > 50.0);

    // Record fast successful responses
    profile.record_success(Duration::from_millis(50), 16384);
    assert_eq!(profile.total_requests, 1);
    assert_eq!(profile.successful_requests, 1);
    assert_eq!(profile.consecutive_failures, 0);
    assert_eq!(profile.smoothed_latency, Duration::from_millis(50));

    // Record second sample with RFC 6298 EMA smoothing
    profile.record_success(Duration::from_millis(70), 16384);
    assert_eq!(profile.successful_requests, 2);
    assert!(profile.smoothed_latency > Duration::from_millis(50));
    assert!(profile.smoothed_latency < Duration::from_millis(70));

    // Health score should remain high for low-latency reliable mirror
    let score = profile.health_score();
    assert!(score > 80.0);

    let summary = profile.summary();
    assert_eq!(summary.endpoint_url, "http://mirror-alpha.example.com");
    assert_eq!(summary.total_requests, 2);
    assert_eq!(summary.successful_requests, 2);
    assert_eq!(summary.failed_requests, 0);
    assert!(!summary.is_penalized);
}

#[test]
fn test_mirror_circuit_breaker_penalty_and_recovery() {
    let mut profile = MirrorHealthProfile::new("http://mirror-faulty.example.com");
    let failure_threshold = 3;
    let base_cooldown_secs = 10;
    let max_cooldown_secs = 120;

    // First and second failures: increments count but below threshold
    profile.record_failure(failure_threshold, base_cooldown_secs, max_cooldown_secs);
    assert_eq!(profile.consecutive_failures, 1);
    assert!(!profile.is_penalized());

    profile.record_failure(failure_threshold, base_cooldown_secs, max_cooldown_secs);
    assert_eq!(profile.consecutive_failures, 2);
    assert!(!profile.is_penalized());

    // Third failure: hits threshold -> triggers circuit breaker penalty
    profile.record_failure(failure_threshold, base_cooldown_secs, max_cooldown_secs);
    assert_eq!(profile.consecutive_failures, 3);
    assert!(profile.is_penalized());
    assert_eq!(profile.penalty_count, 1);
    assert_eq!(profile.health_score(), 0.0);

    // Further failure: increases backoff cooldown
    profile.record_failure(failure_threshold, base_cooldown_secs, max_cooldown_secs);
    assert_eq!(profile.consecutive_failures, 4);
    assert!(profile.is_penalized());
    assert_eq!(profile.penalty_count, 2);

    // Successful probe: recovers mirror, clears penalty and resets consecutive failures
    profile.record_success(Duration::from_millis(120), 16384);
    assert!(!profile.is_penalized());
    assert_eq!(profile.consecutive_failures, 0);
    assert!(profile.health_score() > 0.0);
}

#[test]
fn test_webseed_mirror_tracker_ranking_and_probing() {
    let mut tracker = WebSeedMirrorTracker::new(2, 5, 60);

    tracker.register_mirror("http://fast.mirror.org");
    tracker.register_mirror("http://slow.mirror.org");
    tracker.register_mirror("http://dead.mirror.org");

    // Fast mirror: low latency successes
    for _ in 0..5 {
        tracker.record_success("http://fast.mirror.org", Duration::from_millis(30), 16384);
    }

    // Slow mirror: high latency successes
    for _ in 0..5 {
        tracker.record_success("http://slow.mirror.org", Duration::from_millis(350), 16384);
    }

    // Dead mirror: 2 consecutive failures -> circuit breaker penalizes it
    tracker.record_failure("http://dead.mirror.org");
    tracker.record_failure("http://dead.mirror.org");

    // Ranked candidate mirrors must place fast mirror first, slow mirror second, and exclude dead mirror
    let ranked = tracker.select_ranked_mirrors();
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0], "http://fast.mirror.org");
    assert_eq!(ranked[1], "http://slow.mirror.org");

    // Penalize remaining mirrors
    tracker.record_failure("http://fast.mirror.org");
    tracker.record_failure("http://fast.mirror.org");
    tracker.record_failure("http://slow.mirror.org");
    tracker.record_failure("http://slow.mirror.org");

    // When all mirrors are penalized, tracker probes earliest expiring cooldown
    let probe_ranked = tracker.select_ranked_mirrors();
    assert!(!probe_ranked.is_empty());

    let summaries = tracker.summaries();
    assert_eq!(summaries.len(), 3);
}

#[test]
fn test_webseed_dynamic_runtime_add_and_remove() {
    let mut tracker = WebSeedMirrorTracker::default();
    assert_eq!(tracker.mirror_count(), 0);

    tracker.register_mirror("http://mirror-1.com");
    tracker.register_mirror("http://mirror-2.com");
    assert_eq!(tracker.mirror_count(), 2);

    assert!(tracker.remove_mirror("http://mirror-1.com"));
    assert_eq!(tracker.mirror_count(), 1);
    assert!(!tracker.remove_mirror("http://nonexistent.com"));
}

#[test]
fn test_context_and_session_webseed_integration() {
    let initial_seeds = vec![
        "http://seed-alpha.net".to_string(),
        "http://seed-beta.net".to_string(),
    ];
    let mut ctx = create_test_context_with_webseeds(initial_seeds);

    // Initial mirrors registered in context
    let summaries1 = ctx.webseed_mirror_summary();
    assert_eq!(summaries1.len(), 2);

    // Add new WebSeed dynamically
    ctx.add_webseed("http://seed-gamma.net");
    assert_eq!(ctx.web_seeds.len(), 3);
    let summaries2 = ctx.webseed_mirror_summary();
    assert_eq!(summaries2.len(), 3);
    assert!(summaries2.iter().any(|s| s.endpoint_url == "http://seed-gamma.net"));

    // Remove WebSeed dynamically
    assert!(ctx.remove_webseed("http://seed-beta.net"));
    assert_eq!(ctx.web_seeds.len(), 2);
    let summaries3 = ctx.webseed_mirror_summary();
    assert_eq!(summaries3.len(), 2);
    assert!(!summaries3.iter().any(|s| s.endpoint_url == "http://seed-beta.net"));
}

#[test]
fn test_session_builder_webseed_configuration() {
    let session = TorrentSession::builder("dummy.torrent", std::env::temp_dir())
        .enable_webseed_health_tracker(true)
        .webseed_failure_threshold(4)
        .webseed_cooldowns(15, 180)
        .webseed_timeout(20);

    let cfg = session.get_config();
    assert!(cfg.enable_webseed_health_tracker);
    assert_eq!(cfg.webseed_failure_threshold, 4);
    assert_eq!(cfg.webseed_base_cooldown_secs, 15);
    assert_eq!(cfg.webseed_max_cooldown_secs, 180);
    assert_eq!(cfg.webseed_timeout_secs, 20);
}
