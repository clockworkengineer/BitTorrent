use bittorrent_rs::{Peer, TorrentContext, RarestFirstSelector};
use bittorrent_rs::session::SessionConfig;
use bittorrent_rs::peer::PeerAction;
use bittorrent_rs::peer_message::PeerMessage;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn test_pex_parsing() {
    let added_bytes = vec![192, 168, 1, 10, 26, 225];
    let dropped_bytes = vec![10, 0, 0, 1, 0, 80];
    
    let added_node = bittorrent_rs::BNode::String(&added_bytes);
    let dropped_node = bittorrent_rs::BNode::String(&dropped_bytes);
    let pex_dict = bittorrent_rs::BNode::Dictionary(vec![
        (b"added", added_node),
        (b"dropped", dropped_node),
    ]);
    let payload = bittorrent_rs::Bencode::encode(&pex_dict);
    
    let (socket, _in_tx, _out_rx) = bittorrent_rs::MockSocket::new();
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, Arc::new(socket));
    
    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        vec![0; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    
    let actions = peer.handle_peer_message(PeerMessage::Extended { ext_id: 2, payload: &payload }, &mut tc).unwrap();
    assert_eq!(actions.len(), 1);
    
    match &actions[0] {
        PeerAction::DiscoverPeers(peers) => {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].ip, "192.168.1.10");
            assert_eq!(peers[0].port, 6881);
        }
        other => panic!("Expected DiscoverPeers action, got {:?}", other),
    }
}

#[test]
fn test_choking_decay_and_sorting() {
    let (socket, _, _) = bittorrent_rs::MockSocket::new();
    let socket = Arc::new(socket);
    
    let mut p1 = Peer::new_with_socket("1.1.1.1".to_string(), 1111, socket.clone());
    p1.peer_interested = true;
    p1.bytes_downloaded_in_interval = 3000;
    
    let mut p2 = Peer::new_with_socket("2.2.2.2".to_string(), 2222, socket.clone());
    p2.peer_interested = true;
    p2.bytes_downloaded_in_interval = 1000;
    
    let mut p3 = Peer::new_with_socket("3.3.3.3".to_string(), 3333, socket.clone());
    p3.peer_interested = true;
    p3.bytes_downloaded_in_interval = 5000;
    
    let mut p4 = Peer::new_with_socket("4.4.4.4".to_string(), 4444, socket.clone());
    p4.peer_interested = true;
    p4.bytes_downloaded_in_interval = 500;
    
    let peers = vec![
        Arc::new(Mutex::new(p1)),
        Arc::new(Mutex::new(p2)),
        Arc::new(Mutex::new(p3)),
        Arc::new(Mutex::new(p4)),
    ];
    
    for peer_arc in &peers {
        let mut p = peer_arc.lock().unwrap();
        let dl_rate = p.bytes_downloaded_in_interval as f64 / 10.0;
        p.rolling_download_rate = p.rolling_download_rate * 0.8 + dl_rate * 0.2;
        p.bytes_downloaded_in_interval = 0;
    }
    
    // Sort peers based on download rate descending
    let mut sorted_peers = peers.clone();
    sorted_peers.sort_by(|a, b| {
        let pa = a.lock().unwrap();
        let pb = b.lock().unwrap();
        pb.rolling_download_rate.partial_cmp(&pa.rolling_download_rate).unwrap()
    });
    
    assert_eq!(sorted_peers[0].lock().unwrap().ip, "3.3.3.3");
    assert_eq!(sorted_peers[1].lock().unwrap().ip, "1.1.1.1");
    assert_eq!(sorted_peers[2].lock().unwrap().ip, "2.2.2.2");
    assert_eq!(sorted_peers[3].lock().unwrap().ip, "4.4.4.4");
}

#[test]
fn test_keep_alive_timers() {
    let (socket, _, _) = bittorrent_rs::MockSocket::new();
    let socket = Arc::new(socket);
    
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket.clone());
    
    assert!(peer.last_message_sent.elapsed() < Duration::from_secs(1));
    assert!(peer.last_message_received.elapsed() < Duration::from_secs(1));
    
    peer.last_message_sent = Instant::now() - Duration::from_secs(130);
    peer.last_message_received = Instant::now() - Duration::from_secs(10);
    
    let last_sent_elapsed = peer.last_message_sent.elapsed();
    let last_recv_elapsed = peer.last_message_received.elapsed();
    
    assert!(last_sent_elapsed > Duration::from_secs(120));
    assert!(last_recv_elapsed <= Duration::from_secs(120));
}

