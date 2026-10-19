use bittorrent_rs::{
    MockSocket, Peer, PeerMessage, SequentialSelector, SessionConfig, Socket,
    TorrentSessionBuilder,
    internals::TorrentContext,
    io_traits::AsyncSocket,
    network::tracker::{TrackerAnnounceContext, TrackerEvent},
    network::webrtc::{
        DataChannelState, MockWebSocketSignalingChannel, SdpDescription, WebRtcSocketAdapter,
        WebTorrentAnnouncer, WebTorrentBridge, WebTorrentIncomingMessage, create_webtorrent_answer,
        create_webtorrent_offer,
    },
    util::info_hash_to_string,
};
use std::sync::{Arc, Mutex};

fn create_mock_context() -> TorrentContext {
    let mut config = SessionConfig::default();
    config.listen_port = 6881;
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xBB; 20],
        vec!["wss://tracker.openwebtorrent.com".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    )
    .expect("Bootstrap failed");

    ctx.init_pieces(4, 16384);
    ctx.storage = Arc::new(bittorrent_rs::MemStorage::new(4 * 16384));
    ctx
}

#[test]
fn test_webtorrent_announce_request_with_offers_and_events() {
    let announcer = WebTorrentAnnouncer::new("wss://tracker.btorrent.xyz");

    let tracker_ctx = TrackerAnnounceContext {
        info_hash: vec![0x42; 20],
        peer_id: "-WT0001-abcdef123456".to_string(),
        port: 0,
        ip: "0.0.0.0".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "wss://tracker.btorrent.xyz".to_string(),
        event: TrackerEvent::Started,
        interval: 120,
        min_interval: 60,
        downloaded: 1024,
        uploaded: 2048,
        left: 32768,
        ipv6: None,
        http_client: Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let offer1 = create_webtorrent_offer("offer-alpha-1");
    let offer2 = create_webtorrent_offer("offer-alpha-2");

    let payload = announcer
        .build_announce_payload_with_offers(&tracker_ctx, vec![offer1, offer2], Some("started"))
        .expect("build announce payload");

    assert!(payload.contains("\"action\":\"announce\""));
    assert!(payload.contains("\"event\":\"started\""));
    assert!(payload.contains("\"peer_id\":\"-WT0001-abcdef123456\""));
    assert!(payload.contains("\"offer_id\":\"offer-alpha-1\""));
    assert!(payload.contains("\"offer_id\":\"offer-alpha-2\""));
    assert!(payload.contains("\"uploaded\":2048"));
    assert!(payload.contains("\"downloaded\":1024"));
    assert!(payload.contains("\"left\":32768"));

    // Test answer payload construction
    let answer = create_webtorrent_answer("offer-alpha-1", "-WT0002-browserpeer99");
    let answer_payload = announcer
        .build_answer_payload(&tracker_ctx.info_hash, &tracker_ctx.peer_id, &answer)
        .expect("build answer payload");

    assert!(answer_payload.contains("\"action\":\"announce\""));
    assert!(answer_payload.contains("\"offer_id\":\"offer-alpha-1\""));
    assert!(answer_payload.contains("\"to_peer_id\":\"-WT0002-browserpeer99\""));
    assert!(answer_payload.contains("\"type\":\"answer\""));
}

#[test]
fn test_webtorrent_scrape_request_and_response_parsing() {
    let announcer = WebTorrentAnnouncer::new("wss://tracker.fastcast.nz");

    let hash1 = vec![0x11; 20];
    let hash2 = vec![0x22; 20];

    let scrape_payload = announcer
        .build_scrape_payload(&[&hash1, &hash2])
        .expect("build scrape payload");

    assert!(scrape_payload.contains("\"action\":\"scrape\""));
    assert!(scrape_payload.contains(&info_hash_to_string(&hash1)));
    assert!(scrape_payload.contains(&info_hash_to_string(&hash2)));

    // Parse valid scrape response
    let hex1 = info_hash_to_string(&hash1);
    let hex2 = info_hash_to_string(&hash2);
    let json_resp = format!(
        r#"{{
            "action": "scrape",
            "files": {{
                "{}": {{ "complete": 15, "incomplete": 3, "downloaded": 120 }},
                "{}": {{ "complete": 5, "incomplete": 1, "downloaded": 40 }}
            }}
        }}"#,
        hex1, hex2
    );

    let scrape = announcer
        .parse_scrape_response(&json_resp)
        .expect("parse scrape response");

    assert_eq!(scrape.complete, 20);
    assert_eq!(scrape.incomplete, 4);
    assert_eq!(scrape.downloaded, 160);

    // Parse scrape failure
    let fail_json = r#"{"action":"scrape","failure reason":"Rate limit exceeded"}"#;
    let err = announcer.parse_scrape_response(fail_json);
    assert!(err.is_err());
}

