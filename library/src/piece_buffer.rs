//! Piece buffer management
//!
//! Provides the `PieceBuffer` struct which holds and assembles incoming sub-blocks
//! of a single BitTorrent piece until the piece is fully downloaded.

use crate::constants::BLOCK_SIZE;
use std::sync::atomic::{AtomicI32, Ordering};

/// A buffer representing a single full torrent piece, constructed incrementally from individual blocks.
#[derive(Debug)]
pub struct PieceBuffer {
    pub number: u32,
    pub length: u32,
    pub buffer: Vec<u8>,
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
            buffer: vec![0u8; length as usize],
            present_blocks: vec![false; block_count as usize],
            block_count: AtomicI32::new(block_count),
        }
    }

    /// Adds a raw block payload to the piece buffer at the position computed from `block_number`.
    pub fn add_block(&mut self, block_data: &[u8], block_number: u32) {
        let block_offset = (block_number as usize) * BLOCK_SIZE;
        let block_length = std::cmp::min(
            std::cmp::min(self.length as usize - block_offset, BLOCK_SIZE),
            block_data.len(),
        );
        self.buffer[block_offset..block_offset + block_length]
            .copy_from_slice(&block_data[..block_length]);
        if !self.present_blocks[block_number as usize] {
            self.present_blocks[block_number as usize] = true;
            self.block_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Adds a block from a raw network packet. Automatically skips the 9-byte peer protocol header.
    pub fn add_block_from_packet(&mut self, packet_buffer: &[u8], block_number: u32) {
        if packet_buffer.len() >= 9 {
            self.add_block(&packet_buffer[9..], block_number);
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
