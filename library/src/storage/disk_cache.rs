//! In-memory block cache and asynchronous write coalescing
//!
//! Provides `CachedBlockStorage`, an in-memory block caching and write coalescing layer
//! that wraps any underlying [`BlockStorage`] (e.g. [`DiskIO`](super::disk_io::DiskIO)).
//!
//! Under high-throughput multi-peer swarms, incoming 16 KiB blocks are randomly distributed
//! across pieces. Without caching, each block incurs a synchronous disk write syscall.
//! `CachedBlockStorage` buffers dirty blocks in memory, serves reads directly from cache,
//! and coalesces contiguous blocks into large sequential disk writes on flush, reducing
//! disk seeks by orders of magnitude and protecting disk integrity by preventing corrupt
//! blocks from being written before hash verification.

use crate::error::BitTorrentError;
use crate::io_traits::{BlockReader, BlockStorage, BlockWriter};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Individual cached data block with offset and metadata.
#[derive(Debug, Clone)]
pub struct CacheBlock {
    pub offset: u64,
    pub data: Vec<u8>,
    pub dirty: bool,
    pub last_accessed: Instant,
}

/// Operational statistics for the disk cache.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheMetrics {
    /// Number of read operations served entirely from memory cache.
    pub cache_hits: u64,
    /// Number of read operations that required fetching from underlying storage.
    pub cache_misses: u64,
    /// Number of flush operations executed.
    pub flushes_performed: u64,
    /// Total bytes written into cache memory.
    pub bytes_written_to_cache: u64,
    /// Total bytes written out to underlying storage.
    pub bytes_flushed_to_storage: u64,
    /// Number of coalesced write operations dispatched to storage.
    pub coalesced_write_operations: u64,
    /// Current number of bytes stored in cache.
    pub current_cached_bytes: usize,
    /// Current number of dirty bytes awaiting flush to storage.
    pub current_dirty_bytes: usize,
}

struct CacheState {
    blocks: BTreeMap<u64, CacheBlock>,
    current_cached_bytes: usize,
    current_dirty_bytes: usize,
}

impl CacheState {
    fn new() -> Self {
        CacheState {
            blocks: BTreeMap::new(),
            current_cached_bytes: 0,
            current_dirty_bytes: 0,
        }
    }
}

/// A high-performance in-memory caching and write-coalescing wrapper for [`BlockStorage`].
pub struct CachedBlockStorage {
    inner: Arc<dyn BlockStorage>,
    max_cache_bytes: usize,
    state: Mutex<CacheState>,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    flushes_performed: AtomicU64,
    bytes_written_to_cache: AtomicU64,
    bytes_flushed_to_storage: AtomicU64,
    coalesced_write_operations: AtomicU64,
}

impl CachedBlockStorage {
    /// Wraps an underlying `BlockStorage` with an in-memory write-coalescing cache of `max_cache_bytes` size.
    pub fn new(inner: Arc<dyn BlockStorage>, max_cache_bytes: usize) -> Self {
        CachedBlockStorage {
            inner,
            max_cache_bytes,
            state: Mutex::new(CacheState::new()),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            flushes_performed: AtomicU64::new(0),
            bytes_written_to_cache: AtomicU64::new(0),
            bytes_flushed_to_storage: AtomicU64::new(0),
            coalesced_write_operations: AtomicU64::new(0),
        }
    }

    /// Returns a reference to the underlying storage instance.
    pub fn inner(&self) -> &Arc<dyn BlockStorage> {
        &self.inner
    }

    /// Retrieves a snapshot of operational metrics from the cache.
    pub fn metrics(&self) -> CacheMetrics {
        let state = self.state.lock().unwrap();
        CacheMetrics {
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            flushes_performed: self.flushes_performed.load(Ordering::Relaxed),
            bytes_written_to_cache: self.bytes_written_to_cache.load(Ordering::Relaxed),
            bytes_flushed_to_storage: self.bytes_flushed_to_storage.load(Ordering::Relaxed),
            coalesced_write_operations: self.coalesced_write_operations.load(Ordering::Relaxed),
            current_cached_bytes: state.current_cached_bytes,
            current_dirty_bytes: state.current_dirty_bytes,
        }
    }

    /// Returns the maximum allowed memory size in bytes for this cache.
    pub fn max_cache_bytes(&self) -> usize {
        self.max_cache_bytes
    }

    /// Flushes all dirty blocks, coalescing contiguous ranges into sequential writes.
    pub fn flush_all(&self) -> Result<(), BitTorrentError> {
        let mut state = self.state.lock().unwrap();
        self.flush_internal(&mut state, None)
    }

