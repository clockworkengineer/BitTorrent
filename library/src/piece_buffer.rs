use crate::constants::BLOCK_SIZE;
use std::sync::atomic::{AtomicI32, Ordering};

#[derive(Debug)]
pub struct PieceBuffer {
    pub number: u32,
    pub length: u32,
    pub buffer: Vec<u8>,
    present_blocks: Vec<bool>,
    block_count: AtomicI32,
}

impl PieceBuffer {
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

    pub fn add_block_from_packet(&mut self, packet_buffer: &[u8], block_number: u32) {
        let block_offset = (block_number as usize) * BLOCK_SIZE;
        let block_length = std::cmp::min(self.length as usize - block_offset, BLOCK_SIZE);
        self.buffer[block_offset..block_offset + block_length]
            .copy_from_slice(&packet_buffer[9..9 + block_length]);
        if !self.present_blocks[block_number as usize] {
            self.present_blocks[block_number as usize] = true;
            self.block_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    pub fn all_blocks_there(&self) -> bool {
        self.block_count.load(Ordering::SeqCst) == 0
    }

    pub fn blocks_present(&self) -> &[bool] {
        &self.present_blocks
    }
}
