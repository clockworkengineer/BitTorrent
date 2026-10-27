//! In-Memory Virtual Swarm Test Framework
//!
//! Validates Attribute 8 of the BitTorrent Library Refactoring Plan:
//! Multi-peer in-memory swarm download completing 100% data transfer with SHA-1
//! hash verification in < 200 ms without physical sockets or disk I/O.

use bittorrent_rs::{
    TorrentStatus,
    constants::BLOCK_SIZE,
    internals::TorrentContext,
    io_traits::{BlockReader, BlockWriter, MemStorage, MockSocket, Socket},
    network::peer::{Peer, PeerAction},
    peer_message::PeerMessage,
    selector::SequentialSelector,
    session::config::SessionConfig,
};
use sha1::Digest;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// An in-memory virtual peer node within the simulated swarm.
pub struct VirtualSwarmNode {
    pub id: String,
    pub is_seeder: bool,
    pub context: Arc<Mutex<TorrentContext>>,
    pub storage: Arc<MemStorage>,
    pub peers: HashMap<String, Arc<Mutex<Peer>>>,
    pub inboxes: HashMap<String, VecDeque<Vec<u8>>>,
}

/// Decodes a wire-format packet including the 4-byte big-endian length prefix.
fn decode_wire_packet<'a>(
    packet: &'a [u8],
) -> Result<PeerMessage<'a>, bittorrent_rs::BitTorrentError> {
    if packet.len() < 4 {
        return Err(bittorrent_rs::BitTorrentError::Parse(
            "Packet shorter than 4-byte length prefix".into(),
        ));
    }
    let length = u32::from_be_bytes([packet[0], packet[1], packet[2], packet[3]]) as usize;
    if length == 0 {
        return Ok(PeerMessage::KeepAlive);
    }
    if packet.len() < 4 + length {
        return Err(bittorrent_rs::BitTorrentError::Parse(
            "Truncated packet payload".into(),
        ));
    }
    PeerMessage::decode(&packet[4..4 + length])
}

impl VirtualSwarmNode {
    pub fn new(
        id: &str,
        is_seeder: bool,
        num_pieces: usize,
        piece_length: u32,
        info_hash: &[u8; 20],
        pieces_info_hash: &[u8],
        initial_data: Option<&[u8]>,
    ) -> Self {
        let total_bytes = num_pieces as u64 * piece_length as u64;
        let storage = Arc::new(MemStorage::new(total_bytes as usize));

        if let Some(data) = initial_data {
            storage
                .write_block(0, data)
                .expect("Failed to write initial seed data");
        }

        let mut config = SessionConfig::default();
        config.enable_async_verification = false; // Synchronous verification for immediate deterministic in-memory checks
        config.super_seeding = false;

        let selector = Arc::new(SequentialSelector);
        let mut ctx = TorrentContext::new_magnet_bootstrap(
            info_hash.to_vec(),
            vec!["http://virtual.swarm/announce".to_string()],
            selector,
            std::path::Path::new("."),
            config,
        )
        .expect("Bootstrap failed");

        ctx.pieces_info_hash = pieces_info_hash.to_vec();
        ctx.init_pieces(num_pieces, piece_length);
        ctx.total_bytes_to_download = total_bytes;
        ctx.transfer_metrics
            .set_total_bytes_to_download(total_bytes);
        ctx.storage = storage.clone();

        if is_seeder {
            ctx.status = TorrentStatus::Seeding;
            for p in 0..num_pieces as u32 {
                ctx.mark_piece_local(p, true);
                ctx.update_bitfield_status(p, true, piece_length);
            }
        } else {
            ctx.status = TorrentStatus::Downloading;
            for p in 0..num_pieces as u32 {
                ctx.mark_piece_missing(p, true);
            }
        }

        VirtualSwarmNode {
            id: id.to_string(),
            is_seeder,
            context: Arc::new(Mutex::new(ctx)),
            storage,
            peers: HashMap::new(),
            inboxes: HashMap::new(),
        }
    }

