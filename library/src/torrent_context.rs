//! Torrent session context
//!
//! Models the state and configuration of an active torrent session, including
//! files, bitfield vectors, missing piece indices, and the connected peer swarm.

use crate::average::Average;
use crate::constants::{BLOCK_SIZE, ENDGAME_THRESHOLD};
use crate::disk_io::DiskIO;
use crate::manual_reset_event::ManualResetEvent;
use crate::metainfo::FileDetails;
use crate::metainfo::MetaInfoFile;
use crate::peer::Peer;
use crate::piece_buffer::PieceBuffer;
use crate::selector::Selector;
use crate::util::get_bitfield_index_and_mask;
use sha1::Digest;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// Placeholder structure representing a tracker client.
#[derive(Debug, Clone)]
pub struct Tracker;

/// Piece availability metadata tracked per piece.
#[derive(Debug, Clone)]
pub struct PieceInfo {
    pub peer_count: usize,
    pub piece_length: u32,
}

/// Assembly queues and stats for downloaded blocks.
#[derive(Debug)]
pub struct AssemblerData {
    pub piece_buffers: HashMap<u32, Arc<Mutex<PieceBuffer>>>,
    pub current_block_requests: usize,
    pub guard_mutex: Mutex<()>,
    pub block_requests_done: ManualResetEvent,
    pub average_assembly_time: Average,
    pub total_timeouts: usize,
}

/// Enumeration of states a torrent transfer session can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentStatus {
    Initialised,
    Seeding,
    Downloading,
    Paused,
    Ended,
}

/// The core session context containing all transfer states, piece indices, and connection maps.
pub struct TorrentContext {
    pub info_hash: Vec<u8>,
    pub tracker_url: String,
    pub tracker_urls: Vec<String>,
    pub number_of_pieces: usize,
    pub piece_length: u32,
    pub pieces_info_hash: Vec<u8>,
    pub bitfield: Vec<u8>,
    pub files_to_download: Vec<FileDetails>,
    pub total_bytes_downloaded: u64,
    pub initial_bytes_downloaded: u64,
    pub total_bytes_to_download: u64,
    pub total_bytes_uploaded: u64,
    pub status: TorrentStatus,
    pub file_name: String,
    pub main_tracker: Option<Tracker>,
    pub callback_data: Option<String>,
    pub call_back: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    pub paused: ManualResetEvent,
    pub download_finished: Arc<ManualResetEvent>,
    pub selector: Selector,
    pub peer_swarm: RwLock<HashMap<String, Arc<Mutex<Peer>>>>,
    pub missing_pieces_count: usize,
    pub maximum_swarm_size: usize,
    pub assembly_data: Mutex<AssemblerData>,
    pub requested_blocks: RwLock<HashSet<(u32, u32)>>,
    pieces_missing: Vec<u8>,
    piece_data: Vec<PieceInfo>,
}

impl TorrentContext {
    /// Creates and initializes a `TorrentContext` from parsed metainfo, creating target directories and scanning existing disk files.
    pub fn new(
        torrent_meta_info: &MetaInfoFile,
        selector: Selector,
        disk_io: &DiskIO,
        download_path: &std::path::Path,
        seeding: bool,
    ) -> Result<Self, crate::error::BitTorrentError> {
        let info_hash = torrent_meta_info.get_info_hash()?;
        let tracker_urls = torrent_meta_info.get_tracker_urls()?;
        let tracker_url = tracker_urls.get(0).cloned().ok_or_else(|| {
            crate::error::BitTorrentError::Parse("Torrent contains no tracker URLs.".into())
        })?;
        let (total_download_length, all_files_to_download) =
            torrent_meta_info.local_files_to_download_list(download_path)?;
        let piece_length = torrent_meta_info.get_piece_length()?;
        let pieces_info_hash = torrent_meta_info.get_pieces_info_hash()?;
        let number_of_pieces = pieces_info_hash.len() / crate::constants::HASH_LENGTH;
        let bitfield = vec![0u8; (number_of_pieces + 7) / 8];
        let pieces_missing = vec![0u8; bitfield.len()];
        let piece_data = vec![
            PieceInfo {
                peer_count: 0,
                piece_length
            };
            number_of_pieces
        ];

        let mut context = TorrentContext {
            info_hash,
            tracker_url,
            number_of_pieces,
            piece_length,
            pieces_info_hash,
            bitfield,
            files_to_download: all_files_to_download,
            total_bytes_downloaded: 0,
            initial_bytes_downloaded: 0,
            total_bytes_to_download: total_download_length,
            total_bytes_uploaded: 0,
            status: if seeding {
                TorrentStatus::Seeding
            } else {
                TorrentStatus::Initialised
            },
            file_name: torrent_meta_info
                .torrent_file_name
                .to_string_lossy()
                .to_string(),
            tracker_urls,
            main_tracker: None,
            callback_data: None,
            call_back: None,
            paused: ManualResetEvent::new(false),
            download_finished: Arc::new(ManualResetEvent::new(false)),
            selector,
            peer_swarm: RwLock::new(HashMap::new()),
            missing_pieces_count: 0,
            maximum_swarm_size: crate::constants::MAXIMUM_SWARM_SIZE,
            assembly_data: Mutex::new(AssemblerData {
                piece_buffers: HashMap::new(),
                current_block_requests: 0,
                guard_mutex: Mutex::new(()),
                block_requests_done: ManualResetEvent::new(false),
                average_assembly_time: Average::default(),
                total_timeouts: 0,
            }),
            requested_blocks: RwLock::new(HashSet::new()),
            pieces_missing,
            piece_data,
        };
        disk_io.create_local_torrent_structure(&context)?;
        if seeding {
            disk_io.fully_downloaded_torrent_bitfield(&mut context)?;
            context.total_bytes_downloaded = context.total_bytes_to_download;
            context.initial_bytes_downloaded = context.total_bytes_to_download;
        } else {
            disk_io.create_torrent_bitfield(&mut context)?;
            context.initial_bytes_downloaded = context.total_bytes_downloaded;
        }
        Ok(context)
    }

