use bittorrent_rs::{
    Socket,
    io_traits::AsyncSocket,
    network::announcer::{AnnouncerEnum, AnnouncerFactory},
    network::peer::Peer,
    network::peer_message::PeerMessage,
    network::tracker::{TrackerAnnounceContext, TrackerEvent},
    network::webrtc::{
        DataChannelState, WebRtcSocketAdapter, WebTorrentAnnouncer, create_webtorrent_answer,
        create_webtorrent_offer,
    },
};
use std::sync::Arc;

#[test]
fn test_webrtc_socket_adapter_lifecycle() {
    let adapter = WebRtcSocketAdapter::new("webtorrent");
    assert_eq!(adapter.label(), "webtorrent");
    assert_eq!(adapter.state(), DataChannelState::Open);

    // Inject data and read it
    adapter.inject_incoming(b"hello webrtc");
    let mut buf = [0u8; 12];
    let n = futures::executor::block_on(adapter.read(&mut buf)).unwrap();
    assert_eq!(n, 12);
    assert_eq!(&buf, b"hello webrtc");

    // Close adapter
    adapter.close();
    assert_eq!(adapter.state(), DataChannelState::Closed);

    let mut buf2 = [0u8; 4];
    let err = futures::executor::block_on(adapter.read(&mut buf2));
    assert!(err.is_err());
}

#[test]
fn test_webrtc_socket_adapter_loopback_bidirectional_transfer() {
    let (client, remote) = WebRtcSocketAdapter::create_pair("webtorrent");

    // Client -> Remote
    let client_msg = b"PING_FROM_CLIENT";
    let sent = futures::executor::block_on(client.write(client_msg)).unwrap();
    assert_eq!(sent, client_msg.len());

    let mut recv_buf = vec![0u8; 16];
    let read_bytes = futures::executor::block_on(remote.read(&mut recv_buf)).unwrap();
    assert_eq!(read_bytes, client_msg.len());
    assert_eq!(&recv_buf[..read_bytes], client_msg);

    // Remote -> Client
    let remote_msg = b"PONG_FROM_REMOTE";
    let sent2 = futures::executor::block_on(remote.write(remote_msg)).unwrap();
    assert_eq!(sent2, remote_msg.len());

    let mut recv_buf2 = vec![0u8; 16];
    let read_bytes2 = futures::executor::block_on(client.read(&mut recv_buf2)).unwrap();
    assert_eq!(read_bytes2, remote_msg.len());
    assert_eq!(&recv_buf2[..read_bytes2], remote_msg);

    // Stats verification
    let client_stats = client.get_stats();
    assert_eq!(client_stats.bytes_sent, 16);
    assert_eq!(client_stats.messages_sent, 1);
    assert_eq!(client_stats.bytes_received, 16);
    assert_eq!(client_stats.messages_received, 1);

    let remote_stats = remote.get_stats();
    assert_eq!(remote_stats.bytes_sent, 16);
    assert_eq!(remote_stats.messages_sent, 1);
    assert_eq!(remote_stats.bytes_received, 16);
    assert_eq!(remote_stats.messages_received, 1);
}

#[test]
fn test_webtorrent_sdp_offer_and_answer_generation() {
    let offer = create_webtorrent_offer("offer-test-42");
    assert_eq!(offer.offer_id, "offer-test-42");
    assert_eq!(offer.offer.sdp_type, "offer");
    assert!(offer.offer.sdp.contains("webrtc-datachannel"));
    assert!(offer.offer.sdp.contains("offer-test-42"));

    let answer = create_webtorrent_answer("offer-test-42", "peer-browser-99");
    assert_eq!(answer.offer_id, "offer-test-42");
    assert_eq!(answer.to_peer_id, Some("peer-browser-99".to_string()));
    assert_eq!(answer.answer.sdp_type, "answer");
    assert!(answer.answer.sdp.contains("webrtc-datachannel"));
}

