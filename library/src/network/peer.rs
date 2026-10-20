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
use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::error::BitTorrentError;
use crate::io_traits::Socket;
use crate::log_debug;
use crate::manager::Manager;
use crate::manual_reset_event::ManualResetEvent;
use crate::network::peer_event::{ContextEventListener, PeerEventListener};
use crate::peer_message::PeerMessage;
use crate::peer_network::PeerNetwork;
use crate::tracker::PeerDetails;
use crate::util::get_bitfield_index_and_mask;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

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
    SendReject {
        index: u32,
        begin: u32,
        length: u32,
    },
    SendAllowedFast(u32),
    SendSuggest(u32),
    SendHaveAll,
    SendHaveNone,
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
    SendHave(u32),
    SendHaveToPeer {
        target_ip: String,
        piece: u32,
    },
    SendSuggestToPeer {
        target_ip: String,
        piece: u32,
    },
    SendAllowedFastToPeer {
        target_ip: String,
        piece: u32,
    },
    SendHashRequest {
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    },
    SendHashes {
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
        hashes: Vec<u8>,
    },
    SendHashReject {
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    },
    SendHolepunch {
        target_ip: String,
        message: crate::network::holepunch::HolepunchMessage,
    },
    HolepunchConnect(core::net::SocketAddr),
    DiscoverPeers(Vec<PeerDetails>),
}

/// Represents a remote peer connection, holding socket state, bitfield arrays, choking/interest flags, and latency stats.
pub struct Peer {
    pub network: Option<PeerNetwork>,
    pub packet_response_timer: Option<std::time::Instant>,
    pub average_packet_response: Average,
    pub connected: bool,
    pub remote_peer_id: Option<Vec<u8>>,
    pub event_listener: Option<Arc<dyn PeerEventListener>>,
    #[deprecated(note = "Direct TorrentContext coupling is deprecated; use event_listener")]
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
    pub supports_fast_extension: bool,
    pub allowed_fast_pieces: Vec<u32>,
    pub suggested_pieces: Vec<u32>,
    pub supports_holepunch: bool,
    pub connection_type: String,
}

impl Peer {
    /// Creates a new `Peer` representing a remote client connected via the provided socket.
    pub fn new_with_socket(ip: String, port: u16, socket: Arc<Socket>) -> Self {
        let connection_type = if ip.starts_with("webrtc:") {
            "WebRTC".to_string()
        } else {
            match socket.as_ref() {
                #[cfg(feature = "utp")]
                Socket::Utp(_) => "uTP".to_string(),
                #[cfg(feature = "webrtc")]
                Socket::WebRtc(_) => "WebRTC".to_string(),
                _ => "TCP".to_string(),
            }
        };

        Peer {
            network: Some(PeerNetwork::new(socket)),
            packet_response_timer: None,
            average_packet_response: Average::default(),
            connected: false,
            remote_peer_id: None,
            event_listener: None,
            #[allow(deprecated)]
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
            supports_fast_extension: false,
            allowed_fast_pieces: Vec::new(),
            suggested_pieces: Vec::new(),
            supports_holepunch: false,
            connection_type,
        }
    }

    /// Creates a new `Peer` representing a remote browser/client connected via WebRTC Data Channel.
    #[cfg(feature = "webrtc")]
    pub fn new_webrtc(peer_id: &str, adapter: crate::network::webrtc::WebRtcSocketAdapter) -> Self {
        let socket = Arc::new(Socket::WebRtc(adapter));
        let mut p = Self::new_with_socket(format!("webrtc:{}", peer_id), 0, socket);
        p.connection_type = "WebRTC".to_string();
        p
    }

    /// Creates a new `Peer` representing a remote client connected via the provided TCP stream.
    pub fn new(ip: String, port: u16, stream: TcpStream) -> Self {
        let socket = Arc::new(Socket::Tcp(crate::peer_network::TcpSocket::new(stream)));
        Self::new_with_socket(ip, port, socket)
    }

