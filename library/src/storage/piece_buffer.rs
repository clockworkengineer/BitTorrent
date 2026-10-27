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
            self.block_sources[block_number as usize] =
                Some(alloc::string::String::from(source_ip));
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

/// Snapshot of operational metrics from the [`BlockBufferPool`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BlockBufferPoolMetrics {
    /// Number of buffers currently available in the pool.
    pub available_buffers: usize,
    /// Number of buffers currently checked out / in use.
    pub active_borrowed_buffers: usize,
    /// Total buffer acquisitions satisfied directly from the pool (0 heap allocations).
    pub pool_hits: u64,
    /// Buffer acquisitions that required allocating a new buffer due to an empty pool.
    pub pool_misses: u64,
    /// Total number of buffers returned back to the pool.
    pub buffers_returned: u64,
    /// Maximum number of buffers the pool will retain.
    pub max_capacity: usize,
    /// Size of each block buffer in bytes (standard: 16 KiB = 16,384 bytes).
    pub block_size: usize,
    /// Total allocations attempted (pool_hits + pool_misses).
    pub total_allocations: u64,
    /// Allocations satisfied without a new heap allocation (same as pool_hits).
    pub reused_allocations: u64,
    /// Maximum capacity synonym for convenience.
    pub capacity: usize,
}

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};
use crate::utils::util::LockExt;

/// A thread-safe slab allocator and object pool for reusable 16 KiB block buffers.
///
/// Under multi-peer wire traffic, allocating new buffers for every 16 KiB wire block
/// creates significant GC/allocator churn. `BlockBufferPool` provides reusable
/// pre-allocated byte buffers, achieving steady-state zero-allocation block transfers.
#[derive(Debug)]
pub struct BlockBufferPool {
    pool: Mutex<Vec<Vec<u8>>>,
    max_capacity: usize,
    block_size: usize,
    hits: AtomicU64,
    misses: AtomicU64,
    returns: AtomicU64,
    active_borrows: AtomicI32,
}

impl Default for BlockBufferPool {
    fn default() -> Self {
        // Standard default: 256 reusable 16 KiB buffers (4 MiB)
        Self::new(256, BLOCK_SIZE)
    }
}

