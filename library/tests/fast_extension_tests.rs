use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::{
    TorrentStatus,
    io_traits::{MemStorage, MockSocket, Socket},
    peer::{Peer, PeerAction},
    peer_message::PeerMessage,
    selector::SequentialSelector,
    session::config::SessionConfig,
    util::generate_allowed_fast_set,
};
use std::sync::{Arc, Mutex};

fn create_test_context(
    num_pieces: usize,
    piece_length: u32,
    info_hash: [u8; 20],
) -> TorrentContext {
    let config = SessionConfig::default();
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        info_hash.to_vec(),
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
    peer
}

#[test]
fn test_bep6_official_test_vectors() {
    let info_hash = [0xaa; 20];
    let ip = "80.4.4.200";
    let num_pieces = 1313;

    // Official BEP 6 test vector for k = 7
    let fast_7 = generate_allowed_fast_set(7, num_pieces, &info_hash, ip);
    assert_eq!(fast_7, vec![1059, 431, 808, 1217, 287, 376, 1188]);

    // Official BEP 6 test vector for k = 9
    let fast_9 = generate_allowed_fast_set(9, num_pieces, &info_hash, ip);
    assert_eq!(fast_9, vec![1059, 431, 808, 1217, 287, 376, 1188, 353, 508]);
}

#[test]
fn test_bep6_subnet_masking_invariance() {
    let info_hash = [0xaa; 20];
    let num_pieces = 1313;

    // IPv4 /24 masking means all hosts in 80.4.4.0/24 share the identical allowed fast set
    let set_host1 = generate_allowed_fast_set(7, num_pieces, &info_hash, "80.4.4.1");
    let set_host2 = generate_allowed_fast_set(7, num_pieces, &info_hash, "80.4.4.200");
    let set_host3 = generate_allowed_fast_set(7, num_pieces, &info_hash, "80.4.4.254");
    assert_eq!(set_host1, set_host2);
    assert_eq!(set_host2, set_host3);

    // Different /24 subnet yields a different set
    let set_diff_subnet = generate_allowed_fast_set(7, num_pieces, &info_hash, "80.4.5.200");
    assert_ne!(set_host1, set_diff_subnet);

    // IPv6 /48 masking
    let ip6_host1 = "2001:db8:abcd:0000::1";
    let ip6_host2 = "2001:db8:abcd:ffff::99";
    let set_ip6_1 = generate_allowed_fast_set(8, num_pieces, &info_hash, ip6_host1);
    let set_ip6_2 = generate_allowed_fast_set(8, num_pieces, &info_hash, ip6_host2);
    assert_eq!(set_ip6_1, set_ip6_2);
}

#[test]
fn test_bep6_edge_cases() {
    let info_hash = [0x11; 20];

    // k = 0
    assert_eq!(
        generate_allowed_fast_set(0, 100, &info_hash, "1.2.3.4"),
        Vec::<u32>::new()
    );

    // num_pieces = 0
    assert_eq!(
        generate_allowed_fast_set(10, 0, &info_hash, "1.2.3.4"),
        Vec::<u32>::new()
    );

    // k > num_pieces: caps at num_pieces without looping infinitely
    let small_pieces = 3;
    let set = generate_allowed_fast_set(10, small_pieces, &info_hash, "1.2.3.4");
    assert_eq!(set.len(), 3);
    for piece in &set {
        assert!(*piece < 3);
    }
}

