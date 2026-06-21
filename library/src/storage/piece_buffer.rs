//! Piece buffer management
//!
//! Provides the `PieceBuffer` struct which holds and assembles incoming sub-blocks
//! of a single BitTorrent piece until the piece is fully downloaded.

use crate::constants::BLOCK_SIZE;
use std::sync::atomic::{AtomicI32, Ordering};

/// A buffer representing metadata for a single torrent piece, constructed incrementally from individual blocks.
#[derive(Debug)]
pub struct PieceBuffer {
    pub number: u32,
    pub length: u32,
    present_blocks: Vec<u64>,
    pub block_sources: Vec<Option<alloc::string::String>>,
    block_count: AtomicI32,
}

impl PieceBuffer {
    /// Creates a new `PieceBuffer` for the specified piece number and length.
    pub fn new(piece_number: u32, length: u32) -> Self {
        let block_count = ((length as usize + BLOCK_SIZE - 1) / BLOCK_SIZE) as i32;
        let word_count = ((block_count as usize) + 63) / 64;
        PieceBuffer {
            number: piece_number,
            length,
            present_blocks: vec![0; word_count],
            block_sources: vec![None; block_count as usize],
            block_count: AtomicI32::new(block_count),
        }
    }

    /// Marks a block index as present in the piece buffer.
    pub fn add_block(&mut self, block_number: u32, source_ip: &str) {
        if !self.is_block_present(block_number) {
            let word_idx = (block_number as usize) / 64;
            let bit_idx = (block_number as usize) % 64;
            self.present_blocks[word_idx] |= 1 << bit_idx;
            self.block_sources[block_number as usize] = Some(alloc::string::String::from(source_ip));
            self.block_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Checks if all sub-blocks for this piece have been received.
    pub fn all_blocks_there(&self) -> bool {
        self.block_count.load(Ordering::SeqCst) == 0
    }

    /// Checks if a specific block index is present in the bitset.
    pub fn is_block_present(&self, block_number: u32) -> bool {
        let word_idx = (block_number as usize) / 64;
        let bit_idx = (block_number as usize) % 64;
        if word_idx < self.present_blocks.len() {
            (self.present_blocks[word_idx] & (1 << bit_idx)) != 0
        } else {
            false
        }
    }
}