    /// Progresses a single step: picks piece requests and handles queued incoming messages.
    pub fn step(&mut self) -> Vec<(String, String, Vec<u8>)> {
        let mut outgoing_packets = Vec::new();
        let mut actions_to_execute = Vec::new();

        // 1. Process all queued incoming wire packets
        for (remote_id, inbox) in &mut self.inboxes {
            while let Some(packet) = inbox.pop_front() {
                if let Ok(msg) = decode_wire_packet(&packet) {
                    if let Some(peer_arc) = self.peers.get(remote_id) {
                        let mut peer = peer_arc.lock().unwrap();
                        let mut ctx_guard = self.context.lock().unwrap();
                        if let Ok(actions) = peer.handle_peer_message(msg, &mut ctx_guard) {
                            for action in actions {
                                actions_to_execute.push((remote_id.clone(), action));
                            }
                        }
                    }
                }
            }
        }

        // 2. Execute resulting actions (e.g. SendPiece)
        for (remote_id, action) in actions_to_execute {
            match action {
                PeerAction::SendPiece {
                    index,
                    begin,
                    block,
                } => {
                    let msg = PeerMessage::Piece {
                        index,
                        begin,
                        block: &block,
                    };
                    outgoing_packets.push((self.id.clone(), remote_id, msg.encode()));
                }
                PeerAction::SendHave(p) => {
                    let msg = PeerMessage::Have(p);
                    outgoing_packets.push((self.id.clone(), remote_id, msg.encode()));
                }
                _ => {}
            }
        }

        // 3. If downloading, generate next block requests for unchoked peers
        if !self.is_seeder {
            let mut new_requests = Vec::new();
            {
                let mut ctx = self.context.lock().unwrap();
                for (remote_id, peer_arc) in &self.peers {
                    let mut peer = peer_arc.lock().unwrap();
                    if peer.peer_choking.wait_one(0) && peer.outstanding_requests_count < 8 {
                        while peer.outstanding_requests_count < 4 {
                            if let Some((pn, begin, length)) =
                                ctx.next_block_request_for_peer(&peer)
                            {
                                peer.outstanding_requests_count += 1;
                                let bi = begin / BLOCK_SIZE as u32;
                                peer.reserved_blocks.push((pn, bi, Instant::now()));
                                new_requests.push((remote_id.clone(), pn, begin, length));
                            } else {
                                break;
                            }
                        }
                    }
                }
            }

            for (remote_id, pn, begin, length) in new_requests {
                let msg = PeerMessage::Request {
                    index: pn,
                    begin,
                    length,
                };
                outgoing_packets.push((self.id.clone(), remote_id, msg.encode()));
            }
        }

        outgoing_packets
    }
}

/// In-memory virtual swarm orchestrator.
pub struct VirtualSwarm {
    pub num_pieces: usize,
    pub piece_length: u32,
    pub total_bytes: usize,
    pub info_hash: [u8; 20],
    pub pieces_info_hash: Vec<u8>,
    pub expected_data: Vec<u8>,
    pub nodes: HashMap<String, VirtualSwarmNode>,
}

impl VirtualSwarm {
    /// Creates a new virtual swarm with deterministic test payload.
    pub fn new(num_pieces: usize, piece_length: u32) -> Self {
        let total_bytes = num_pieces * piece_length as usize;
        let mut expected_data = Vec::with_capacity(total_bytes);

        let mut pieces_info_hash = Vec::new();
        for p in 0..num_pieces {
            let piece_byte = ((p * 37 + 13) % 251) as u8;
            let piece_slice = vec![piece_byte; piece_length as usize];
            expected_data.extend_from_slice(&piece_slice);

            let hash = sha1::Sha1::digest(&piece_slice);
            pieces_info_hash.extend_from_slice(&hash);
        }

        let info_hash = [0x5A; 20];

        VirtualSwarm {
            num_pieces,
            piece_length,
            total_bytes,
            info_hash,
            pieces_info_hash,
            expected_data,
            nodes: HashMap::new(),
        }
    }

    /// Adds a Seeder node possessing 100% of the pieces.
    pub fn add_seeder(&mut self, id: &str) {
        let node = VirtualSwarmNode::new(
            id,
            true,
            self.num_pieces,
            self.piece_length,
            &self.info_hash,
            &self.pieces_info_hash,
            Some(&self.expected_data),
        );
        self.nodes.insert(id.to_string(), node);
    }

    /// Adds a partial Seeder node possessing a specific subset of pieces.
    pub fn add_partial_seeder(&mut self, id: &str, pieces: &[u32]) {
        let node = VirtualSwarmNode::new(
            id,
            true,
            self.num_pieces,
            self.piece_length,
            &self.info_hash,
            &self.pieces_info_hash,
            None,
        );

        // Populate only specified pieces
        for &p in pieces {
            let start = p as usize * self.piece_length as usize;
            let end = start + self.piece_length as usize;
            node.storage
                .write_block(start as u64, &self.expected_data[start..end])
                .unwrap();
            let mut ctx = node.context.lock().unwrap();
            ctx.mark_piece_local(p, true);
            ctx.update_bitfield_status(p, true, self.piece_length);
        }

        self.nodes.insert(id.to_string(), node);
    }

