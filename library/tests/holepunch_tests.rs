use bittorrent_rs::{
    MockSocket, Peer, PeerMessage, SequentialSelector, SessionConfig, Socket,
    TorrentSessionBuilder,
    internals::TorrentContext,
    network::holepunch::{
        HolepunchAddrType, HolepunchErrorCode, HolepunchMessage, HolepunchMessageType,
    },
    network::peer::PeerAction,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};

fn create_mock_context() -> TorrentContext {
    let mut config = SessionConfig::default();
    config.listen_port = 6881;
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xAA; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    ctx.init_pieces(4, 16384);
    ctx
}

fn create_mock_peer(ip: &str, port: u16, context: Arc<Mutex<TorrentContext>>) -> Peer {
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket(ip.to_string(), port, socket);
    peer.set_torrent_context(context);
    peer.connected = true;
    peer.remote_piece_bitfield = vec![0u8; 1];
    peer
}

#[test]
fn test_holepunch_encoding_decoding_ipv4() {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), 51413);

    // 1. Rendezvous
    let r_msg = HolepunchMessage::rendezvous(addr);
    assert_eq!(r_msg.msg_type, HolepunchMessageType::Rendezvous);
    assert_eq!(r_msg.addr_type(), HolepunchAddrType::IPv4);
    assert_eq!(r_msg.err_code, HolepunchErrorCode::None);
    let encoded_r = r_msg.encode();
    assert_eq!(encoded_r.len(), 12);
    assert_eq!(encoded_r[0], 0x00); // Rendezvous
    assert_eq!(encoded_r[1], 0x00); // IPv4
    assert_eq!(&encoded_r[2..6], &[192, 168, 1, 50]);
    assert_eq!(
        u16::from_be_bytes(encoded_r[6..8].try_into().unwrap()),
        51413
    );
    assert_eq!(u32::from_be_bytes(encoded_r[8..12].try_into().unwrap()), 0);

    let decoded_r = HolepunchMessage::decode(&encoded_r).expect("Decode Rendezvous IPv4");
    assert_eq!(decoded_r.msg_type, HolepunchMessageType::Rendezvous);
    assert_eq!(decoded_r.endpoint, addr);
    assert_eq!(decoded_r.err_code, HolepunchErrorCode::None);

    // 2. Connect
    let c_msg = HolepunchMessage::connect(addr);
    assert_eq!(c_msg.msg_type, HolepunchMessageType::Connect);
    let encoded_c = c_msg.encode();
    assert_eq!(encoded_c[0], 0x01); // Connect
    let decoded_c = HolepunchMessage::decode(&encoded_c).expect("Decode Connect IPv4");
    assert_eq!(decoded_c.msg_type, HolepunchMessageType::Connect);
    assert_eq!(decoded_c.endpoint, addr);

    // 3. Error
    let e_msg = HolepunchMessage::error(addr, HolepunchErrorCode::NoSuchPeer);
    assert_eq!(e_msg.msg_type, HolepunchMessageType::Error);
    assert_eq!(e_msg.err_code, HolepunchErrorCode::NoSuchPeer);
    let encoded_e = e_msg.encode();
    assert_eq!(encoded_e[0], 0x02); // Error
    assert_eq!(
        u32::from_be_bytes(encoded_e[8..12].try_into().unwrap()),
        0x01
    );
    let decoded_e = HolepunchMessage::decode(&encoded_e).expect("Decode Error IPv4");
    assert_eq!(decoded_e.msg_type, HolepunchMessageType::Error);
    assert_eq!(decoded_e.err_code, HolepunchErrorCode::NoSuchPeer);
}

