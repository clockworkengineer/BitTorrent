use bittorrent_rs::{
    MockSocket, Peer, PeerMessage, RarestFirstSelector, SessionConfig, Socket, TorrentStatus,
    internals::TorrentContext,
    network::pex::{PexFlags, PexMessage},
    network::tracker::PeerDetails,
};
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn test_pex_dual_stack_encode_decode_roundtrip() {
    let info_hash = vec![0x33u8; 20];
    let mut msg = PexMessage::new();

    // IPv4 added peers
    let peer_v4_1 = PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "192.0.2.1".to_string(),
        port: 6881,
    };
    let flags_v4_1 = PexFlags::new(PexFlags::PREF_ENCRYPT | PexFlags::OUTGOING);

    let peer_v4_2 = PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "198.51.100.42".to_string(),
        port: 51413,
    };
    let flags_v4_2 = PexFlags::new(PexFlags::SEED | PexFlags::UTP);

    msg.add_v4(peer_v4_1, flags_v4_1);
    msg.add_v4(peer_v4_2, flags_v4_2);

    // IPv4 dropped peer
    let dropped_v4 = PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "203.0.113.99".to_string(),
        port: 8080,
    };
    msg.drop_v4(dropped_v4);

    // IPv6 added peers
    let peer_v6_1 = PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "2001:db8::1".to_string(),
        port: 6881,
    };
    let flags_v6_1 = PexFlags::new(PexFlags::PREF_ENCRYPT | PexFlags::HOLEPUNCH);

    let peer_v6_2 = PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "2001:db8:85a3::8a2e:370:7334".to_string(),
        port: 49152,
    };
    let flags_v6_2 = PexFlags::new(PexFlags::SEED);

    msg.add_v6(peer_v6_1, flags_v6_1);
    msg.add_v6(peer_v6_2, flags_v6_2);

    // IPv6 dropped peer
    let dropped_v6 = PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "2001:db8::ffff".to_string(),
        port: 12345,
    };
    msg.drop_v6(dropped_v6);

    // Encode
    let encoded = msg.encode();
    assert!(!encoded.is_empty());

    // Validate lexicographical dictionary ordering in raw bencode
    let raw_str = String::from_utf8_lossy(&encoded);
    let pos_added = raw_str.find("5:added").expect("missing added");
    let pos_added_f = raw_str.find("7:added.f").expect("missing added.f");
    let pos_added6 = raw_str.find("6:added6").expect("missing added6");
    let pos_added6_f = raw_str.find("8:added6.f").expect("missing added6.f");
    let pos_dropped = raw_str.find("7:dropped").expect("missing dropped");
    let pos_dropped6 = raw_str.find("8:dropped6").expect("missing dropped6");

    assert!(pos_added < pos_added_f, "added must precede added.f");
    assert!(
        pos_added_f < pos_added6,
        "added.f must precede added6 ('.' < '6')"
    );
    assert!(pos_added6 < pos_added6_f, "added6 must precede added6.f");
    assert!(pos_added6_f < pos_dropped, "added6.f must precede dropped");
    assert!(pos_dropped < pos_dropped6, "dropped must precede dropped6");

    // Decode and verify roundtrip equivalence
    let decoded = PexMessage::decode(&encoded, &info_hash).expect("decode failed");

    // IPv4 added
    assert_eq!(decoded.added_v4.len(), 2);
    assert_eq!(decoded.added_v4[0].details.ip, "192.0.2.1");
    assert_eq!(decoded.added_v4[0].details.port, 6881);
    assert!(decoded.added_v4[0].flags.is_pref_encrypt());
    assert!(decoded.added_v4[0].flags.is_outgoing());
    assert!(!decoded.added_v4[0].flags.is_seed());

    assert_eq!(decoded.added_v4[1].details.ip, "198.51.100.42");
    assert_eq!(decoded.added_v4[1].details.port, 51413);
    assert!(decoded.added_v4[1].flags.is_seed());
    assert!(decoded.added_v4[1].flags.is_utp());

    // IPv4 dropped
    assert_eq!(decoded.dropped_v4.len(), 1);
    assert_eq!(decoded.dropped_v4[0].ip, "203.0.113.99");
    assert_eq!(decoded.dropped_v4[0].port, 8080);

    // IPv6 added
    assert_eq!(decoded.added_v6.len(), 2);
    assert_eq!(decoded.added_v6[0].details.ip, "2001:db8::1");
    assert_eq!(decoded.added_v6[0].details.port, 6881);
    assert!(decoded.added_v6[0].flags.is_pref_encrypt());
    assert!(decoded.added_v6[0].flags.is_holepunch());

    assert_eq!(
        decoded.added_v6[1].details.ip,
        "2001:db8:85a3::8a2e:370:7334"
    );
    assert_eq!(decoded.added_v6[1].details.port, 49152);
    assert!(decoded.added_v6[1].flags.is_seed());

    // IPv6 dropped
    assert_eq!(decoded.dropped_v6.len(), 1);
    assert_eq!(decoded.dropped_v6[0].ip, "2001:db8::ffff");
    assert_eq!(decoded.dropped_v6[0].port, 12345);
}

