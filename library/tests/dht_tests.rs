#![cfg(all(feature = "std", feature = "dht"))]

use bittorrent_rs::dht::{count_leading_zeros, xor_distance, RoutingTable, DhtNode, Dht};
use std::sync::mpsc::channel;

#[test]
fn test_count_leading_zeros() {
    let a = [0x00; 20];
    let b = [0x00; 20];
    assert_eq!(count_leading_zeros(&a, &b), 160);

    let mut c = [0x00; 20];
    c[0] = 0x80; // leading zero of XOR should be 0
    assert_eq!(count_leading_zeros(&a, &c), 0);

    let mut d = [0x00; 20];
    d[0] = 0x01; // leading zeros should be 7
    assert_eq!(count_leading_zeros(&a, &d), 7);
}

#[test]
fn test_xor_distance() {
    let a = [0x55; 20];
    let b = [0xAA; 20];
    let expected = [0xFF; 20];
    assert_eq!(xor_distance(&a, &b), expected);
}

#[test]
fn test_routing_table() {
    let local_id = [0x00; 20];
    let mut table = RoutingTable::new(local_id);

    let mut node1_id = [0x00; 20];
    node1_id[19] = 1;
    let node1 = DhtNode {
        id: node1_id,
        ip: "1.1.1.1".to_string(),
        port: 1000,
    };

    let mut node2_id = [0x00; 20];
    node2_id[19] = 2;
    let node2 = DhtNode {
        id: node2_id,
        ip: "2.2.2.2".to_string(),
        port: 2000,
    };

    table.add_node(node1.clone());
    table.add_node(node2.clone());

    let closest = table.closest_nodes(&local_id, 1);
    assert_eq!(closest.len(), 1);
    assert_eq!(closest[0].id, node1_id);

    let closest_two = table.closest_nodes(&local_id, 5);
    assert_eq!(closest_two.len(), 2);
}

#[test]
fn test_dht_query_server_and_client() {
    // Instantiate DHT on random ports
    let dht1 = Dht::new(0).unwrap();
    let dht2 = Dht::new(0).unwrap();

    dht1.start().unwrap();
    dht2.start().unwrap();

    // Populate dht1's routing table with dht2
    let dht2_node = DhtNode {
        id: dht2.node_id,
        ip: "127.0.0.1".to_string(),
        port: dht2.socket.local_addr().unwrap().port(),
    };
    dht1.routing_table.lock().unwrap().add_node(dht2_node.clone());

    // Perform recursive lookup for dummy info hash
    let (tx, _rx) = channel();
    let info_hash = [0xbb; 20];
    dht1.lookup_peers(info_hash, tx);

    // Stop services
    dht1.stop();
    dht2.stop();
}