#[test]
fn test_holepunch_encoding_decoding_ipv6() {
    let v6_addr = Ipv6Addr::new(0x2001, 0x0db8, 0x85a3, 0, 0, 0x8a2e, 0x0370, 0x7334);
    let addr = SocketAddr::new(IpAddr::V6(v6_addr), 6882);

    let c_msg = HolepunchMessage::connect(addr);
    assert_eq!(c_msg.addr_type(), HolepunchAddrType::IPv6);
    let encoded = c_msg.encode();
    assert_eq!(encoded.len(), 24);
    assert_eq!(encoded[0], 0x01); // Connect
    assert_eq!(encoded[1], 0x01); // IPv6
    assert_eq!(&encoded[2..18], &v6_addr.octets());
    assert_eq!(
        u16::from_be_bytes(encoded[18..20].try_into().unwrap()),
        6882
    );
    assert_eq!(u32::from_be_bytes(encoded[20..24].try_into().unwrap()), 0);

    let decoded = HolepunchMessage::decode(&encoded).expect("Decode Connect IPv6");
    assert_eq!(decoded.msg_type, HolepunchMessageType::Connect);
    assert_eq!(decoded.endpoint, addr);
    assert_eq!(decoded.err_code, HolepunchErrorCode::None);

    let e_msg = HolepunchMessage::error(addr, HolepunchErrorCode::NotConnected);
    let encoded_e = e_msg.encode();
    let decoded_e = HolepunchMessage::decode(&encoded_e).expect("Decode Error IPv6");
    assert_eq!(decoded_e.err_code, HolepunchErrorCode::NotConnected);
}

#[test]
fn test_holepunch_decode_invalid_inputs() {
    // Truncated buffer (< 2 bytes)
    assert!(HolepunchMessage::decode(&[]).is_err());
    assert!(HolepunchMessage::decode(&[0x00]).is_err());

    // Truncated IPv4 (< 12 bytes)
    assert!(HolepunchMessage::decode(&[0x00, 0x00, 1, 2, 3, 4, 0, 80]).is_err());

    // Truncated IPv6 (< 24 bytes)
    assert!(
        HolepunchMessage::decode(&[0x00, 0x01, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]).is_err()
    );

    // Invalid message type (e.g. 0x03)
    let mut valid_ipv4 = HolepunchMessage::connect(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        8080,
    ))
    .encode();
    valid_ipv4[0] = 0x03;
    assert!(HolepunchMessage::decode(&valid_ipv4).is_err());

    // Invalid addr type (e.g. 0x02)
    valid_ipv4[0] = 0x01; // Restore Connect
    valid_ipv4[1] = 0x02; // Invalid addr_type
    assert!(HolepunchMessage::decode(&valid_ipv4).is_err());

    // Unknown error code (e.g. 0x05)
    valid_ipv4[1] = 0x00; // Restore IPv4
    valid_ipv4[8..12].copy_from_slice(&5u32.to_be_bytes());
    assert!(HolepunchMessage::decode(&valid_ipv4).is_err());
}

#[test]
fn test_holepunch_relay_rendezvous_success() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    // Target peer in swarm
    let target_ip = "192.168.1.100";
    let target_port = 50000;
    let target_peer = Arc::new(Mutex::new({
        let mut p = create_mock_peer(target_ip, target_port, ctx_arc.clone());
        p.supports_holepunch = true;
        p
    }));

    {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .peer_swarm
            .write()
            .unwrap()
            .insert(target_ip.to_string(), target_peer);
    }

    let target_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), target_port);
    let initiator_ip = "10.0.0.1";
    let initiator_port = 45000;

    let res = {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard.handle_holepunch_rendezvous(initiator_ip, initiator_port, target_addr)
    };

    assert!(res.is_ok());
    let (connect_initiator, connect_target) = res.unwrap();

    assert_eq!(connect_initiator.msg_type, HolepunchMessageType::Connect);
    assert_eq!(connect_initiator.endpoint, target_addr);

    assert_eq!(connect_target.msg_type, HolepunchMessageType::Connect);
    assert_eq!(
        connect_target.endpoint,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), initiator_port)
    );
}

