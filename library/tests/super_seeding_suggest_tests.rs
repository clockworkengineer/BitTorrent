use bittorrent_rs::{
    internals::TorrentContext,
    network::peer::PeerAction,
    MockSocket, Peer, PeerMessage, SequentialSelector, SessionConfig, Socket, TorrentStatus,
};
use std::sync::{Arc, Mutex};

fn create_seeding_context(num_pieces: usize, piece_length: u32, super_seeding: bool) -> TorrentContext {
    let mut config = SessionConfig::default();
    config.super_seeding = super_seeding;
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
    ctx.status = TorrentStatus::Seeding;

    for p in 0..num_pieces as u32 {
        ctx.mark_piece_local(p, true);
    }

    ctx
}

fn create_mock_peer(ip: &str, context: Arc<Mutex<TorrentContext>>, supports_fast: bool) -> Peer {
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket(ip.to_string(), 6881, socket);
    peer.set_torrent_context(context);
    peer.connected = true;
    peer.supports_fast_extension = supports_fast;
    peer.remote_piece_bitfield = vec![0u8; 1];
    peer
}

#[test]
fn test_super_seeding_with_fast_extension_sends_suggest_and_allowed_fast() {
    let ctx = create_seeding_context(4, 16384, true);
    let ctx_arc = Arc::new(Mutex::new(ctx));

    // Peer A supports Fast Extension (BEP 6)
    let mut peer_a = create_mock_peer("192.168.1.10", ctx_arc.clone(), true);
    // Peer B does NOT support Fast Extension
    let mut peer_b = create_mock_peer("192.168.1.20", ctx_arc.clone(), false);

    // Peer A expresses interest
    let actions_a = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer_a.handle_peer_message(PeerMessage::Interested, &mut *ctx_guard).unwrap()
    };

    // Peer A should receive Have(0), Suggest(0), and AllowedFast(0)
    assert!(actions_a.iter().any(|a| matches!(a, PeerAction::SendHave(0))));
    assert!(actions_a.iter().any(|a| matches!(a, PeerAction::SendSuggest(0))));
    assert!(actions_a.iter().any(|a| matches!(a, PeerAction::SendAllowedFast(0))));

    // Peer B expresses interest
    let actions_b = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer_b.handle_peer_message(PeerMessage::Interested, &mut *ctx_guard).unwrap()
    };

    // Peer B should ONLY receive Have(1) without Suggest or AllowedFast
    assert!(actions_b.iter().any(|a| matches!(a, PeerAction::SendHave(1))));
    assert!(!actions_b.iter().any(|a| matches!(a, PeerAction::SendSuggest(_))));
    assert!(!actions_b.iter().any(|a| matches!(a, PeerAction::SendAllowedFast(_))));
}

#[test]
fn test_super_seeding_propagation_sends_suggest_to_peer() {
    let ctx = create_seeding_context(4, 16384, true);
    let ctx_arc = Arc::new(Mutex::new(ctx));

    let peer_a_ip = "10.0.0.1";
    let peer_b_ip = "10.0.0.2";

    let peer_a = Arc::new(Mutex::new(create_mock_peer(peer_a_ip, ctx_arc.clone(), true)));
    let mut peer_b = create_mock_peer(peer_b_ip, ctx_arc.clone(), true);

    {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard.add_peer(peer_a);
    }

    // Offer piece 0 to Peer A, piece 1 to Peer B
    {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        assert_eq!(ctx_guard.offer_super_seed_piece(peer_a_ip, &[0x00]), Some(0));
        assert_eq!(ctx_guard.offer_super_seed_piece(peer_b_ip, &[0x00]), Some(1));
    }

    // Peer B announces Have(0) -> Swarm validation!
    let actions = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer_b.handle_peer_message(PeerMessage::Have(0), &mut *ctx_guard).unwrap()
    };

    // Peer A should receive Have(2), Suggest(2), and AllowedFast(2) routed to them
    assert!(actions.iter().any(|a| matches!(a, PeerAction::SendHaveToPeer { target_ip, piece: 2 } if target_ip == peer_a_ip)));
    assert!(actions.iter().any(|a| matches!(a, PeerAction::SendSuggestToPeer { target_ip, piece: 2 } if target_ip == peer_a_ip)));
    assert!(actions.iter().any(|a| matches!(a, PeerAction::SendAllowedFastToPeer { target_ip, piece: 2 } if target_ip == peer_a_ip)));
}

