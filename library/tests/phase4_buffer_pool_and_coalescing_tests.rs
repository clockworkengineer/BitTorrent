//! Integration tests for Phase 4: Performance & Zero-Allocation Buffer Pooling
//!
//! Validates `BlockBufferPool` slab allocation, RAII `PooledBlockBuffer`,
//! write-coalescing integration with `CachedBlockStorage`, and `Peer` buffer pooling.

use bittorrent_rs::{
    BitTorrentError,
    constants::BLOCK_SIZE,
    io_traits::{BlockReader, BlockWriter, MemStorage, MockSocket, Socket},
    network::peer::Peer,
    storage::disk_cache::CachedBlockStorage,
    storage::piece_buffer::{BlockBufferPool, PooledBlockBuffer},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// Counting mock storage that tracks write invocations and transferred bytes.
struct CoalesceTrackerStorage {
    inner: MemStorage,
    write_count: AtomicUsize,
    bytes_transferred: AtomicUsize,
}

impl CoalesceTrackerStorage {
    fn new(capacity: usize) -> Self {
        CoalesceTrackerStorage {
            inner: MemStorage::new(capacity),
            write_count: AtomicUsize::new(0),
            bytes_transferred: AtomicUsize::new(0),
        }
    }

    fn write_count(&self) -> usize {
        self.write_count.load(Ordering::SeqCst)
    }

    fn total_bytes(&self) -> usize {
        self.bytes_transferred.load(Ordering::SeqCst)
    }
}

impl BlockReader for CoalesceTrackerStorage {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        self.inner.read_block(offset, buffer)
    }
}

impl BlockWriter for CoalesceTrackerStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        self.write_count.fetch_add(1, Ordering::SeqCst);
        self.bytes_transferred
            .fetch_add(data.len(), Ordering::SeqCst);
        self.inner.write_block(offset, data)
    }
}

#[test]
fn test_block_buffer_pool_basic_lifecycle() {
    let pool = BlockBufferPool::new(4, BLOCK_SIZE);
    assert_eq!(pool.available(), 0);

    // Initial allocation from empty pool
    let mut buf = pool.acquire(BLOCK_SIZE);
    assert!(buf.is_empty());
    assert!(buf.capacity() >= BLOCK_SIZE);

    buf.extend_from_slice(&[0xAA; BLOCK_SIZE]);
    assert_eq!(buf.len(), BLOCK_SIZE);

    // Release back into pool
    pool.release(buf);
    assert_eq!(pool.available(), 1);

    let metrics = pool.metrics();
    assert_eq!(metrics.total_allocations, 1);
    assert_eq!(metrics.reused_allocations, 0);
    assert_eq!(metrics.available_buffers, 1);
    assert_eq!(metrics.capacity, 4);

    // Acquire again - should hit pool reuse with 0 reallocations
    let reused = pool.acquire(BLOCK_SIZE);
    assert!(reused.is_empty());
    assert!(reused.capacity() >= BLOCK_SIZE);

    let metrics_after = pool.metrics();
    assert_eq!(metrics_after.total_allocations, 2);
    assert_eq!(metrics_after.reused_allocations, 1);
    assert_eq!(metrics_after.available_buffers, 0);
}

#[test]
fn test_block_buffer_pool_capacity_limit() {
    let pool = BlockBufferPool::new(2, BLOCK_SIZE);

    let buf1 = pool.acquire(BLOCK_SIZE);
    let buf2 = pool.acquire(BLOCK_SIZE);
    let buf3 = pool.acquire(BLOCK_SIZE);

    pool.release(buf1);
    pool.release(buf2);
    // Pool capacity is 2, third release should be dropped
    pool.release(buf3);

    assert_eq!(pool.available(), 2);
    assert_eq!(pool.metrics().capacity, 2);
}

#[test]
fn test_pooled_block_buffer_raii_guard() {
    let pool = Arc::new(BlockBufferPool::new(4, BLOCK_SIZE));

    {
        let mut guard = PooledBlockBuffer::acquire(pool.clone(), BLOCK_SIZE);
        assert_eq!(guard.len(), 0);
        guard.extend_from_slice(&[0x55; 1024]);
        assert_eq!(guard.len(), 1024);
        assert_eq!(pool.available(), 0);
    }
    // Guard dropped, buffer returned to pool automatically
    assert_eq!(pool.available(), 1);

    // Re-acquire through RAII
    let guard2 = PooledBlockBuffer::acquire(pool.clone(), BLOCK_SIZE);
    assert_eq!(guard2.len(), 0);
    assert_eq!(pool.available(), 0);

    let m = pool.metrics();
    assert_eq!(m.reused_allocations, 1);
}

#[test]
fn test_pooled_block_buffer_take() {
    let pool = Arc::new(BlockBufferPool::new(4, BLOCK_SIZE));

    let raw_vec = {
        let mut guard = PooledBlockBuffer::acquire(pool.clone(), BLOCK_SIZE);
        guard.extend_from_slice(&[1, 2, 3, 4]);
        guard.take()
    };
    // Because take() consumed the guard, it was not returned to pool on drop
    assert_eq!(pool.available(), 0);
    assert_eq!(raw_vec, vec![1, 2, 3, 4]);

    // Manually release
    pool.release(raw_vec);
    assert_eq!(pool.available(), 1);
}

