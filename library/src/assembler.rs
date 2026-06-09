//! Block and piece assembly tracking
//!
//! Manages active block downloads, in-progress piece assembly buffers,
//! requested blocks, and assembly time statistics.

use crate::average::Average;
use crate::manual_reset_event::ManualResetEvent;
use crate::piece_buffer::PieceBuffer;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

/// Manages block download lists, active piece buffers, and assembly statistics.
pub struct Assembler {
    pub piece_buffers: Mutex<HashMap<u32, Arc<Mutex<PieceBuffer>>>>,
    pub requested_blocks: RwLock<HashSet<(u32, u32)>>,
    pub current_block_requests: std::sync::atomic::AtomicUsize,
    pub block_requests_done: ManualResetEvent,
    pub average_assembly_time: Mutex<Average>,
    pub total_timeouts: std::sync::atomic::AtomicUsize,
}

impl std::fmt::Debug for Assembler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assembler").finish()
    }
}

impl Assembler {
    /// Creates a new, empty `Assembler` instance.
    pub fn new() -> Self {
        Assembler {
            piece_buffers: Mutex::new(HashMap::new()),
            requested_blocks: RwLock::new(HashSet::new()),
            current_block_requests: std::sync::atomic::AtomicUsize::new(0),
            block_requests_done: ManualResetEvent::new(false),
            average_assembly_time: Mutex::new(Average::default()),
            total_timeouts: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Checks if a block request has been registered in the assembler.
    pub fn is_block_requested(&self, piece_number: u32, block_index: u32) -> bool {
        self.requested_blocks
            .read()
            .unwrap()
            .contains(&(piece_number, block_index))
    }

    /// Marks a block index as requested/reserved in the assembler.
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
}