#[test]
fn test_pex_flags_bit_operations() {
    let f = PexFlags::new(PexFlags::PREF_ENCRYPT | PexFlags::SEED | PexFlags::UTP);
    assert!(f.is_pref_encrypt());
    assert!(f.is_seed());
    assert!(f.is_utp());
    assert!(!f.is_holepunch());
    assert!(!f.is_outgoing());
    assert_eq!(f.bits(), 0x01 | 0x02 | 0x04);

    let f2 = PexFlags::new(PexFlags::HOLEPUNCH | PexFlags::OUTGOING);
    assert!(!f2.is_pref_encrypt());
    assert!(!f2.is_seed());
    assert!(f2.is_holepunch());
    assert!(f2.is_outgoing());
    assert_eq!(f2.bits(), 0x08 | 0x10);
}

#[test]
fn test_pex_handling_seed_filters_seeds() {
    let info_hash = vec![0x11u8; 20];
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let mut peer = Peer::new_with_socket(
        "127.0.0.1".to_string(),
        6881,
        Arc::new(Socket::Mock(socket)),
    );

    let mut config = SessionConfig::default();
    config.enable_pex = true;
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        info_hash.clone(),
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    )
    .unwrap();

    // Mark our local client as a seed (status = Seeding)
    tc.status = TorrentStatus::Seeding;

    let mut pex = PexMessage::new();
    // Leech peer
    pex.add_v4(
        PeerDetails {
            info_hash: info_hash.clone(),
            peer_id: None,
            ip: "10.0.0.1".to_string(),
            port: 6881,
        },
        PexFlags::new(0),
    );

    // Seed peer
    pex.add_v4(
        PeerDetails {
            info_hash: info_hash.clone(),
            peer_id: None,
            ip: "10.0.0.2".to_string(),
            port: 6882,
        },
        PexFlags::new(PexFlags::SEED),
    );

    let payload = pex.encode();
    let actions = peer
        .handle_peer_message(
            PeerMessage::Extended {
                ext_id: 2,
                payload: &payload,
            },
            &mut tc,
        )
        .unwrap();

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        bittorrent_rs::network::peer::PeerAction::DiscoverPeers(peers) => {
            // Seed peer 10.0.0.2 must be filtered out! Only leech peer 10.0.0.1 should be discovered.
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].ip, "10.0.0.1");
        }
        other => panic!("Expected DiscoverPeers, got {:?}", other),
    }
}

#[test]
fn test_pex_ipv6_disabled_filters_added6_and_dropped6() {
    let info_hash = vec![0x22u8; 20];
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let mut peer = Peer::new_with_socket(
        "127.0.0.1".to_string(),
        6881,
        Arc::new(Socket::Mock(socket)),
    );

    let mut config = SessionConfig::default();
    config.enable_ipv6 = false; // IPv6 disabled!
    config.enable_pex = true;
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        info_hash.clone(),
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    )
    .unwrap();

    let mut pex = PexMessage::new();
    pex.add_v4(
        PeerDetails {
            info_hash: info_hash.clone(),
            peer_id: None,
            ip: "192.168.1.100".to_string(),
            port: 6881,
        },
        PexFlags::new(0),
    );
    pex.add_v6(
        PeerDetails {
            info_hash: info_hash.clone(),
            peer_id: None,
            ip: "2001:db8::beef".to_string(),
            port: 6881,
        },
        PexFlags::new(0),
    );
    pex.drop_v6(PeerDetails {
        info_hash: info_hash.clone(),
        peer_id: None,
        ip: "2001:db8::dead".to_string(),
        port: 6881,
    });

    let payload = pex.encode();
    let actions = peer
        .handle_peer_message(
            PeerMessage::Extended {
                ext_id: 2,
                payload: &payload,
            },
            &mut tc,
        )
        .unwrap();

    assert_eq!(actions.len(), 1);
    match &actions[0] {
        bittorrent_rs::network::peer::PeerAction::DiscoverPeers(peers) => {
            // IPv6 peer must be excluded
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].ip, "192.168.1.100");
        }
        other => panic!("Expected DiscoverPeers, got {:?}", other),
    }

    // Dropped IPv6 peers must also be ignored when IPv6 is disabled
    let dropped = tc.pex_dropped_peers.lock().unwrap();
    assert!(dropped.is_empty());
}

