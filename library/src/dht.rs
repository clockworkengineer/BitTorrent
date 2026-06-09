use std::net::{UdpSocket, ToSocketAddrs};
use std::sync::{Arc, Mutex, mpsc::{Sender, channel}};
use std::thread;
use std::time::Duration;
use alloc::vec::Vec;
use alloc::string::{String, ToString};
use alloc::collections::BTreeMap;
use crate::error::BitTorrentError;
use crate::bencode::{BNode, Bencode};
use crate::tracker::PeerDetails;
use sha1::Digest;

pub type NodeId = [u8; 20];

#[derive(Clone, Debug, PartialEq)]
pub struct DhtNode {
    pub id: NodeId,
    pub ip: String,
    pub port: u16,
}

/// Computes the XOR distance between two Node IDs.
pub fn xor_distance(a: &NodeId, b: &NodeId) -> [u8; 20] {
    let mut dist = [0u8; 20];
    for i in 0..20 {
        dist[i] = a[i] ^ b[i];
    }
    dist
}

/// Counts leading zeros of the XOR distance between two Node IDs.
pub fn count_leading_zeros(a: &NodeId, b: &NodeId) -> usize {
    let mut zeros = 0;
    for i in 0..20 {
        let x = a[i] ^ b[i];
        if x == 0 {
            zeros += 8;
        } else {
            zeros += x.leading_zeros() as usize;
            break;
        }
    }
    zeros
}

/// Kademlia Routing Table grouping nodes into 160 buckets.
#[derive(Debug)]
pub struct RoutingTable {
    pub local_id: NodeId,
    pub buckets: Vec<Vec<DhtNode>>,
}

impl RoutingTable {
    pub fn new(local_id: NodeId) -> Self {
        let mut buckets = Vec::with_capacity(160);
        for _ in 0..160 {
            buckets.push(Vec::new());
        }
        RoutingTable { local_id, buckets }
    }

    /// Adds a node to the routing table.
    pub fn add_node(&mut self, node: DhtNode) {
        if node.id == self.local_id {
            return;
        }
        let bucket_idx = count_leading_zeros(&self.local_id, &node.id).min(159);
        let bucket = &mut self.buckets[bucket_idx];
        
        if let Some(pos) = bucket.iter().position(|n| n.id == node.id) {
            // Update node position to mark it as recently active
            let active_node = bucket.remove(pos);
            bucket.push(active_node);
        } else if bucket.len() < 8 {
            bucket.push(node);
        }
    }

    /// Returns the closest nodes to a target ID.
    pub fn closest_nodes(&self, target: &NodeId, count: usize) -> Vec<DhtNode> {
        let mut all_nodes = Vec::new();
        for bucket in &self.buckets {
            for node in bucket {
                all_nodes.push(node.clone());
            }
        }
        
        all_nodes.sort_by(|a, b| {
            let dist_a = xor_distance(&a.id, target);
            let dist_b = xor_distance(&b.id, target);
            dist_a.cmp(&dist_b)
        });
        
        all_nodes.truncate(count);
        all_nodes
    }
}

// --- KRPC Bencode serialization helpers ---
fn encode_dict_start(out: &mut Vec<u8>) {
    out.push(b'd');
}
fn encode_dict_end(out: &mut Vec<u8>) {
    out.push(b'e');
}
fn encode_string(key: &str, out: &mut Vec<u8>) {
    out.extend_from_slice(format!("{}:{}", key.len(), key).as_bytes());
}
fn encode_bytes(bytes: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(format!("{}:", bytes.len()).as_bytes());
    out.extend_from_slice(bytes);
}

pub struct Dht {
    pub node_id: NodeId,
    pub socket: UdpSocket,
    pub routing_table: Arc<Mutex<RoutingTable>>,
    pub outstanding_queries: Arc<Mutex<BTreeMap<Vec<u8>, Sender<Vec<u8>>>>>,
    pub peer_cache: Arc<Mutex<BTreeMap<NodeId, Vec<(String, u16)>>>>,
    pub token_salt: u32,
    running: Arc<Mutex<bool>>,
}

