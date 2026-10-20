use bittorrent_rs::io_traits::{MockSocket, Socket};
use bittorrent_rs::network::peer_event::{
    ContextEventListener, NullPeerEventListener, PeerEventListener, RecordingPeerEventListener,
};
use bittorrent_rs::peer::Peer;
use bittorrent_rs::peer_message::PeerMessage;
use bittorrent_rs::selector::RarestFirstSelector;
use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::SessionConfig;
use bittorrent_rs::SwarmReadAccess;
use std::sync::Arc;

#[test]
fn test_peer_isolated_instantiation_without_context() {
    let (mock_sock, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(mock_sock));
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket);

    // Verify Peer starts without context or listener
    #[allow(deprecated)]
    let tc_is_none = peer.tc.is_none();
    assert!(tc_is_none);
    assert!(peer.event_listener.is_none());
    assert_eq!(peer.remote_piece_bitfield.len(), 0);

    // Initialize bitfield directly without requiring TorrentContext (DIP)
    peer.init_bitfield(16, 2);
    assert_eq!(peer.number_of_missing_pieces, 16);
    assert_eq!(peer.remote_piece_bitfield.len(), 2);

    // Attach a NullPeerEventListener
    peer.set_event_listener(Arc::new(NullPeerEventListener));
    assert!(peer.event_listener.is_some());

    // Close peer without panicking
    peer.close();
    assert!(!peer.connected);
    assert!(peer.event_listener.is_none());
}

#[test]
fn test_peer_recording_event_listener_notifications() {
    let (mock_sock, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(mock_sock));
    let mut peer = Peer::new_with_socket("192.168.1.50".to_string(), 6881, socket);
    peer.init_bitfield(8, 1);

    let recording = Arc::new(RecordingPeerEventListener::new());
    peer.set_event_listener(recording.clone());

    // 1. Direct event listener calls & verification
    recording.on_peer_have("192.168.1.50", 3);
    assert_eq!(recording.haves.lock().unwrap().len(), 1);
    assert_eq!(recording.haves.lock().unwrap()[0], ("192.168.1.50".to_string(), 3));

    recording.on_peer_bitfield("192.168.1.50", &[0b1010_0000]);
    assert_eq!(recording.bitfields.lock().unwrap().len(), 1);
    assert_eq!(
        recording.bitfields.lock().unwrap()[0],
        ("192.168.1.50".to_string(), vec![0b1010_0000])
    );

    recording.on_block_received("192.168.1.50", 1, 0, &[1, 2, 3, 4]);
    assert_eq!(recording.blocks.lock().unwrap().len(), 1);
    assert_eq!(
        recording.blocks.lock().unwrap()[0],
        ("192.168.1.50".to_string(), 1, 0, 4)
    );

    recording.on_peer_choked("192.168.1.50");
    assert_eq!(recording.chokes.lock().unwrap().len(), 1);

    recording.on_peer_unchoked("192.168.1.50");
    assert_eq!(recording.unchokes.lock().unwrap().len(), 1);

    // 2. Peer close emits on_peer_disconnected
    peer.close();
    assert_eq!(recording.disconnects.lock().unwrap().len(), 1);
    assert_eq!(recording.disconnects.lock().unwrap()[0], "192.168.1.50".to_string());
}

#[test]
fn test_peer_message_dispatches_to_event_listener() {
    let (mock_sock, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(mock_sock));
    let mut peer = Peer::new_with_socket("10.0.0.1".to_string(), 51413, socket);
    peer.init_bitfield(8, 1);

    let recording = Arc::new(RecordingPeerEventListener::new());
    peer.set_event_listener(recording.clone());

    let selector = Arc::new(RarestFirstSelector);
    let temp_dir = std::env::temp_dir().join("solid_phase2_msg_test");
    let config = SessionConfig::default();
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0u8; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &temp_dir,
        config,
    )
    .expect("context creation succeeds");
    ctx.init_pieces(8, 16384);

    // Dispatch Choke message
    let _ = peer.handle_peer_message(PeerMessage::Choke, &mut ctx);
    assert_eq!(recording.chokes.lock().unwrap().len(), 1);

    // Dispatch Unchoke message
    let _ = peer.handle_peer_message(PeerMessage::Unchoke, &mut ctx);
    assert_eq!(recording.unchokes.lock().unwrap().len(), 1);

    // Dispatch Have message
    let _ = peer.handle_peer_message(PeerMessage::Have(5), &mut ctx);
    assert_eq!(recording.haves.lock().unwrap().len(), 1);
    assert_eq!(recording.haves.lock().unwrap()[0], ("10.0.0.1".to_string(), 5));

    // Dispatch Bitfield message
    let _ = peer.handle_peer_message(PeerMessage::Bitfield(&[0b1111_0000]), &mut ctx);
    assert_eq!(recording.bitfields.lock().unwrap().len(), 1);
    assert_eq!(
        recording.bitfields.lock().unwrap()[0],
        ("10.0.0.1".to_string(), vec![0b1111_0000])
    );
}

#[test]
fn test_context_event_listener_adapter() {
    let selector = Arc::new(RarestFirstSelector);
    let temp_dir = std::env::temp_dir().join("solid_phase2_adapter_test");
    let config = SessionConfig::default();
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0u8; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &temp_dir,
        config,
    )
    .expect("context creation succeeds");
    ctx.init_pieces(4, 16384);

    let (mock_sock, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(mock_sock));
    let peer = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "10.0.0.2".to_string(),
        6881,
        socket,
    )));

    // Add peer to context
    assert!(ctx.add_peer(peer.clone()));
    assert_eq!(ctx.connected_peer_count(), 1);

    let ctx_arc = Arc::new(std::sync::Mutex::new(ctx));
    let listener = ContextEventListener::new(ctx_arc.clone());

    // Notify peer disconnected via listener
    listener.on_peer_disconnected("10.0.0.2");
    assert_eq!(ctx_arc.lock().unwrap().connected_peer_count(), 0);
}
