use std::net::UdpSocket;
use std::thread;
use bittorrent_rs::announcer::{AnnouncerFactory, Announcer};
use bittorrent_rs::tracker::{TrackerAnnounceContext, TrackerEvent};
use bittorrent_rs::peer_id;

#[test]
fn test_udp_tracker_announce() {
    // 1. Bind mock UDP tracker server
    let server_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let server_port = server_socket.local_addr().unwrap().port();
    
    // Spawn server thread to answer requests
    let handle = thread::spawn(move || {
        let mut buf = vec![0u8; 1500];
        
        // --- 1. Connect Phase ---
        let (n, client_addr) = server_socket.recv_from(&mut buf).unwrap();
        assert_eq!(n, 16);
        let protocol_id = u64::from_be_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(protocol_id, 0x41727101980);
        let action = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(action, 0); // Connect
        let transaction_id = u32::from_be_bytes(buf[12..16].try_into().unwrap());
        
        // Send connect response
        let mock_conn_id = 0x123456789abcdef0u64;
        let mut response = Vec::new();
        response.extend_from_slice(&0u32.to_be_bytes()); // action = 0 (connect)
        response.extend_from_slice(&transaction_id.to_be_bytes());
        response.extend_from_slice(&mock_conn_id.to_be_bytes());
        server_socket.send_to(&response, client_addr).unwrap();
        
        // --- 2. Announce Phase ---
        let (n, client_addr) = server_socket.recv_from(&mut buf).unwrap();
        assert!(n >= 98);
        let conn_id = u64::from_be_bytes(buf[0..8].try_into().unwrap());
        assert_eq!(conn_id, mock_conn_id);
        let action = u32::from_be_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(action, 1); // Announce
        let transaction_id = u32::from_be_bytes(buf[12..16].try_into().unwrap());
        
        // Send announce response with 1 peer: 1.2.3.4:6881
        let mut response = Vec::new();
        response.extend_from_slice(&1u32.to_be_bytes()); // action = 1 (announce)
        response.extend_from_slice(&transaction_id.to_be_bytes());
        response.extend_from_slice(&1800u32.to_be_bytes()); // interval = 1800
        response.extend_from_slice(&10u32.to_be_bytes()); // leechers = 10
        response.extend_from_slice(&5u32.to_be_bytes()); // seeders = 5
        // Peer IP: 1.2.3.4 (4 bytes), Port: 6881 (2 bytes)
        response.extend_from_slice(&[1, 2, 3, 4, 26, 225]);
        server_socket.send_to(&response, client_addr).unwrap();
    });
    
    // 2. Perform announcement using library
    let tracker_url = format!("udp://127.0.0.1:{}", server_port);
    let mut announcer = match AnnouncerFactory::create(&tracker_url).unwrap() {
        bittorrent_rs::announcer::AnnouncerEnum::Udp(ann) => ann,
        _ => panic!("Expected Udp announcer"),
    };
    
    let context = TrackerAnnounceContext {
        info_hash: vec![0xaa; 20],
        peer_id: peer_id::get(),
        port: 6881,
        ip: "127.0.0.1".to_string(),
        compact: true,
        no_peer_id: true,
        key: None,
        tracker_id: None,
        num_wanted: 10,
        tracker_url: tracker_url.clone(),
        event: TrackerEvent::Started,
        interval: 0,
        min_interval: 0,
        downloaded: 0,
        uploaded: 0,
        left: 1000,
        #[cfg(feature = "http-tracker")]
        http_client: std::sync::Arc::new(bittorrent_rs::UreqHttpClient),
    };
    
    // Announce
    let response = announcer.announce(&context).unwrap();
    assert_eq!(response.interval, 1800);
    assert_eq!(response.peer_list.len(), 1);
    assert_eq!(response.peer_list[0].ip, "1.2.3.4");
    assert_eq!(response.peer_list[0].port, 6881);
    
    handle.join().unwrap();
}