    /// Flushes all dirty blocks falling within a specific byte offset range.
    pub fn flush_range(&self, start: u64, length: u64) -> Result<(), BitTorrentError> {
        let mut state = self.state.lock().unwrap();
        self.flush_internal(&mut state, Some((start, start.saturating_add(length))))
    }

    /// Flushes all dirty blocks associated with a specific torrent piece index.
    pub fn flush_piece(&self, piece_number: u32, piece_length: u32) -> Result<(), BitTorrentError> {
        let start = piece_number as u64 * piece_length as u64;
        self.flush_range(start, piece_length as u64)
    }

    /// Discards cached blocks falling within a specific byte offset range without flushing to disk.
    /// Useful for dropping unverified or corrupted blocks that failed hash checks.
    pub fn discard_range(&self, start: u64, length: u64) {
        let mut state = self.state.lock().unwrap();
        let end = start.saturating_add(length);
        let offsets_to_remove: Vec<u64> = state
            .blocks
            .range(start..end)
            .map(|(offset, _)| *offset)
            .collect();

        for offset in offsets_to_remove {
            if let Some(removed) = state.blocks.remove(&offset) {
                state.current_cached_bytes = state
                    .current_cached_bytes
                    .saturating_sub(removed.data.len());
                if removed.dirty {
                    state.current_dirty_bytes =
                        state.current_dirty_bytes.saturating_sub(removed.data.len());
                }
            }
        }
    }

    /// Discards all cached blocks for a specific piece index without flushing to disk.
    pub fn discard_piece(&self, piece_number: u32, piece_length: u32) {
        let start = piece_number as u64 * piece_length as u64;
        self.discard_range(start, piece_length as u64);
    }

    /// Clears all blocks from the cache immediately.
    pub fn clear(&self) {
        let mut state = self.state.lock().unwrap();
        state.blocks.clear();
        state.current_cached_bytes = 0;
        state.current_dirty_bytes = 0;
    }

    /// Internal worker that finds dirty blocks, coalesces contiguous sequences, and writes them to storage.
    fn flush_internal(
        &self,
        state: &mut CacheState,
        range_filter: Option<(u64, u64)>,
    ) -> Result<(), BitTorrentError> {
        // Coalesce contiguous dirty blocks directly in one pass (BTreeMap is already sorted by offset)
        let mut chunks: Vec<(u64, Vec<u8>)> = Vec::new();
        let mut flushed_offsets: Vec<u64> = Vec::new();

        for (&offset, block) in state.blocks.iter() {
            if !block.dirty {
                continue;
            }
            if let Some((range_start, range_end)) = range_filter {
                let block_end = offset.saturating_add(block.data.len() as u64);
                if block_end <= range_start || offset >= range_end {
                    continue;
                }
            }
            flushed_offsets.push(offset);
            if let Some((last_offset, last_data)) = chunks.last_mut() {
                let last_end = *last_offset + last_data.len() as u64;
                if offset == last_end {
                    last_data.extend_from_slice(&block.data);
                    continue;
                }
            }
            chunks.push((offset, block.data.clone()));
        }

        if chunks.is_empty() {
            return Ok(());
        }

        // Write each coalesced chunk to underlying storage
        for (start_offset, coalesced_data) in &chunks {
            self.inner.write_block(*start_offset, coalesced_data)?;
            self.coalesced_write_operations
                .fetch_add(1, Ordering::Relaxed);
            self.bytes_flushed_to_storage
                .fetch_add(coalesced_data.len() as u64, Ordering::Relaxed);
        }

        // Mark flushed blocks as clean in the cache
        let mut bytes_flushed_total = 0usize;
        for offset in flushed_offsets {
            if let Some(block) = state.blocks.get_mut(&offset) {
                if block.dirty {
                    block.dirty = false;
                    bytes_flushed_total += block.data.len();
                }
            }
        }

        state.current_dirty_bytes = state
            .current_dirty_bytes
            .saturating_sub(bytes_flushed_total);
        self.flushes_performed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Evicts clean or oldest dirty blocks if cache size exceeds capacity.
    fn evict_if_needed(&self, state: &mut CacheState) -> Result<(), BitTorrentError> {
        if state.current_cached_bytes <= self.max_cache_bytes {
            return Ok(());
        }

        // Pass 1: Evict clean blocks first using LRU
        let mut clean_blocks: Vec<(u64, usize, Instant)> = state
            .blocks
            .iter()
            .filter(|(_, b)| !b.dirty)
            .map(|(&off, b)| (off, b.data.len(), b.last_accessed))
            .collect();

        clean_blocks.sort_by_key(|(_, _, last_accessed)| *last_accessed);

        for (offset, size, _) in clean_blocks {
            state.blocks.remove(&offset);
            state.current_cached_bytes = state.current_cached_bytes.saturating_sub(size);
            if state.current_cached_bytes <= self.max_cache_bytes {
                return Ok(());
            }
        }

        // Pass 2: If still overflowing, flush all dirty blocks and then evict
        if state.current_cached_bytes > self.max_cache_bytes {
            self.flush_internal(state, None)?;

            // Now all blocks are clean, evict LRU until under max_cache_bytes
            let mut remaining_blocks: Vec<(u64, usize, Instant)> = state
                .blocks
                .iter()
                .map(|(&off, b)| (off, b.data.len(), b.last_accessed))
                .collect();
            remaining_blocks.sort_by_key(|(_, _, last_accessed)| *last_accessed);

            for (offset, size, _) in remaining_blocks {
                state.blocks.remove(&offset);
                state.current_cached_bytes = state.current_cached_bytes.saturating_sub(size);
                if state.current_cached_bytes <= self.max_cache_bytes {
                    break;
                }
            }
        }

        Ok(())
    }
}

impl BlockWriter for CachedBlockStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        if data.is_empty() {
            return Ok(());
        }

