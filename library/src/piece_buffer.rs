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
    present_blocks: Vec<bool>,
    block_count: AtomicI32,
}

impl PieceBuffer {
    /// Creates a new `PieceBuffer` for the specified piece number and length.
    pub fn new(piece_number: u32, length: u32) -> Self {
        let block_count = ((length as usize + BLOCK_SIZE - 1) / BLOCK_SIZE) as i32;
        PieceBuffer {
            number: piece_number,
            length,
            present_blocks: vec![false; block_count as usize],
            block_count: AtomicI32::new(block_count),
        }
    }

    /// Marks a block index as present in the piece buffer.
    pub fn add_block(&mut self, block_number: u32) {
        if !self.present_blocks[block_number as usize] {
            self.present_blocks[block_number as usize] = true;
            self.block_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Checks if all sub-blocks for this piece have been received.
    pub fn all_blocks_there(&self) -> bool {
        self.block_count.load(Ordering::SeqCst) == 0
    }

    /// Returns a reference to the boolean array indicating which blocks are present.
    pub fn blocks_present(&self) -> &[bool] {
        &self.present_blocks
    }
}
