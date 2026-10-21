use bittorrent_rs::core::views::{MockSwarmView, PeerSnapshot};
use bittorrent_rs::session::choking_policy::{
    BitTyrantPolicy, ChokingPolicy, PolicyChokingStrategy, RoundRobinPolicy,
    StandardTitForTatPolicy,
};
use bittorrent_rs::session::config::SessionConfig;
use std::sync::Arc;

fn make_peer(
    ip: &str,
    port: u16,
    peer_interested: bool,
    download_rate: f64,
    upload_rate: f64,
) -> PeerSnapshot {
    PeerSnapshot {
        ip: ip.to_string(),
        port,
        am_choking: true,
        am_interested: false,
        peer_choking: true,
        peer_interested,
        download_rate,
        upload_rate,
    }
}

#[test]
fn test_standard_tit_for_tat_downloading_mode() {
    let policy = StandardTitForTatPolicy::new();

    let swarm = MockSwarmView::new()
        .with_peer(make_peer("10.0.0.1", 6881, true, 100.0, 10.0)) // A
        .with_peer(make_peer("10.0.0.2", 6881, true, 50.0, 10.0)) // B
        .with_peer(make_peer("10.0.0.3", 6881, true, 200.0, 10.0)) // C
        .with_peer(make_peer("10.0.0.4", 6881, true, 10.0, 10.0)) // D
        .with_peer(make_peer("10.0.0.5", 6881, false, 500.0, 10.0)); // E (disinterested)

    // With max_upload_slots = 3:
    // 2 regular unchoke slots (max_slots - 1)
    // 1 optimistic unchoke slot
    let decision = policy.evaluate(&swarm, 3, false);

    // Regular unchoked peers must be highest download speed: C (200.0) then A (100.0)
    assert_eq!(decision.unchoked_peers.len(), 2);
    assert_eq!(decision.unchoked_peers[0], "10.0.0.3");
    assert_eq!(decision.unchoked_peers[1], "10.0.0.1");

    // Optimistic unchoke peer must be either B or D
    let opt_peer = decision
        .optimistically_unchoked_peer
        .expect("should have optimistic unchoke");
    assert!(opt_peer == "10.0.0.2" || opt_peer == "10.0.0.4");

    // Disinterested peer E must be in choked peers
    assert!(decision.choked_peers.contains(&"10.0.0.5".to_string()));

    // The peer not chosen for optimistic unchoke among B & D must be choked
    let other_peer = if opt_peer == "10.0.0.2" {
        "10.0.0.4"
    } else {
        "10.0.0.2"
    };
    assert!(decision.choked_peers.contains(&other_peer.to_string()));
}

#[test]
fn test_standard_tit_for_tat_seeding_mode() {
    let policy = StandardTitForTatPolicy::new();

    let swarm = MockSwarmView::new()
        .with_peer(make_peer("10.0.0.1", 6881, true, 0.0, 10.0)) // A
        .with_peer(make_peer("10.0.0.2", 6881, true, 0.0, 100.0)) // B
        .with_peer(make_peer("10.0.0.3", 6881, true, 0.0, 50.0)) // C
        .with_peer(make_peer("10.0.0.4", 6881, true, 0.0, 20.0)); // D

    // In seeding mode, ranks peers by upload rate descending
    let decision = policy.evaluate(&swarm, 3, true);

    // Regular unchoke slots = 2: B (100.0), C (50.0)
    assert_eq!(decision.unchoked_peers.len(), 2);
    assert_eq!(decision.unchoked_peers[0], "10.0.0.2");
    assert_eq!(decision.unchoked_peers[1], "10.0.0.3");

    // Optimistic unchoke from remaining {A, D}
    let opt = decision
        .optimistically_unchoked_peer
        .expect("optimistic unchoke should exist");
    assert!(opt == "10.0.0.1" || opt == "10.0.0.4");
}

#[test]
fn test_standard_tit_for_tat_optimistic_rotation() {
    let policy = StandardTitForTatPolicy::new();

    let swarm = MockSwarmView::new()
        .with_peer(make_peer("10.0.0.1", 6881, true, 100.0, 10.0)) // Regular
        .with_peer(make_peer("10.0.0.2", 6881, true, 20.0, 10.0)) // Candidate 1
        .with_peer(make_peer("10.0.0.3", 6881, true, 10.0, 10.0)); // Candidate 2

    // Round 0 (round % 3 == 0): Initial pick
    let d0 = policy.evaluate(&swarm, 2, false);
    let opt0 = d0.optimistically_unchoked_peer.unwrap();

    // Round 1 (round % 3 == 1): Must keep same optimistic peer
    let d1 = policy.evaluate(&swarm, 2, false);
    assert_eq!(d1.optimistically_unchoked_peer.as_ref(), Some(&opt0));

    // Round 2 (round % 3 == 2): Must keep same optimistic peer
    let d2 = policy.evaluate(&swarm, 2, false);
    assert_eq!(d2.optimistically_unchoked_peer.as_ref(), Some(&opt0));

    // Round 3 (round % 3 == 0): Rotates to next optimistic peer
    let d3 = policy.evaluate(&swarm, 2, false);
    let opt3 = d3.optimistically_unchoked_peer.unwrap();
    let expected_other = if opt0 == "10.0.0.2" {
        "10.0.0.3"
    } else {
        "10.0.0.2"
    };
    assert_eq!(opt3, expected_other);
}

