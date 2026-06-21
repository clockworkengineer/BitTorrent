use bittorrent_rs::{
    Peer, SessionConfig, BNode, Bencode, PeerMessage, Socket, MockSocket, RarestFirstSelector
};
use bittorrent_rs::internals::TorrentContext;
use std::sync::{Arc, Mutex};

#[test]
fn test_mock_peer_handshake_and_choke_flow() {
    let (socket, in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket.clone());

    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let context = TorrentContext::new_magnet_bootstrap(
        vec![0xAA; 20],
        vec!["http://127.0.0.1/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    let context = Arc::new(Mutex::new(context));
    peer.set_torrent_context(context.clone());

    // Verify initial connection states
    assert!(!peer.connected);
    assert!(!peer.peer_choking.wait_one(0));

    // Simulate receiving handshake from peer
    let remote_peer_id = *b"-RS0001-111122223333";
    let handshake_payload = PeerMessage::handshake_encode(&vec![0xAA; 20], &remote_peer_id).unwrap();
    in_tx.send(handshake_payload).unwrap();

    // Send Unchoke message from the remote side
    let unchoke_action = peer.handle_peer_message(PeerMessage::Unchoke, &mut context.lock().unwrap()).unwrap();
    assert!(unchoke_action.is_empty());
    assert!(peer.peer_choking.wait_one(0));
}

#[test]
fn test_mock_peer_bitfield_and_pex() {
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket.clone());

    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let context = TorrentContext::new_magnet_bootstrap(
        vec![0xBB; 20],
        vec!["http://127.0.0.1/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    let context = Arc::new(Mutex::new(context));
    peer.set_torrent_context(context.clone());

    // Send a bitfield from the remote peer
    let bitfield_payload = vec![0xFF, 0x00];
    let action_res = peer.handle_peer_message(PeerMessage::Bitfield(&bitfield_payload), &mut context.lock().unwrap());
    assert!(action_res.is_ok());
    assert!(peer.is_piece_on_remote_peer(0));
    assert!(!peer.is_piece_on_remote_peer(8));

    // Send PEX message
    let added_bytes = vec![192, 168, 1, 15, 26, 225];
    let dropped_bytes = vec![];
    let pex_dict = BNode::Dictionary(vec![
        (b"added".as_slice(), BNode::String(&added_bytes)),
        (b"dropped".as_slice(), BNode::String(&dropped_bytes)),
    ]);
    let pex_payload = Bencode::encode(&pex_dict);

    let pex_action = peer.handle_peer_message(PeerMessage::Extended { ext_id: 2, payload: &pex_payload }, &mut context.lock().unwrap()).unwrap();
    assert_eq!(pex_action.len(), 1);
}