#[test]
fn test_webtorrent_incoming_message_demuxing() {
    let announcer = WebTorrentAnnouncer::new("wss://tracker.openwebtorrent.com");

    // 1. Inbound Offer
    let json_offer = r#"{
        "action": "announce",
        "info_hash": "0123456789abcdef0123456789abcdef01234567",
        "peer_id": "browser-peer-1",
        "offer_id": "offer-in-001",
        "offer": {
            "type": "offer",
            "sdp": "v=0\r\no=- 123 2 IN IP4 127.0.0.1\r\n"
        }
    }"#;
    let msg1 = announcer.parse_incoming_message(json_offer).unwrap();
    match msg1 {
        WebTorrentIncomingMessage::InboundOffer {
            offer_id,
            peer_id,
            offer,
            ..
        } => {
            assert_eq!(offer_id, "offer-in-001");
            assert_eq!(peer_id, "browser-peer-1");
            assert_eq!(offer.sdp_type, "offer");
        }
        _ => panic!("Expected InboundOffer"),
    }

    // 2. Inbound Answer
    let json_answer = r#"{
        "action": "announce",
        "info_hash": "0123456789abcdef0123456789abcdef01234567",
        "peer_id": "browser-peer-2",
        "offer_id": "offer-out-002",
        "answer": {
            "type": "answer",
            "sdp": "v=0\r\no=- 456 2 IN IP4 127.0.0.1\r\n"
        }
    }"#;
    let msg2 = announcer.parse_incoming_message(json_answer).unwrap();
    match msg2 {
        WebTorrentIncomingMessage::InboundAnswer {
            offer_id,
            peer_id,
            answer,
            ..
        } => {
            assert_eq!(offer_id, "offer-out-002");
            assert_eq!(peer_id, "browser-peer-2");
            assert_eq!(answer.sdp_type, "answer");
        }
        _ => panic!("Expected InboundAnswer"),
    }

    // 3. Peer Announcement
    let json_peer = r#"{
        "action": "announce",
        "interval": 120,
        "complete": 8,
        "incomplete": 2,
        "peer_id": "browser-peer-3"
    }"#;
    let msg3 = announcer.parse_incoming_message(json_peer).unwrap();
    match msg3 {
        WebTorrentIncomingMessage::PeerAnnouncement(resp) => {
            assert_eq!(resp.complete, 8);
            assert_eq!(resp.incomplete, 2);
            assert_eq!(resp.peer_list.len(), 1);
            assert_eq!(resp.peer_list[0].ip, "webrtc:browser-peer");
        }
        _ => panic!("Expected PeerAnnouncement"),
    }

    // 4. Failure
    let json_fail = r#"{"action":"announce","failure reason":"Invalid info_hash"}"#;
    let msg4 = announcer.parse_incoming_message(json_fail).unwrap();
    match msg4 {
        WebTorrentIncomingMessage::Error(reason) => {
            assert_eq!(reason, "Invalid info_hash");
        }
        _ => panic!("Expected Error"),
    }
}

#[test]
fn test_webtorrent_bridge_coordinator_offer_and_answer_handshake() {
    let bridge = WebTorrentBridge::new();

    // 1. Client generates offers
    let offers = bridge.generate_offers(3);
    assert_eq!(offers.len(), 3);
    assert_eq!(
        bridge
            .offers_sent
            .load(std::sync::atomic::Ordering::Relaxed),
        3
    );

    let offer1_id = &offers[0].offer_id;

    // 2. Simulated browser peer answers offer 1
    let dummy_answer = SdpDescription {
        sdp_type: "answer".to_string(),
        sdp: "v=0...".to_string(),
    };

    let _adapter = bridge
        .handle_inbound_answer(offer1_id, "peer-browser-abc", &dummy_answer)
        .expect("handle answer");

    assert_eq!(
        bridge
            .answers_received
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );

    // Socket is registered in bridge active peers
    let found_socket = bridge.get_peer_socket("webrtc:peer-browser-abc");
    assert!(found_socket.is_some());
    assert_eq!(found_socket.unwrap().state(), DataChannelState::Open);

    // Unknown offer fails
    let err = bridge.handle_inbound_answer("offer-nonexistent", "peer-xyz", &dummy_answer);
    assert!(err.is_err());

    let stats = bridge.stats();
    assert_eq!(stats.offers_sent, 3);
    assert_eq!(stats.answers_received, 1);
    assert_eq!(stats.active_webrtc_peers, 1);
}