#[test]
fn test_holepunch_relay_rendezvous_errors() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    // 1. NoSelf: Target endpoint is relaying peer itself (127.0.0.1:6881)
    let loopback_relay = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6881);
    {
        let ctx_guard = ctx_arc.lock().unwrap();
        let err = ctx_guard
            .handle_holepunch_rendezvous("10.0.0.1", 45000, loopback_relay)
            .unwrap_err();
        assert_eq!(err, HolepunchErrorCode::NoSelf);
    }

    // 2. NotConnected: Target peer not in swarm
    let missing_target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 200)), 51413);
    {
        let ctx_guard = ctx_arc.lock().unwrap();
        let err = ctx_guard
            .handle_holepunch_rendezvous("10.0.0.1", 45000, missing_target)
            .unwrap_err();
        assert_eq!(err, HolepunchErrorCode::NotConnected);
    }

    // 3. NoSupport: Target peer connected, but does NOT support ut_holepunch
    let target_ip = "192.168.1.150";
    let target_port = 50000;
    let unsupported_peer = Arc::new(Mutex::new({
        let mut p = create_mock_peer(target_ip, target_port, ctx_arc.clone());
        p.supports_holepunch = false;
        p
    }));

    {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .peer_swarm
            .write()
            .unwrap()
            .insert(target_ip.to_string(), unsupported_peer);
    }

    let unsupported_target =
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 150)), target_port);
    {
        let ctx_guard = ctx_arc.lock().unwrap();
        let err = ctx_guard
            .handle_holepunch_rendezvous("10.0.0.1", 45000, unsupported_target)
            .unwrap_err();
        assert_eq!(err, HolepunchErrorCode::NoSupport);
    }
}

#[test]
fn test_peer_handle_holepunch_rendezvous_and_connect_actions() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    // Target peer in swarm with holepunch support
    let target_ip = "192.168.1.100";
    let target_port = 50000;
    let target_peer = Arc::new(Mutex::new({
        let mut p = create_mock_peer(target_ip, target_port, ctx_arc.clone());
        p.supports_holepunch = true;
        p
    }));

    {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .peer_swarm
            .write()
            .unwrap()
            .insert(target_ip.to_string(), target_peer);
    }

    // Initiator peer sending Rendezvous to relay
    let initiator_ip = "10.0.0.1";
    let initiator_port = 45000;
    let mut relay_peer = create_mock_peer(initiator_ip, initiator_port, ctx_arc.clone());
    relay_peer
        .extension_ids
        .insert("ut_holepunch".to_string(), 3);

    let rendezvous_payload = HolepunchMessage::rendezvous(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
        target_port,
    ))
    .encode();

    let ext_msg = PeerMessage::Extended {
        ext_id: 3,
        payload: &rendezvous_payload,
    };

    let actions = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        relay_peer
            .handle_peer_message(ext_msg, &mut ctx_guard)
            .expect("Handle rendezvous")
    };

    // Relay must generate two SendHolepunch actions (one for initiator, one for target)
    assert_eq!(actions.len(), 2);
    match &actions[0] {
        PeerAction::SendHolepunch { target_ip, message } => {
            assert_eq!(target_ip, initiator_ip);
            assert_eq!(message.msg_type, HolepunchMessageType::Connect);
        }
        _ => panic!("Expected SendHolepunch for initiator"),
    }
    match &actions[1] {
        PeerAction::SendHolepunch { target_ip, message } => {
            assert_eq!(target_ip, "192.168.1.100");
            assert_eq!(message.msg_type, HolepunchMessageType::Connect);
        }
        _ => panic!("Expected SendHolepunch for target"),
    }

    let (r_rx, c_tx) = {
        let ctx_guard = ctx_arc.lock().unwrap();
        (
            ctx_guard
                .holepunch_rendezvous_received
                .load(std::sync::atomic::Ordering::Relaxed),
            ctx_guard
                .holepunch_connect_sent
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    };
    assert_eq!(r_rx, 1);
    assert_eq!(c_tx, 2);
}

#[test]
fn test_peer_handle_holepunch_connect_dispatches_connection() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    let mut peer = create_mock_peer("10.0.0.1", 45000, ctx_arc.clone());
    peer.extension_ids.insert("ut_holepunch".to_string(), 3);

    let connect_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 5)), 6885);
    let connect_payload = HolepunchMessage::connect(connect_addr).encode();

    let ext_msg = PeerMessage::Extended {
        ext_id: 3,
        payload: &connect_payload,
    };

    let actions = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer.handle_peer_message(ext_msg, &mut ctx_guard)
            .expect("Handle connect")
    };

    assert_eq!(actions.len(), 1);
    match actions[0] {
        PeerAction::HolepunchConnect(addr) => {
            assert_eq!(addr, connect_addr);
        }
        _ => panic!("Expected HolepunchConnect action"),
    }

    let c_rx = {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .holepunch_connect_received
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    assert_eq!(c_rx, 1);

    // If peer is already in swarm, connect action is ignored
    {
        let connected_peer = Arc::new(Mutex::new(create_mock_peer(
            "172.16.0.5",
            6885,
            ctx_arc.clone(),
        )));
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .peer_swarm
            .write()
            .unwrap()
            .insert("172.16.0.5".to_string(), connected_peer);
    }

    let actions2 = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer.handle_peer_message(
            PeerMessage::Extended {
                ext_id: 3,
                payload: &connect_payload,
            },
            &mut ctx_guard,
        )
        .expect("Handle duplicate connect")
    };
    assert!(actions2.is_empty());
}