    /// Adds a Leecher node starting with 0 pieces.
    pub fn add_leecher(&mut self, id: &str) {
        let node = VirtualSwarmNode::new(
            id,
            false,
            self.num_pieces,
            self.piece_length,
            &self.info_hash,
            &self.pieces_info_hash,
            None,
        );
        self.nodes.insert(id.to_string(), node);
    }

    /// Connects two nodes with in-memory sockets and exchanges initial handshakes and bitfields.
    pub fn connect(&mut self, node_a: &str, node_b: &str) {
        let (sock_a, sock_b) = MockSocket::pair();
        let sock_a = Arc::new(Socket::Mock(sock_a));
        let sock_b = Arc::new(Socket::Mock(sock_b));

        // Create peer representations
        let mut peer_on_a = Peer::new_with_socket(node_b.to_string(), 6881, sock_a);
        peer_on_a.connected = true;
        peer_on_a.am_interested = true;
        peer_on_a.am_choking = false;

        let mut peer_on_b = Peer::new_with_socket(node_a.to_string(), 6881, sock_b);
        peer_on_b.connected = true;
        peer_on_b.am_interested = true;
        peer_on_b.am_choking = false;

        // Exchange bitfields
        let bitfield_a = self
            .nodes
            .get(node_a)
            .unwrap()
            .context
            .lock()
            .unwrap()
            .bitfield
            .clone();
        let bitfield_b = self
            .nodes
            .get(node_b)
            .unwrap()
            .context
            .lock()
            .unwrap()
            .bitfield
            .clone();

        peer_on_a.remote_piece_bitfield = bitfield_b.clone();
        peer_on_b.remote_piece_bitfield = bitfield_a.clone();

        // If either is seeder, unchoke the other
        if self.nodes.get(node_a).unwrap().is_seeder {
            peer_on_b.peer_choking.set();
        }
        if self.nodes.get(node_b).unwrap().is_seeder {
            peer_on_a.peer_choking.set();
        }

        // Add to peer swarm in context
        {
            let a = self.nodes.get_mut(node_a).unwrap();
            let arc_peer = Arc::new(Mutex::new(peer_on_a));
            a.context.lock().unwrap().add_peer(arc_peer.clone());
            a.peers.insert(node_b.to_string(), arc_peer);
            a.inboxes.insert(node_b.to_string(), VecDeque::new());
        }

        {
            let b = self.nodes.get_mut(node_b).unwrap();
            let arc_peer = Arc::new(Mutex::new(peer_on_b));
            b.context.lock().unwrap().add_peer(arc_peer.clone());
            b.peers.insert(node_a.to_string(), arc_peer);
            b.inboxes.insert(node_a.to_string(), VecDeque::new());
        }

        // Deliver initial Unchoke and Bitfield messages to inboxes
        if self.nodes.get(node_a).unwrap().is_seeder {
            let inbox_b = self
                .nodes
                .get_mut(node_b)
                .unwrap()
                .inboxes
                .get_mut(node_a)
                .unwrap();
            inbox_b.push_back(PeerMessage::Bitfield(&bitfield_a).encode());
            inbox_b.push_back(PeerMessage::Unchoke.encode());
        }
        if self.nodes.get(node_b).unwrap().is_seeder {
            let inbox_a = self
                .nodes
                .get_mut(node_a)
                .unwrap()
                .inboxes
                .get_mut(node_b)
                .unwrap();
            inbox_a.push_back(PeerMessage::Bitfield(&bitfield_b).encode());
            inbox_a.push_back(PeerMessage::Unchoke.encode());
        }
    }

    /// Runs simulation iterations until the target leecher is 100% complete.
    pub fn run_until_complete(
        &mut self,
        leecher_id: &str,
        max_steps: usize,
    ) -> Result<std::time::Duration, String> {
        let start = Instant::now();

        for step_idx in 0..max_steps {
            // Check if leecher completed download
            let is_complete = {
                let node = self.nodes.get(leecher_id).ok_or("Leecher not found")?;
                let ctx = node.context.lock().unwrap();
                ctx.is_download_complete()
            };

            if is_complete {
                return Ok(start.elapsed());
            }

            // Step each node and collect in-flight packets
            let mut all_packets = Vec::new();
            for node in self.nodes.values_mut() {
                let packets = node.step();
                all_packets.extend(packets);
            }

            if all_packets.is_empty() {
                // Check once more before declaring deadlock
                let is_complete = {
                    let node = self.nodes.get(leecher_id).unwrap();
                    let ctx = node.context.lock().unwrap();
                    ctx.is_download_complete()
                };
                if is_complete {
                    return Ok(start.elapsed());
                }
                return Err(format!(
                    "Simulation stalled at step {}: no in-flight packets and leecher not complete",
                    step_idx
                ));
            }

            // Deliver packets to recipient inboxes
            for (sender_id, recipient_id, packet) in all_packets {
                if let Some(recipient) = self.nodes.get_mut(&recipient_id) {
                    if let Some(inbox) = recipient.inboxes.get_mut(&sender_id) {
                        inbox.push_back(packet);
                    }
                }
            }
        }

        Err(format!(
            "Simulation reached max_steps ({}) without completion",
            max_steps
        ))
    }

