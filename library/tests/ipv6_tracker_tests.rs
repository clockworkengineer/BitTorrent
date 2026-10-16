use bittorrent_rs::{
    bencode::{BNode, Bencode},
    network::announcer::HttpAnnouncer,
    network::tracker::{PeerDetails, TrackerAnnounceContext, TrackerEvent},
    session::builder::TorrentSessionBuilder,
    session::config::SessionConfig,
    util::{
        decode_compact_ipv6_peers, encode_compact_ipv4_peers,
        encode_compact_ipv6_peers, format_socket_addr,
    },
};
use std::net::SocketAddr;
use std::str::FromStr;

#[test]
fn test_format_socket_addr_ipv4_and_ipv6() {
    let ipv4_addr = format_socket_addr("192.168.1.100", 6881);
    assert_eq!(ipv4_addr, "192.168.1.100:6881");
    assert!(SocketAddr::from_str(&ipv4_addr).is_ok());

    let ipv6_addr = format_socket_addr("2001:db8::1", 6881);
    assert_eq!(ipv6_addr, "[2001:db8::1]:6881");
    assert!(SocketAddr::from_str(&ipv6_addr).is_ok());

    let bracketed = format_socket_addr("[2001:db8::1]", 8080);
    assert_eq!(bracketed, "[2001:db8::1]:8080");
    assert!(SocketAddr::from_str(&bracketed).is_ok());
}

#[test]
fn test_compact_ipv6_peers_roundtrip() {
    let peers = vec![
        ("2001:db8::1".to_string(), 6881),
        ("fe80::1ff:fe23:4567:890a".to_string(), 51413),
        ("::1".to_string(), 8080),
    ];

    let encoded = encode_compact_ipv6_peers(&peers);
    assert_eq!(encoded.len(), 3 * 18);

    let decoded = decode_compact_ipv6_peers(&encoded, 0);
    assert_eq!(decoded.len(), 3);
    assert_eq!(decoded[0].0, "2001:db8::1");
    assert_eq!(decoded[0].1, 6881);
    assert_eq!(decoded[1].0, "fe80::1ff:fe23:4567:890a");
    assert_eq!(decoded[1].1, 51413);
    assert_eq!(decoded[2].0, "::1");
    assert_eq!(decoded[2].1, 8080);
}

#[test]
fn test_compact_ipv6_peers_offset_and_truncation() {
    let peers = vec![
        ("2001:db8::1".to_string(), 6881),
        ("::1".to_string(), 8080),
    ];
    let mut encoded = vec![0xAA; 4]; // 4-byte prefix
    encoded.extend(encode_compact_ipv6_peers(&peers));
    encoded.extend(vec![0xBB; 10]); // incomplete 10 bytes trailing

    let decoded = decode_compact_ipv6_peers(&encoded, 4);
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].0, "2001:db8::1");
    assert_eq!(decoded[1].0, "::1");
}

#[test]
fn test_peer_details_ipv6_helpers() {
    let p_v4 = PeerDetails {
        info_hash: vec![0; 20],
        peer_id: None,
        ip: "192.168.1.1".to_string(),
        port: 6881,
    };
    assert!(p_v4.is_ipv4());
    assert!(!p_v4.is_ipv6());
    assert_eq!(p_v4.endpoint(), "192.168.1.1:6881");

    let p_v6 = PeerDetails {
        info_hash: vec![0; 20],
        peer_id: None,
        ip: "2001:db8::1".to_string(),
        port: 6881,
    };
    assert!(!p_v6.is_ipv4());
    assert!(p_v6.is_ipv6());
    assert_eq!(p_v6.endpoint(), "[2001:db8::1]:6881");
}

#[test]
fn test_session_config_and_builder_ipv6() {
    let config = SessionConfig::default()
        .with_ipv6("2001:db8::dead:beef")
        .with_enable_ipv6(true);
    assert_eq!(config.ipv6.as_deref(), Some("2001:db8::dead:beef"));
    assert!(config.enable_ipv6);

    let builder = TorrentSessionBuilder::new("sample.torrent", ".")
        .ipv6("2001:db8::abcd".to_string())
        .enable_ipv6(true);
    assert_eq!(builder.get_config().ipv6.as_deref(), Some("2001:db8::abcd"));
    assert!(builder.get_config().enable_ipv6);
}

