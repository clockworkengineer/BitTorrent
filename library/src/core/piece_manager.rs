//! Piece accounting, bitfield state, and rarity scheduling manager (SRP & ISP).
//!
//! Encapsulates local and remote bitfields, piece lengths, SHA1 hashes,
//! in-flight verification tracking, and rarest-first scheduling.

use alloc::vec::Vec;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Mutex;

use sha1::Digest;

use crate::constants::{BLOCK_SIZE, ENDGAME_THRESHOLD, HASH_LENGTH};
use crate::core::piece_store::{PieceInfo, PieceRarity};
use crate::core::views::{PieceAvailabilityView, RemotePeerPieceView};
use crate::util::get_bitfield_index_and_mask;
use crate::utils::util::LockExt;

/// Manages piece bitfields, availability, SHA1 checksum verification, and rarest-first scheduling.
pub struct PieceManager {
    /// Total number of pieces in the torrent.
    pub number_of_pieces: usize,
    /// Standard piece length in bytes (last piece may be smaller).
    pub piece_length: u32,
    /// Concatenated 20-byte SHA1 hashes for all pieces.
    pub pieces_info_hash: Vec<u8>,
    /// Local bitfield indicating verified downloaded pieces (1 = local, 0 = missing).
    pub bitfield: Vec<u8>,
    /// Bitfield tracking missing pieces needing to be requested (1 = missing, 0 = acquired).
    pub pieces_missing: Vec<u8>,
    /// Per-piece metadata tracking peer availability count and individual piece lengths.
    pub piece_data: Vec<PieceInfo>,
    /// Count of pieces remaining to be downloaded and verified.
    pub missing_pieces_count: usize,
    /// Set of piece numbers currently undergoing asynchronous hash verification.
    pub pieces_verifying: Mutex<HashSet<u32>>,
    /// Priority queue scheduling rarest pieces first.
    pub piece_priority_queue: Mutex<BinaryHeap<PieceRarity>>,
}

impl PieceManager {
    /// Creates a new `PieceManager` with allocated bitfields and priority queues.
    pub fn new(number_of_pieces: usize, piece_length: u32, pieces_info_hash: Vec<u8>) -> Self {
        let bitfield = vec![0u8; (number_of_pieces + 7) / 8];
        let pieces_missing = vec![0u8; bitfield.len()];
        let piece_data = vec![
            PieceInfo {
                peer_count: 0,
                piece_length,
            };
            number_of_pieces
        ];

        let piece_priority_queue = {
            let mut pq = BinaryHeap::new();
            for piece_index in 0..number_of_pieces as u32 {
                pq.push(PieceRarity {
                    peer_count: 0,
                    piece_index,
                });
            }
            pq
        };

        Self {
            number_of_pieces,
            piece_length,
            pieces_info_hash,
            bitfield,
            pieces_missing,
            piece_data,
            missing_pieces_count: number_of_pieces,
            pieces_verifying: Mutex::new(HashSet::new()),
            piece_priority_queue: Mutex::new(piece_priority_queue),
        }
    }

    /// Creates an empty `PieceManager` (e.g. for magnet bootstrap before metadata is fetched).
    pub fn empty() -> Self {
        Self {
            number_of_pieces: 0,
            piece_length: 0,
            pieces_info_hash: Vec::new(),
            bitfield: Vec::new(),
            pieces_missing: Vec::new(),
            piece_data: Vec::new(),
            missing_pieces_count: 0,
            pieces_verifying: Mutex::new(HashSet::new()),
            piece_priority_queue: Mutex::new(BinaryHeap::new()),
        }
    }

    /// Re-initializes piece vectors when torrent metainfo is acquired.
    pub fn init_pieces(&mut self, number_of_pieces: usize, piece_length: u32, pieces_info_hash: Vec<u8>) {
        self.number_of_pieces = number_of_pieces;
        self.piece_length = piece_length;
        self.pieces_info_hash = pieces_info_hash;
        self.bitfield = vec![0u8; (number_of_pieces + 7) / 8];
        self.pieces_missing = vec![0u8; self.bitfield.len()];
        self.piece_data = vec![
            PieceInfo {
                peer_count: 0,
                piece_length,
            };
            number_of_pieces
        ];
        self.missing_pieces_count = number_of_pieces;

        let mut pq = self.piece_priority_queue.lock_safe();
        pq.clear();
        for piece_index in 0..number_of_pieces as u32 {
            pq.push(PieceRarity {
                peer_count: 0,
                piece_index,
            });
        }
    }

