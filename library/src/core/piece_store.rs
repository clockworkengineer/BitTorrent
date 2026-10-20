//! Piece storage and bitfield state tracking.
//!
//! Encapsulates piece lengths, SHA1 hashes, local bitfields, missing piece tracking,
//! and priority queue scheduling.

use alloc::vec::Vec;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Mutex;

/// Availability metadata tracked per piece.
#[derive(Debug, Clone)]
pub struct PieceInfo {
    pub peer_count: usize,
    pub piece_length: u32,
}

/// Represents a piece rarity entry in the priority queue.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PieceRarity {
    pub peer_count: usize,
    pub piece_index: u32,
}

impl Ord for PieceRarity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other
            .peer_count
            .cmp(&self.peer_count)
            .then_with(|| other.piece_index.cmp(&self.piece_index))
    }
}

impl PartialOrd for PieceRarity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Manages piece bitfields, hash verification state, and piece rarity priority queues.
pub struct PieceStore {
    pub number_of_pieces: usize,
    pub piece_length: u32,
    pub pieces_info_hash: Vec<u8>,
    pub bitfield: Vec<u8>,
    pub pieces_missing: Vec<u8>,
    pub piece_data: Vec<PieceInfo>,
    pub missing_pieces_count: usize,
    pub pieces_verifying: Mutex<HashSet<u32>>,
    pub piece_priority_queue: Mutex<BinaryHeap<PieceRarity>>,
}

impl PieceStore {
    /// Creates a new `PieceStore` with allocated bitfields and priority queues.
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

        PieceStore {
            number_of_pieces,
            piece_length,
            pieces_info_hash,
            bitfield,
            pieces_missing,
            piece_data,
            missing_pieces_count: 0,
            pieces_verifying: Mutex::new(HashSet::new()),
            piece_priority_queue: Mutex::new(piece_priority_queue),
        }
    }

    /// Creates an empty `PieceStore` (e.g. for magnet bootstrap).
    pub fn empty() -> Self {
        PieceStore {
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

    /// Re-initializes pieces when metadata is fetched.
    pub fn init_pieces(&mut self, number_of_pieces: usize, piece_length: u32) {
        self.number_of_pieces = number_of_pieces;
        self.piece_length = piece_length;
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
        let mut pq = self.piece_priority_queue.lock().unwrap();
        pq.clear();
        for piece_index in 0..number_of_pieces as u32 {
            pq.push(PieceRarity {
                peer_count: 0,
                piece_index,
            });
        }
    }

    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let (byte_index, bit_mask) = crate::util::get_bitfield_index_and_mask(piece_number);
        if byte_index < self.bitfield.len() {
            if local {
                self.bitfield[byte_index] |= bit_mask;
            } else {
                self.bitfield[byte_index] &= !bit_mask;
            }
        }
    }

    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = crate::util::get_bitfield_index_and_mask(piece_number);
        if byte_index >= self.bitfield.len() {
            return false;
        }
        self.bitfield[byte_index] & bit_mask != 0
    }

    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let (byte_index, bit_mask) = crate::util::get_bitfield_index_and_mask(piece_number);
        if byte_index < self.pieces_missing.len() {
            if missing {
                self.pieces_missing[byte_index] |= bit_mask;
            } else {
                self.pieces_missing[byte_index] &= !bit_mask;
            }
        }
    }

    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = crate::util::get_bitfield_index_and_mask(piece_number);
        if byte_index >= self.pieces_missing.len() {
            return false;
        }
        self.pieces_missing[byte_index] & bit_mask != 0
    }

    pub fn is_piece_verifying(&self, piece_number: u32) -> bool {
        if let Ok(guard) = self.pieces_verifying.lock() {
            guard.contains(&piece_number)
        } else {
            false
        }
    }

    pub fn set_piece_verifying(&self, piece_number: u32, verifying: bool) {
        if let Ok(mut guard) = self.pieces_verifying.lock() {
            if verifying {
                guard.insert(piece_number);
            } else {
                guard.remove(&piece_number);
            }
        }
    }

    pub fn get_piece_peer_count(&self, piece_number: u32) -> usize {
        let idx = piece_number as usize;
        if idx < self.piece_data.len() {
            self.piece_data[idx].peer_count
        } else {
            0
        }
    }

    pub fn set_piece_length(&mut self, piece_number: u32, length: u32) {
        let idx = piece_number as usize;
        if idx < self.piece_data.len() {
            self.piece_data[idx].piece_length = length;
        }
    }

    pub fn get_piece_length(&self, piece_number: u32) -> u32 {
        let idx = piece_number as usize;
        if idx < self.piece_data.len() {
            self.piece_data[idx].piece_length
        } else {
            self.piece_length
        }
    }

    pub fn update_bitfield_status(&mut self, piece_number: u32, valid: bool) {
        if valid {
            self.mark_piece_local(piece_number, true);
            self.mark_piece_missing(piece_number, false);
            if self.missing_pieces_count > 0 {
                self.missing_pieces_count -= 1;
            }
        } else {
            self.mark_piece_local(piece_number, false);
            self.mark_piece_missing(piece_number, true);
        }
    }

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
}