#[test]
fn test_pex_private_torrent_enforcement() {
    let info_hash = vec![0x44u8; 20];
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let mut peer = Peer::new_with_socket(
        "127.0.0.1".to_string(),
        6881,
        Arc::new(Socket::Mock(socket)),
    );

    let mut config = SessionConfig::default();
    config.enable_pex = true;
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        info_hash.clone(),
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    )
    .unwrap();

    // Mark as private torrent
    tc.is_private = true;
    assert!(!tc.is_pex_enabled(), "Private torrent must disable PEX");

    let mut pex = PexMessage::new();
    pex.add_v4(
        PeerDetails {
            info_hash: info_hash.clone(),
            peer_id: None,
            ip: "10.10.10.10".to_string(),
            port: 6881,
        },
        PexFlags::new(0),
    );

    let payload = pex.encode();
    let actions = peer
        .handle_peer_message(
            PeerMessage::Extended {
                ext_id: 2,
                payload: &payload,
            },
            &mut tc,
        )
        .unwrap();

    // Must not discover any peers from PEX on a private torrent!
    assert!(
        actions.is_empty(),
        "Private torrent must ignore incoming PEX messages"
    );
    assert_eq!(
        tc.pex_added_peers_count
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[test]
fn test_pex_differential_generation_and_capping() {
    let info_hash = vec![0x55u8; 20];
    let config = SessionConfig::default()
        .with_enable_pex(true)
        .with_enable_ipv6(true);
    let selector = Arc::new(RarestFirstSelector);
    let tc = TorrentContext::new_magnet_bootstrap(
        info_hash.clone(),
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    )
    .unwrap();

    // Add 3 peers to the swarm: 2 IPv4 and 1 IPv6
    let (s1, _, _) = MockSocket::new();
    let p1 = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "192.168.1.1".to_string(),
        6881,
        Arc::new(Socket::Mock(s1)),
    )));
    let (s2, _, _) = MockSocket::new();
    let p2 = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "192.168.1.2".to_string(),
        6882,
        Arc::new(Socket::Mock(s2)),
    )));
    let (s3, _, _) = MockSocket::new();
    let p3 = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "2001:db8::3".to_string(),
        6883,
        Arc::new(Socket::Mock(s3)),
    )));

    {
        let mut swarm = tc.peer_swarm.write().unwrap();
        swarm.insert("192.168.1.1".to_string(), p1);
        swarm.insert("192.168.1.2".to_string(), p2);
        swarm.insert("2001:db8::3".to_string(), p3);
    }

    // Broadcast 1: Exclude peer 192.168.1.1 (the recipient)
    let sent_peers = HashMap::new();
    let (pex1, new_sent) = tc.generate_pex_message(&sent_peers, Some("192.168.1.1"));

    // Recipient excluded, so remaining are 192.168.1.2 (IPv4) and 2001:db8::3 (IPv6)
    assert_eq!(pex1.added_v4.len(), 1);
    assert_eq!(pex1.added_v4[0].details.ip, "192.168.1.2");
    assert_eq!(pex1.added_v6.len(), 1);
    assert_eq!(pex1.added_v6[0].details.ip, "2001:db8::3");
    assert!(pex1.dropped_v4.is_empty());
    assert!(pex1.dropped_v6.is_empty());

    // Broadcast 2: Swarm unchanged -> no new added or dropped
    let (pex2, new_sent2) = tc.generate_pex_message(&new_sent, Some("192.168.1.1"));
    assert!(
        pex2.is_empty(),
        "Unchanged swarm must yield empty PEX message"
    );

    // Broadcast 3: Disconnect 192.168.1.2 from swarm -> must generate dropped_v4
    {
        let mut swarm = tc.peer_swarm.write().unwrap();
        swarm.remove("192.168.1.2");
    }
    let (pex3, _) = tc.generate_pex_message(&new_sent2, Some("192.168.1.1"));
    assert_eq!(pex3.dropped_v4.len(), 1);
    assert_eq!(pex3.dropped_v4[0].ip, "192.168.1.2");
    assert!(pex3.added_v4.is_empty());
}