impl Dht {
    pub fn new(port: u16) -> Result<Self, BitTorrentError> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", port))
            .or_else(|_| UdpSocket::bind("0.0.0.0:0"))
            .map_err(BitTorrentError::Io)?;
        
        socket.set_read_timeout(Some(Duration::from_millis(500))).map_err(BitTorrentError::Io)?;

        let mut node_id = [0u8; 20];
        for i in 0..20 {
            node_id[i] = rand::random();
        }

        let token_salt = rand::random::<u32>();

        Ok(Dht {
            node_id,
            socket,
            routing_table: Arc::new(Mutex::new(RoutingTable::new(node_id))),
            outstanding_queries: Arc::new(Mutex::new(BTreeMap::new())),
            peer_cache: Arc::new(Mutex::new(BTreeMap::new())),
            token_salt,
            running: Arc::new(Mutex::new(false)),
        })
    }

    /// Encodes a compact peer list element.
    fn encode_compact_peers(peers: &[(String, u16)]) -> Vec<Vec<u8>> {
        let mut result = Vec::new();
        for (ip, port) in peers {
            if let Ok(ip_addr) = ip.parse::<std::net::Ipv4Addr>() {
                let mut peer_bytes = Vec::new();
                peer_bytes.extend_from_slice(&ip_addr.octets());
                peer_bytes.extend_from_slice(&port.to_be_bytes());
                result.push(peer_bytes);
            }
        }
        result
    }

    /// Starts the background UDP socket listener thread.
    pub fn start(&self) -> Result<(), BitTorrentError> {
        let mut running_guard = self.running.lock().unwrap();
        if *running_guard {
            return Ok(());
        }
        *running_guard = true;

        let socket = self.socket.try_clone().map_err(BitTorrentError::Io)?;
        let routing_table = self.routing_table.clone();
        let peer_cache = self.peer_cache.clone();
        let token_salt = self.token_salt;
        let local_id = self.node_id;
        let outstanding_queries = self.outstanding_queries.clone();
        let running = self.running.clone();

        thread::spawn(move || {
            let mut buf = vec![0u8; 2048];
            while *running.lock().unwrap() {
                match socket.recv_from(&mut buf) {
                    Ok((n, src_addr)) => {
                        if let Ok(bnode) = Bencode::decode(&buf[..n]) {
                            let msg_type = bnode.dict_get(b"y").and_then(|n| n.as_string());
                            let tid = bnode.dict_get(b"t").and_then(|n| n.as_string());

                            if let (Some(y), Some(t)) = (msg_type, tid) {
                                if y == b"q" {
                                    // Handle incoming query requests
                                    let query = bnode.dict_get(b"q").and_then(|n| n.as_string());
                                    let args = bnode.dict_get(b"a");
                                    if let (Some(q), Some(a)) = (query, args) {
                                        let sender_id = a.dict_get(b"id").and_then(|n| n.as_string());
                                        if let Some(s_id) = sender_id {
                                            let mut id = [0u8; 20];
                                            if s_id.len() == 20 {
                                                id.copy_from_slice(s_id);
                                                routing_table.lock().unwrap().add_node(DhtNode {
                                                    id,
                                                    ip: src_addr.ip().to_string(),
                                                    port: src_addr.port(),
                                                });
                                            }
                                        }

                                        let mut reply = Vec::new();
                                        encode_dict_start(&mut reply);
                                        // r: response payload
                                        encode_string("r", &mut reply);
                                        encode_dict_start(&mut reply);
                                        
                                        if q == b"ping" {
                                            encode_string("id", &mut reply);
                                            encode_bytes(&local_id, &mut reply);
                                            encode_dict_end(&mut reply); // end r
                                            
                                            // t: tid
                                            encode_string("t", &mut reply);
                                            encode_bytes(t, &mut reply);
                                            // y: "r"
                                            encode_string("y", &mut reply);
                                            encode_string("r", &mut reply);
                                            encode_dict_end(&mut reply);
                                            
                                            let _ = socket.send_to(&reply, src_addr);
                                        } else if q == b"find_node" {
                                            let target_id_bytes = a.dict_get(b"target").and_then(|n| n.as_string());
                                            if let Some(target_bytes) = target_id_bytes {
                                                let mut target = [0u8; 20];
                                                if target_bytes.len() == 20 {
                                                    target.copy_from_slice(target_bytes);
                                                }
                                                let closest = routing_table.lock().unwrap().closest_nodes(&target, 8);
                                                let mut compact_nodes = Vec::new();
                                                for node in closest {
                                                    compact_nodes.extend_from_slice(&node.id);
                                                    if let Ok(ip_addr) = node.ip.parse::<std::net::Ipv4Addr>() {
                                                        compact_nodes.extend_from_slice(&ip_addr.octets());
                                                    } else {
                                                        compact_nodes.extend_from_slice(&[0, 0, 0, 0]);
                                                    }
                                                    compact_nodes.extend_from_slice(&node.port.to_be_bytes());
                                                }

                                                encode_string("id", &mut reply);
                                                encode_bytes(&local_id, &mut reply);
                                                encode_string("nodes", &mut reply);
                                                encode_bytes(&compact_nodes, &mut reply);
                                                encode_dict_end(&mut reply); // end r
                                                
                                                encode_string("t", &mut reply);
                                                encode_bytes(t, &mut reply);
                                                encode_string("y", &mut reply);
                                                encode_string("r", &mut reply);
                                                encode_dict_end(&mut reply);

                                                let _ = socket.send_to(&reply, src_addr);
                                            }
                                        } else if q == b"get_peers" {
                                            let ih_bytes = a.dict_get(b"info_hash").and_then(|n| n.as_string());
                                            if let Some(ih) = ih_bytes {
                                                let mut target = [0u8; 20];
                                                if ih.len() == 20 {
                                                    target.copy_from_slice(ih);
                                                }
                                                
                                                // Generate token
                                                let token = {
                                                    let mut hasher = sha1::Sha1::default();
                                                    hasher.update(src_addr.ip().to_string().as_bytes());
                                                    hasher.update(&token_salt.to_be_bytes());
                                                    hasher.finalize().to_vec()
                                                };

                                                encode_string("id", &mut reply);
                                                encode_bytes(&local_id, &mut reply);

                                                let cached_peers = peer_cache.lock().unwrap().get(&target).cloned();
                                                if let Some(peers) = cached_peers {
                                                    encode_string("token", &mut reply);
                                                    encode_bytes(&token, &mut reply);
                                                    encode_string("values", &mut reply);
                                                    // Encode compact peers list
                                                    let compact = Self::encode_compact_peers(&peers);
                                                    reply.push(b'l');
                                                    for item in compact {
                                                        encode_bytes(&item, &mut reply);
                                                    }
                                                    reply.push(b'e');
                                                } else {
                                                    let closest = routing_table.lock().unwrap().closest_nodes(&target, 8);
                                                    let mut compact_nodes = Vec::new();
                                                    for node in closest {
                                                        compact_nodes.extend_from_slice(&node.id);
                                                        if let Ok(ip_addr) = node.ip.parse::<std::net::Ipv4Addr>() {
                                                            compact_nodes.extend_from_slice(&ip_addr.octets());
                                                        } else {
                                                            compact_nodes.extend_from_slice(&[0, 0, 0, 0]);
                                                        }
                                                        compact_nodes.extend_from_slice(&node.port.to_be_bytes());
                                                    }
                                                    encode_string("nodes", &mut reply);
                                                    encode_bytes(&compact_nodes, &mut reply);
                                                    encode_string("token", &mut reply);
                                                    encode_bytes(&token, &mut reply);
                                                }
                                                encode_dict_end(&mut reply); // end r
                                                
                                                encode_string("t", &mut reply);
                                                encode_bytes(t, &mut reply);
                                                encode_string("y", &mut reply);
                                                encode_string("r", &mut reply);
                                                encode_dict_end(&mut reply);

                                                let _ = socket.send_to(&reply, src_addr);
                                            }
                                        } else if q == b"announce_peer" {
                                            let ih_bytes = a.dict_get(b"info_hash").and_then(|n| n.as_string());
                                            let port_node = a.dict_get(b"port").and_then(|n| n.as_number_bytes());
                                            let token_bytes = a.dict_get(b"token").and_then(|n| n.as_string());
                                            
                                            if let (Some(ih), Some(p_bytes), Some(tok)) = (ih_bytes, port_node, token_bytes) {
                                                let p_str = String::from_utf8_lossy(p_bytes);
                                                let port = p_str.parse::<u16>().unwrap_or(0);
                                                let expected_token = {
                                                    let mut hasher = sha1::Sha1::default();
                                                    hasher.update(src_addr.ip().to_string().as_bytes());
                                                    hasher.update(&token_salt.to_be_bytes());
                                                    hasher.finalize().to_vec()
                                                };

                                                if tok == expected_token.as_slice() && port > 0 {
                                                    let mut target = [0u8; 20];
                                                    if ih.len() == 20 {
                                                        target.copy_from_slice(ih);
                                                    }
                                                    let mut cache = peer_cache.lock().unwrap();
                                                    let list = cache.entry(target).or_insert_with(Vec::new);
                                                    let ip = src_addr.ip().to_string();
                                                    if !list.iter().any(|(existing_ip, existing_port)| *existing_ip == ip && *existing_port == port) {
                                                        list.push((ip, port));
                                                    }
                                                }

                                                encode_string("id", &mut reply);
                                                encode_bytes(&local_id, &mut reply);
                                                encode_dict_end(&mut reply); // end r
                                                
                                                encode_string("t", &mut reply);
                                                encode_bytes(t, &mut reply);
                                                encode_string("y", &mut reply);
                                                encode_string("r", &mut reply);
                                                encode_dict_end(&mut reply);

                                                let _ = socket.send_to(&reply, src_addr);
                                            }
                                        }
                                    }
                                } else if y == b"r" {
                                    // Match incoming response to outstanding query
                                    let mut queries = outstanding_queries.lock().unwrap();
                                    if let Some(sender) = queries.remove(t) {
                                        let _ = sender.send(buf[..n].to_vec());
                                    }
                                }
                            }
                        }
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        // Periodic socket check
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stops the background listener thread.
    pub fn stop(&self) {
        *self.running.lock().unwrap() = false;
    }



    /// Performs bootstrap seeding from common routers.
    pub fn bootstrap(&self) {
        let routers = vec![
            "router.bittorrent.com:6881",
            "router.utorrent.com:6881",
            "dht.transmissionbt.com:6881",
        ];

        let local_id = self.node_id;
        for router in routers {
            let mut out = Vec::new();
            encode_dict_start(&mut out);
            encode_string("a", &mut out);
            encode_dict_start(&mut out);
            encode_string("id", &mut out);
            encode_bytes(&local_id, &mut out);
            encode_string("target", &mut out);
            encode_bytes(&local_id, &mut out);
            encode_dict_end(&mut out);
            
            encode_string("q", &mut out);
            encode_string("find_node", &mut out);
            
            let tid = vec![1, 2];
            encode_string("t", &mut out);
            encode_bytes(&tid, &mut out);
            
            encode_string("y", &mut out);
            encode_string("q", &mut out);
            encode_dict_end(&mut out);

            // Seed bootstrap nodes asynchronously
            let socket = self.socket.try_clone().unwrap();
            let addr_str = router.to_string();
            thread::spawn(move || {
                if let Ok(mut resolved) = addr_str.to_socket_addrs() {
                    if let Some(addr) = resolved.next() {
                        let _ = socket.send_to(&out, addr);
                    }
                }
            });
        }
    }

    /// Recursively queries nodes closer to a target info_hash to discover active peers.
    pub fn lookup_peers(&self, info_hash: NodeId, peer_sender: Sender<PeerDetails>) {
        let routing_table = self.routing_table.clone();
        let socket_clone = self.socket.try_clone().unwrap();
        let local_id = self.node_id;
        let outstanding_queries = self.outstanding_queries.clone();

        thread::spawn(move || {
            let mut queried = Vec::new();
            let mut candidates = routing_table.lock().unwrap().closest_nodes(&info_hash, 8);

            if candidates.is_empty() {
                // If table is empty, wait for bootstrapping to find nodes
                thread::sleep(Duration::from_millis(500));
                candidates = routing_table.lock().unwrap().closest_nodes(&info_hash, 8);
            }

            let mut count = 0;
            while !candidates.is_empty() && count < 20 {
                let node = candidates.remove(0);
                if queried.contains(&node.id) {
                    continue;
                }
                queried.push(node.id);
                count += 1;

                // Send get_peers query
                let mut out = Vec::new();
                encode_dict_start(&mut out);
                encode_string("a", &mut out);
                encode_dict_start(&mut out);
                encode_string("id", &mut out);
                encode_bytes(&local_id, &mut out);
                encode_string("info_hash", &mut out);
                encode_bytes(&info_hash, &mut out);
                encode_dict_end(&mut out);
                
                encode_string("q", &mut out);
                encode_string("get_peers", &mut out);
                
                let tid = vec![node.id[0], node.id[1]];
                encode_string("t", &mut out);
                encode_bytes(&tid, &mut out);
                encode_string("y", &mut out);
                encode_string("q", &mut out);
                encode_dict_end(&mut out);

                let (tx, rx) = channel();
                outstanding_queries.lock().unwrap().insert(tid.clone(), tx);
                
                let target_addr = format!("{}:{}", node.ip, node.port);
                if let Ok(mut resolved) = target_addr.to_socket_addrs() {
                    if let Some(addr) = resolved.next() {
                        let _ = socket_clone.send_to(&out, addr);
                    }
                }

                // Await response with short timeout
                if let Ok(reply) = rx.recv_timeout(Duration::from_secs(1)) {
                    if let Ok(bnode) = Bencode::decode(&reply) {
                        let r = bnode.dict_get(b"r");
                        if let Some(r_node) = r {
                            // Track nodes we discovered to populate routing table
                            let mut id = [0u8; 20];
                            id.copy_from_slice(&node.id);
                            routing_table.lock().unwrap().add_node(node.clone());

                            // 1. Check if values contains peers list
                            let values = r_node.dict_get(b"values");
                            if let Some(BNode::List(list)) = values {
                                for item in list {
                                    if let Some(peer_bytes) = item.as_string() {
                                        if peer_bytes.len() == 6 {
                                            let ip = format!("{}.{}.{}.{}", peer_bytes[0], peer_bytes[1], peer_bytes[2], peer_bytes[3]);
                                            let port = u16::from_be_bytes(peer_bytes[4..6].try_into().unwrap());
                                            let _ = peer_sender.send(PeerDetails {
                                                info_hash: info_hash.to_vec(),
                                                peer_id: None,
                                                ip,
                                                port,
                                            });
                                        }
                                    }
                                }
                            }

                            // 2. Check closer nodes
                            let nodes = r_node.dict_get(b"nodes");
                            if let Some(BNode::String(nodes_bytes)) = nodes {
                                let closer = parse_compact_nodes_slice(nodes_bytes);
                                for c_node in closer {
                                    if !queried.contains(&c_node.id) && !candidates.iter().any(|c| c.id == c_node.id) {
                                        candidates.push(c_node);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }
}

/// Decodes compact node representations into `DhtNode` objects.
fn parse_compact_nodes_slice(bytes: &[u8]) -> Vec<DhtNode> {
    let mut nodes = Vec::new();
    let mut offset = 0;
    while offset + 26 <= bytes.len() {
        let mut id = [0u8; 20];
        id.copy_from_slice(&bytes[offset..offset+20]);
        let ip = format!("{}.{}.{}.{}", bytes[offset+20], bytes[offset+21], bytes[offset+22], bytes[offset+23]);
        let port = u16::from_be_bytes(bytes[offset+24..offset+26].try_into().unwrap());
        nodes.push(DhtNode { id, ip, port });
        offset += 26;
    }
    nodes
}