#[test]
fn test_webtorrent_announce_request_payload_construction() {
    let announcer = WebTorrentAnnouncer::new("wss://tracker.openwebtorrent.com");

    let info_hash = vec![0x12; 20];
    let tracker_ctx = TrackerAnnounceContext {
        info_hash,
        peer_id: "-BR0110-123456789012".to_string(),
        port: 0,
        ip: "0.0.0.0".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "wss://tracker.openwebtorrent.com".to_string(),
        event: TrackerEvent::Started,
        interval: 120,
        min_interval: 60,
        downloaded: 1048576,
        uploaded: 524288,
        left: 2097152,
        ipv6: None,
        http_client: Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let payload = announcer
        .build_announce_payload(&tracker_ctx, "offer-abc-123")
        .unwrap();
    assert!(payload.contains("\"action\":\"announce\""));
    assert!(payload.contains("\"peer_id\":\"-BR0110-123456789012\""));
    assert!(payload.contains("\"offer_id\":\"offer-abc-123\""));
    assert!(payload.contains("\"uploaded\":524288"));
    assert!(payload.contains("\"downloaded\":1048576"));
    assert!(payload.contains("\"left\":2097152"));
}

#[test]
fn test_webtorrent_announce_response_parsing() {
    let announcer = WebTorrentAnnouncer::new("wss://tracker.fastcast.nz");

    // Success response
    let json_success = r#"{
        "action": "announce",
        "interval": 180,
        "complete": 10,
        "incomplete": 3,
        "peer_id": "browser-peer-xyz"
    }"#;

    let res = announcer.parse_announce_response(json_success).unwrap();
    assert!(!res.failure);
    assert_eq!(res.interval, 180);
    assert_eq!(res.complete, 10);
    assert_eq!(res.incomplete, 3);
    assert_eq!(res.peer_list.len(), 1);
    assert_eq!(res.peer_list[0].ip, "webrtc:browser-peer");
    assert_eq!(
        res.peer_list[0].peer_id,
        Some("browser-peer-xyz".to_string())
    );

    // Failure response
    let json_fail = r#"{
        "action": "announce",
        "failure reason": "Torrent not registered"
    }"#;

    let res_err = announcer.parse_announce_response(json_fail).unwrap();
    assert!(res_err.failure);
    assert_eq!(res_err.status_message, "Torrent not registered");
}

#[test]
fn test_announcer_factory_recognizes_websocket_trackers() {
    let ws_announcer = AnnouncerFactory::create("ws://tracker.btorrent.xyz:80/announce").unwrap();
    assert!(matches!(ws_announcer, AnnouncerEnum::WebTorrent(_)));

    let wss_announcer = AnnouncerFactory::create("wss://tracker.openwebtorrent.com").unwrap();
    assert!(matches!(wss_announcer, AnnouncerEnum::WebTorrent(_)));

    let trait_announcer = AnnouncerFactory::create_announcer("wss://tracker.fastcast.nz").unwrap();
    let tracker_ctx = TrackerAnnounceContext {
        info_hash: vec![0x33; 20],
        peer_id: "-BR0110-webtorrent01".to_string(),
        port: 0,
        ip: "0.0.0.0".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "wss://tracker.fastcast.nz".to_string(),
        event: TrackerEvent::None,
        interval: 120,
        min_interval: 60,
        downloaded: 0,
        uploaded: 0,
        left: 5000,
        ipv6: None,
        http_client: Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let mut dyn_announcer = trait_announcer;
    let res = dyn_announcer.announce(&tracker_ctx).unwrap();
    assert!(!res.failure);
    assert_eq!(res.status_message, "WebTorrent WebSocket signaling ready");
}

#[test]
fn test_peer_wire_protocol_over_webrtc_data_channel() {
    let (client_adapter, remote_adapter) = WebRtcSocketAdapter::create_pair("webtorrent");

    let client_peer = Peer::new_webrtc("browser-peer-01", client_adapter);
    assert_eq!(client_peer.connection_type, "WebRTC");

    let remote_peer = Peer::new_with_socket(
        "webrtc:desktop-client-02".to_string(),
        0,
        Arc::new(Socket::WebRtc(remote_adapter)),
    );
    assert_eq!(remote_peer.connection_type, "WebRTC");

    // Write BitTorrent protocol messages over WebRTC
    let test_have = PeerMessage::Have(7);
    let wire_bytes = test_have.encode();

    let write_res = futures::executor::block_on(client_peer.write(&wire_bytes));
    assert!(write_res.is_ok());

    let mut incoming = [0u8; 9];
    let net = remote_peer.network.unwrap();
    let read_msg = futures::executor::block_on(net.read_message(&mut incoming)).unwrap();
    match read_msg {
        PeerMessage::Have(idx) => assert_eq!(idx, 7),
        _ => panic!("Expected PeerMessage::Have(7)"),
    }
}