#[test]
fn test_webtorrent_bridge_coordinator_inbound_browser_offer() {
    let bridge = WebTorrentBridge::new();

    let dummy_offer = SdpDescription {
        sdp_type: "offer".to_string(),
        sdp: "v=0...".to_string(),
    };

    let (answer, socket) = bridge
        .handle_inbound_offer("browser-offer-99", "peer-browser-xyz", &dummy_offer)
        .expect("handle inbound offer");

    assert_eq!(answer.offer_id, "browser-offer-99");
    assert_eq!(answer.to_peer_id, Some("peer-browser-xyz".to_string()));
    assert_eq!(answer.answer.sdp_type, "answer");
    assert_eq!(socket.state(), DataChannelState::Open);

    let stats = bridge.stats();
    assert_eq!(stats.offers_received, 1);
    assert_eq!(stats.answers_sent, 1);
    assert_eq!(stats.active_webrtc_peers, 1);

    // Clean up peer socket
    bridge.remove_peer_socket("webrtc:peer-browser-xyz");
    assert!(bridge.get_peer_socket("webrtc:peer-browser-xyz").is_none());
    assert_eq!(bridge.stats().active_webrtc_peers, 0);
}

#[test]
fn test_webtorrent_announcer_with_mock_websocket_signaling() {
    let mock_channel = Arc::new(MockWebSocketSignalingChannel::new());

    // Script mock responses
    mock_channel.push_response(
        r#"{
        "action": "announce",
        "interval": 180,
        "complete": 25,
        "incomplete": 5,
        "peer_id": "browser-peer-alpha"
    }"#,
    );

    mock_channel.push_response(
        r#"{
        "action": "scrape",
        "files": {
            "0123456789abcdef0123456789abcdef01234567": {
                "complete": 50,
                "incomplete": 10,
                "downloaded": 500
            }
        }
    }"#,
    );

    let mut announcer =
        WebTorrentAnnouncer::with_client("wss://tracker.openwebtorrent.com", mock_channel.clone());

    let tracker_ctx = TrackerAnnounceContext {
        info_hash: vec![0x12; 20],
        peer_id: "-WT0001-abcdef123456".to_string(),
        port: 0,
        ip: "0.0.0.0".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "wss://tracker.openwebtorrent.com".to_string(),
        event: TrackerEvent::None,
        interval: 120,
        min_interval: 60,
        downloaded: 0,
        uploaded: 0,
        left: 5000,
        ipv6: None,
        http_client: Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    use bittorrent_rs::network::announcer::Announcer;

    // 1. Announce via mock WebSocket channel
    let ann_resp = announcer.announce(&tracker_ctx).expect("announce");
    assert_eq!(ann_resp.interval, 180);
    assert_eq!(ann_resp.complete, 25);
    assert_eq!(ann_resp.incomplete, 5);
    assert_eq!(ann_resp.peer_list.len(), 1);
    assert!(ann_resp.peer_list[0].is_webrtc());

    // 2. Scrape via mock WebSocket channel
    let scrape_resp = announcer.scrape(&tracker_ctx).expect("scrape");
    assert_eq!(scrape_resp.complete, 50);
    assert_eq!(scrape_resp.incomplete, 10);
    assert_eq!(scrape_resp.downloaded, 500);

    // Verify frames sent
    let sent_frames = mock_channel.get_sent_messages();
    assert_eq!(sent_frames.len(), 2);
    assert!(sent_frames[0].contains("\"action\":\"announce\""));
    assert!(sent_frames[1].contains("\"action\":\"scrape\""));
}

