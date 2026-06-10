//! Remote peer connection and state tracking
//!
//! Models a connection to a remote BitTorrent peer. Handles peer wire protocol
//! state, message transmitting/receiving, block requesting, and bitfield syncing.
//!
//! # Peer Wire Protocol State Machine
//!
//! ```text
//!       [ Start / Discovered ]
//!                 |
//!                 v  (TCP Connection Established)
//!        [ Handshake Sent ]
//!                 |
//!                 v  (Receive Peer Handshake & Bitfield)
//!       [ Connected / Handshaked ]
//!                 |
//!        +--------+--------+
//!        |                 |
//!        v                 v
//!   (Our State)       (Peer State)
//!  Am Interested     Peer Choking
//!  Am Choking        Peer Interested
//!        |                 |
//!        v (We Unchoke)    v (Peer Unchoke)
//!  +------------+    +------------+
//!  | Can Upload |    | Can Request| <---+ (Send Request)
//!  +------------+    +------------+     |
//!                          |            | (Receive Block)
//!                          +------------+
//! ```

use crate::average::Average;
use crate::io_traits::AsyncSocket;
use crate::error::BitTorrentError;
use crate::manual_reset_event::ManualResetEvent;
use crate::peer_message::PeerMessage;
use crate::peer_network::PeerNetwork;
use crate::torrent_context::TorrentContext;
use crate::tracker::PeerDetails;
use crate::manager::Manager;
use crate::util::get_bitfield_index_and_mask;
use crate::log_debug;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use alloc::vec::Vec;
use alloc::string::String;
use alloc::collections::BTreeMap;
use alloc::string::ToString;

/// Actions returned by message handling to be executed asynchronously outside locks.
#[derive(Debug)]
pub enum PeerAction {
    SendUnchoke,
    SendChoke,
    SendPiece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    BroadcastCancel {
        index: u32,
        begin: u32,
        length: u32,
        block_index: Option<u32>,
    },
    SendExtended {
        ext_id: u8,
        payload: Vec<u8>,
    },
    DiscoverPeers(Vec<PeerDetails>),
}

/// Represents a remote peer connection, holding socket state, bitfield arrays, choking/interest flags, and latency stats.
pub struct Peer {
    pub network: Option<PeerNetwork>,
    pub packet_response_timer: Option<std::time::Instant>,
    pub average_packet_response: Average,
    pub connected: bool,
    pub remote_peer_id: Option<Vec<u8>>,
    pub tc: Option<Arc<Mutex<TorrentContext>>>,
    pub remote_piece_bitfield: Vec<u8>,
    pub ip: String,
    pub port: u16,
    pub am_interested: bool,
    pub am_choking: bool,
    pub peer_choking: ManualResetEvent,
    pub peer_interested: bool,
    pub number_of_missing_pieces: usize,
    pub outstanding_requests_count: usize,
    pub reserved_blocks: Vec<(u32, u32, std::time::Instant)>,
    pub supports_extensions: bool,
    pub extension_ids: BTreeMap<String, u8>,
    pub metadata_size: Option<usize>,
    pub bytes_downloaded_in_interval: usize,
    pub bytes_uploaded_in_interval: usize,
    pub rolling_download_rate: f64,
    pub rolling_upload_rate: f64,
    pub last_message_sent: std::time::Instant,
    pub last_message_received: std::time::Instant,
}

impl Peer {
    /// Creates a new `Peer` representing a remote client connected via the provided socket.
    pub fn new_with_socket(ip: String, port: u16, socket: Arc<dyn AsyncSocket>) -> Self {
        Peer {
            network: Some(PeerNetwork::new(socket)),
            packet_response_timer: None,
            average_packet_response: Average::default(),
            connected: false,
            remote_peer_id: None,
            tc: None,
            remote_piece_bitfield: Vec::new(),
            ip,
            port,
            am_interested: false,
            am_choking: true,
            peer_choking: ManualResetEvent::new(false),
            peer_interested: false,
            number_of_missing_pieces: 0,
            outstanding_requests_count: 0,
            reserved_blocks: Vec::new(),
            supports_extensions: false,
            extension_ids: BTreeMap::new(),
            metadata_size: None,
            bytes_downloaded_in_interval: 0,
            bytes_uploaded_in_interval: 0,
            rolling_download_rate: 0.0,
            rolling_upload_rate: 0.0,
            last_message_sent: std::time::Instant::now(),
            last_message_received: std::time::Instant::now(),
        }
    }

