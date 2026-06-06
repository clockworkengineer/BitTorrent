//! Remote peer connection and state tracking
//!
//! Models a connection to a remote BitTorrent peer. Handles peer wire protocol
//! state, message transmitting/receiving, block requesting, and bitfield syncing.

use crate::average::Average;
use crate::disk_io::DiskIO;
use crate::error::BitTorrentError;
use crate::manual_reset_event::ManualResetEvent;
use crate::peer_message::PeerMessage;
use crate::peer_network::PeerNetwork;
use crate::torrent_context::TorrentContext;
use crate::util::get_bitfield_index_and_mask;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};

static PEER_LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

/// Appends a debug message to `debug.log`.
fn log(msg: &str) {
    let file = PEER_LOG.get_or_init(|| {
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open("debug.log")
            .expect("cannot open debug.log");
        Mutex::new(f)
    });
    if let Ok(mut f) = file.lock() {
        let _ = writeln!(f, "{}", msg);
        let _ = f.flush();
    }
}

/// Represents a remote peer connection, holding socket state, bitfield arrays, choking/interest flags, and latency stats.
pub struct Peer {
    network: Option<PeerNetwork>,
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
    pub reserved_blocks: Vec<(u32, u32)>,
}

impl Peer {
    /// Creates a new `Peer` representing a remote client connected via the provided TCP stream.
    pub fn new(ip: String, port: u16, stream: TcpStream) -> Self {
        Peer {
            network: Some(PeerNetwork::new(stream)),
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
        }
    }

    /// Links the peer to a specific `TorrentContext`, initializing the peer's remote bitfield capacity.
    pub fn set_torrent_context(&mut self, tc: Arc<Mutex<TorrentContext>>) {
        self.tc = Some(tc.clone());
        let tc_guard = tc.lock().unwrap();
        self.number_of_missing_pieces = tc_guard.number_of_pieces;
        self.remote_piece_bitfield = vec![0u8; tc_guard.bitfield.len()];
    }

    /// Helper to write raw bytes to the peer connection stream.
    pub fn write(&self, buffer: &[u8]) -> std::io::Result<usize> {
        if let Some(net) = &self.network {
            net.write(buffer)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        }
    }

    /// Helper to read raw bytes from the peer connection stream.
    pub fn read(&self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(net) = &self.network {
            net.read(buffer, buffer.len())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        }
    }

    /// Performs the BitTorrent handshake over the socket, verifying info hash correctness.
    pub fn handshake(
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
        net.write_handshake(info_hash, local_peer_id)?;
        let (remote_info_hash, remote_peer_id) = net.read_handshake()?;
        if remote_info_hash != info_hash {
            return Err(BitTorrentError::Parse(
                "Peer handshake info hash mismatch".into(),
            ));
        }
        self.connected = true;
        self.remote_peer_id = Some(remote_peer_id.clone());
        net.start_reads();
        Ok(remote_peer_id)
    }

    /// Sends an encoded `PeerMessage` to the remote peer.
    pub fn send_message(&self, message: PeerMessage) -> Result<usize, BitTorrentError> {
        let net = self.network.as_ref().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.write_message(message)
    }

    /// Receives and decodes the next message from the remote peer.
    pub fn read_message(&mut self) -> Result<PeerMessage, BitTorrentError> {
        let net = self.network.as_mut().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.read_message()
    }

