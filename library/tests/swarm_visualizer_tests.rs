use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::parse_peer_client;
use bittorrent_rs::peer::Peer;
use bittorrent_rs::selector::SequentialSelector;
use bittorrent_rs::session::config::SessionConfig;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

#[test]
fn test_peer_id_client_parsing_variety() {
    // Azureus style clients
    assert_eq!(
        parse_peer_client(b"-qB4620-abcdefghijkl"),
        "qBittorrent 4.6.2"
    );
    assert_eq!(parse_peer_client(b"-TR3000-123456789012"), "Transmission 3");
    assert_eq!(parse_peer_client(b"-UT3550-xxxxxxxxxxxx"), "µTorrent 3.5.5");
    assert_eq!(parse_peer_client(b"-uT2210-yyyyyyyyyyyy"), "µTorrent 2.2.1");
    assert_eq!(parse_peer_client(b"-DE2030-zzzzzzzzzzzz"), "Deluge 2.0.3");
    assert_eq!(
        parse_peer_client(b"-AZ5760-wwwwwwwwwwww"),
        "Vuze / Azureus 5.7.6"
    );
    assert_eq!(
        parse_peer_client(b"-WD0140-vvvvvvvvvvvv"),
        "WebTorrent 0.1.4"
    );
    assert_eq!(
        parse_peer_client(b"-LT1280-uuuuuuuuuuuu"),
        "libtorrent (Rasterbar) 1.2.8"
    );
    assert_eq!(parse_peer_client(b"-BC0150-tttttttttttt"), "BitComet 0.1.5");
    assert_eq!(parse_peer_client(b"-TT1200-ssssssssssss"), "Tixati 1.2");

    // Shadow style
    assert_eq!(parse_peer_client(b"M4-2-0--ShadowStyle!"), "Mainline 4-2-0");
    assert_eq!(parse_peer_client(b"S5-8-7--ShadowClient"), "Shadow 5-8-7");

    // Fallback
    assert_eq!(parse_peer_client(b"Short"), "Unknown");
    assert_eq!(
        parse_peer_client(&[0xAA, 0xBB, 0xCC, 0xDD, 0x01, 0x02, 0x03, 0x04]),
        "AABBCCDD..."
    );
}

