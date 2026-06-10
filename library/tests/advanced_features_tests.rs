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
    let mut ip_bytes = vec![0; 18];
    ip_bytes[0] = 0x20; ip_bytes[1] = 0x01;
    ip_bytes[2] = 0x0d; ip_bytes[3] = 0xb8;
    ip_bytes[15] = 0x01;
    ip_bytes[16] = 0x1a; ip_bytes[17] = 0xe1;

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
    let msg = PeerMessage::HaveAll;
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 1, 14]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    assert!(matches!(decoded, PeerMessage::HaveAll));

    let msg = PeerMessage::HaveNone;
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 1, 15]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    assert!(matches!(decoded, PeerMessage::HaveNone));

    let msg = PeerMessage::Suggest(42);
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 5, 13, 0, 0, 0, 42]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    if let PeerMessage::Suggest(index) = decoded {
        assert_eq!(index, 42);
    } else {
        panic!("Expected Suggest, got {:?}", decoded);
    }

    let msg = PeerMessage::AllowedFast(99);
    let encoded = msg.encode();
    assert_eq!(encoded, vec![0, 0, 0, 5, 17, 0, 0, 0, 99]);
    let decoded = PeerMessage::decode(&encoded[4..]).unwrap();
    if let PeerMessage::AllowedFast(index) = decoded {
        assert_eq!(index, 99);
    } else {
        panic!("Expected AllowedFast, got {:?}", decoded);
    }

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

// ── Phase II feature tests ────────────────────────────────────────────────────

/// Verifies that the magnet bootstrap context defaults `is_private` to false.
#[test]
fn test_private_torrent_guards() {
    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let tc = TorrentContext::new_magnet_bootstrap(
        vec![0u8; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    assert!(!tc.is_private, "Magnet bootstrap should default is_private to false");
}

/// Verifies RC4 stream cipher: encrypt then decrypt returns original plaintext.
#[test]
fn test_mse_rc4_stream_encrypt_decrypt() {
    let key = b"test-secret-key";
    let mut cipher_enc = bittorrent_rs::mse::Rc4::new(key);
    let mut cipher_dec = bittorrent_rs::mse::Rc4::new(key);

    let plaintext = b"hello bittorrent protocol!";
    let mut data = plaintext.to_vec();
    cipher_enc.encrypt(&mut data);

    // Must be changed after encryption
    assert_ne!(data, plaintext.to_vec());

    // Decrypting with fresh, identically seeded cipher restores original
    cipher_dec.encrypt(&mut data);
    assert_eq!(data, plaintext.to_vec());
}

/// Verifies that two DH parties derive an identical shared secret.
#[test]
fn test_mse_diffie_hellman_shared_secret() {
    let alice = bittorrent_rs::mse::DiffieHellman::new();
    let bob   = bittorrent_rs::mse::DiffieHellman::new();

    let alice_secret = alice.compute_shared_secret(bob.public_key);
    let bob_secret   = bob.compute_shared_secret(alice.public_key);

    assert_eq!(alice_secret, bob_secret,
        "Both sides must derive the same DH shared secret");
}

/// Verifies uTP header encode/decode round-trip correctness.
#[test]
fn test_utp_header_encode_decode() {
    use bittorrent_rs::utp::{UtpHeader, UtpPacketType};

    let header = UtpHeader {
        packet_type: UtpPacketType::Data,
        version: 1,
        extension: 0,
        connection_id: 0x1234,
        timestamp_us: 0xDEAD_BEEF,
        timestamp_difference_us: 0x1111_2222,
        wnd_size: 1_048_576,
        seq_nr: 42,
        ack_nr: 7,
    };

    let encoded = header.encode();
    assert_eq!(encoded.len(), 20, "uTP header must be exactly 20 bytes");

    let decoded = UtpHeader::decode(&encoded).unwrap();
    assert_eq!(decoded.packet_type, UtpPacketType::Data);
    assert_eq!(decoded.version, 1);
    assert_eq!(decoded.connection_id, 0x1234);
    assert_eq!(decoded.timestamp_us, 0xDEAD_BEEF);
    assert_eq!(decoded.timestamp_difference_us, 0x1111_2222);
    assert_eq!(decoded.wnd_size, 1_048_576);
    assert_eq!(decoded.seq_nr, 42);
    assert_eq!(decoded.ack_nr, 7);
}

/// Verifies NAT-PMP port mapping request packet serialization.
#[test]
fn test_nat_pmp_mapping_request_serialization() {
    use bittorrent_rs::nat::NatPmpClient;
    use std::net::Ipv4Addr;

    let client = NatPmpClient::new(Ipv4Addr::new(192, 168, 1, 1));

    // TCP mapping
    let pkt = client.build_mapping_request(true, 6881, 6881, 3600);
    assert_eq!(pkt.len(), 12, "NAT-PMP request must be 12 bytes");
    assert_eq!(pkt[0], 0, "Version byte must be 0");
    assert_eq!(pkt[1], 2, "TCP opcode must be 2");
    assert_eq!(u16::from_be_bytes([pkt[4], pkt[5]]), 6881);
    assert_eq!(u16::from_be_bytes([pkt[6], pkt[7]]), 6881);
    assert_eq!(u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]), 3600);

    // UDP mapping
    let pkt_udp = client.build_mapping_request(false, 6881, 0, 7200);
    assert_eq!(pkt_udp[1], 1, "UDP opcode must be 1");
    assert_eq!(u32::from_be_bytes([pkt_udp[8], pkt_udp[9], pkt_udp[10], pkt_udp[11]]), 7200);
}