    /// Verifies byte-for-byte integrity between expected data and leecher memory storage.
    pub fn verify_integrity(&self, leecher_id: &str) -> bool {
        let leecher = match self.nodes.get(leecher_id) {
            Some(n) => n,
            None => return false,
        };

        let mut actual_data = vec![0u8; self.total_bytes];
        if leecher.storage.read_block(0, &mut actual_data).is_err() {
            return false;
        }

        actual_data == self.expected_data
    }
}

#[test]
fn test_virtual_swarm_single_seeder_single_leecher() {
    // 4 pieces of 16 KiB = 64 KiB
    let mut swarm = VirtualSwarm::new(4, BLOCK_SIZE as u32);
    swarm.add_seeder("seeder-1");
    swarm.add_leecher("leecher-1");
    swarm.connect("seeder-1", "leecher-1");

    let elapsed = swarm
        .run_until_complete("leecher-1", 500)
        .expect("Virtual swarm single seeder download failed");

    // Attribute 8: In-memory swarm download completed in <200 ms
    assert!(
        elapsed.as_millis() < 200,
        "Virtual swarm execution took {:?}, exceeding 200 ms threshold",
        elapsed
    );

    // Verify 100% data integrity
    assert!(
        swarm.verify_integrity("leecher-1"),
        "Leecher storage data mismatch with expected payload"
    );

    // Verify bitfield shows 100% complete
    let leecher = swarm.nodes.get("leecher-1").unwrap();
    let ctx = leecher.context.lock().unwrap();
    assert_eq!(ctx.progress_percent(), 100.0);
    assert_eq!(ctx.status, TorrentStatus::Seeding);
}

#[test]
fn test_virtual_swarm_multi_seeder_distribution() {
    // 4 pieces total: seeder A has pieces [0, 1], seeder B has pieces [2, 3]
    let mut swarm = VirtualSwarm::new(4, BLOCK_SIZE as u32);
    swarm.add_partial_seeder("seeder-A", &[0, 1]);
    swarm.add_partial_seeder("seeder-B", &[2, 3]);
    swarm.add_leecher("leecher-multi");

    swarm.connect("seeder-A", "leecher-multi");
    swarm.connect("seeder-B", "leecher-multi");

    let elapsed = swarm
        .run_until_complete("leecher-multi", 500)
        .expect("Multi-seeder swarm download failed");

    assert!(
        elapsed.as_millis() < 200,
        "Multi-seeder swarm took {:?}, exceeding 200 ms threshold",
        elapsed
    );

    assert!(
        swarm.verify_integrity("leecher-multi"),
        "Multi-seeder leecher data integrity verification failed"
    );

    let leecher = swarm.nodes.get("leecher-multi").unwrap();
    let ctx = leecher.context.lock().unwrap();
    assert_eq!(ctx.progress_percent(), 100.0);
}

#[test]
fn test_virtual_swarm_multi_peer_cross_distribution() {
    // 4 pieces = 64 KiB
    let mut swarm = VirtualSwarm::new(4, BLOCK_SIZE as u32);
    swarm.add_seeder("origin-seed");
    swarm.add_leecher("leecher-alpha");
    swarm.add_leecher("leecher-beta");

    // Form swarm mesh: Seeder connected to both leechers, and leechers connected to each other
    swarm.connect("origin-seed", "leecher-alpha");
    swarm.connect("origin-seed", "leecher-beta");
    swarm.connect("leecher-alpha", "leecher-beta");

    let elapsed = swarm
        .run_until_complete("leecher-alpha", 500)
        .expect("Leecher alpha download failed");

    assert!(elapsed.as_millis() < 200);
    assert!(swarm.verify_integrity("leecher-alpha"));

    let elapsed_beta = swarm
        .run_until_complete("leecher-beta", 500)
        .expect("Leecher beta download failed");

    assert!(elapsed_beta.as_millis() < 200);
    assert!(swarm.verify_integrity("leecher-beta"));
}