#[test]
fn test_block_buffer_pool_concurrent_threads() {
    let pool = Arc::new(BlockBufferPool::new(16, BLOCK_SIZE));
    let mut handles = Vec::new();

    for thread_idx in 0..8 {
        let pool_clone = pool.clone();
        handles.push(thread::spawn(move || {
            for i in 0..50 {
                let mut buf = pool_clone.acquire(BLOCK_SIZE);
                buf.resize(BLOCK_SIZE, ((thread_idx * 50 + i) % 256) as u8);
                // simulate short work
                assert_eq!(buf.len(), BLOCK_SIZE);
                pool_clone.release(buf);
            }
        }));
    }

    for h in handles {
        h.join().expect("thread failed");
    }

    let m = pool.metrics();
    assert_eq!(m.total_allocations, 400);
    assert!(m.reused_allocations > 0);
    assert!(m.available_buffers <= 16);
}

#[test]
fn test_cached_block_storage_coalescing_with_buffer_pool() {
    let tracker = Arc::new(CoalesceTrackerStorage::new(1024 * 1024));
    let pool = Arc::new(BlockBufferPool::new(32, BLOCK_SIZE));
    let cache = CachedBlockStorage::new_with_pool(tracker.clone(), 512 * 1024, pool.clone());

    // Write 4 contiguous 16 KiB blocks (total 64 KiB)
    let block0 = vec![0x11u8; BLOCK_SIZE];
    let block1 = vec![0x22u8; BLOCK_SIZE];
    let block2 = vec![0x33u8; BLOCK_SIZE];
    let block3 = vec![0x44u8; BLOCK_SIZE];

    cache.write_block(0, &block0).unwrap();
    cache.write_block(BLOCK_SIZE as u64, &block1).unwrap();
    cache.write_block(2 * BLOCK_SIZE as u64, &block2).unwrap();
    cache.write_block(3 * BLOCK_SIZE as u64, &block3).unwrap();

    // Prior to flush, underlying storage received 0 writes
    assert_eq!(tracker.write_count(), 0);

    // Flush all dirty blocks
    cache.flush_all().unwrap();

    // Verify coalescing: 4 contiguous blocks must be coalesced into 1 write
    assert_eq!(tracker.write_count(), 1);
    assert_eq!(tracker.total_bytes(), 4 * BLOCK_SIZE);

    let m = cache.metrics();
    assert_eq!(m.coalesced_write_operations, 1);
    assert_eq!(m.bytes_flushed_to_storage, 4 * BLOCK_SIZE as u64);

    // Verify underlying data integrity
    let mut verify_buf = vec![0u8; 4 * BLOCK_SIZE];
    tracker.read_block(0, &mut verify_buf).unwrap();
    assert_eq!(&verify_buf[..BLOCK_SIZE], &block0[..]);
    assert_eq!(&verify_buf[BLOCK_SIZE..2 * BLOCK_SIZE], &block1[..]);
    assert_eq!(&verify_buf[2 * BLOCK_SIZE..3 * BLOCK_SIZE], &block2[..]);
    assert_eq!(&verify_buf[3 * BLOCK_SIZE..4 * BLOCK_SIZE], &block3[..]);
}

#[test]
fn test_cached_block_storage_non_contiguous_coalescing() {
    let tracker = Arc::new(CoalesceTrackerStorage::new(1024 * 1024));
    let pool = Arc::new(BlockBufferPool::new(16, BLOCK_SIZE));
    let cache = CachedBlockStorage::new_with_pool(tracker.clone(), 256 * 1024, pool);

    // Write block 0 (at 0) and block 2 (at 32768), leaving a gap at 16384
    let block0 = vec![0xAAu8; BLOCK_SIZE];
    let block2 = vec![0xBBu8; BLOCK_SIZE];

    cache.write_block(0, &block0).unwrap();
    cache.write_block(2 * BLOCK_SIZE as u64, &block2).unwrap();

    assert_eq!(tracker.write_count(), 0);

    cache.flush_all().unwrap();

    // Non-contiguous blocks cannot be coalesced into 1 write; must be 2 separate writes
    assert_eq!(tracker.write_count(), 2);
    assert_eq!(tracker.total_bytes(), 2 * BLOCK_SIZE);
    assert_eq!(cache.metrics().coalesced_write_operations, 2);
}

#[test]
fn test_peer_buffer_pool_integration() {
    let (mock_sock, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(mock_sock));
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket);

    let custom_pool = Arc::new(BlockBufferPool::new(8, BLOCK_SIZE));
    peer.set_buffer_pool(custom_pool.clone());

    // Peer acquires buffer
    let mut buf = peer.acquire_buffer(BLOCK_SIZE);
    assert!(buf.is_empty());
    assert!(buf.capacity() >= BLOCK_SIZE);
    buf.extend_from_slice(&[0xFF; BLOCK_SIZE]);

    // Peer releases buffer back to pool
    peer.release_buffer(buf);

    assert_eq!(custom_pool.available(), 1);
    let metrics = custom_pool.metrics();
    assert_eq!(metrics.total_allocations, 1);
}