        if offset.checked_add(data.len() as u64).is_none() {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "write offset + length overflowed u64",
            )));
        }

        let mut state = self.state.lock().unwrap();

        // Check if block already exists at exact offset
        if let Some(existing) = state.blocks.get_mut(&offset) {
            let old_len = existing.data.len();
            let old_dirty = existing.dirty;
            existing.data = data.to_vec();
            existing.dirty = true;
            existing.last_accessed = Instant::now();
            state.current_cached_bytes = state.current_cached_bytes.saturating_sub(old_len);
            if old_dirty {
                state.current_dirty_bytes = state.current_dirty_bytes.saturating_sub(old_len);
            }
        } else {
            state.blocks.insert(
                offset,
                CacheBlock {
                    offset,
                    data: data.to_vec(),
                    dirty: true,
                    last_accessed: Instant::now(),
                },
            );
        }

        state.current_cached_bytes += data.len();
        state.current_dirty_bytes += data.len();
        self.bytes_written_to_cache
            .fetch_add(data.len() as u64, Ordering::Relaxed);

        self.evict_if_needed(&mut state)?;
        Ok(())
    }

    fn flush(&self) -> Result<(), BitTorrentError> {
        self.flush_all()
    }

    fn discard_range(&self, offset: u64, length: u64) {
        self.discard_range(offset, length);
    }
}

impl BlockReader for CachedBlockStorage {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        let target_len = buffer.len();
        if target_len == 0 {
            return Ok(0);
        }
        let target_start = offset;
        let target_end = match offset.checked_add(target_len as u64) {
            Some(end) => end,
            None => {
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "read offset + length overflowed u64",
                )));
            }
        };

        let mut state = self.state.lock().unwrap();
        let now = Instant::now();

        // Check for exact match first (standard 16 KiB block read path)
        if let Some(block) = state.blocks.get_mut(&offset) {
            if block.data.len() >= target_len {
                buffer.copy_from_slice(&block.data[..target_len]);
                block.last_accessed = now;
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(target_len);
            }
        }

        // Check for any overlapping blocks in the cache
        let mut covered = vec![false; target_len];
        let mut any_cache_hit = false;

        for (&block_offset, block) in state.blocks.iter_mut() {
            let block_end = block_offset + block.data.len() as u64;
            if block_offset < target_end && block_end > target_start {
                let overlap_start = std::cmp::max(target_start, block_offset);
                let overlap_end = std::cmp::min(target_end, block_end);

                let src_start = (overlap_start - block_offset) as usize;
                let src_end = (overlap_end - block_offset) as usize;
                let dst_start = (overlap_start - target_start) as usize;
                let dst_end = (overlap_end - target_start) as usize;

                buffer[dst_start..dst_end].copy_from_slice(&block.data[src_start..src_end]);
                for c in &mut covered[dst_start..dst_end] {
                    *c = true;
                }
                block.last_accessed = now;
                any_cache_hit = true;
            }
        }

        // If entire buffer was satisfied from cache
        if covered.iter().all(|&c| c) {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(target_len);
        }

        // For any gaps not covered by cache, read from underlying storage
        drop(state); // Drop lock while performing underlying I/O

        let mut i = 0;
        while i < target_len {
            if !covered[i] {
                let start_gap = i;
                while i < target_len && !covered[i] {
                    i += 1;
                }
                let gap_len = i - start_gap;
                let gap_offset = target_start + start_gap as u64;
                self.inner
                    .read_block(gap_offset, &mut buffer[start_gap..start_gap + gap_len])?;
            } else {
                i += 1;
            }
        }

        if any_cache_hit {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }

        Ok(target_len)
    }
}

impl Drop for CachedBlockStorage {
    fn drop(&mut self) {
        // Automatically flush any remaining dirty data on drop
        let _ = self.flush_all();
    }
}