    /// Transmits an Interested message to the peer.
    pub fn send_interested(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Interested)
    }

    /// Transmits a Not Interested message to the peer.
    pub fn send_not_interested(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::NotInterested)
    }

    /// Transmits a Request message to download a specific block.
    pub fn send_request(
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
    }

    /// Transmits a Have message to announce possession of a complete piece.
    pub fn send_have(&self, piece_index: u32) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Have(piece_index))
    }

    /// Transmits a Bitfield message to share local piece availability.
    pub fn send_bitfield(&self, bitfield: Vec<u8>) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Bitfield(bitfield))
    }

    /// Transmits an Unchoke message to inform peers we are willing to serve.
    pub fn send_unchoke(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Unchoke)
    }

    /// Closes the peer network connection and updates local swarm bitfields.
    pub fn close(&mut self) {
        if self.connected {
            if let Some(tc) = &self.tc {
                tc.lock().unwrap().unmerge_piece_bitfield(self);
            }
            self.connected = false;
        }
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
        message: PeerMessage,
        tc: &mut TorrentContext,
        disk_io: &DiskIO,
    ) -> Result<(), BitTorrentError> {
        match message {
            PeerMessage::KeepAlive => {}
            PeerMessage::Choke => {
                log(&format!(
                    "[peer {}:{}] CHOKED by remote",
                    self.ip, self.port
                ));
                self.peer_choking.reset();
            }
            PeerMessage::Unchoke => {
                log(&format!(
                    "[peer {}:{}] UNCHOKED by remote",
                    self.ip, self.port
                ));
                self.peer_choking.set();
            }
            PeerMessage::Interested => {
                self.peer_interested = true;
                if self.am_choking {
                    self.send_unchoke()?;
                    self.am_choking = false;
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
            }
            PeerMessage::Bitfield(bitfield) => {
                self.set_remote_bitfield(bitfield);
                tc.merge_piece_bitfield(self);
            }
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => {
                self.outstanding_requests_count = self.outstanding_requests_count.saturating_sub(1);
                let block_index = begin / crate::constants::BLOCK_SIZE as u32;
                self.reserved_blocks
                    .retain(|&(p, b)| !(p == index && b == block_index));
                if tc.is_endgame() {
                    let cancel_length = std::cmp::min(
                        crate::constants::BLOCK_SIZE as u32,
                        tc.get_piece_length(index).saturating_sub(begin),
                    );
                    self.broadcast_cancel(tc, index, begin, cancel_length, Some(block_index));
                }
                log(&format!(
                    "[peer {}:{}] PIECE index={} begin={} len={} outstanding={}",
                    self.ip,
                    self.port,
                    index,
                    begin,
                    block.len(),
                    self.outstanding_requests_count
                ));
                let piece_complete = tc.process_piece_block(disk_io, index, begin, &block)?;
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
                        self.broadcast_cancel(tc, index, begin, length, None);
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
                    match disk_io.read_piece_block(tc, index, begin, length) {
                        Ok(block) => {
                            let _ = self.send_message(PeerMessage::Piece {
                                index,
                                begin,
                                block,
                            });
                            tc.total_bytes_uploaded += length as u64;
                        }
                        Err(e) => {
                            log(&format!(
                                "[peer {}:{}] failed to read piece {} for upload: {}",
                                self.ip, self.port, index, e
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn broadcast_cancel(
        &self,
        tc: &TorrentContext,
        index: u32,
        begin: u32,
        length: u32,
        block_index: Option<u32>,
    ) {
        let swarm = tc.peer_swarm.read().unwrap();
        for (peer_ip, peer_arc) in swarm.iter() {
            if peer_ip == &self.ip {
                continue;
            }
            if let Ok(other_peer) = peer_arc.try_lock() {
                let should_send = match block_index {
                    Some(bi) => other_peer.reserved_blocks.contains(&(index, bi)),
                    None => true,
                };
                if should_send {
                    let _ = other_peer.send_message(PeerMessage::Cancel {
                        index,
                        begin,
                        length,
                    });
                }
            }
        }
    }

    /// Gets the length of the last parsed packet from the network.
    pub fn get_packet_length(&self) -> usize {
        if let Some(net) = &self.network {
            net.packet_length
        } else {
            0
        }
    }

    /// Returns a copy of the peer's raw TCP read buffer.
    pub fn read_buffer(&self) -> Vec<u8> {
        if let Some(net) = &self.network {
            net.read_buffer.clone()
        } else {
            Vec::new()
        }
    }
}