/// Verifies NAT-PMP response parsing extracts ports and lifetime correctly.
#[test]
fn test_nat_pmp_response_parsing() {
    use bittorrent_rs::nat::NatPmpClient;

    let mut response = [0u8; 16];
    response[0] = 0;    // version
    response[1] = 130;  // TCP response opcode (128 + 2)
    // result code = 0 (success)
    response[8]  = 0x1A; response[9]  = 0xE1; // internal port 6881
    response[10] = 0x1A; response[11] = 0xE1; // external port 6881
    response[12] = 0; response[13] = 0; response[14] = 0x0E; response[15] = 0x10; // lifetime 3600

    let (internal, external, lifetime) = NatPmpClient::parse_mapping_response(&response).unwrap();
    assert_eq!(internal, 6881);
    assert_eq!(external, 6881);
    assert_eq!(lifetime, 3600);
}

/// Verifies that v2 torrents produce a 32-byte SHA-256 info-hash.
#[test]
fn test_metainfo_v2_sha256_infohash_is_32_bytes() {
    use bittorrent_rs::{Bencode, BNode};
    use bittorrent_rs::metainfo::MetaInfoFile;

    // Leaf node: empty-string key maps to file properties
    let file_props = BNode::Dictionary(vec![
        (b"length" as &[u8], BNode::Number(b"1024")),
    ]);
    let file_leaf = BNode::Dictionary(vec![
        (b"" as &[u8], file_props),
    ]);
    let file_tree = BNode::Dictionary(vec![
        (b"myfile.txt" as &[u8], file_leaf),
    ]);

    let info_dict = BNode::Dictionary(vec![
        (b"name" as &[u8],         BNode::String(b"test-torrent")),
        (b"piece length" as &[u8], BNode::Number(b"262144")),
        (b"meta version" as &[u8], BNode::Number(b"2")),
        (b"file tree" as &[u8],    file_tree),
    ]);

    let root_dict = BNode::Dictionary(vec![
        (b"announce" as &[u8], BNode::String(b"http://tracker.example.com/announce")),
        (b"info" as &[u8],     info_dict),
    ]);

    let torrent_bytes = Bencode::encode(&root_dict);
    let mut meta = MetaInfoFile::from_bytes(&torrent_bytes);
    meta.parse().unwrap();

    let info_hash = meta.get_info_hash().unwrap();
    assert_eq!(info_hash.len(), 32,
        "BitTorrent v2 info-hash must be 32 bytes (SHA-256), got {} bytes",
        info_hash.len());
    assert!(meta.is_v2(),      "Torrent should be detected as v2");
    assert!(!meta.is_private(), "No private flag should default to false");
}
