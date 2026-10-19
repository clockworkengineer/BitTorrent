use bittorrent_rs::{
    FileDetails, FilePriority, MerkleTree, MockSocket, Peer, PeerMessage, SequentialSelector,
    SessionConfig, Socket, internals::TorrentContext, network::peer::PeerAction,
};
use std::sync::{Arc, Mutex};

fn create_mock_v2_context(file_len: u64, piece_len: u32, pieces_root: [u8; 32]) -> TorrentContext {
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

    let num_pieces = (file_len as usize).div_ceil(piece_len as usize);
    ctx.init_pieces(num_pieces, piece_len);
    ctx.total_bytes_to_download = file_len;

    ctx.files_to_download.push(FileDetails {
        name: "test.bin".to_string(),
        torrent_path: "test.bin".to_string(),
        length: file_len,
        md5sum: None,
        offset: 0,
        priority: FilePriority::Normal,
        pieces_root: Some(pieces_root),
    });

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
fn test_peer_message_bep52_hash_request_roundtrip() {
    let pieces_root = [0xAA; 32];
    let req = PeerMessage::HashRequest {
        pieces_root,
        base_layer: 0,
        index: 4,
        length: 2,
        proof_layers: 3,
    };

    let encoded = req.encode();
    // 4 bytes length prefix + 1 byte ID (21) + 48 bytes payload = 53 bytes
    assert_eq!(encoded.len(), 53);
    assert_eq!(u32::from_be_bytes(encoded[0..4].try_into().unwrap()), 49);
    assert_eq!(encoded[4], 21); // ID 21: HashRequest
    assert_eq!(&encoded[5..37], &pieces_root);
    assert_eq!(u32::from_be_bytes(encoded[37..41].try_into().unwrap()), 0); // base_layer
    assert_eq!(u32::from_be_bytes(encoded[41..45].try_into().unwrap()), 4); // index
    assert_eq!(u32::from_be_bytes(encoded[45..49].try_into().unwrap()), 2); // length
    assert_eq!(u32::from_be_bytes(encoded[49..53].try_into().unwrap()), 3); // proof_layers

    let decoded = PeerMessage::decode(&encoded[4..]).expect("Decode HashRequest");
    match decoded {
        PeerMessage::HashRequest {
            pieces_root: dec_root,
            base_layer,
            index,
            length,
            proof_layers,
        } => {
            assert_eq!(dec_root, pieces_root);
            assert_eq!(base_layer, 0);
            assert_eq!(index, 4);
            assert_eq!(length, 2);
            assert_eq!(proof_layers, 3);
        }
        _ => panic!("Expected HashRequest"),
    }
}

#[test]
fn test_peer_message_bep52_hash_reject_roundtrip() {
    let pieces_root = [0xBB; 32];
    let rej = PeerMessage::HashReject {
        pieces_root,
        base_layer: 1,
        index: 8,
        length: 4,
        proof_layers: 2,
    };

    let encoded = rej.encode();
    assert_eq!(encoded.len(), 53);
    assert_eq!(u32::from_be_bytes(encoded[0..4].try_into().unwrap()), 49);
    assert_eq!(encoded[4], 23); // ID 23: HashReject

    let decoded = PeerMessage::decode(&encoded[4..]).expect("Decode HashReject");
    match decoded {
        PeerMessage::HashReject {
            pieces_root: dec_root,
            base_layer,
            index,
            length,
            proof_layers,
        } => {
            assert_eq!(dec_root, pieces_root);
            assert_eq!(base_layer, 1);
            assert_eq!(index, 8);
            assert_eq!(length, 4);
            assert_eq!(proof_layers, 2);
        }
        _ => panic!("Expected HashReject"),
    }
}

#[test]
fn test_peer_message_bep52_hashes_roundtrip() {
    let pieces_root = [0xCC; 32];
    let mut raw_hashes = Vec::new();
    for i in 0..4 {
        raw_hashes.extend_from_slice(&[i as u8 + 1; 32]);
    }

    let hashes_msg = PeerMessage::Hashes {
        pieces_root,
        base_layer: 0,
        index: 0,
        length: 2,
        proof_layers: 2,
        hashes: &raw_hashes,
    };

    let encoded = hashes_msg.encode();
    // 4 bytes len + 1 byte ID (22) + 48 byte header + 4 * 32 hashes = 53 + 128 = 181
    assert_eq!(encoded.len(), 181);
    assert_eq!(
        u32::from_be_bytes(encoded[0..4].try_into().unwrap()),
        49 + 128
    );
    assert_eq!(encoded[4], 22); // ID 22: Hashes

    let decoded = PeerMessage::decode(&encoded[4..]).expect("Decode Hashes");
    match decoded {
        PeerMessage::Hashes {
            pieces_root: dec_root,
            base_layer,
            index,
            length,
            proof_layers,
            hashes,
        } => {
            assert_eq!(dec_root, pieces_root);
            assert_eq!(base_layer, 0);
            assert_eq!(index, 0);
            assert_eq!(length, 2);
            assert_eq!(proof_layers, 2);
            assert_eq!(hashes, raw_hashes.as_slice());
        }
        _ => panic!("Expected Hashes"),
    }
}

#[test]
fn test_peer_message_bep52_malformed_payload_rejected() {
    // Truncated HashRequest (less than 48 bytes)
    assert!(PeerMessage::decode(&[21, 0, 1, 2, 3]).is_err());
    // Truncated HashReject
    assert!(PeerMessage::decode(&[23, 0, 1, 2, 3]).is_err());
    // Hashes payload not aligned to 32 bytes after 48-byte header
    let mut bad_hashes = vec![22];
    bad_hashes.extend_from_slice(&[0u8; 48]);
    bad_hashes.extend_from_slice(&[0u8; 15]); // 15 bytes is not a multiple of 32
    assert!(PeerMessage::decode(&bad_hashes).is_err());
}

#[test]
fn test_merkle_tree_generate_and_verify_hashes_response() {
    // 64 KiB file = exactly 4 blocks of 16 KiB
    let file_data = vec![0x42u8; 64 * 1024];
    let tree = MerkleTree::from_bytes(&file_data);
    let root = tree.root();
    assert_eq!(tree.num_blocks(), 4);

    // Request 2 blocks at offset 0 (blocks 0 and 1) with proof_layers = 2
    let response_data = tree
        .generate_hashes_response(0, 0, 2, 2)
        .expect("Generated hashes response");

    // base = 2 hashes (64 bytes) + 1 uncle hash at height 1 (32 bytes) = 96 bytes (3 hashes)
    assert_eq!(response_data.len(), 3 * 32);

    // Verify response cryptographically
    let verified = MerkleTree::verify_hashes_response(&root, 0, 0, 2, 2, &response_data)
        .expect("Verification succeeds");
    assert_eq!(verified.len(), 2);
    assert_eq!(verified[0], tree.block_hash(0).unwrap());
    assert_eq!(verified[1], tree.block_hash(1).unwrap());

    // Request blocks 2 and 3
    let response_data_2 = tree
        .generate_hashes_response(0, 2, 2, 2)
        .expect("Generated hashes response for blocks 2..4");
    let verified_2 = MerkleTree::verify_hashes_response(&root, 0, 2, 2, 2, &response_data_2)
        .expect("Verification succeeds for blocks 2..4");
    assert_eq!(verified_2.len(), 2);
    assert_eq!(verified_2[0], tree.block_hash(2).unwrap());
    assert_eq!(verified_2[1], tree.block_hash(3).unwrap());

    // Corrupted uncle hash should fail verification
    let mut corrupted = response_data.clone();
    corrupted[64] ^= 0xFF; // Flip byte in uncle hash
    assert!(MerkleTree::verify_hashes_response(&root, 0, 0, 2, 2, &corrupted).is_err());

    // Corrupted base hash should fail verification
    let mut corrupted_base = response_data.clone();
    corrupted_base[0] ^= 0xFF; // Flip byte in base hash
    assert!(MerkleTree::verify_hashes_response(&root, 0, 0, 2, 2, &corrupted_base).is_err());
}

#[test]
fn test_torrent_context_bep52_hash_transfer_and_block_verification() {
    let file_data = vec![0x77u8; 32 * 1024]; // 2 blocks of 16 KiB
    let tree = MerkleTree::from_bytes(&file_data);
    let root = tree.root();

    let tc = create_mock_v2_context(32 * 1024, 32 * 1024, root);
    tc.register_file_merkle_tree(0, tree.clone());

    // 1. Peer requests hashes
    let hash_resp = tc
        .handle_hash_request(root, 0, 0, 2, 2)
        .expect("TorrentContext generated response");
    assert_eq!(hash_resp.len(), 2 * 32); // 2 blocks, depth is 1 so root covers both without uncles

    // 2. Receiving peer processes hashes response
    let tc_receiver = create_mock_v2_context(32 * 1024, 32 * 1024, root);
    let verified = tc_receiver
        .handle_hashes_response(root, 0, 0, 2, 2, &hash_resp)
        .expect("Hashes verified and registered");
    assert_eq!(verified.len(), 2);

    // 3. Verify block 0 passes verify_v2_block using verified leaf hashes!
    let block_0_data = &file_data[0..16 * 1024];
    assert!(tc_receiver.verify_v2_block(0, 0, block_0_data).is_ok());

    // Block 1 passes
    let block_1_data = &file_data[16 * 1024..32 * 1024];
    assert!(tc_receiver.verify_v2_block(0, 1, block_1_data).is_ok());

    // Corrupted block fails
    let mut corrupt_block = block_0_data.to_vec();
    corrupt_block[0] ^= 0xFF;
    assert!(tc_receiver.verify_v2_block(0, 0, &corrupt_block).is_err());
}

#[test]
fn test_peer_handle_peer_message_bep52_flow() {
    let file_data = vec![0x99u8; 32 * 1024];
    let tree = MerkleTree::from_bytes(&file_data);
    let root = tree.root();

    let tc_seeder = create_mock_v2_context(32 * 1024, 32 * 1024, root);
    tc_seeder.register_file_merkle_tree(0, tree.clone());
    let tc_seeder_arc = Arc::new(Mutex::new(tc_seeder));

    let mut peer_seeder = create_mock_peer("127.0.0.1", tc_seeder_arc.clone());
    let mut tc_guard_seeder = tc_seeder_arc.lock().unwrap();

    // 1. Peer seeder receives HashRequest -> should respond with SendHashes
    let req = PeerMessage::HashRequest {
        pieces_root: root,
        base_layer: 0,
        index: 0,
        length: 2,
        proof_layers: 1,
    };
    let actions = peer_seeder
        .handle_peer_message(req, &mut *tc_guard_seeder)
        .expect("Handled HashRequest");

    assert_eq!(actions.len(), 1);
    let payload_hashes = match &actions[0] {
        PeerAction::SendHashes {
            pieces_root,
            base_layer,
            index,
            length,
            proof_layers,
            hashes,
        } => {
            assert_eq!(*pieces_root, root);
            assert_eq!(*base_layer, 0);
            assert_eq!(*index, 0);
            assert_eq!(*length, 2);
            assert_eq!(*proof_layers, 1);
            assert_eq!(hashes.len(), 64);
            hashes.clone()
        }
        _ => panic!("Expected SendHashes action"),
    };

    // Check stats on seeder
    let (req_sent, req_rcv, h_sent, h_rcv, rej_sent, rej_rcv) = tc_guard_seeder.v2_hash_stats();
    assert_eq!(req_rcv, 1);
    assert_eq!(h_sent, 1);
    assert_eq!(req_sent, 0);
    assert_eq!(h_rcv, 0);
    assert_eq!(rej_sent, 0);
    assert_eq!(rej_rcv, 0);

    // 2. Downloader peer issues hash request and handles response
    let tc_downloader = create_mock_v2_context(32 * 1024, 32 * 1024, root);
    let tc_downloader_arc = Arc::new(Mutex::new(tc_downloader));
    let mut peer_downloader = create_mock_peer("127.0.0.2", tc_downloader_arc.clone());
    let mut tc_guard_dl = tc_downloader_arc.lock().unwrap();

    let request_action = peer_downloader.request_v2_hashes(root, 0, 0, 2, 1, &mut *tc_guard_dl);
    match request_action {
        PeerAction::SendHashRequest { pieces_root, .. } => {
            assert_eq!(pieces_root, root);
        }
        _ => panic!("Expected SendHashRequest"),
    }
    assert_eq!(tc_guard_dl.v2_hash_stats().0, 1); // req_sent = 1

    // Receive the Hashes message on downloader
    let hashes_msg = PeerMessage::Hashes {
        pieces_root: root,
        base_layer: 0,
        index: 0,
        length: 2,
        proof_layers: 1,
        hashes: &payload_hashes,
    };
    let dl_actions = peer_downloader
        .handle_peer_message(hashes_msg, &mut *tc_guard_dl)
        .expect("Handled Hashes message");
    assert!(dl_actions.is_empty());
    assert_eq!(tc_guard_dl.v2_hash_stats().3, 1); // hashes_received = 1

    // 3. Requesting an unknown root should trigger SendHashReject
    let unknown_req = PeerMessage::HashRequest {
        pieces_root: [0xDE; 32],
        base_layer: 0,
        index: 0,
        length: 2,
        proof_layers: 1,
    };
    let rej_actions = peer_seeder
        .handle_peer_message(unknown_req, &mut *tc_guard_seeder)
        .expect("Handled unknown HashRequest");
    assert_eq!(rej_actions.len(), 1);
    match &rej_actions[0] {
        PeerAction::SendHashReject { pieces_root, .. } => {
            assert_eq!(*pieces_root, [0xDE; 32]);
        }
        _ => panic!("Expected SendHashReject action"),
    }
    assert_eq!(tc_guard_seeder.v2_hash_stats().4, 1); // rejects_sent = 1
}
