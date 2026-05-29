use crate::average::Average;
use crate::constants::BLOCK_SIZE;
use crate::disk_io::DiskIO;
use crate::manual_reset_event::ManualResetEvent;
use crate::metainfo::FileDetails;
use crate::metainfo::MetaInfoFile;
use crate::peer::Peer;
use crate::piece_buffer::PieceBuffer;
use crate::selector::Selector;
use sha1::Digest;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug, Clone)]
pub struct Tracker;

#[derive(Debug, Clone)]
pub struct PieceInfo {
    pub peer_count: usize,
    pub piece_length: u32,
}

#[derive(Debug)]
pub struct AssemblerData {
    pub piece_buffer: Option<Arc<Mutex<PieceBuffer>>>,
    pub current_block_requests: usize,
    pub guard_mutex: Mutex<()>,
    pub block_requests_done: ManualResetEvent,
    pub average_assembly_time: Average,
    pub total_timeouts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentStatus {
    Initialised,
    Seeding,
    Downloading,
    Paused,
    Ended,
}

pub struct TorrentContext {
    pub info_hash: Vec<u8>,
    pub tracker_url: String,
    pub number_of_pieces: usize,
    pub piece_length: u32,
    pub pieces_info_hash: Vec<u8>,
    pub bitfield: Vec<u8>,
    pub files_to_download: Vec<FileDetails>,
    pub total_bytes_downloaded: u64,
    pub total_bytes_to_download: u64,
    pub total_bytes_uploaded: u64,
    pub status: TorrentStatus,
    pub file_name: String,
    pub main_tracker: Option<Tracker>,
    pub callback_data: Option<String>,
    pub call_back: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    pub paused: ManualResetEvent,
    pub download_finished: ManualResetEvent,
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
    pub fn new(
        torrent_meta_info: &MetaInfoFile,
        selector: Selector,
        disk_io: &DiskIO,
        download_path: &std::path::Path,
        seeding: bool,
    ) -> Result<Self, crate::error::BitTorrentError> {
        let info_hash = torrent_meta_info.get_info_hash()?;
        let tracker_url = torrent_meta_info.get_tracker()?;
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
            main_tracker: None,
            callback_data: None,
            call_back: None,
            paused: ManualResetEvent::new(false),
            download_finished: ManualResetEvent::new(false),
            selector,
            peer_swarm: RwLock::new(HashMap::new()),
            missing_pieces_count: 0,
            maximum_swarm_size: crate::constants::MAXIMUM_SWARM_SIZE,
            assembly_data: Mutex::new(AssemblerData {
                piece_buffer: None,
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
            context.total_bytes_downloaded = 0;
            context.total_bytes_to_download = 0;
        } else {
            disk_io.create_torrent_bitfield(&mut context)?;
            context.total_bytes_to_download = context
                .total_bytes_to_download
                .saturating_sub(context.total_bytes_downloaded);
            context.total_bytes_downloaded = 0;
        }
        Ok(context)
    }

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

    pub fn progress_percent(&self) -> f32 {
        if self.total_bytes_to_download == 0 {
            return 100.0;
        }

        let percent =
            self.total_bytes_downloaded as f64 / self.total_bytes_to_download as f64 * 100.0;
        percent.min(100.0) as f32
    }

    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let byte_index = (piece_number >> 3) as usize;
        let bit_mask = 0x80 >> (piece_number & 0x7);
        if local {
            self.bitfield[byte_index] |= bit_mask;
        } else {
            self.bitfield[byte_index] &= !bit_mask;
        }
    }

    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let byte_index = (piece_number >> 3) as usize;
        let bit_mask = 0x80 >> (piece_number & 0x7);
        self.bitfield[byte_index] & bit_mask != 0
    }

    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let byte_index = (piece_number >> 3) as usize;
        let bit_mask = 0x80 >> (piece_number & 0x7);
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

    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let byte_index = (piece_number >> 3) as usize;
        let bit_mask = 0x80 >> (piece_number & 0x7);
        self.pieces_missing[byte_index] & bit_mask != 0
    }

    pub fn merge_piece_bitfield(&mut self, remote_peer: &Peer) {
        let mut piece_number = 0u32;
        for byte in &remote_peer.remote_piece_bitfield {
            for bit in &[0x80u8, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01] {
                if byte & bit != 0 {
                    self.piece_data[piece_number as usize].peer_count += 1;
                }
                piece_number += 1;
                if piece_number as usize >= self.number_of_pieces {
                    break;
                }
            }
        }
    }

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

    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        if self.total_bytes_to_download < self.total_bytes_downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(self.total_bytes_to_download - self.total_bytes_downloaded)
    }

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
        if !piece_there {
            self.mark_piece_missing(piece_number, true);
        }
    }

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
        if assembly_data
            .piece_buffer
            .as_ref()
            .map(|buffer| buffer.lock().unwrap().number != piece_number)
            .unwrap_or(true)
        {
            assembly_data.piece_buffer = Some(Arc::new(Mutex::new(PieceBuffer::new(
                piece_number,
                piece_length,
            ))));
        }

        let piece_buffer_arc = assembly_data.piece_buffer.as_ref().unwrap().clone();
        let mut piece_buffer = piece_buffer_arc.lock().unwrap();
        piece_buffer.add_block(block_data, block_index);
        let piece_complete = piece_buffer.all_blocks_there();

        if piece_complete {
            let finished_piece = piece_buffer.buffer.clone();
            drop(piece_buffer);
            drop(assembly_data);

            if self.check_piece_hash(piece_number, &finished_piece, finished_piece.len() as u32) {
                disk_io.write_piece(self, piece_number, &finished_piece)?;
                self.update_bitfield_from_buffer(
                    piece_number,
                    &finished_piece,
                    finished_piece.len() as u32,
                );
                self.clear_piece_requests(piece_number);
                let mut assembly_data = self.assembly_data.lock().unwrap();
                assembly_data.piece_buffer = None;
                return Ok(true);
            } else {
                self.clear_piece_requests(piece_number);
                let mut assembly_data = self.assembly_data.lock().unwrap();
                assembly_data.piece_buffer = None;
                return Err(crate::error::BitTorrentError::Parse(
                    "Piece failed hash verification".to_string(),
                ));
            }
        }

        Ok(false)
    }

    pub fn unmerge_piece_bitfield(&mut self, remote_peer: &Peer) {
        let mut piece_number = 0u32;
        for byte in &remote_peer.remote_piece_bitfield {
            for bit in &[0x80u8, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01] {
                if byte & bit != 0 {
                    self.piece_data[piece_number as usize].peer_count = self.piece_data
                        [piece_number as usize]
                        .peer_count
                        .saturating_sub(1);
                }
                piece_number += 1;
                if piece_number as usize >= self.number_of_pieces {
                    break;
                }
            }
        }
    }

    pub fn get_piece_length(&self, piece_number: u32) -> u32 {
        self.piece_data[piece_number as usize].piece_length
    }

    pub fn set_piece_length(&mut self, piece_number: u32, piece_length: u32) {
        if piece_length <= self.piece_length {
            self.piece_data[piece_number as usize].piece_length = piece_length;
        } else {
            panic!("Piece length larger than maximum for torrent.");
        }
    }

    pub fn is_space_in_swarm(&self, ip: &str) -> bool {
        !ip.is_empty()
            && self.peer_swarm.read().unwrap().get(ip).is_none()
            && self.peer_swarm.read().unwrap().len() < self.maximum_swarm_size
    }

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

    pub fn is_block_requested(&self, piece_number: u32, block_index: u32) -> bool {
        self.requested_blocks
            .read()
            .unwrap()
            .contains(&(piece_number, block_index))
    }

    pub fn reserve_block_request(&self, piece_number: u32, block_index: u32) -> bool {
        self.requested_blocks
            .write()
            .unwrap()
            .insert((piece_number, block_index))
    }

    pub fn release_block_request(&self, piece_number: u32, block_index: u32) {
        self.requested_blocks
            .write()
            .unwrap()
            .remove(&(piece_number, block_index));
    }

    pub fn clear_piece_requests(&self, piece_number: u32) {
        self.requested_blocks
            .write()
            .unwrap()
            .retain(|(piece, _)| *piece != piece_number);
    }

    pub fn select_next_piece_for_peer(&mut self, remote_peer: &Peer) -> Option<u32> {
        let mut candidates: Vec<(usize, u32)> = (0..self.number_of_pieces as u32)
            .filter(|piece| {
                !self.is_piece_local(*piece) && remote_peer.is_piece_on_remote_peer(*piece)
            })
            .map(|piece| (self.piece_data[piece as usize].peer_count, piece))
            .collect();

        candidates.sort_by_key(|(count, piece)| (*count, *piece));
        candidates.into_iter().map(|(_, piece)| piece).next()
    }

    pub fn next_block_request_for_peer(&mut self, peer: &Peer) -> Option<(u32, u32, u32)> {
        if self.status != TorrentStatus::Downloading {
            return None;
        }
        if let Some(piece_number) = self.select_next_piece_for_peer(peer) {
            if let Some((begin, length)) = self.next_pending_block(piece_number) {
                let block_index = begin / BLOCK_SIZE as u32;
                if self.reserve_block_request(piece_number, block_index) {
                    return Some((piece_number, begin, length));
                }
            }
        }
        None
    }

    pub fn increment_peer_count(&mut self, piece_number: u32) {
        self.piece_data[piece_number as usize].peer_count += 1;
    }

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

    pub fn is_peer_in_swarm(&self, ip: &str) -> bool {
        !ip.is_empty() && self.peer_swarm.read().unwrap().contains_key(ip)
    }

    pub fn number_of_unchoked_peers(&self) -> usize {
        self.peer_swarm
            .read()
            .unwrap()
            .values()
            .filter(|peer| !peer.lock().unwrap().peer_choking.wait_one(0))
            .count()
    }
}