#[test]
fn test_round_robin_policy_fairness() {
    let policy = RoundRobinPolicy::new();

    let swarm = MockSwarmView::new()
        .with_peer(make_peer("10.0.0.1", 6881, true, 0.0, 0.0))
        .with_peer(make_peer("10.0.0.2", 6881, true, 0.0, 0.0))
        .with_peer(make_peer("10.0.0.3", 6881, true, 0.0, 0.0))
        .with_peer(make_peer("10.0.0.4", 6881, false, 0.0, 0.0)); // disinterested

    // Round 1 with max 2 upload slots
    let d1 = policy.evaluate(&swarm, 2, false);
    assert_eq!(d1.unchoked_peers.len(), 2);
    assert_eq!(d1.unchoked_peers, vec!["10.0.0.1", "10.0.0.2"]);
    assert!(d1.choked_peers.contains(&"10.0.0.3".to_string()));
    assert!(d1.choked_peers.contains(&"10.0.0.4".to_string()));

    // Round 2
    let d2 = policy.evaluate(&swarm, 2, false);
    assert_eq!(d2.unchoked_peers.len(), 2);
    assert_eq!(d2.unchoked_peers, vec!["10.0.0.3", "10.0.0.1"]);

    // Round 3
    let d3 = policy.evaluate(&swarm, 2, false);
    assert_eq!(d3.unchoked_peers.len(), 2);
    assert_eq!(d3.unchoked_peers, vec!["10.0.0.2", "10.0.0.3"]);
}

#[test]
fn test_bittyrant_policy_efficiency_ratio() {
    let policy = BitTyrantPolicy::new();

    // Ratio = download_rate / upload_rate.max(1.0)
    // Peer A: 100 dl / 50 ul = 2.0
    // Peer B: 80 dl / 10 ul  = 8.0  (highest efficiency!)
    // Peer C: 300 dl / 200 ul = 1.5 (lowest efficiency despite high dl)
    let swarm = MockSwarmView::new()
        .with_peer(make_peer("10.0.0.1", 6881, true, 100.0, 50.0)) // ratio 2.0
        .with_peer(make_peer("10.0.0.2", 6881, true, 80.0, 10.0)) // ratio 8.0
        .with_peer(make_peer("10.0.0.3", 6881, true, 300.0, 200.0)); // ratio 1.5

    let decision = policy.evaluate(&swarm, 2, false);
    assert_eq!(decision.unchoked_peers.len(), 2);
    // B has highest ratio (8.0), followed by A (2.0)
    assert_eq!(decision.unchoked_peers[0], "10.0.0.2");
    assert_eq!(decision.unchoked_peers[1], "10.0.0.1");

    // C must be choked
    assert_eq!(decision.choked_peers, vec!["10.0.0.3"]);
}

#[test]
fn test_empty_and_disinterested_swarms() {
    let policy = StandardTitForTatPolicy::new();

    // Completely empty swarm
    let empty_swarm = MockSwarmView::new();
    let d_empty = policy.evaluate(&empty_swarm, 4, false);
    assert!(d_empty.unchoked_peers.is_empty());
    assert!(d_empty.optimistically_unchoked_peer.is_none());
    assert!(d_empty.choked_peers.is_empty());

    // Swarm with only disinterested peers
    let no_interest_swarm = MockSwarmView::new()
        .with_peer(make_peer("10.0.0.1", 6881, false, 50.0, 0.0))
        .with_peer(make_peer("10.0.0.2", 6881, false, 75.0, 0.0));
    let d_no_interest = policy.evaluate(&no_interest_swarm, 4, false);
    assert!(d_no_interest.unchoked_peers.is_empty());
    assert!(d_no_interest.optimistically_unchoked_peer.is_none());
    assert_eq!(d_no_interest.choked_peers.len(), 2);
}

#[test]
fn test_session_config_choking_policy_builder() {
    let tyrant = Arc::new(BitTyrantPolicy::new());
    let config = SessionConfig::default().with_choking_policy(tyrant);
    assert_eq!(config.max_connections, 50);

    let strategy = Arc::new(PolicyChokingStrategy::new(
        Arc::new(RoundRobinPolicy::new()),
    ));
    let config2 = SessionConfig::default().with_choking_strategy(strategy);
    assert_eq!(config2.max_connections, 50);
}
