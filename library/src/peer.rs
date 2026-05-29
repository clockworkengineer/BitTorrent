use crate::average::Average;
use crate::error::BitTorrentError;
use crate::manual_reset_event::ManualResetEvent;
use crate::disk_io::DiskIO;
use crate::peer_message::PeerMessage;
use crate::peer_network::PeerNetwork;
use crate::torrent_context::TorrentContext;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

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
}

impl Peer {
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
        }
    }

    pub fn set_torrent_context(&mut self, tc: Arc<Mutex<TorrentContext>>) {
        self.tc = Some(tc.clone());
        let tc_guard = tc.lock().unwrap();
        self.number_of_missing_pieces = tc_guard.number_of_pieces as usize;
        self.remote_piece_bitfield = vec![0u8; tc_guard.bitfield.len()];
    }

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

    pub fn send_message(&self, message: PeerMessage) -> Result<usize, BitTorrentError> {
        let net = self.network.as_ref().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.write_message(message)
    }

    pub fn read_message(&mut self) -> Result<PeerMessage, BitTorrentError> {
        let net = self.network.as_mut().ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "No network available",
            ))
        })?;
        net.read_message()
    }

    pub fn send_interested(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Interested)
    }

    pub fn send_not_interested(&self) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::NotInterested)
    }

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

    pub fn send_have(&self, piece_index: u32) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Have(piece_index))
    }

    pub fn send_bitfield(&self, bitfield: Vec<u8>) -> Result<usize, BitTorrentError> {
        self.send_message(PeerMessage::Bitfield(bitfield))
    }

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

    pub fn is_piece_on_remote_peer(&self, piece_number: u32) -> bool {
        let byte_index = (piece_number >> 3) as usize;
        let bit_mask = 0x80 >> (piece_number & 0x7);
        if let Some(_) = self.tc {
            if byte_index < self.remote_piece_bitfield.len() {
                return (self.remote_piece_bitfield[byte_index] & bit_mask) != 0;
            }
            return false;
        }
        false
    }

    pub fn set_piece_on_remote_peer(&mut self, piece_number: u32) {
        if !self.is_piece_on_remote_peer(piece_number) {
            let byte_index = (piece_number >> 3) as usize;
            let bit_mask = 0x80 >> (piece_number & 0x7);
            if byte_index < self.remote_piece_bitfield.len() {
                self.remote_piece_bitfield[byte_index] |= bit_mask;
            }
            if let Some(tc) = &self.tc {
                tc.lock().unwrap().increment_peer_count(piece_number);
            }
            self.number_of_missing_pieces = self.number_of_missing_pieces.saturating_sub(1);
        }
    }

    pub fn set_remote_bitfield(&mut self, bitfield: Vec<u8>) {
        let previous_bitfield = self.remote_piece_bitfield.clone();
        self.remote_piece_bitfield = bitfield;
        if let Some(tc) = &self.tc {
            let tc_guard = tc.lock().unwrap();
            let number_of_pieces = tc_guard.number_of_pieces as u32;
            drop(tc_guard);
            for piece_number in 0..number_of_pieces {
                if self.is_piece_on_remote_peer(piece_number)
                    && !Peer::bitfield_has_piece(&previous_bitfield, piece_number)
                {
                    self.set_piece_on_remote_peer(piece_number);
                }
            }
        }
    }

    pub fn is_remote_interesting(&self, tc: &TorrentContext) -> bool {
        for piece_number in 0..tc.number_of_pieces as u32 {
            if !tc.is_piece_local(piece_number) && self.is_piece_on_remote_peer(piece_number) {
                return true;
            }
        }
        false
    }

    pub fn handle_peer_message(
        &mut self,
        message: PeerMessage,
        tc: &mut TorrentContext,
        disk_io: &DiskIO,
    ) -> Result<(), BitTorrentError> {
        match message {
            PeerMessage::KeepAlive => {}
            PeerMessage::Choke => {
                self.peer_choking.reset();
            }
            PeerMessage::Unchoke => {
                self.peer_choking.set();
            }
            PeerMessage::Interested => {
                self.peer_interested = true;
            }
            PeerMessage::NotInterested => {
                self.peer_interested = false;
            }
            PeerMessage::Have(index) => {
                self.set_piece_on_remote_peer(index);
            }
            PeerMessage::Bitfield(bitfield) => {
                self.set_remote_bitfield(bitfield);
            }
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => {
                tc.process_piece_block(disk_io, index, begin, &block)?;
            }
            PeerMessage::Cancel { .. } | PeerMessage::Request { .. } | PeerMessage::Port(_) => {}
        }
        Ok(())
    }

    fn bitfield_has_piece(bitfield: &[u8], piece_number: u32) -> bool {
        let byte_index = (piece_number >> 3) as usize;
        let bit_mask = 0x80 >> (piece_number & 0x7);
        if byte_index >= bitfield.len() {
            return false;
        }
        bitfield[byte_index] & bit_mask != 0
    }

    pub fn place_block_into_piece(&mut self, piece_number: u32, block_offset: u32) {
        if let Some(tc) = &self.tc {
            let tc_guard = tc.lock().unwrap();
            let mut assembly_data = tc_guard.assembly_data.lock().unwrap();
            if let Some(piece_buffer) = assembly_data.piece_buffer.clone() {
                let mut buffer_lock = piece_buffer.lock().unwrap();
                if piece_number == buffer_lock.number {
                    let block_number = block_offset / crate::constants::BLOCK_SIZE as u32;
                    let should_decrement = !buffer_lock.blocks_present()[block_number as usize];
                    {
                        let _guard = assembly_data.guard_mutex.lock().unwrap();
                        buffer_lock.add_block_from_packet(&self.read_buffer(), block_number);
                    }
                    if should_decrement {
                        assembly_data.current_block_requests =
                            assembly_data.current_block_requests.saturating_sub(1);
                    }
                    if assembly_data.current_block_requests == 0 {
                        assembly_data.block_requests_done.set();
                    }
                    self.outstanding_requests_count =
                        self.outstanding_requests_count.saturating_sub(1);
                }
            }
        }
    }

    pub fn get_packet_length(&self) -> usize {
        if let Some(net) = &self.network {
            net.packet_length
        } else {
            0
        }
    }

    pub fn read_buffer(&self) -> Vec<u8> {
        if let Some(net) = &self.network {
            net.read_buffer.clone()
        } else {
            Vec::new()
        }
    }
}