    /// Creates a new `Peer` representing a remote client connected via the provided TCP stream.
    pub fn new(ip: String, port: u16, stream: TcpStream) -> Self {
        let socket = Arc::new(crate::peer_network::TcpSocket::new(stream));
        Self::new_with_socket(ip, port, socket)
    }

    /// Updates the timestamp when we send a message to this peer.
    pub fn update_last_message_sent(&mut self) {
        self.last_message_sent = std::time::Instant::now();
    }

    /// Links the peer to a specific `TorrentContext`, initializing the peer's remote bitfield capacity.
    pub fn set_torrent_context(&mut self, tc: Arc<Mutex<TorrentContext>>) {
        self.tc = Some(tc.clone());
        let tc_guard = tc.lock().unwrap();
        self.number_of_missing_pieces = tc_guard.number_of_pieces;
        self.remote_piece_bitfield = vec![0u8; tc_guard.bitfield.len()];
    }

    /// Helper to write raw bytes to the peer connection stream.
    pub async fn write(&self, buffer: &[u8]) -> Result<usize, BitTorrentError> {
        if let Some(net) = &self.network {
            net.write(buffer).await
        } else {
            Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            )))
        }
    }

    /// Helper to read raw bytes from the peer connection stream.
    pub async fn read(&self, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        if let Some(net) = &self.network {
            net.read(buffer, buffer.len()).await
        } else {
            Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            )))
        }
    }

    /// Performs the BitTorrent handshake over the socket, verifying info hash correctness.
    pub async fn handshake(
        &mut self,
        info_hash: &[u8],
        local_peer_id: &[u8],
    ) -> Result<Vec<u8>, BitTorrentError> {
        let net = self.network.as_ref().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.write_handshake(info_hash, local_peer_id).await?;
        let (remote_info_hash, remote_peer_id, reserved) = net.read_handshake().await?;
        if remote_info_hash != info_hash {
            return Err(BitTorrentError::Parse(
                "Peer handshake info hash mismatch".into(),
            ));
        }
        self.connected = true;
        self.remote_peer_id = Some(remote_peer_id.clone());
        self.supports_extensions = (reserved[5] & 0x10) != 0;
        net.start_reads();
        Ok(remote_peer_id)
    }

    /// Sends an encoded `PeerMessage` to the remote peer.
    pub async fn send_message(&self, message: PeerMessage<'_>) -> Result<usize, BitTorrentError> {
        let net = self.network.as_ref().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.write_message(message).await
    }

    /// Receives and decodes the next message from the remote peer.
    pub async fn read_message<'a>(&mut self, read_buffer: &'a mut [u8]) -> Result<PeerMessage<'a>, BitTorrentError> {
        let net = self.network.as_mut().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.read_message(read_buffer).await
    }

    /// Transmits an Interested message to the peer.
    pub async fn send_interested(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Interested).await
    }

    /// Transmits a Not Interested message to the peer.
    pub async fn send_not_interested(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::NotInterested).await
    }

    /// Transmits a Request message to download a specific block.
    pub async fn send_request(
        &self,
        index: u32,
        begin: u32,
        length: u32,
    ) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Request {
            index,
            begin,
            length,
        }).await
    }

    /// Transmits a Have message to announce possession of a complete piece.
    pub async fn send_have(&self, piece_index: u32) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Have(piece_index)).await
    }

    /// Transmits a Bitfield message to share local piece availability.
    pub async fn send_bitfield(&self, bitfield: &[u8]) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Bitfield(bitfield)).await
    }

    /// Transmits an Unchoke message to inform peers we are willing to serve.
    pub async fn send_unchoke(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Unchoke).await
    }

    /// Closes the peer network connection.
    pub fn close(&mut self) {
        self.connected = false;
        if let Some(net) = &self.network {
            net.close();
        }
        self.network = None;
    }

    /// Checks if the remote peer's bitfield indicates they have the specified piece.
    pub fn is_piece_on_remote_peer(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if let Some(_) = self.tc {
            if byte_index < self.remote_piece_bitfield.len() {
                return (self.remote_piece_bitfield[byte_index] & bit_mask) != 0;
            }
            return false;
        }
        false
    }

    /// Marks the specified piece as complete on the remote peer and updates missing piece counts.
    pub fn set_piece_on_remote_peer(&mut self, piece_number: u32) {
        if !self.is_piece_on_remote_peer(piece_number) {
            let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
            if byte_index < self.remote_piece_bitfield.len() {
                self.remote_piece_bitfield[byte_index] |= bit_mask;
            }
            self.number_of_missing_pieces = self.number_of_missing_pieces.saturating_sub(1);
        }
    }

    /// Sets the entire remote bitfield vector and updates the count of missing pieces.
    pub fn set_remote_bitfield(&mut self, bitfield: Vec<u8>) {
        self.remote_piece_bitfield = bitfield;
        let pieces_on_remote: usize = self
            .remote_piece_bitfield
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum();
        self.number_of_missing_pieces = self
            .number_of_missing_pieces
            .saturating_sub(pieces_on_remote);
    }

    /// Checks if this remote peer has any pieces that we still need to download.
    pub fn is_remote_interesting(&self, tc: &TorrentContext) -> bool {
        for piece_number in 0..tc.number_of_pieces as u32 {
            if !tc.is_piece_local(piece_number) && self.is_piece_on_remote_peer(piece_number) {
                return true;
            }
        }
        false
    }

    /// Processes an incoming protocol message from the peer, updating connection states, logging events, and writing pieces to disk.
    pub fn handle_peer_message(
        &mut self,
        message: PeerMessage<'_>,
        tc: &mut TorrentContext,
    ) -> Result<Vec<PeerAction>, BitTorrentError> {
        self.last_message_received = std::time::Instant::now();
        let mut actions = Vec::new();
        match message {
            PeerMessage::KeepAlive => {}
            PeerMessage::Choke => {
                log_debug!(
                    "[peer {}:{}] CHOKED by remote",
                    self.ip, self.port
                );
                self.peer_choking.reset();
            }
            PeerMessage::Unchoke => {
                log_debug!(
                    "[peer {}:{}] UNCHOKED by remote",
                    self.ip, self.port
                );
                self.peer_choking.set();
            }
            PeerMessage::Interested => {
                self.peer_interested = true;
            }
            PeerMessage::NotInterested => {
                self.peer_interested = false;
            }
            PeerMessage::Have(index) => {
                let was_new = !self.is_piece_on_remote_peer(index);
                self.set_piece_on_remote_peer(index);
                if was_new {
                    tc.increment_peer_count(index);
                }
            }
            PeerMessage::Bitfield(bitfield) => {
                self.set_remote_bitfield(bitfield.to_vec());
                tc.merge_piece_bitfield(self);
            }
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => {
                self.bytes_downloaded_in_interval += block.len();
                self.outstanding_requests_count = self.outstanding_requests_count.saturating_sub(1);
                let block_index = begin / crate::constants::BLOCK_SIZE as u32;
                self.reserved_blocks
                    .retain(|&(p, b, _)| !(p == index && b == block_index));
                if tc.is_endgame() {
                    let cancel_length = std::cmp::min(
                        crate::constants::BLOCK_SIZE as u32,
                        tc.get_piece_length(index).saturating_sub(begin),
                    );
                    actions.push(PeerAction::BroadcastCancel {
                        index,
                        begin,
                        length: cancel_length,
                        block_index: Some(block_index),
                    });
                }

                let storage = tc.storage.clone();
                let piece_complete = tc.process_piece_block(&*storage, index, begin, block, &self.ip)?;
                // In endgame mode, cancel duplicate requests to other peers for the same block.
                if piece_complete {
                    let pieces_remaining = (0..tc.number_of_pieces as u32)
                        .filter(|&p| !tc.is_piece_local(p))
                        .count();
                    if pieces_remaining <= crate::constants::ENDGAME_THRESHOLD {
                        let length = std::cmp::min(
                            crate::constants::BLOCK_SIZE as u32,
                            tc.get_piece_length(index).saturating_sub(begin),
                        );
                        actions.push(PeerAction::BroadcastCancel {
                            index,
                            begin,
                            length,
                            block_index: None,
                        });
                    }
                }
            }
            PeerMessage::Cancel { .. } | PeerMessage::Port(_) => {}
            PeerMessage::Request {
                index,
                begin,
                length,
            } => {
                // Serve the block if we have the piece and are not choking the remote peer.
                if !self.am_choking && tc.is_piece_local(index) {
                    let offset = index as u64 * tc.piece_length as u64 + begin as u64;
                    let mut block = vec![0u8; length as usize];
                    match tc.storage.read_block(offset, &mut block) {
                        Ok(_) => {
                            actions.push(PeerAction::SendPiece {
                                index,
                                begin,
                                block,
                            });
                            tc.total_bytes_uploaded.fetch_add(length as u64, std::sync::atomic::Ordering::Relaxed);
                            self.bytes_uploaded_in_interval += length as usize;
                        }
                        Err(e) => {
                            log_debug!(
                                "[peer {}:{}] failed to read piece {} for upload: {}",
                                self.ip, self.port, index, e
                            );
                        }
                    }
                }
            }
            PeerMessage::Extended { ext_id, payload } => {
                if ext_id == 0 {
                    if let Ok(bnode) = crate::bencode::Bencode::decode(payload) {
                        if let Some(m_node) = bnode.dict_get(b"m") {
                            if let crate::bencode::BNode::Dictionary(entries) = m_node {
                                for (key_bytes, val_node) in entries {
                                    if let crate::bencode::BNode::Number(num_bytes) = val_node {
                                        if let Ok(num_str) = core::str::from_utf8(num_bytes) {
                                            if let Ok(id) = num_str.parse::<u8>() {
                                                if let Ok(key_str) = core::str::from_utf8(key_bytes) {
                                                    self.extension_ids.insert(key_str.to_string(), id);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(crate::bencode::BNode::Number(num_bytes)) = bnode.dict_get(b"metadata_size") {
                            if let Ok(num_str) = core::str::from_utf8(num_bytes) {
                                if let Ok(size) = num_str.parse::<usize>() {
                                    self.metadata_size = Some(size);
                                    tc.metadata_size = Some(size);
                                }
                            }
                        }
                    }
                } else if ext_id == 1 {
                    if let Ok((bnode, consumed)) = crate::bencode::Bencode::decode_partial(payload) {
                        let msg_type = bnode.dict_get(b"msg_type")
                            .and_then(|n| n.as_number_bytes())
                            .and_then(|b| core::str::from_utf8(b).ok())
                            .and_then(|s| s.parse::<u8>().ok());
                        let piece = bnode.dict_get(b"piece")
                            .and_then(|n| n.as_number_bytes())
                            .and_then(|b| core::str::from_utf8(b).ok())
                            .and_then(|s| s.parse::<u32>().ok());
                        
                        match msg_type {
                            Some(1) => { // Data
                                if let Some(p) = piece {
                                    let raw_data = &payload[consumed..];
                                    tc.metadata_pieces.insert(p, raw_data.to_vec());
                                    if let Some(crate::bencode::BNode::Number(num_bytes)) = bnode.dict_get(b"total_size") {
                                        if let Ok(num_str) = core::str::from_utf8(num_bytes) {
                                            if let Ok(size) = num_str.parse::<usize>() {
                                                self.metadata_size = Some(size);
                                                tc.metadata_size = Some(size);
                                            }
                                        }
                                    }
                                }
                            }
                            Some(0) => { // Request
                                if let Some(ref metadata) = tc.info_dict_bytes {
                                    if let Some(p) = piece {
                                        let offset = p as usize * 16384;
                                        if offset < metadata.len() {
                                            let end = (offset + 16384).min(metadata.len());
                                            let chunk = &metadata[offset..end];
                                            let header = alloc::format!("d8:msg_typei1e5:piecei{}e10:total_sizei{}ee", p, metadata.len());
                                            let mut res_payload = header.into_bytes();
                                            res_payload.extend_from_slice(chunk);
                                            actions.push(PeerAction::SendExtended {
                                                ext_id: *self.extension_ids.get("ut_metadata").unwrap_or(&1),
                                                payload: res_payload,
                                            });
                                        }
                                    }
                                } else {
                                    if let Some(p) = piece {
                                        let header = alloc::format!("d8:msg_typei2e5:piecei{}eee", p);
                                        actions.push(PeerAction::SendExtended {
                                            ext_id: *self.extension_ids.get("ut_metadata").unwrap_or(&1),
                                            payload: header.into_bytes(),
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                } else if ext_id == 2 {
                    if let Ok(bnode) = crate::bencode::Bencode::decode(payload) {
                        if let Some(added_node) = bnode.dict_get(b"added") {
                            if let Some(added_bytes) = added_node.as_string() {
                                let mut added_peers = Vec::new();
                                for chunk in added_bytes.chunks_exact(6) {
                                    let ip = format!("{}.{}.{}.{}", chunk[0], chunk[1], chunk[2], chunk[3]);
                                    let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                                    added_peers.push(PeerDetails {
                                        info_hash: tc.info_hash.clone(),
                                        peer_id: None,
                                        ip,
                                        port,
                                    });
                                }
                                actions.push(PeerAction::DiscoverPeers(added_peers));
                            }
                        }
                    }
                }
            }
        }
        Ok(actions)
    }

    /// Executes peer action commands (Unchoke, Choke, SendPiece, BroadcastCancel, DiscoverPeers).
    pub async fn execute_actions(
        peer: &Mutex<Peer>,
        actions: Vec<PeerAction>,
        net: &PeerNetwork,
        context: &Mutex<TorrentContext>,
        peer_details_ip: &str,
        manager: &Option<Arc<Manager>>,
    ) -> Result<(), BitTorrentError> {
        for action in actions {
            match action {
                PeerAction::SendUnchoke => {
                    net.write_message(PeerMessage::Unchoke).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendChoke => {
                    net.write_message(PeerMessage::Choke).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendPiece { index, begin, block } => {
                    let msg = PeerMessage::Piece { index, begin, block: &block };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendExtended { ext_id, payload } => {
                    let msg = PeerMessage::Extended { ext_id, payload: &payload };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::BroadcastCancel { index, begin, length, block_index } => {
                    let peers: Vec<PeerNetwork> = {
                        let ctx_guard = context.lock().unwrap();
                        let swarm = ctx_guard.peer_swarm.read().unwrap();
                        swarm
                            .iter()
                            .filter_map(|(ip, peer_arc)| {
                                if ip == peer_details_ip {
                                    return None;
                                }
                                let other_peer = peer_arc.lock().unwrap();
                                let should_send = match block_index {
                                    Some(bi) => other_peer.reserved_blocks.iter().any(|&(p, b, _)| p == index && b == bi),
                                    None => true,
                                };
                                if should_send {
                                    other_peer.network.clone()
                                } else {
                                    None
                                }
                            })
                            .collect()
                    };
                    for peer_net in peers {
                        let _ = peer_net.write_message(PeerMessage::Cancel { index, begin, length }).await;
                    }
                }
                PeerAction::DiscoverPeers(peers) => {
                    for details in peers {
                        let should_connect = {
                            let ctx = context.lock().unwrap();
                            !ctx.is_peer_in_swarm(&details.ip) && !ctx.is_peer_blacklisted(&details.ip)
                        };
                        if should_connect {
                            if let Some(mgr) = manager {
                                if mgr.is_peer_dead(&details.ip) {
                                    continue;
                                }
                            }
                            let tc_arc = {
                                let p = peer.lock().unwrap();
                                p.tc.clone()
                            };
                            if let Some(ctx2) = tc_arc {
                                let mgr2 = manager.clone();
                                std::thread::spawn(move || {
                                    futures::executor::block_on(crate::session::worker::handle_peer_session(
                                        details, ctx2, mgr2,
                                    ));
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
