use bittorrent_rs::{
    peer::PeerAction, MockSocket, Peer, PeerMessage, SequentialSelector, SessionConfig, Socket,
    TorrentSessionBuilder, TorrentStatus,
};
use bittorrent_rs::internals::TorrentContext;
use std::sync::{Arc, Mutex};

fn create_seeding_context(num_pieces: usize, piece_length: u32, super_seeding: bool) -> TorrentContext {
    let mut config = SessionConfig::default();
    config.super_seeding = super_seeding;
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xBB; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    ctx.init_pieces(num_pieces, piece_length);
    ctx.total_bytes_to_download = num_pieces as u64 * piece_length as u64;
    ctx.status = TorrentStatus::Seeding;

    // Mark all pieces as locally possessed (we are a seed)
    for p in 0..num_pieces as u32 {
        ctx.mark_piece_local(p, true);
    }

    ctx
}

fn create_mock_peer(ip: &str, context: Arc<Mutex<TorrentContext>>) -> Peer {
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket(ip.to_string(), 6881, socket);
    peer.set_torrent_context(context);
    peer.connected = true;
    peer.remote_piece_bitfield = vec![0u8; 1];
    peer
}

#[test]
fn test_super_seeding_state_activation() {
    let mut ctx = create_seeding_context(4, 16384, true);
    assert!(ctx.is_super_seeding());

    // Disabling super-seeding turns it off
    ctx.set_super_seeding(false);
    assert!(!ctx.is_super_seeding());

    // Re-enabling turns it back on
    ctx.set_super_seeding(true);
    assert!(ctx.is_super_seeding());

    // If status is not Seeding and download is not complete, it is inactive
    ctx.status = TorrentStatus::Downloading;
    ctx.bitfield = vec![0x00]; // pretend we don't have pieces
    assert!(!ctx.is_super_seeding());
}

#[test]
fn test_offer_super_seed_piece_distribution() {
    let mut ctx = create_seeding_context(4, 16384, true);

    let peer_a_ip = "192.168.1.10";
    let peer_b_ip = "192.168.1.20";
    let empty_bitfield = vec![0x00];

    // Offer first piece to Peer A
    let piece_a = ctx.offer_super_seed_piece(peer_a_ip, &empty_bitfield);
    assert_eq!(piece_a, Some(0));

    // Peer A should NOT be offered another piece while piece 0 is pending swarm validation
    let piece_a_repeat = ctx.offer_super_seed_piece(peer_a_ip, &empty_bitfield);
    assert_eq!(piece_a_repeat, None);

    // Offer to Peer B should choose an unseeded piece (piece 1)
    let piece_b = ctx.offer_super_seed_piece(peer_b_ip, &empty_bitfield);
    assert_eq!(piece_b, Some(1));

    // Verify upload permissions
    assert!(ctx.can_upload_piece_to_peer(peer_a_ip, 0));
    assert!(!ctx.can_upload_piece_to_peer(peer_a_ip, 1));
    assert!(ctx.can_upload_piece_to_peer(peer_b_ip, 1));
    assert!(!ctx.can_upload_piece_to_peer(peer_b_ip, 0));
}

#[test]
fn test_super_seeding_swarm_have_validation() {
    let mut ctx = create_seeding_context(4, 16384, true);

    let peer_a_ip = "10.0.0.1";
    let peer_b_ip = "10.0.0.2";
    let empty_bitfield = vec![0x00];

    // Offer piece 0 to Peer A
    assert_eq!(ctx.offer_super_seed_piece(peer_a_ip, &empty_bitfield), Some(0));
    // Offer piece 1 to Peer B
    assert_eq!(ctx.offer_super_seed_piece(peer_b_ip, &empty_bitfield), Some(1));

    // Peer A reports Have(0) - this is just Peer A acknowledging what was offered, NOT swarm validation
    let res_self = ctx.on_peer_have(peer_a_ip, 0);
    assert_eq!(res_self, None);
    // Peer A still has piece 0 pending
    assert_eq!(ctx.offer_super_seed_piece(peer_a_ip, &empty_bitfield), None);

    // Peer B reports Have(0) - Swarm validation! Peer A successfully uploaded piece 0 to Peer B!
    // Set up peer_swarm in ctx so ctx can look up Peer A's bitfield
    let ctx_arc = Arc::new(Mutex::new(ctx));
    let peer_a = Arc::new(Mutex::new(create_mock_peer(peer_a_ip, ctx_arc.clone())));
    {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard.add_peer(peer_a);
    }

    let validation_res = ctx_arc.lock().unwrap().on_peer_have(peer_b_ip, 0);
    assert!(validation_res.is_some());
    let (target_peer, next_piece) = validation_res.unwrap();
    assert_eq!(target_peer, peer_a_ip);
    // Piece 0 and 1 are already taken/distributed, so Peer A should be offered piece 2
    assert_eq!(next_piece, 2);
}

#[test]
fn test_super_seeding_peer_disconnect_frees_piece() {
    let mut ctx = create_seeding_context(4, 16384, true);

    let peer_a_ip = "192.168.2.10";
    let peer_b_ip = "192.168.2.20";
    let empty_bitfield = vec![0x00];

    // Offer piece 0 to Peer A
    assert_eq!(ctx.offer_super_seed_piece(peer_a_ip, &empty_bitfield), Some(0));
    assert_eq!(ctx.offer_super_seed_piece(peer_b_ip, &empty_bitfield), Some(1));

    // Peer A disconnects before swarm validates piece 0
    ctx.on_peer_disconnect(peer_a_ip);

    // Now piece 0 is freed and can be offered to a new peer C
    let peer_c_ip = "192.168.2.30";
    let piece_c = ctx.offer_super_seed_piece(peer_c_ip, &empty_bitfield);
    assert_eq!(piece_c, Some(0));
}

#[test]
fn test_super_seeding_auto_exit_when_peer_is_seed() {
    let ctx = create_seeding_context(4, 16384, true);
    let ctx_arc = Arc::new(Mutex::new(ctx));

    let mut peer = create_mock_peer("192.168.3.10", ctx_arc.clone());

    // Peer connects and sends a complete bitfield (they are already a seed!)
    // 4 pieces (bits 0,1,2,3 set in high nibble: 0b11110000 = 0xF0)
    let full_bitfield_bytes = vec![0xF0];

    let actions = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer.handle_peer_message(PeerMessage::Bitfield(&full_bitfield_bytes), &mut *ctx_guard)
            .expect("Handle bitfield failed")
    };

    // Since peer is a seed, super-seeding mode should automatically exit!
    assert!(!ctx_arc.lock().unwrap().is_super_seeding());
    // No have action was sent to a seeder
    assert!(!actions.iter().any(|a| matches!(a, PeerAction::SendHave(_))));
}

#[test]
fn test_super_seeding_builder_and_config() {
    let config = SessionConfig::default();
    assert!(!config.super_seeding);

    let builder = TorrentSessionBuilder::new("test.torrent", ".").super_seeding(true);
    assert!(builder.get_config().super_seeding);
}