#[test]
fn test_cross_swarm_bridge_piece_distribution() {
    let ctx = create_mock_context();
    let ctx_arc = Arc::new(Mutex::new(ctx));

    // 1. Setup a standard TCP peer and a WebRTC browser peer
    let (tcp_socket, _in_tx, _out_rx) = MockSocket::new();
    let mut tcp_peer = Peer::new_with_socket(
        "192.168.1.50".to_string(),
        6881,
        Arc::new(Socket::Mock(tcp_socket)),
    );
    tcp_peer.set_torrent_context(ctx_arc.clone());
    tcp_peer.connected = true;

    // Create a paired WebRTC Data Channel for browser peer
    let (bridge_adapter, browser_adapter) = WebRtcSocketAdapter::create_pair("webtorrent");
    let mut browser_peer = Peer::new_with_socket(
        "webrtc:browser-peer-99".to_string(),
        0,
        Arc::new(Socket::WebRtc(bridge_adapter)),
    );
    browser_peer.set_torrent_context(ctx_arc.clone());
    browser_peer.connected = true;
    browser_peer.connection_type = "WebRTC".to_string();

    {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard
            .peer_swarm
            .write()
            .unwrap()
            .insert("192.168.1.50".to_string(), Arc::new(Mutex::new(tcp_peer)));
        ctx_guard.peer_swarm.write().unwrap().insert(
            "webrtc:browser-peer-99".to_string(),
            Arc::new(Mutex::new(browser_peer)),
        );
    }

    // 2. Simulate piece download from TCP peer: write block to storage, mark piece complete
    let block_data = vec![0xEE; 16384];
    {
        let mut ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard.storage.write_block(0, &block_data).unwrap();
        // Set bitfield bit for piece 0
        ctx_guard.bitfield[0] |= 0b1000_0000;
        ctx_guard.record_webrtc_bridge_transfer(false, 16384);
    }

    // 3. Browser peer requests piece 0 block 0 over WebRTC Data Channel
    let req_msg = PeerMessage::Request {
        index: 0,
        begin: 0,
        length: 16384,
    };
    let req_bytes = req_msg.encode();
    futures::executor::block_on(browser_adapter.write(&req_bytes)).expect("browser write request");

    // Read and verify the request on the bridge adapter
    let mut read_buf = [0u8; 17];
    let n = futures::executor::block_on(
        ctx_arc
            .lock()
            .unwrap()
            .peer_swarm
            .read()
            .unwrap()
            .get("webrtc:browser-peer-99")
            .unwrap()
            .lock()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .read_message(&mut read_buf),
    )
    .unwrap();

    match n {
        PeerMessage::Request {
            index,
            begin,
            length,
        } => {
            assert_eq!(index, 0);
            assert_eq!(begin, 0);
            assert_eq!(length, 16384);
        }
        _ => panic!("Expected Request message from browser peer"),
    }

    // Bridge serves piece 0 from local storage to browser peer
    let piece_msg = PeerMessage::Piece {
        index: 0,
        begin: 0,
        block: &block_data,
    };
    let piece_wire = piece_msg.encode();
    futures::executor::block_on(
        ctx_arc
            .lock()
            .unwrap()
            .peer_swarm
            .read()
            .unwrap()
            .get("webrtc:browser-peer-99")
            .unwrap()
            .lock()
            .unwrap()
            .network
            .as_ref()
            .unwrap()
            .write_message(PeerMessage::Piece {
                index: 0,
                begin: 0,
                block: &block_data,
            }),
    )
    .unwrap();

    // Browser receives the piece over WebRTC
    let mut incoming_block = vec![0u8; piece_wire.len()];
    let read_len = futures::executor::block_on(browser_adapter.read(&mut incoming_block)).unwrap();
    assert_eq!(read_len, piece_wire.len());

    // Verify bridge metrics
    let stats = {
        let ctx_guard = ctx_arc.lock().unwrap();
        ctx_guard.webtorrent_bridge_stats()
    };
    assert_eq!(stats.tcp_to_webrtc_bytes, 16384);
}

#[test]
fn test_session_builder_and_config_webtorrent_bridge() {
    let default_config = SessionConfig::default();
    assert!(default_config.enable_webtorrent);
    assert!(default_config.webtorrent_bridge_mode);
    assert_eq!(default_config.webtorrent_max_web_peers, 25);

    let temp_dir = std::env::temp_dir().join("bt_webtorrent_builder_test");
    let _ = std::fs::create_dir_all(&temp_dir);

    let builder = TorrentSessionBuilder::from_magnet(
        "magnet:?xt=urn:btih:0123456789012345678901234567890123456789",
        &temp_dir,
    )
    .enable_webtorrent(true)
    .webtorrent_bridge_mode(true)
    .webtorrent_max_web_peers(40);

    let session = builder.build().expect("Build session");
    let (enabled, bridge, max_peers) = {
        let ctx = session.context.lock().unwrap();
        (
            ctx.is_webtorrent_enabled(),
            ctx.is_webtorrent_bridge_enabled(),
            ctx.config.webtorrent_max_web_peers,
        )
    };

    assert!(enabled);
    assert!(bridge);
    assert_eq!(max_peers, 40);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