    /// Updates the timestamp when we send a message to this peer.
    pub fn update_last_message_sent(&mut self) {
        self.last_message_sent = std::time::Instant::now();
    }

    /// Sets an abstract event listener sink for peer events (DIP).
    pub fn set_event_listener(&mut self, listener: Arc<dyn PeerEventListener>) {
        self.event_listener = Some(listener);
    }

    /// Initializes the peer bitfield without requiring a TorrentContext (DIP).
    pub fn init_bitfield(&mut self, number_of_pieces: usize, bitfield_len: usize) {
        self.number_of_missing_pieces = number_of_pieces;
        self.remote_piece_bitfield = vec![0u8; bitfield_len];
    }

    /// Links the peer to a specific `TorrentContext`, initializing the peer's remote bitfield capacity.
    pub fn set_torrent_context(&mut self, tc: Arc<Mutex<TorrentContext>>) {
        let (num_pieces, bitfield_len) = {
            let tc_guard = tc.lock().unwrap();
            (tc_guard.number_of_pieces, tc_guard.bitfield.len())
        };
        self.init_bitfield(num_pieces, bitfield_len);
        self.event_listener = Some(Arc::new(ContextEventListener::new(tc.clone())));
        #[allow(deprecated)]
        {
            self.tc = Some(tc);
        }
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
        self.supports_fast_extension = (reserved[7] & 0x04) != 0;
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
    pub async fn read_message<'a>(
        &mut self,
        read_buffer: &'a mut [u8],
    ) -> Result<PeerMessage<'a>, BitTorrentError> {
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
        })
        .await
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
        if let Some(listener) = self.event_listener.take() {
            listener.on_peer_disconnected(&self.ip);
        }
        #[allow(deprecated)]
        {
            self.tc = None;
        }
    }

    /// Checks if the remote peer's bitfield indicates they have the specified piece.
    pub fn is_piece_on_remote_peer(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.remote_piece_bitfield.len() {
            (self.remote_piece_bitfield[byte_index] & bit_mask) != 0
        } else {
            false
        }
    }

    /// Checks if the remote peer has all pieces in the torrent (is a complete seed).
    pub fn is_seed(&self, total_pieces: usize) -> bool {
        if total_pieces == 0 {
            return false;
        }
        (0..total_pieces as u32).all(|p| self.is_piece_on_remote_peer(p))
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

    /// Checks if a piece index is in the Allowed Fast set that we grant to this remote peer.
    pub fn is_peer_allowed_fast(&self, piece: u32, tc: &TorrentContext) -> bool {
        if !self.supports_fast_extension || tc.number_of_pieces == 0 {
            return false;
        }
        let fast_set = crate::util::generate_allowed_fast_set(
            10,
            tc.number_of_pieces,
            &tc.info_hash,
            &self.ip,
        );
        fast_set.contains(&piece)
    }

    /// Computes the canonical Allowed Fast set of pieces that we grant to this remote peer.
    pub fn generate_allowed_fast_set(&self, k: usize, tc: &TorrentContext) -> Vec<u32> {
        if !self.supports_fast_extension || tc.number_of_pieces == 0 {
            return Vec::new();
        }
        crate::util::generate_allowed_fast_set(k, tc.number_of_pieces, &tc.info_hash, &self.ip)
    }

    /// Checks if the remote peer allows us to request a specific piece while choked.
    pub fn is_allowed_fast_from_remote(&self, piece: u32) -> bool {
        self.supports_fast_extension && self.allowed_fast_pieces.contains(&piece)
    }

    /// Creates a `PeerAction::SendHashRequest` to request Merkle tree hashes for a BitTorrent v2 file.
    pub fn request_v2_hashes(
        &self,
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
        #[allow(unused_variables)] tc: &mut TorrentContext,
    ) -> PeerAction {
        #[cfg(feature = "v2")]
        {
            tc.v2_hash_requests_sent
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        PeerAction::SendHashRequest {
            pieces_root,
            base_layer,
            index,
            length,
            proof_layers,
        }
    }

    /// Initiates a BEP 55 Holepunch Rendezvous request to this peer (acting as relay) targeting `target_endpoint`.
    pub fn initiate_holepunch(
        &self,
        target_endpoint: core::net::SocketAddr,
        tc: &mut TorrentContext,
    ) -> PeerAction {
        tc.holepunch_rendezvous_sent
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let message = crate::network::holepunch::HolepunchMessage::rendezvous(target_endpoint);
        PeerAction::SendHolepunch {
            target_ip: self.ip.clone(),
            message,
        }
    }

    #[inline]
    fn push_super_seed_actions(&self, piece: u32, actions: &mut Vec<PeerAction>) {
        actions.push(PeerAction::SendHave(piece));
        if self.supports_fast_extension {
            actions.push(PeerAction::SendSuggest(piece));
            actions.push(PeerAction::SendAllowedFast(piece));
        }
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
                log_debug!("[peer {}:{}] CHOKED by remote", self.ip, self.port);
                if self.peer_choking.wait_one(0) {
                    tc.unchoked_peers_count
                        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
                self.peer_choking.reset();
                if let Some(ref listener) = self.event_listener {
                    listener.on_peer_choked(&self.ip);
                }
            }
            PeerMessage::Unchoke => {
                log_debug!("[peer {}:{}] UNCHOKED by remote", self.ip, self.port);
                if !self.peer_choking.wait_one(0) {
                    tc.unchoked_peers_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                self.peer_choking.set();
                if let Some(ref listener) = self.event_listener {
                    listener.on_peer_unchoked(&self.ip);
                }
            }
            PeerMessage::Interested => {
                self.peer_interested = true;
                if tc.is_super_seeding() {
                    if let Some(piece) =
                        tc.offer_super_seed_piece(&self.ip, &self.remote_piece_bitfield)
                    {
                        self.push_super_seed_actions(piece, &mut actions);
                    }
                }
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
                if let Some(ref listener) = self.event_listener {
                    listener.on_peer_have(&self.ip, index);
                }
                if tc.is_super_seeding() {
                    if self.is_seed(tc.number_of_pieces) {
                        tc.set_super_seeding(false);
                    } else {
                        if let Some((target_ip, next_piece)) = tc.on_peer_have(&self.ip, index) {
                            if target_ip == self.ip {
                                self.push_super_seed_actions(next_piece, &mut actions);
                            } else {
                                actions.push(PeerAction::SendHaveToPeer {
                                    target_ip: target_ip.clone(),
                                    piece: next_piece,
                                });
                                actions.push(PeerAction::SendSuggestToPeer {
                                    target_ip: target_ip.clone(),
                                    piece: next_piece,
                                });
                                actions.push(PeerAction::SendAllowedFastToPeer {
                                    target_ip,
                                    piece: next_piece,
                                });
                            }
                        }
                        if let Some(piece) =
                            tc.offer_super_seed_piece(&self.ip, &self.remote_piece_bitfield)
                        {
                            self.push_super_seed_actions(piece, &mut actions);
                        }
                    }
                }
            }
            PeerMessage::Bitfield(bitfield) => {
                self.set_remote_bitfield(bitfield.to_vec());
                tc.merge_piece_bitfield(self);
                if let Some(ref listener) = self.event_listener {
                    listener.on_peer_bitfield(&self.ip, bitfield);
                }
                if tc.is_super_seeding() {
                    if self.is_seed(tc.number_of_pieces) {
                        tc.set_super_seeding(false);
                    } else if let Some(piece) =
                        tc.offer_super_seed_piece(&self.ip, &self.remote_piece_bitfield)
                    {
                        self.push_super_seed_actions(piece, &mut actions);
                    }
                }
            }
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => {
                self.bytes_downloaded_in_interval += block.len();
                self.outstanding_requests_count = self.outstanding_requests_count.saturating_sub(1);
                let block_index = begin / crate::constants::BLOCK_SIZE as u32;
                let request_time = self
                    .reserved_blocks
                    .iter()
                    .find(|&&(p, b, _)| p == index && b == block_index)
                    .map(|&(_, _, t)| t);
                self.reserved_blocks
                    .retain(|&(p, b, _)| !(p == index && b == block_index));
                if let Some(sent_time) = request_time {
                    let rtt = sent_time.elapsed();
                    tc.record_peer_block_sample(&self.ip, rtt, block.len());
                }
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
                let piece_complete =
                    tc.process_piece_block(&*storage, index, begin, block, &self.ip)?;
                if let Some(ref listener) = self.event_listener {
                    listener.on_block_received(&self.ip, index, begin, block);
                }
                // In endgame mode, cancel duplicate requests to other peers for the same block.
                if piece_complete {
                    self.suggested_pieces.retain(|&p| p != index);
                    self.allowed_fast_pieces.retain(|&p| p != index);
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
                let is_allowed_fast =
                    self.supports_fast_extension && self.is_peer_allowed_fast(index, tc);
                let can_serve = (!self.am_choking || is_allowed_fast)
                    && tc.is_piece_local(index)
                    && tc.can_upload_piece_to_peer(&self.ip, index);

                if can_serve {
                    let offset = index as u64 * tc.piece_length as u64 + begin as u64;
                    let mut block = vec![0u8; length as usize];
                    match tc.storage.read_block(offset, &mut block) {
                        Ok(_) => {
                            actions.push(PeerAction::SendPiece {
                                index,
                                begin,
                                block,
                            });
                            tc.total_bytes_uploaded
                                .fetch_add(length as u64, std::sync::atomic::Ordering::Relaxed);
                            self.bytes_uploaded_in_interval += length as usize;
                        }
                        Err(e) => {
                            log_debug!(
                                "[peer {}:{}] failed to read piece {} for upload: {}",
                                self.ip,
                                self.port,
                                index,
                                e
                            );
                            if self.supports_fast_extension {
                                actions.push(PeerAction::SendReject {
                                    index,
                                    begin,
                                    length,
                                });
                            }
                        }
                    }
                } else if self.supports_fast_extension {
                    // BEP 6: Reject requests for pieces that cannot be satisfied or when choked
                    actions.push(PeerAction::SendReject {
                        index,
                        begin,
                        length,
                    });
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
                                                if let Ok(key_str) = core::str::from_utf8(key_bytes)
                                                {
                                                    self.extension_ids
                                                        .insert(key_str.to_string(), id);
                                                    if key_str == "ut_holepunch" {
                                                        self.supports_holepunch = true;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(crate::bencode::BNode::Number(num_bytes)) =
                            bnode.dict_get(b"metadata_size")
                        {
                            if let Ok(num_str) = core::str::from_utf8(num_bytes) {
                                if let Ok(size) = num_str.parse::<usize>() {
                                    self.metadata_size = Some(size);
                                    tc.metadata_size = Some(size);
                                }
                            }
                        }
                    }
                } else if ext_id == 1 {
                    if let Ok((bnode, consumed)) = crate::bencode::Bencode::decode_partial(payload)
                    {
                        let msg_type = bnode
                            .dict_get(b"msg_type")
                            .and_then(|n| n.as_number_bytes())
                            .and_then(|b| core::str::from_utf8(b).ok())
                            .and_then(|s| s.parse::<u8>().ok());
                        let piece = bnode
                            .dict_get(b"piece")
                            .and_then(|n| n.as_number_bytes())
                            .and_then(|b| core::str::from_utf8(b).ok())
                            .and_then(|s| s.parse::<u32>().ok());

                        match msg_type {
                            Some(1) => {
                                // Data
                                if let Some(p) = piece {
                                    let raw_data = &payload[consumed..];
                                    tc.metadata_pieces.insert(p, raw_data.to_vec());
                                    if let Some(crate::bencode::BNode::Number(num_bytes)) =
                                        bnode.dict_get(b"total_size")
                                    {
                                        if let Ok(num_str) = core::str::from_utf8(num_bytes) {
                                            if let Ok(size) = num_str.parse::<usize>() {
                                                self.metadata_size = Some(size);
                                                tc.metadata_size = Some(size);
                                            }
                                        }
                                    }
                                }
                            }
                            Some(0) => {
                                // Request
                                if let Some(ref metadata) = tc.info_dict_bytes {
                                    if let Some(p) = piece {
                                        let offset = p as usize * 16384;
                                        if offset < metadata.len() {
                                            let end = (offset + 16384).min(metadata.len());
                                            let chunk = &metadata[offset..end];
                                            let header = alloc::format!(
                                                "d8:msg_typei1e5:piecei{}e10:total_sizei{}ee",
                                                p,
                                                metadata.len()
                                            );
                                            let mut res_payload = header.into_bytes();
                                            res_payload.extend_from_slice(chunk);
                                            actions.push(PeerAction::SendExtended {
                                                ext_id: *self
                                                    .extension_ids
                                                    .get("ut_metadata")
                                                    .unwrap_or(&1),
                                                payload: res_payload,
                                            });
                                        }
                                    }
                                } else {
                                    if let Some(p) = piece {
                                        let header =
                                            alloc::format!("d8:msg_typei2e5:piecei{}eee", p);
                                        actions.push(PeerAction::SendExtended {
                                            ext_id: *self
                                                .extension_ids
                                                .get("ut_metadata")
                                                .unwrap_or(&1),
                                            payload: header.into_bytes(),
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                } else if ext_id == 2 {
                    if !tc.is_pex_enabled() {
                        return Ok(actions);
                    }
                    if let Ok(pex_msg) =
                        crate::network::pex::PexMessage::decode(payload, &tc.info_hash)
                    {
                        let am_seed = tc.status == TorrentStatus::Seeding
                            || tc.is_download_complete()
                            || (tc.number_of_pieces > 0
                                && (0..tc.number_of_pieces as u32).all(|p| tc.is_piece_local(p)));
                        let mut discovered = Vec::new();

                        for peer in pex_msg.added_v4 {
                            if am_seed && peer.flags.is_seed() {
                                continue;
                            }
                            discovered.push(peer.details);
                        }

                        if tc.config.enable_ipv6 {
                            for peer in pex_msg.added_v6 {
                                if am_seed && peer.flags.is_seed() {
                                    continue;
                                }
                                discovered.push(peer.details);
                            }
                        }

                        tc.record_pex_added(discovered.len());

                        let mut dropped = pex_msg.dropped_v4;
                        if tc.config.enable_ipv6 {
                            dropped.extend(pex_msg.dropped_v6);
                        }
                        tc.record_pex_dropped(&dropped);

                        if !discovered.is_empty() {
                            actions.push(PeerAction::DiscoverPeers(discovered));
                        }
                    }
                } else if ext_id == 3 {
                    if !tc.is_holepunch_enabled() {
                        return Ok(actions);
                    }
                    if let Ok(hp_msg) = crate::network::holepunch::HolepunchMessage::decode(payload)
                    {
                        match hp_msg.msg_type {
                            crate::network::holepunch::HolepunchMessageType::Rendezvous => {
                                tc.holepunch_rendezvous_received
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                match tc.handle_holepunch_rendezvous(
                                    &self.ip,
                                    self.port,
                                    hp_msg.endpoint,
                                ) {
                                    Ok((connect_initiator, connect_target)) => {
                                        tc.holepunch_connect_sent
                                            .fetch_add(2, std::sync::atomic::Ordering::Relaxed);
                                        actions.push(PeerAction::SendHolepunch {
                                            target_ip: self.ip.clone(),
                                            message: connect_initiator,
                                        });
                                        actions.push(PeerAction::SendHolepunch {
                                            target_ip: hp_msg.endpoint.ip().to_string(),
                                            message: connect_target,
                                        });
                                    }
                                    Err(err_code) => {
                                        tc.holepunch_error_sent
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                        let err_msg =
                                            crate::network::holepunch::HolepunchMessage::error(
                                                hp_msg.endpoint,
                                                err_code,
                                            );
                                        actions.push(PeerAction::SendHolepunch {
                                            target_ip: self.ip.clone(),
                                            message: err_msg,
                                        });
                                    }
                                }
                            }
                            crate::network::holepunch::HolepunchMessageType::Connect => {
                                tc.holepunch_connect_received
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let endpoint_ip = hp_msg.endpoint.ip().to_string();
                                if !tc.is_peer_in_swarm(&endpoint_ip) {
                                    actions.push(PeerAction::HolepunchConnect(hp_msg.endpoint));
                                }
                            }
                            crate::network::holepunch::HolepunchMessageType::Error => {
                                tc.holepunch_error_received
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
            PeerMessage::Suggest(index) => {
                self.suggested_pieces.push(index);
            }
            PeerMessage::HaveAll => {
                if self.remote_piece_bitfield.len() != tc.bitfield.len() {
                    self.remote_piece_bitfield = vec![0xFF; tc.bitfield.len()];
                } else {
                    self.remote_piece_bitfield.fill(0xFF);
                }
                self.number_of_missing_pieces = 0;
                tc.merge_piece_bitfield(self);
            }
            PeerMessage::HaveNone => {
                if self.remote_piece_bitfield.len() != tc.bitfield.len() {
                    self.remote_piece_bitfield = vec![0x00; tc.bitfield.len()];
                } else {
                    self.remote_piece_bitfield.fill(0x00);
                }
                self.number_of_missing_pieces = tc.number_of_pieces;
            }
            PeerMessage::Reject {
                index,
                begin,
                length: _length,
            } => {
                let block_index = begin / crate::constants::BLOCK_SIZE as u32;
                self.reserved_blocks
                    .retain(|&(p, b, _)| !(p == index && b == block_index));
                self.outstanding_requests_count = self.outstanding_requests_count.saturating_sub(1);
                tc.release_block_request(index, block_index);
            }
            PeerMessage::AllowedFast(index) => {
                self.allowed_fast_pieces.push(index);
            }
            PeerMessage::HashRequest {
                pieces_root,
                base_layer,
                index,
                length,
                proof_layers,
            } => {
                #[cfg(feature = "v2")]
                {
                    tc.v2_hash_requests_received
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    match tc.handle_hash_request(
                        pieces_root,
                        base_layer,
                        index,
                        length,
                        proof_layers,
                    ) {
                        Ok(hashes) => {
                            tc.v2_hashes_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            actions.push(PeerAction::SendHashes {
                                pieces_root,
                                base_layer,
                                index,
                                length,
                                proof_layers,
                                hashes,
                            });
                        }
                        Err(_) => {
                            tc.v2_hash_rejects_sent
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            actions.push(PeerAction::SendHashReject {
                                pieces_root,
                                base_layer,
                                index,
                                length,
                                proof_layers,
                            });
                        }
                    }
                }
                #[cfg(not(feature = "v2"))]
                {
                    actions.push(PeerAction::SendHashReject {
                        pieces_root,
                        base_layer,
                        index,
                        length,
                        proof_layers,
                    });
                }
            }
            PeerMessage::Hashes {
                pieces_root,
                base_layer,
                index,
                length,
                proof_layers,
                hashes,
            } => {
                #[cfg(feature = "v2")]
                {
                    tc.v2_hashes_received
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let _ = tc.handle_hashes_response(
                        pieces_root,
                        base_layer,
                        index,
                        length,
                        proof_layers,
                        hashes,
                    );
                }
            }
            PeerMessage::HashReject { .. } => {
                #[cfg(feature = "v2")]
                {
                    tc.v2_hash_rejects_received
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendReject {
                    index,
                    begin,
                    length,
                } => {
                    let msg = PeerMessage::Reject {
                        index,
                        begin,
                        length,
                    };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendAllowedFast(piece) => {
                    net.write_message(PeerMessage::AllowedFast(piece)).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendSuggest(piece) => {
                    net.write_message(PeerMessage::Suggest(piece)).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHaveAll => {
                    net.write_message(PeerMessage::HaveAll).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHaveNone => {
                    net.write_message(PeerMessage::HaveNone).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendExtended { ext_id, payload } => {
                    let msg = PeerMessage::Extended {
                        ext_id,
                        payload: &payload,
                    };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHave(piece) => {
                    net.write_message(PeerMessage::Have(piece)).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHaveToPeer { target_ip, piece } => {
                    let target_net = {
                        let ctx_guard = context.lock().unwrap();
                        let swarm = ctx_guard.peer_swarm.read().unwrap();
                        swarm
                            .get(&target_ip)
                            .and_then(|p| p.lock().ok())
                            .and_then(|p| p.network.clone())
                    };
                    if let Some(target_net) = target_net {
                        let _ = target_net.write_message(PeerMessage::Have(piece)).await;
                    }
                }
                PeerAction::SendSuggestToPeer { target_ip, piece } => {
                    let (target_net, supports_fast) = {
                        let ctx_guard = context.lock().unwrap();
                        let swarm = ctx_guard.peer_swarm.read().unwrap();
                        if let Some(other_peer) = swarm.get(&target_ip).and_then(|p| p.lock().ok())
                        {
                            (
                                other_peer.network.clone(),
                                other_peer.supports_fast_extension,
                            )
                        } else {
                            (None, false)
                        }
                    };
                    if let Some(target_net) = target_net {
                        if supports_fast {
                            let _ = target_net.write_message(PeerMessage::Suggest(piece)).await;
                        }
                    }
                }
                PeerAction::SendAllowedFastToPeer { target_ip, piece } => {
                    let (target_net, supports_fast) = {
                        let ctx_guard = context.lock().unwrap();
                        let swarm = ctx_guard.peer_swarm.read().unwrap();
                        if let Some(other_peer) = swarm.get(&target_ip).and_then(|p| p.lock().ok())
                        {
                            (
                                other_peer.network.clone(),
                                other_peer.supports_fast_extension,
                            )
                        } else {
                            (None, false)
                        }
                    };
                    if let Some(target_net) = target_net {
                        if supports_fast {
                            let _ = target_net
                                .write_message(PeerMessage::AllowedFast(piece))
                                .await;
                        }
                    }
                }
                PeerAction::SendHashRequest {
                    pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                } => {
                    let msg = PeerMessage::HashRequest {
                        pieces_root,
                        base_layer,
                        index,
                        length,
                        proof_layers,
                    };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHashes {
                    pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                    hashes,
                } => {
                    let msg = PeerMessage::Hashes {
                        pieces_root,
                        base_layer,
                        index,
                        length,
                        proof_layers,
                        hashes: &hashes,
                    };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHashReject {
                    pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                } => {
                    let msg = PeerMessage::HashReject {
                        pieces_root,
                        base_layer,
                        index,
                        length,
                        proof_layers,
                    };
                    net.write_message(msg).await?;
                    peer.lock().unwrap().update_last_message_sent();
                }
                PeerAction::SendHolepunch { target_ip, message } => {
                    let target_info = {
                        let ctx_guard = context.lock().unwrap();
                        let swarm = ctx_guard.peer_swarm.read().unwrap();
                        swarm
                            .get(&target_ip)
                            .and_then(|p| p.lock().ok())
                            .and_then(|p| {
                                p.network.clone().map(|n| {
                                    (n, *p.extension_ids.get("ut_holepunch").unwrap_or(&3))
                                })
                            })
                    };
                    if let Some((target_net, ext_id)) = target_info {
                        let payload = message.encode();
                        let msg = PeerMessage::Extended {
                            ext_id,
                            payload: &payload,
                        };
                        let _ = target_net.write_message(msg).await;
                    }
                }
                PeerAction::HolepunchConnect(addr) => {
                    let ip = addr.ip().to_string();
                    let port = addr.port();
                    let (details, should_connect) = {
                        let ctx = context.lock().unwrap();
                        let details = crate::network::tracker::PeerDetails {
                            ip: ip.clone(),
                            port,
                            peer_id: None,
                            info_hash: ctx.info_hash.clone(),
                        };
                        let is_v6_disabled = details.is_ipv6() && !ctx.config.enable_ipv6;
                        let should = !is_v6_disabled
                            && !ctx.is_peer_in_swarm(&ip)
                            && !ctx.is_peer_blacklisted(&ip);
                        (details, should)
                    };
                    if should_connect {
                        let is_dead = manager
                            .as_ref()
                            .map(|m| m.is_peer_dead(&ip))
                            .unwrap_or(false);
                        if !is_dead {
                            let tc_arc = {
                                let p = peer.lock().unwrap();
                                #[allow(deprecated)]
                                p.tc.clone()
                            };
                            if let Some(ctx2) = tc_arc {
                                let mgr2 = manager.clone();
                                std::thread::spawn(move || {
                                    futures::executor::block_on(
                                        crate::session::worker::handle_peer_session(
                                            details, ctx2, mgr2,
                                        ),
                                    );
                                });
                            }
                        }
                    }
                }
                PeerAction::BroadcastCancel {
                    index,
                    begin,
                    length,
                    block_index,
                } => {
                    let peers: Vec<(String, Arc<Mutex<Peer>>)> = {
                        let ctx_guard = context.lock().unwrap();
                        let swarm = ctx_guard.peer_swarm.read().unwrap();
                        swarm
                            .iter()
                            .filter(|&(ip, _)| ip != peer_details_ip)
                            .map(|(ip, peer_arc)| (ip.clone(), peer_arc.clone()))
                            .collect()
                    };

                    let mut nets_to_cancel = Vec::new();
                    for (_ip, peer_arc) in peers {
                        if let Ok(other_peer) = peer_arc.lock() {
                            let should_send = match block_index {
                                Some(bi) => other_peer
                                    .reserved_blocks
                                    .iter()
                                    .any(|&(p, b, _)| p == index && b == bi),
                                None => true,
                            };
                            if should_send {
                                if let Some(ref net) = other_peer.network {
                                    nets_to_cancel.push(net.clone());
                                }
                            }
                        }
                    }

                    for peer_net in nets_to_cancel {
                        let _ = peer_net
                            .write_message(PeerMessage::Cancel {
                                index,
                                begin,
                                length,
                            })
                            .await;
                    }
                }
                PeerAction::DiscoverPeers(peers) => {
                    for details in peers {
                        let should_connect = {
                            let ctx = context.lock().unwrap();
                            let is_v6_disabled = details.is_ipv6() && !ctx.config.enable_ipv6;
                            !is_v6_disabled
                                && !ctx.is_peer_in_swarm(&details.ip)
                                && !ctx.is_peer_blacklisted(&details.ip)
                        };
                        if should_connect {
                            if let Some(mgr) = manager {
                                if mgr.is_peer_dead(&details.ip) {
                                    continue;
                                }
                            }
                            let tc_arc = {
                                let p = peer.lock().unwrap();
                                #[allow(deprecated)]
                                p.tc.clone()
                            };
                            if let Some(ctx2) = tc_arc {
                                let mgr2 = manager.clone();
                                std::thread::spawn(move || {
                                    futures::executor::block_on(
                                        crate::session::worker::handle_peer_session(
                                            details, ctx2, mgr2,
                                        ),
                                    );
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