#[test]
fn test_bep6_wire_messages_encode_decode() {
    // 1. Suggest Piece (ID 13, 0x0D)
    let suggest = PeerMessage::Suggest(42);
    let encoded = suggest.encode();
    assert_eq!(encoded.len(), 9); // 4-byte len (5) + 1-byte id (13) + 4-byte index
    assert_eq!(encoded[0..4], 5u32.to_be_bytes());
    assert_eq!(encoded[4], 13);
    assert_eq!(encoded[5..9], 42u32.to_be_bytes());

    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    match decoded {
        PeerMessage::Suggest(idx) => assert_eq!(idx, 42),
        other => panic!("Expected Suggest, got {:?}", other),
    }

    // 2. Have All (ID 14, 0x0E)
    let have_all = PeerMessage::HaveAll;
    let encoded = have_all.encode();
    assert_eq!(encoded, vec![0, 0, 0, 1, 14]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    assert!(matches!(decoded, PeerMessage::HaveAll));

    // 3. Have None (ID 15, 0x0F)
    let have_none = PeerMessage::HaveNone;
    let encoded = have_none.encode();
    assert_eq!(encoded, vec![0, 0, 0, 1, 15]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    assert!(matches!(decoded, PeerMessage::HaveNone));

    // 4. Reject Request (ID 16, 0x10)
    let reject = PeerMessage::Reject {
        index: 12,
        begin: 16384,
        length: 16384,
    };
    let encoded = reject.encode();
    assert_eq!(encoded.len(), 17); // 4-byte len (13) + 1-byte id (16) + 3*4-byte u32
    assert_eq!(encoded[0..4], 13u32.to_be_bytes());
    assert_eq!(encoded[4], 16);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    match decoded {
        PeerMessage::Reject {
            index,
            begin,
            length,
        } => {
            assert_eq!(index, 12);
            assert_eq!(begin, 16384);
            assert_eq!(length, 16384);
        }
        other => panic!("Expected Reject, got {:?}", other),
    }

    // 5. Allowed Fast (ID 17, 0x11)
    let allowed_fast = PeerMessage::AllowedFast(88);
    let encoded = allowed_fast.encode();
    assert_eq!(encoded.len(), 9);
    assert_eq!(encoded[0..4], 5u32.to_be_bytes());
    assert_eq!(encoded[4], 17);
    assert_eq!(encoded[5..9], 88u32.to_be_bytes());
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    match decoded {
        PeerMessage::AllowedFast(idx) => assert_eq!(idx, 88),
        other => panic!("Expected AllowedFast, got {:?}", other),
    }
}

#[test]
fn test_handshake_fast_extension_bit() {
    let info_hash = [0x55; 20];
    let peer_id = [0x77; 20];

    let handshake = PeerMessage::handshake_encode(&info_hash, &peer_id).unwrap();
    assert_eq!(handshake.len(), 68);

    // Reserved bytes are at offset 20..28
    let reserved = &handshake[20..28];
    // BEP 6 requires reserved[7] |= 0x04
    assert_ne!(
        reserved[7] & 0x04,
        0,
        "Fast Extension bit in reserved[7] must be set"
    );

    // Decode handshake
    let (decoded_hash, decoded_id, decoded_reserved) =
        PeerMessage::handshake_decode(&handshake).unwrap();
    assert_eq!(decoded_hash, info_hash.to_vec());
    assert_eq!(decoded_id, peer_id.to_vec());
    assert_ne!(decoded_reserved[7] & 0x04, 0);
}

#[test]
fn test_have_all_and_have_none_handling() {
    let info_hash = [0x33; 20];
    let num_pieces = 16;
    let piece_len = 16384;

    // Test HaveAll
    let ctx = Arc::new(Mutex::new(create_test_context(
        num_pieces, piece_len, info_hash,
    )));
    let mut peer = create_mock_peer("192.168.1.50", ctx.clone());
    peer.supports_fast_extension = true;

    assert_eq!(peer.number_of_missing_pieces, num_pieces);
    let actions = peer
        .handle_peer_message(PeerMessage::HaveAll, &mut ctx.lock().unwrap())
        .unwrap();
    assert!(actions.is_empty());
    assert_eq!(peer.number_of_missing_pieces, 0);
    assert!(peer.is_seed(num_pieces));
    for p in 0..num_pieces as u32 {
        assert!(peer.is_piece_on_remote_peer(p));
    }

    // Test HaveNone
    let actions = peer
        .handle_peer_message(PeerMessage::HaveNone, &mut ctx.lock().unwrap())
        .unwrap();
    assert!(actions.is_empty());
    assert_eq!(peer.number_of_missing_pieces, num_pieces);
    for p in 0..num_pieces as u32 {
        assert!(!peer.is_piece_on_remote_peer(p));
    }
}

#[test]
fn test_suggest_and_allowed_fast_handling() {
    let info_hash = [0x44; 20];
    let ctx = Arc::new(Mutex::new(create_test_context(10, 16384, info_hash)));
    let mut peer = create_mock_peer("10.0.0.1", ctx.clone());
    peer.supports_fast_extension = true;

    // Suggest piece
    peer.handle_peer_message(PeerMessage::Suggest(3), &mut ctx.lock().unwrap())
        .unwrap();
    peer.handle_peer_message(PeerMessage::Suggest(7), &mut ctx.lock().unwrap())
        .unwrap();
    assert_eq!(peer.suggested_pieces, vec![3, 7]);

    // Allowed Fast piece
    peer.handle_peer_message(PeerMessage::AllowedFast(2), &mut ctx.lock().unwrap())
        .unwrap();
    peer.handle_peer_message(PeerMessage::AllowedFast(5), &mut ctx.lock().unwrap())
        .unwrap();
    assert_eq!(peer.allowed_fast_pieces, vec![2, 5]);
    assert!(peer.is_allowed_fast_from_remote(2));
    assert!(peer.is_allowed_fast_from_remote(5));
    assert!(!peer.is_allowed_fast_from_remote(6));
}

#[test]
fn test_choked_peer_request_rejected_when_not_allowed_fast() {
    let info_hash = [0xaa; 20];
    let num_pieces = 1313;
    let piece_len = 16384;
    let ctx = Arc::new(Mutex::new(create_test_context(
        num_pieces, piece_len, info_hash,
    )));
    let mut peer = create_mock_peer("80.4.4.200", ctx.clone());
    peer.supports_fast_extension = true;
    peer.am_choking = true; // We are choking the remote peer

    // Peer requests piece 0 (which is NOT in the allowed fast set for 80.4.4.200)
    let actions = peer
        .handle_peer_message(
            PeerMessage::Request {
                index: 0,
                begin: 0,
                length: 16384,
            },
            &mut ctx.lock().unwrap(),
        )
        .unwrap();

    // Must return SendReject
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        PeerAction::SendReject {
            index,
            begin,
            length,
        } => {
            assert_eq!(*index, 0);
            assert_eq!(*begin, 0);
            assert_eq!(*length, 16384);
        }
        other => panic!("Expected SendReject, got {:?}", other),
    }
}

#[test]
fn test_choked_peer_request_served_when_allowed_fast() {
    let info_hash = [0xaa; 20];
    let num_pieces = 1313;
    let piece_len = 16384;
    let ctx = Arc::new(Mutex::new(create_test_context(
        num_pieces, piece_len, info_hash,
    )));

    // Piece 1059 is the very first allowed fast piece for 80.4.4.200!
    {
        let mut guard = ctx.lock().unwrap();
        guard.mark_piece_local(1059, true);
        let block_data = vec![0xEEu8; 16384];
        let offset = 1059u64 * piece_len as u64;
        guard.storage.write_block(offset, &block_data).unwrap();
    }

    let mut peer = create_mock_peer("80.4.4.200", ctx.clone());
    peer.supports_fast_extension = true;
    peer.am_choking = true; // We are choking the remote peer!

    assert!(peer.is_peer_allowed_fast(1059, &ctx.lock().unwrap()));

    let actions = peer
        .handle_peer_message(
            PeerMessage::Request {
                index: 1059,
                begin: 0,
                length: 16384,
            },
            &mut ctx.lock().unwrap(),
        )
        .unwrap();

    // Must serve piece via SendPiece despite being choked!
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        PeerAction::SendPiece {
            index,
            begin,
            block,
        } => {
            assert_eq!(*index, 1059);
            assert_eq!(*begin, 0);
            assert_eq!(block.len(), 16384);
            assert_eq!(block[0], 0xEE);
        }
        other => panic!(
            "Expected SendPiece for allowed fast request, got {:?}",
            other
        ),
    }
}