#[test]
fn test_piece_availability_and_swarm_rarity() {
    let temp_dir = std::env::temp_dir().join(format!("bt_swarm_test_{}", rand::random::<u32>()));
    let _ = std::fs::create_dir_all(&temp_dir);

    let info_hash = vec![0x12; 20];
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        info_hash,
        vec!["http://tracker.example.com/announce".into()],
        Arc::new(SequentialSelector),
        &temp_dir,
        SessionConfig::default(),
    )
    .expect("Bootstrap failed");

    // Setup 10 pieces
    ctx.number_of_pieces = 10;
    ctx.piece_length = 16384;
    ctx.bitfield = vec![0b1010_0000, 0b1000_0000]; // Pieces 0, 2, and 8 downloaded locally

    // Peer 1 holds pieces 0, 1, 2 (0b1110_0000, 0b0000_0000)
    #[allow(deprecated)]
    let peer_1 = Arc::new(Mutex::new(Peer {
        network: None,
        packet_response_timer: None,
        average_packet_response: bittorrent_rs::Average::default(),
        connected: true,
        remote_peer_id: Some(b"-qB4600-peer11111111".to_vec()),
        event_listener: None,
        tc: None,
        remote_piece_bitfield: vec![0b1110_0000, 0b0000_0000],
        ip: "192.168.1.100".to_string(),
        port: 6881,
        am_interested: true,
        am_choking: false,
        peer_choking: bittorrent_rs::manual_reset_event::ManualResetEvent::new(true),
        peer_interested: true,
        number_of_missing_pieces: 7,
        outstanding_requests_count: 0,
        reserved_blocks: Vec::new(),
        supports_extensions: true,
        extension_ids: std::collections::BTreeMap::new(),
        metadata_size: None,
        bytes_downloaded_in_interval: 1024,
        bytes_uploaded_in_interval: 0,
        rolling_download_rate: 512000.0,
        rolling_upload_rate: 0.0,
        last_message_sent: std::time::Instant::now(),
        last_message_received: std::time::Instant::now(),
        supports_fast_extension: true,
        allowed_fast_pieces: Vec::new(),
        suggested_pieces: Vec::new(),
        supports_holepunch: true,
        connection_type: bittorrent_rs::ConnectionType::Tcp,
        peer_is_seed: false,
        buffer_pool: Arc::new(bittorrent_rs::BlockBufferPool::default()),
    }));

    // Peer 2 holds pieces 2, 3, 8, 9 (0b0011_0000, 0b1100_0000)
    #[allow(deprecated)]
    let peer_2 = Arc::new(Mutex::new(Peer {
        network: None,
        packet_response_timer: None,
        average_packet_response: bittorrent_rs::Average::default(),
        connected: true,
        remote_peer_id: Some(b"-TR4000-peer22222222".to_vec()),
        event_listener: None,
        tc: None,
        remote_piece_bitfield: vec![0b0011_0000, 0b1100_0000],
        ip: "10.0.0.50".to_string(),
        port: 51413,
        am_interested: true,
        am_choking: false,
        peer_choking: bittorrent_rs::manual_reset_event::ManualResetEvent::new(true),
        peer_interested: false,
        number_of_missing_pieces: 6,
        outstanding_requests_count: 0,
        reserved_blocks: Vec::new(),
        supports_extensions: false,
        extension_ids: std::collections::BTreeMap::new(),
        metadata_size: None,
        bytes_downloaded_in_interval: 0,
        bytes_uploaded_in_interval: 2048,
        rolling_download_rate: 0.0,
        rolling_upload_rate: 128000.0,
        last_message_sent: std::time::Instant::now(),
        last_message_received: std::time::Instant::now(),
        supports_fast_extension: false,
        allowed_fast_pieces: Vec::new(),
        suggested_pieces: Vec::new(),
        supports_holepunch: false,
        connection_type: bittorrent_rs::ConnectionType::WebRtc,
        peer_is_seed: false,
        buffer_pool: Arc::new(bittorrent_rs::BlockBufferPool::default()),
    }));

    let mut swarm = HashMap::new();
    swarm.insert("192.168.1.100:6881".to_string(), peer_1);
    swarm.insert("10.0.0.50:51413".to_string(), peer_2);
    ctx.peer_swarm = std::sync::Arc::new(RwLock::new(swarm));

    let (local_have, swarm_rarity) = ctx.get_piece_availability();
    assert_eq!(local_have.len(), 10);
    assert_eq!(swarm_rarity.len(), 10);

    // Verify local bitfield
    assert!(local_have[0]);
    assert!(!local_have[1]);
    assert!(local_have[2]);
    assert!(!local_have[3]);
    assert!(local_have[8]);
    assert!(!local_have[9]);

    // Verify swarm rarity counts
    // Piece 0: held by Peer 1 -> 1
    assert_eq!(swarm_rarity[0], 1);
    // Piece 1: held by Peer 1 -> 1
    assert_eq!(swarm_rarity[1], 1);
    // Piece 2: held by Peer 1 and Peer 2 -> 2
    assert_eq!(swarm_rarity[2], 2);
    // Piece 3: held by Peer 2 -> 1
    assert_eq!(swarm_rarity[3], 1);
    // Pieces 4, 5, 6, 7: held by nobody in swarm -> 0
    assert_eq!(swarm_rarity[4], 0);
    assert_eq!(swarm_rarity[5], 0);
    assert_eq!(swarm_rarity[6], 0);
    assert_eq!(swarm_rarity[7], 0);
    // Piece 8: held by Peer 2 -> 1
    assert_eq!(swarm_rarity[8], 1);
    // Piece 9: held by Peer 2 -> 1
    assert_eq!(swarm_rarity[9], 1);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