#[test]
fn test_tracker_announce_context_ipv6_filtering() {
    let ctx = TrackerAnnounceContext {
        info_hash: vec![1; 20],
        peer_id: "-TR2940-123456789012".to_string(),
        port: 6881,
        ip: "192.168.1.50".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "http://tracker.example.com/announce".to_string(),
        event: TrackerEvent::None,
        interval: 1800,
        min_interval: 900,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        ipv6: Some("2001:db8::50".to_string()),
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let peers = vec![
        ("2001:db8::50".to_string(), 6881), // Should be filtered out (matches ctx.ipv6)
        ("2001:db8::100".to_string(), 6882), // Kept
        ("::1".to_string(), 51413),          // Kept
    ];
    let encoded = encode_compact_ipv6_peers(&peers);
    let list = ctx.get_compact_ipv6_peer_list(&encoded, 0);

    assert_eq!(list.len(), 2);
    assert_eq!(list[0].ip, "2001:db8::100");
    assert_eq!(list[1].ip, "::1");
}

#[test]
fn test_http_announce_peers6_compact_decoding() {
    let ctx = TrackerAnnounceContext {
        info_hash: vec![1; 20],
        peer_id: "-TR2940-123456789012".to_string(),
        port: 6881,
        ip: "192.168.1.1".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "http://tracker.example.com/announce".to_string(),
        event: TrackerEvent::None,
        interval: 1800,
        min_interval: 900,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        ipv6: None,
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let ipv6_peers = vec![
        ("2001:db8::beef".to_string(), 5000),
        ("2001:db8::cafe".to_string(), 6000),
    ];
    let peers6_bytes = encode_compact_ipv6_peers(&ipv6_peers);

    let resp_node = BNode::Dictionary(vec![
        (b"interval", BNode::Number(b"1800")),
        (b"peers6", BNode::String(&peers6_bytes)),
    ]);
    let bencoded = Bencode::encode(&resp_node);

    let resp = HttpAnnouncer::decode_announce_response(&ctx, &bencoded).expect("Decode failed");
    assert_eq!(resp.interval, 1800);
    assert_eq!(resp.peer_list.len(), 2);
    assert_eq!(resp.peer_list[0].ip, "2001:db8::beef");
    assert_eq!(resp.peer_list[0].port, 5000);
    assert_eq!(resp.peer_list[1].ip, "2001:db8::cafe");
    assert_eq!(resp.peer_list[1].port, 6000);
}

#[test]
fn test_http_announce_dual_stack_decoding() {
    let ctx = TrackerAnnounceContext {
        info_hash: vec![1; 20],
        peer_id: "-TR2940-123456789012".to_string(),
        port: 6881,
        ip: "192.168.1.1".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "http://tracker.example.com/announce".to_string(),
        event: TrackerEvent::None,
        interval: 1800,
        min_interval: 900,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        ipv6: None,
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let ipv4_peers = vec![("10.0.0.1".to_string(), 6881)];
    let ipv6_peers = vec![("2001:db8::1".to_string(), 6881)];

    let peers_bytes = encode_compact_ipv4_peers(&ipv4_peers);
    let peers6_bytes = encode_compact_ipv6_peers(&ipv6_peers);

    let resp_node = BNode::Dictionary(vec![
        (b"interval", BNode::Number(b"1800")),
        (b"peers", BNode::String(&peers_bytes)),
        (b"peers6", BNode::String(&peers6_bytes)),
    ]);
    let bencoded = Bencode::encode(&resp_node);

    let resp = HttpAnnouncer::decode_announce_response(&ctx, &bencoded).expect("Decode failed");
    assert_eq!(resp.peer_list.len(), 2);
    assert_eq!(resp.peer_list[0].ip, "10.0.0.1");
    assert!(resp.peer_list[0].is_ipv4());
    assert_eq!(resp.peer_list[1].ip, "2001:db8::1");
    assert!(resp.peer_list[1].is_ipv6());
}

#[test]
fn test_http_announce_dictionary_peer_list_with_ipv6() {
    let ctx = TrackerAnnounceContext {
        info_hash: vec![1; 20],
        peer_id: "-TR2940-123456789012".to_string(),
        port: 6881,
        ip: "192.168.1.1".to_string(),
        compact: false,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "http://tracker.example.com/announce".to_string(),
        event: TrackerEvent::None,
        interval: 1800,
        min_interval: 900,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        ipv6: None,
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let peer1_dict = BNode::Dictionary(vec![
        (b"ip", BNode::String(b"2001:db8::1234")),
        (b"port", BNode::Number(b"6881")),
    ]);
    let peer2_dict = BNode::Dictionary(vec![
        (b"ip", BNode::String(b"[2001:db8::5678]")),
        (b"port", BNode::Number(b"6882")),
    ]);
    let resp_node = BNode::Dictionary(vec![
        (b"interval", BNode::Number(b"1800")),
        (b"peers", BNode::List(vec![peer1_dict, peer2_dict])),
    ]);
    let bencoded = Bencode::encode(&resp_node);

    let resp = HttpAnnouncer::decode_announce_response(&ctx, &bencoded).expect("Decode failed");
    assert_eq!(resp.peer_list.len(), 2);
    assert_eq!(resp.peer_list[0].ip, "2001:db8::1234");
    assert_eq!(resp.peer_list[0].port, 6881);
    assert_eq!(resp.peer_list[1].ip, "2001:db8::5678");
    assert_eq!(resp.peer_list[1].port, 6882);
}

#[test]
fn test_build_announce_url_dual_stack() {
    let announcer = HttpAnnouncer;
    let mut ctx = TrackerAnnounceContext {
        info_hash: vec![2; 20],
        peer_id: "-TR2940-123456789012".to_string(),
        port: 6881,
        ip: "192.168.1.1".to_string(),
        compact: true,
        no_peer_id: false,
        key: None,
        tracker_id: None,
        num_wanted: 50,
        tracker_url: "http://tracker.example.com/announce".to_string(),
        event: TrackerEvent::None,
        interval: 1800,
        min_interval: 900,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        ipv6: None,
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::io_traits::UreqHttpClient),
    };

    let url_without_ipv6 = announcer.build_announce_url(&ctx);
    assert!(!url_without_ipv6.contains("&ipv6="));

    ctx.ipv6 = Some("2001:db8::dead:beef".to_string());
    let url_with_ipv6 = announcer.build_announce_url(&ctx);
    assert!(url_with_ipv6.contains("&ipv6=2001%3Adb8%3A%3Adead%3Abeef"));
}