    /// Validates the structure and length constraints of context data fields.
    pub fn validate(&self) -> Result<(), crate::error::BitTorrentError> {
        if self.number_of_pieces == 0 {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent must contain at least one piece.".to_string(),
            ));
        }
        if self.piece_length == 0 {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent piece length must be greater than zero.".to_string(),
            ));
        }
        if self.files_to_download.is_empty() {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent contains no files to download.".to_string(),
            ));
        }
        if self.pieces_info_hash.len() != self.number_of_pieces * crate::constants::HASH_LENGTH {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent pieces hash list length does not match number of pieces.".to_string(),
            ));
        }
        let expected_bitfield_length = (self.number_of_pieces + 7) / 8;
        if self.bitfield.len() != expected_bitfield_length {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent bitfield length is invalid.".to_string(),
            ));
        }
        Ok(())
    }

    /// Sets the status to `Downloading` if initialized.
    pub fn start_downloading(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status == TorrentStatus::Downloading {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent is already downloading.".to_string(),
            ));
        }
        if self.status == TorrentStatus::Ended {
            return Err(crate::error::BitTorrentError::Parse(
                "Cannot restart a finished torrent.".to_string(),
            ));
        }
        self.status = TorrentStatus::Downloading;
        Ok(())
    }

    /// Pauses the torrent download thread, updating state flags.
    pub fn pause(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status != TorrentStatus::Downloading {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent can only be paused while downloading.".to_string(),
            ));
        }
        self.status = TorrentStatus::Paused;
        self.paused.set();
        Ok(())
    }

    /// Resumes the paused torrent download thread.
    pub fn resume(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status != TorrentStatus::Paused {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent can only be resumed when paused.".to_string(),
            ));
        }
        self.status = TorrentStatus::Downloading;
        self.paused.reset();
        Ok(())
    }

    /// Transitions the state to `Ended` and signals download completion.
    pub fn stop(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status == TorrentStatus::Ended {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent has already been stopped.".to_string(),
            ));
        }
        self.status = TorrentStatus::Ended;
        self.download_finished.set();
        Ok(())
    }

    /// Computes percentage completion from bytes downloaded versus total.
    pub fn progress_percent(&self) -> f32 {
        if self.total_bytes_to_download == 0 {
            return 100.0;
        }

        let percent =
            self.total_bytes_downloaded as f64 / self.total_bytes_to_download as f64 * 100.0;
        percent.min(100.0) as f32
    }

    /// Sets or clears a specific piece completion bit in the local bitfield.
    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if local {
            self.bitfield[byte_index] |= bit_mask;
        } else {
            self.bitfield[byte_index] &= !bit_mask;
        }
    }

    /// Checks if a specific piece has been fully downloaded and verified locally.
    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        self.bitfield[byte_index] & bit_mask != 0
    }

    /// Sets or clears a specific piece index in the missing pieces tracking vector.
    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if missing {
            if self.pieces_missing[byte_index] & bit_mask == 0 {
                self.pieces_missing[byte_index] |= bit_mask;
                self.missing_pieces_count += 1;
            }
        } else if self.pieces_missing[byte_index] & bit_mask != 0 {
            self.pieces_missing[byte_index] &= !bit_mask;
            self.missing_pieces_count = self.missing_pieces_count.saturating_sub(1);
        }
    }

    /// Checks if a specific piece index is currently marked as missing.
    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        self.pieces_missing[byte_index] & bit_mask != 0
    }

    fn update_piece_peer_counts(&mut self, remote_peer: &Peer, increment: bool) {
        let mut piece_number = 0u32;
        for byte in &remote_peer.remote_piece_bitfield {
            for bit in &[0x80u8, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01] {
                if byte & bit != 0 {
                    let idx = piece_number as usize;
                    if increment {
                        self.piece_data[idx].peer_count += 1;
                    } else {
                        self.piece_data[idx].peer_count = self.piece_data[idx].peer_count.saturating_sub(1);
                    }
                }
                piece_number += 1;
                if piece_number as usize >= self.number_of_pieces {
                    break;
                }
            }
        }
    }

    /// Increments peer counts for all pieces present in a newly connected peer's bitfield.
    pub fn merge_piece_bitfield(&mut self, remote_peer: &Peer) {
        self.update_piece_peer_counts(remote_peer, true);
    }

    /// Computes the SHA-1 checksum of the piece buffer and compares it to the expected metainfo hash.
    pub fn check_piece_hash(
        &self,
        piece_number: u32,
        piece_buffer: &[u8],
        number_of_bytes: u32,
    ) -> bool {
        let hash = sha1::Sha1::digest(&piece_buffer[..number_of_bytes as usize]);
        let offset = piece_number as usize * crate::constants::HASH_LENGTH;
        hash.iter()
            .zip(&self.pieces_info_hash[offset..offset + crate::constants::HASH_LENGTH])
            .all(|(a, b)| a == b)
    }

    /// Returns the number of bytes remaining to be downloaded.
    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        if self.total_bytes_to_download < self.total_bytes_downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(self.total_bytes_to_download - self.total_bytes_downloaded)
    }

    /// Integrates a verified piece buffer, updating local bitfields, download speeds, and completion metrics.
    pub fn update_bitfield_from_buffer(
        &mut self,
        piece_number: u32,
        piece_buffer: &[u8],
        number_of_bytes: u32,
    ) {
        let piece_there = self.check_piece_hash(piece_number, piece_buffer, number_of_bytes);
        if piece_there {
            self.total_bytes_downloaded += number_of_bytes as u64;
        }
        self.set_piece_length(piece_number, number_of_bytes);
        self.mark_piece_local(piece_number, piece_there);
        self.mark_piece_missing(piece_number, !piece_there);
    }

    /// Checks if the session download is complete.
    pub fn is_download_complete(&self) -> bool {
        self.bytes_left_to_download().unwrap_or(1) == 0
    }

    /// Returns true when the torrent has entered endgame mode.
    pub fn is_endgame(&self) -> bool {
        self.missing_pieces_count <= ENDGAME_THRESHOLD
    }

    /// Checks and updates status to `Seeding` if downloading is completed.
    pub fn try_complete_download(&mut self) {
        if self.status == TorrentStatus::Downloading && self.is_download_complete() {
            self.status = TorrentStatus::Seeding;
            self.download_finished.set();
        }
    }

    /// Appends incoming sub-block data, writing the fully assembled piece to disk upon completion and hash validation.
    pub fn process_piece_block(
        &mut self,
        disk_io: &DiskIO,
        piece_number: u32,
        begin: u32,
        block_data: &[u8],
    ) -> Result<bool, crate::error::BitTorrentError> {
        let piece_length = self.get_piece_length(piece_number);
        let block_index = begin / BLOCK_SIZE as u32;

        let mut assembly_data = self.assembly_data.lock().unwrap();
        let piece_buffer_arc = assembly_data
            .piece_buffers
            .entry(piece_number)
            .or_insert_with(|| Arc::new(Mutex::new(PieceBuffer::new(piece_number, piece_length))))
            .clone();
        drop(assembly_data);

        let piece_buffer_arc2 = piece_buffer_arc.clone();
        let mut piece_buffer = piece_buffer_arc2.lock().unwrap();
        piece_buffer.add_block(block_data, block_index);
        let piece_complete = piece_buffer.all_blocks_there();

        if piece_complete {
            let finished_piece = piece_buffer.buffer.clone();
            drop(piece_buffer);

            if self.check_piece_hash(piece_number, &finished_piece, finished_piece.len() as u32) {
                println!(
                    "Piece {} passed hash verification ({} bytes), writing to disk",
                    piece_number,
                    finished_piece.len()
                );
                disk_io.write_piece(self, piece_number, &finished_piece)?;
                self.update_bitfield_from_buffer(
                    piece_number,
                    &finished_piece,
                    finished_piece.len() as u32,
                );
                // Broadcast Have to all connected peers so they know we have this piece.
                {
                    let swarm = self.peer_swarm.read().unwrap();
                    for peer_arc in swarm.values() {
                        if let Ok(peer_guard) = peer_arc.try_lock() {
                            let _ = peer_guard.send_have(piece_number);
                        }
                    }
                }
                self.try_complete_download();
                self.clear_piece_requests(piece_number);
                self.assembly_data
                    .lock()
                    .unwrap()
                    .piece_buffers
                    .remove(&piece_number);
                return Ok(true);
            } else {
                println!("Piece {} failed hash verification", piece_number);
                self.clear_piece_requests(piece_number);
                self.assembly_data
                    .lock()
                    .unwrap()
                    .piece_buffers
                    .remove(&piece_number);
                return Err(crate::error::BitTorrentError::Parse(
                    "Piece failed hash verification".to_string(),
                ));
            }
        }

        Ok(false)
    }

    /// Registers a peer connection in the active swarm. Returns `true` if registered, `false` if swarm is full.
    pub fn add_peer(&self, peer: Arc<Mutex<Peer>>) -> bool {
        let ip = peer.lock().unwrap().ip.clone();
        if self.is_space_in_swarm(&ip) {
            self.peer_swarm.write().unwrap().insert(ip, peer).is_none()
        } else {
            false
        }
    }

    /// Unregisters and drops a peer connection from the swarm by IP address.
    pub fn remove_peer(&self, ip: &str) {
        self.peer_swarm.write().unwrap().remove(ip);
    }

    /// Safely terminates connection streams and unregisters all active peers in the swarm.
    pub fn disconnect_all_peers(&self) {
        let mut swarm = self.peer_swarm.write().unwrap();
        for peer in swarm.values() {
            peer.lock().unwrap().close();
        }
        swarm.clear();
    }

    /// Decrements piece peer counts when a peer disconnects.
    pub fn unmerge_piece_bitfield(&mut self, remote_peer: &Peer) {
        self.update_piece_peer_counts(remote_peer, false);
    }

    /// Returns the length of the specified piece.
    pub fn get_piece_length(&self, piece_number: u32) -> u32 {
        self.piece_data[piece_number as usize].piece_length
    }

    /// Sets the byte length for a given piece index.
    pub fn set_piece_length(&mut self, piece_number: u32, piece_length: u32) {
        if piece_length <= self.piece_length {
            self.piece_data[piece_number as usize].piece_length = piece_length;
        } else {
            panic!("Piece length larger than maximum for torrent.");
        }
    }

    /// Helper asserting whether a peer IP can join the swarm (not duplicate and swarm capacity not exceeded).
    pub fn is_space_in_swarm(&self, ip: &str) -> bool {
        !ip.is_empty()
            && self.peer_swarm.read().unwrap().get(ip).is_none()
            && self.peer_swarm.read().unwrap().len() < self.maximum_swarm_size
    }

    /// Finds the next unrequested block offset and length within a given piece.
    pub fn next_pending_block(&self, piece_number: u32) -> Option<(u32, u32)> {
        let piece_length = self.get_piece_length(piece_number);
        let block_count = ((piece_length as usize + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32;
        for block_index in 0..block_count {
            if !self.is_block_requested(piece_number, block_index) {
                let begin = block_index * BLOCK_SIZE as u32;
                let length = min(BLOCK_SIZE as u32, piece_length.saturating_sub(begin));
                return Some((begin, length));
            }
        }
        None
    }

    /// Checks if a block request has been registered in the context.
    pub fn is_block_requested(&self, piece_number: u32, block_index: u32) -> bool {
        self.requested_blocks
            .read()
            .unwrap()
            .contains(&(piece_number, block_index))
    }

    /// Marks a block index as requested/reserved in the session context.
    pub fn reserve_block_request(&self, piece_number: u32, block_index: u32) -> bool {
        self.requested_blocks
            .write()
            .unwrap()
            .insert((piece_number, block_index))
    }

    /// Releases a block reservation, allowing other peers to request it.
    pub fn release_block_request(&self, piece_number: u32, block_index: u32) {
        self.requested_blocks
            .write()
            .unwrap()
            .remove(&(piece_number, block_index));
    }

    /// Drops all block request reservations registered under a given piece index.
    pub fn clear_piece_requests(&self, piece_number: u32) {
        self.requested_blocks
            .write()
            .unwrap()
            .retain(|(piece, _)| *piece != piece_number);
    }

    fn get_sorted_missing_pieces_for_peer(&self, peer: &Peer) -> Vec<(usize, u32)> {
        let mut candidates: Vec<(usize, u32)> = (0..self.number_of_pieces as u32)
            .filter(|&piece| !self.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
            .map(|piece| (self.piece_data[piece as usize].peer_count, piece))
            .collect();
        candidates.sort_by_key(|(count, piece)| (*count, *piece));
        candidates
    }

    /// Selects the rarest missing piece that the remote peer possesses.
    pub fn select_next_piece_for_peer(&mut self, remote_peer: &Peer) -> Option<u32> {
        self.get_sorted_missing_pieces_for_peer(remote_peer)
            .into_iter()
            .map(|(_, piece)| piece)
            .next()
    }

    /// Identifies and reserves the next download block from a piece available on the specified peer.
    /// In endgame mode (few pieces remaining), allows duplicate requests to speed up completion.
    pub fn next_block_request_for_peer(&mut self, peer: &Peer) -> Option<(u32, u32, u32)> {
        if self.status != TorrentStatus::Downloading {
            return None;
        }
        let endgame = self.is_endgame();
        let candidates = self.get_sorted_missing_pieces_for_peer(peer);

        let pieces_remaining = candidates.len();
        let in_endgame = endgame && pieces_remaining > 0;

        for (_, piece_number) in candidates {
            if in_endgame {
                // Endgame: request any block of the piece, even if already reserved by another peer.
                let piece_length = self.get_piece_length(piece_number);
                let block_count = ((piece_length as usize + BLOCK_SIZE - 1) / BLOCK_SIZE) as u32;
                for block_index in 0..block_count {
                    let begin = block_index * BLOCK_SIZE as u32;
                    let length = min(BLOCK_SIZE as u32, piece_length.saturating_sub(begin));
                    self.reserve_block_request(piece_number, block_index);
                    return Some((piece_number, begin, length));
                }
            } else if let Some((begin, length)) = self.next_pending_block(piece_number) {
                let block_index = begin / BLOCK_SIZE as u32;
                if self.reserve_block_request(piece_number, block_index) {
                    return Some((piece_number, begin, length));
                }
                if endgame {
                    return Some((piece_number, begin, length));
                }
            }
        }
        None
    }

    /// Increments the local peer availability count for a given piece.
    pub fn increment_peer_count(&mut self, piece_number: u32) {
        self.piece_data[piece_number as usize].peer_count += 1;
    }

    /// Walks piece availability vectors to identify the next missing piece starting search from `start_piece`.
    pub fn find_next_missing_piece(&self, start_piece: u32) -> (bool, u32) {
        let mut current_piece = start_piece;
        loop {
            if self.is_piece_missing(current_piece)
                && self.piece_data[current_piece as usize].peer_count > 0
            {
                return (true, current_piece);
            }
            current_piece = (current_piece + 1) % self.number_of_pieces as u32;
            if current_piece == start_piece {
                break;
            }
        }
        (false, current_piece)
    }

    /// Estimates the current download transfer rate in bytes per second.
    pub fn bytes_per_second(&self) -> i64 {
        let seconds = self
            .assembly_data
            .lock()
            .unwrap()
            .average_assembly_time
            .get() as f64
            / 1000.0;
        if seconds != 0.0 {
            (self.piece_length as f64 / seconds) as i64
        } else {
            0
        }
    }

    /// Checks if a peer is in the active connection swarm map.
    pub fn is_peer_in_swarm(&self, ip: &str) -> bool {
        !ip.is_empty() && self.peer_swarm.read().unwrap().contains_key(ip)
    }

    /// Counts how many peers in the swarm have unchoked us.
    pub fn number_of_unchoked_peers(&self) -> usize {
        self.peer_swarm
            .read()
            .unwrap()
            .values()
            .filter(|peer| {
                peer.try_lock()
                    .map(|p| p.peer_choking.wait_one(0))
                    .unwrap_or(false)
            })
            .count()
    }
}