#[test]
fn test_peer_initiate_holepunch_helper() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    let relay_peer = create_mock_peer("10.0.0.1", 45000, ctx_arc.clone());
    let target_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)), 51413);

    let action = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        relay_peer.initiate_holepunch(target_addr, &mut ctx_guard)
    };

    match action {
        PeerAction::SendHolepunch { target_ip, message } => {
            assert_eq!(target_ip, "10.0.0.1");
            assert_eq!(message.msg_type, HolepunchMessageType::Rendezvous);
            assert_eq!(message.endpoint, target_addr);
        }
        _ => panic!("Expected SendHolepunch action"),
    }

    let r_sent = {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .holepunch_rendezvous_sent
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    assert_eq!(r_sent, 1);
}

#[test]
fn test_peer_handle_holepunch_rendezvous_error_response() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    // Relay peer, but requested target is NOT in swarm
    let initiator_ip = "10.0.0.1";
    let mut relay_peer = create_mock_peer(initiator_ip, 45000, ctx_arc.clone());
    relay_peer
        .extension_ids
        .insert("ut_holepunch".to_string(), 3);

    let rendezvous_payload = HolepunchMessage::rendezvous(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 99)),
        50000,
    ))
    .encode();

    let ext_msg = PeerMessage::Extended {
        ext_id: 3,
        payload: &rendezvous_payload,
    };

    let actions = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        relay_peer
            .handle_peer_message(ext_msg, &mut ctx_guard)
            .expect("Handle rendezvous to unknown peer")
    };

    // Relay generates an error response back to initiator
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        PeerAction::SendHolepunch { target_ip, message } => {
            assert_eq!(target_ip, initiator_ip);
            assert_eq!(message.msg_type, HolepunchMessageType::Error);
            assert_eq!(message.err_code, HolepunchErrorCode::NotConnected);
        }
        _ => panic!("Expected SendHolepunch with Error"),
    }

    let err_sent = {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .holepunch_error_sent
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    assert_eq!(err_sent, 1);
}

#[test]
fn test_peer_handle_holepunch_error_incoming() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    let mut peer = create_mock_peer("10.0.0.1", 45000, ctx_arc.clone());
    peer.extension_ids.insert("ut_holepunch".to_string(), 3);

    let err_payload = HolepunchMessage::error(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 99)), 50000),
        HolepunchErrorCode::NoSupport,
    )
    .encode();

    let ext_msg = PeerMessage::Extended {
        ext_id: 3,
        payload: &err_payload,
    };

    let actions = {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        peer.handle_peer_message(ext_msg, &mut ctx_guard)
            .expect("Handle incoming error")
    };

    assert!(actions.is_empty());
    let err_rx = {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .holepunch_error_received
            .load(std::sync::atomic::Ordering::Relaxed)
    };
    assert_eq!(err_rx, 1);
}

#[test]
fn test_session_config_and_builder_holepunch() {
    let default_config = SessionConfig::default();
    assert!(default_config.enable_holepunch);

    let temp_dir = std::env::temp_dir().join("bt_holepunch_builder_test");
    let _ = std::fs::create_dir_all(&temp_dir);
    let builder = TorrentSessionBuilder::from_magnet(
        "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
        &temp_dir,
    )
    .enable_holepunch(false);
    let session = builder.build().expect("Build session");
    let enable_hp = {
        let ctx = session.context.lock().unwrap();
        ctx.config.enable_holepunch
    };
    assert!(!enable_hp);
    let _ = std::fs::remove_dir_all(&temp_dir);
}