#[test]
fn test_pex_ipv6_parsing() {
    // IPv6: [2001:db8::1]:6881 -> 16 bytes for IP + 2 bytes for port
    let mut ip_bytes = vec![0; 18];
    ip_bytes[0] = 0x20; ip_bytes[1] = 0x01;
    ip_bytes[2] = 0x0d; ip_bytes[3] = 0xb8;
    ip_bytes[15] = 0x01;
    ip_bytes[16] = 0x1a; ip_bytes[17] = 0xe1; // Port 6881

    let added6_node = bittorrent_rs::BNode::String(&ip_bytes);
    let pex_dict = bittorrent_rs::BNode::Dictionary(vec![
        (b"added6", added6_node),
    ]);
    let payload = bittorrent_rs::Bencode::encode(&pex_dict);
    
    let (socket, _in_tx, _out_rx) = bittorrent_rs::MockSocket::new();
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, Arc::new(socket));
    
    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        vec![0; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    
    let actions = peer.handle_peer_message(PeerMessage::Extended { ext_id: 2, payload: &payload }, &mut tc).unwrap();
    assert_eq!(actions.len(), 1);
    
    match &actions[0] {
        PeerAction::DiscoverPeers(peers) => {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].ip, "2001:db8::1");
            assert_eq!(peers[0].port, 6881);
        }
        other => panic!("Expected DiscoverPeers action, got {:?}", other),
    }
}

#[test]
fn test_fast_extension_messages_encode_decode() {
    // 1. HaveAll
    let msg = PeerMessage::HaveAll;
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 1, 14]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    assert!(matches!(decoded, PeerMessage::HaveAll));

    // 2. HaveNone
    let msg = PeerMessage::HaveNone;
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 1, 15]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    assert!(matches!(decoded, PeerMessage::HaveNone));

    // 3. Suggest
    let msg = PeerMessage::Suggest(42);
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 5, 13, 0, 0, 0, 42]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    if let PeerMessage::Suggest(index) = decoded {
        assert_eq!(index, 42);
    } else {
        panic!("Expected Suggest, got {:?}", decoded);
    }

    // 4. AllowedFast
    let msg = PeerMessage::AllowedFast(99);
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 5, 17, 0, 0, 0, 99]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    if let PeerMessage::AllowedFast(index) = decoded {
        assert_eq!(index, 99);
    } else {
        panic!("Expected AllowedFast, got {:?}", decoded);
    }

    // 5. Reject
    let msg = PeerMessage::Reject { index: 7, begin: 1024, length: 16384 };
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 13, 16, 0, 0, 0, 7, 0, 0, 4, 0, 0, 0, 64, 0]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    if let PeerMessage::Reject { index, begin, length } = decoded {
        assert_eq!(index, 7);
        assert_eq!(begin, 1024);
        assert_eq!(length, 16384);
    } else {
        panic!("Expected Reject, got {:?}", decoded);
    }
}

#[test]
fn test_lsd_announcement_format() {
    let info_hash = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
    let local_port = 6881;
    let infohash_hex = bittorrent_rs::util::info_hash_to_string(&info_hash);
    let packet = format!(
        "BT-SEARCH * HTTP/1.1\r\n\
         Host: 239.192.152.143:6771\r\n\
         Port: {}\r\n\
         Infohash: {}\r\n\
         \r\n",
        local_port, infohash_hex
    );
    
    assert!(packet.contains("BT-SEARCH * HTTP/1.1"));
    assert!(packet.contains("Host: 239.192.152.143:6771"));
    assert!(packet.contains("Port: 6881"));
    assert!(packet.contains("Infohash: 0102030405060708090a0b0c0d0e0f1011121314"));
}

#[derive(Debug)]
struct MockScrapeHttpClient {
    response_body: Vec<u8>,
}

impl bittorrent_rs::io_traits::HttpClient for MockScrapeHttpClient {
    fn get(&self, _url: &str) -> Result<Vec<u8>, bittorrent_rs::BitTorrentError> {
        Ok(self.response_body.clone())
    }
}

#[test]
fn test_http_scrape_parsing() {
    let info_hash = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
    
    // Build mocked scrape bencode response:
    // d5:filesd20:infohash_bytesd8:completei12e10:downloadedi350e10:incompletei4eeee
    // Let's build the nodes:
    let complete_node = bittorrent_rs::BNode::Number(b"12");
    let downloaded_node = bittorrent_rs::BNode::Number(b"350");
    let incomplete_node = bittorrent_rs::BNode::Number(b"4");
    
    let stats_dict = bittorrent_rs::BNode::Dictionary(vec![
        (b"complete", complete_node),
        (b"downloaded", downloaded_node),
        (b"incomplete", incomplete_node),
    ]);
    
    let files_dict = bittorrent_rs::BNode::Dictionary(vec![
        (&info_hash, stats_dict),
    ]);
    
    let root_dict = bittorrent_rs::BNode::Dictionary(vec![
        (b"files", files_dict),
    ]);
    
    let response_body = bittorrent_rs::Bencode::encode(&root_dict);
    let mock_client = std::sync::Arc::new(MockScrapeHttpClient { response_body });
    
    let mut config = SessionConfig::default();
    config.http_client = mock_client;
    
    let selector = Arc::new(RarestFirstSelector);
    let tc = TorrentContext::new_magnet_bootstrap(
        info_hash,
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    
    let mut tracker = bittorrent_rs::Tracker::new(Arc::new(Mutex::new(tc))).unwrap();
    let scrape_res = tracker.scrape().unwrap();
    
    assert_eq!(scrape_res.complete, 12);
    assert_eq!(scrape_res.downloaded, 350);
    assert_eq!(scrape_res.incomplete, 4);
}