impl BlockBufferPool {
    /// Global shared pool instance across the library.
    pub fn global() -> &'static Arc<BlockBufferPool> {
        static GLOBAL_POOL: OnceLock<Arc<BlockBufferPool>> = OnceLock::new();
        GLOBAL_POOL.get_or_init(|| Arc::new(BlockBufferPool::default()))
    }

    /// Creates a new `BlockBufferPool` with the specified maximum capacity and block size.
    pub fn new(max_capacity: usize, block_size: usize) -> Self {
        BlockBufferPool {
            pool: Mutex::new(Vec::with_capacity(max_capacity.min(64))),
            max_capacity,
            block_size,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            returns: AtomicU64::new(0),
            active_borrows: AtomicI32::new(0),
        }
    }

    /// Creates a new `BlockBufferPool` with `count` preallocated block buffers ready for immediate use.
    pub fn with_preallocated(max_capacity: usize, block_size: usize, count: usize) -> Self {
        let prealloc_count = count.min(max_capacity);
        let mut preallocated = Vec::with_capacity(prealloc_count);
        for _ in 0..prealloc_count {
            preallocated.push(Vec::with_capacity(block_size));
        }
        BlockBufferPool {
            pool: Mutex::new(preallocated),
            max_capacity,
            block_size,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            returns: AtomicU64::new(0),
            active_borrows: AtomicI32::new(0),
        }
    }

    /// Returns the number of buffers currently available in the pool.
    pub fn available(&self) -> usize {
        self.pool.lock_safe().len()
    }

    /// Returns the maximum capacity of the pool.
    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    /// Acquires a reusable buffer with at least `capacity` bytes from the pool.
    /// If no buffer is available or if none meets the capacity, allocates a new one.
    pub fn acquire(&self, required_capacity: usize) -> Vec<u8> {
        self.active_borrows.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.pool.lock_safe();
        while let Some(mut buf) = guard.pop() {
            if buf.capacity() >= required_capacity {
                buf.clear();
                self.hits.fetch_add(1, Ordering::Relaxed);
                return buf;
            }
        }
        drop(guard);
        self.misses.fetch_add(1, Ordering::Relaxed);
        Vec::with_capacity(required_capacity.max(self.block_size))
    }

    /// Acquires a standard 16 KiB block buffer from the pool.
    pub fn acquire_block(&self) -> Vec<u8> {
        self.acquire(self.block_size)
    }

    /// Returns a buffer to the pool for reuse if the pool has not reached its maximum capacity.
    pub fn release(&self, mut buf: Vec<u8>) {
        self.active_borrows.fetch_sub(1, Ordering::Relaxed);
        if buf.capacity() >= self.block_size {
            let mut guard = self.pool.lock_safe();
            if guard.len() < self.max_capacity {
                buf.clear();
                guard.push(buf);
                self.returns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Acquires an RAII-guarded pooled buffer that automatically returns itself to the pool when dropped.
    pub fn acquire_scoped(self: &Arc<Self>) -> PooledBlockBuffer {
        PooledBlockBuffer::acquire_block(Arc::clone(self))
    }

    /// Acquires an RAII-guarded pooled buffer of at least `capacity` bytes.
    pub fn acquire_scoped_capacity(self: &Arc<Self>, capacity: usize) -> PooledBlockBuffer {
        PooledBlockBuffer::acquire(Arc::clone(self), capacity)
    }

    /// Retrieves an operational metrics snapshot.
    pub fn metrics(&self) -> BlockBufferPoolMetrics {
        let available = self.pool.lock_safe().len();
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        BlockBufferPoolMetrics {
            available_buffers: available,
            active_borrowed_buffers: self.active_borrows.load(Ordering::Relaxed).max(0) as usize,
            pool_hits: hits,
            pool_misses: misses,
            buffers_returned: self.returns.load(Ordering::Relaxed),
            max_capacity: self.max_capacity,
            block_size: self.block_size,
            total_allocations: hits + misses,
            reused_allocations: hits,
            capacity: self.max_capacity,
        }
    }

    /// Clears all retained buffers from memory.
    pub fn clear(&self) {
        self.pool.lock_safe().clear();
    }
}

/// RAII handle wrapping a pooled buffer that returns the buffer to its pool on `Drop`.
#[derive(Debug)]
pub struct PooledBlockBuffer {
    buffer: Option<Vec<u8>>,
    pool: Arc<BlockBufferPool>,
}

impl PooledBlockBuffer {
    /// Acquires an RAII-guarded buffer with at least `required_capacity` from `pool`.
    pub fn acquire(pool: Arc<BlockBufferPool>, required_capacity: usize) -> Self {
        let buf = pool.acquire(required_capacity);
        PooledBlockBuffer {
            buffer: Some(buf),
            pool,
        }
    }

    /// Acquires an RAII-guarded 16 KiB block buffer from `pool`.
    pub fn acquire_block(pool: Arc<BlockBufferPool>) -> Self {
        Self::acquire(pool, BLOCK_SIZE)
    }

    /// Extracts the inner `Vec<u8>`, disassociating it from the pool (will not return on drop).
    pub fn take(mut self) -> Vec<u8> {
        self.buffer.take().unwrap_or_default()
    }

    /// Returns a reference to the underlying buffer.
    pub fn as_vec(&self) -> &Vec<u8> {
        self.buffer.as_ref().expect("Buffer has been taken")
    }

    /// Returns a mutable reference to the underlying buffer.
    pub fn as_vec_mut(&mut self) -> &mut Vec<u8> {
        self.buffer.as_mut().expect("Buffer has been taken")
    }
}

impl std::ops::Deref for PooledBlockBuffer {
    type Target = Vec<u8>;
    fn deref(&self) -> &Self::Target {
        self.as_vec()
    }
}

impl std::ops::DerefMut for PooledBlockBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_vec_mut()
    }
}

impl AsRef<[u8]> for PooledBlockBuffer {
    fn as_ref(&self) -> &[u8] {
        self.as_vec().as_slice()
    }
}

impl AsMut<[u8]> for PooledBlockBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_vec_mut().as_mut_slice()
    }
}

impl Drop for PooledBlockBuffer {
    fn drop(&mut self) {
        if let Some(buf) = self.buffer.take() {
            self.pool.release(buf);
        }
    }
}