    /// Sets or clears a specific piece completion bit in the local bitfield.
    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.bitfield.len() {
            if local {
                self.bitfield[byte_index] |= bit_mask;
            } else {
                self.bitfield[byte_index] &= !bit_mask;
            }
        }
    }

    /// Checks if a piece is marked local in the bitfield.
    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index >= self.bitfield.len() {
            return false;
        }
        self.bitfield[byte_index] & bit_mask != 0
    }

    /// Returns the total number of locally completed pieces.
    pub fn completed_pieces_count(&self) -> usize {
        (0..self.number_of_pieces as u32)
            .filter(|&p| self.is_piece_local(p))
            .count()
    }

    /// Sets or clears a specific piece index in the missing pieces tracking vector.
    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.pieces_missing.len() {
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
    }

    /// Checks if a piece is marked missing.
    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index >= self.pieces_missing.len() {
            return false;
        }
        self.pieces_missing[byte_index] & bit_mask != 0
    }

    /// Checks if a piece is currently undergoing cryptographic hash verification.
    pub fn is_piece_verifying(&self, piece_number: u32) -> bool {
        let guard = self.pieces_verifying.lock_safe();
        guard.contains(&piece_number)
    }

    /// Sets the verification in-flight flag for a piece.
    pub fn set_piece_verifying(&self, piece_number: u32, verifying: bool) {
        let mut guard = self.pieces_verifying.lock_safe();
        if verifying {
            guard.insert(piece_number);
        } else {
            guard.remove(&piece_number);
        }
    }

    /// Returns count of pieces currently in verification.
    pub fn verifying_pieces_count(&self) -> usize {
        self.pieces_verifying.lock_safe().len()
    }

    /// Updates piece bitfields and status based on verification result.
    pub fn update_bitfield_status(&mut self, piece_number: u32, valid: bool) {
        if valid {
            self.mark_piece_local(piece_number, true);
            self.mark_piece_missing(piece_number, false);
        } else {
            self.mark_piece_local(piece_number, false);
            self.mark_piece_missing(piece_number, true);
        }
    }

    /// Validates a downloaded piece payload against its SHA1 info hash.
    pub fn verify_piece_hash(&self, piece_number: u32, piece_data: &[u8]) -> bool {
        let offset = piece_number as usize * HASH_LENGTH;
        if offset + HASH_LENGTH > self.pieces_info_hash.len() {
            return false;
        }
        let expected_hash = &self.pieces_info_hash[offset..offset + HASH_LENGTH];
        let mut hasher = sha1::Sha1::new();
        hasher.update(piece_data);
        let actual_hash = hasher.finalize();
        actual_hash.as_slice() == expected_hash
    }

    /// Checks if all pieces in the torrent have been verified locally.
    pub fn is_download_complete(&self) -> bool {
        if self.number_of_pieces == 0 || self.bitfield.is_empty() {
            return false;
        }
        let full_bytes = self.number_of_pieces / 8;
        let rem_bits = self.number_of_pieces % 8;
        for &byte in &self.bitfield[..full_bytes] {
            if byte != 0xFF {
                return false;
            }
        }
        if rem_bits > 0 {
            let mask = !((1u8 << (8 - rem_bits)) - 1);
            if (self.bitfield[full_bytes] & mask) != mask {
                return false;
            }
        }
        true
    }

    /// Pops the rarest piece available on the given remote peer from the priority queue.
    pub fn pop_rarest_piece<P: RemotePeerPieceView>(&self, peer: &P) -> Option<PieceRarity> {
        let mut pq = self.piece_priority_queue.lock_safe();
        let mut temporary_list = Vec::new();
        let mut result = None;

        while let Some(popped) = pq.pop() {
            if self.is_piece_local(popped.piece_index)
                || self.is_piece_verifying(popped.piece_index)
            {
                continue;
            }

            if peer.is_piece_on_remote_peer(popped.piece_index) {
                result = Some(popped);
                break;
            } else {
                temporary_list.push(popped);
            }
        }

        for item in temporary_list {
            pq.push(item);
        }

        result
    }

    /// Returns true if the torrent has reached endgame mode (<= 20 pieces missing).
    pub fn check_endgame_mode(&self) -> bool {
        self.missing_pieces_count > 0 && self.missing_pieces_count <= ENDGAME_THRESHOLD
    }

    /// Returns the length in bytes of the piece at index `piece_number`.
    pub fn get_piece_length(&self, piece_number: u32) -> u32 {
        let idx = piece_number as usize;
        if idx < self.piece_data.len() {
            self.piece_data[idx].piece_length
        } else {
            self.piece_length
        }
    }

    /// Sets the byte length for a specific piece.
    pub fn set_piece_length(&mut self, piece_number: u32, length: u32) {
        let idx = piece_number as usize;
        if idx < self.piece_data.len() {
            self.piece_data[idx].piece_length = length;
        }
    }

    /// Returns the swarm peer availability count for a piece.
    pub fn get_piece_peer_count(&self, piece_number: u32) -> usize {
        let idx = piece_number as usize;
        if idx < self.piece_data.len() {
            self.piece_data[idx].peer_count
        } else {
            0
        }
    }

    /// Calculates block count for a piece given standard 16 KiB block sizing.
    pub fn blocks_in_piece(&self, piece_number: u32) -> u32 {
        let len = self.get_piece_length(piece_number);
        (len + BLOCK_SIZE as u32 - 1) / BLOCK_SIZE as u32
    }
}

impl PieceAvailabilityView for PieceManager {
    fn number_of_pieces(&self) -> usize {
        self.number_of_pieces
    }

    fn piece_length(&self, piece: u32) -> u32 {
        self.get_piece_length(piece)
    }

    fn is_piece_local(&self, piece: u32) -> bool {
        self.is_piece_local(piece)
    }

    fn is_piece_wanted(&self, piece: u32) -> bool {
        !self.is_piece_local(piece)
    }

    fn is_piece_verifying(&self, piece: u32) -> bool {
        self.is_piece_verifying(piece)
    }

    fn peer_count_for_piece(&self, piece: u32) -> usize {
        self.get_piece_peer_count(piece)
    }
}
