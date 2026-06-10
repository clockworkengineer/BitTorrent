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