#[test]
fn test_downloader_prioritizes_suggested_piece_requests() {
    let mut config = SessionConfig::default();
    config.block_size = 16384;
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xCC; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    // 10 pieces of 16384 bytes
    ctx.init_pieces(10, 16384);
    ctx.status = TorrentStatus::Downloading;

    let ctx_arc = Arc::new(Mutex::new(ctx));
    let mut peer = create_mock_peer("192.168.5.5", ctx_arc.clone(), true);

    // Remote peer has pieces 0, 1, 2, 3, 4, 5
    peer.remote_piece_bitfield = vec![0b11111100, 0x00];

    // Peer suggests piece 4!
    peer.suggested_pieces.push(4);

    let mut ctx_guard = ctx_arc.lock().unwrap();
    let request = ctx_guard.next_block_request_for_peer(&peer);

    // Downloader must prioritize suggested piece 4 over earlier pieces (0, 1, 2, 3)!
    assert_eq!(request, Some((4, 0, 16384)));
}

#[test]
fn test_downloader_choked_prioritizes_suggested_and_allowed_fast() {
    let mut config = SessionConfig::default();
    config.block_size = 16384;
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xDD; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    ctx.init_pieces(10, 16384);
    ctx.status = TorrentStatus::Downloading;

    let ctx_arc = Arc::new(Mutex::new(ctx));
    let mut peer = create_mock_peer("192.168.6.6", ctx_arc.clone(), true);

    // Peer has pieces 0..8
    peer.remote_piece_bitfield = vec![0b11111111, 0x80];

    // Peer gives Allowed Fast for pieces 1 and 6, and explicitly suggests piece 6!
    peer.allowed_fast_pieces.push(1);
    peer.allowed_fast_pieces.push(6);
    peer.suggested_pieces.push(6);

    let mut ctx_guard = ctx_arc.lock().unwrap();
    let request = ctx_guard.next_allowed_fast_block_request_for_peer(&peer);

    // Must prioritize suggested piece 6 over piece 1!
    assert_eq!(request, Some((6, 0, 16384)));
}

#[test]
fn test_super_seeding_metrics_tracking() {
    let mut ctx = create_seeding_context(4, 16384, true);

    let (offered, propagated) = ctx.super_seed_stats();
    assert_eq!(offered, 0);
    assert_eq!(propagated, 0);

    // Offer piece 0 to Peer 1
    ctx.offer_super_seed_piece("1.1.1.1", &[0x00]);
    assert_eq!(ctx.super_seed_stats(), (1, 0));

    // Offer piece 1 to Peer 2
    ctx.offer_super_seed_piece("2.2.2.2", &[0x00]);
    assert_eq!(ctx.super_seed_stats(), (2, 0));

    // Peer 2 announces Have(0) -> piece 0 propagated!
    ctx.on_peer_have("2.2.2.2", 0);
    assert_eq!(ctx.super_seed_stats(), (2, 1));
}

#[test]
fn test_suggest_next_piece_for_peer_helper() {
    let ctx = create_seeding_context(4, 16384, false);
    let ctx_arc = Arc::new(Mutex::new(ctx));

    let mut peer = create_mock_peer("1.2.3.4", ctx_arc.clone(), true);
    // Peer already has pieces 0 and 1
    peer.remote_piece_bitfield = vec![0b11000000];

    let ctx_guard = ctx_arc.lock().unwrap();
    let suggested = ctx_guard.suggest_next_piece_for_peer(&peer);

    // Should suggest piece 2 (the first piece the peer does not have)
    assert_eq!(suggested, Some(2));
}