#[test]
fn test_reject_message_releases_reserved_block() {
    let info_hash = [0x99; 20];
    let ctx = Arc::new(Mutex::new(create_test_context(10, 16384, info_hash)));
    let mut peer = create_mock_peer("192.168.1.10", ctx.clone());
    peer.supports_fast_extension = true;

    // Simulate sending a request for piece 2, block 0
    {
        let guard = ctx.lock().unwrap();
        guard.reserve_block_request(2, 0);
    }
    peer.reserved_blocks.push((2, 0, std::time::Instant::now()));
    peer.outstanding_requests_count = 1;

    // Receive Reject for piece 2, begin 0, length 16384
    let actions = peer
        .handle_peer_message(
            PeerMessage::Reject {
                index: 2,
                begin: 0,
                length: 16384,
            },
            &mut ctx.lock().unwrap(),
        )
        .unwrap();

    assert!(actions.is_empty());
    assert_eq!(peer.outstanding_requests_count, 0);
    assert!(peer.reserved_blocks.is_empty());

    // The block reservation in ctx should now be released, allowing re-reservation
    let guard = ctx.lock().unwrap();
    assert!(
        guard.reserve_block_request(2, 0),
        "Block must be released and available for reservation"
    );
}

#[test]
fn test_next_allowed_fast_block_request() {
    let info_hash = [0x55; 20];
    let ctx = Arc::new(Mutex::new(create_test_context(10, 16384, info_hash)));
    let mut peer = create_mock_peer("192.168.1.20", ctx.clone());
    peer.supports_fast_extension = true;

    // Remote peer has pieces 0, 1, 2
    peer.set_piece_on_remote_peer(0);
    peer.set_piece_on_remote_peer(1);
    peer.set_piece_on_remote_peer(2);

    // Remote peer grants us Allowed Fast only for piece 2
    peer.allowed_fast_pieces = vec![2];

    let mut guard = ctx.lock().unwrap();
    // next_allowed_fast_block_request_for_peer must select piece 2
    let req = guard.next_allowed_fast_block_request_for_peer(&peer);
    assert_eq!(req, Some((2, 0, 16384)));
}
