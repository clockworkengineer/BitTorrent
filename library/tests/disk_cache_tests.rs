use bittorrent_rs::{
    io_traits::{BlockReader, BlockWriter, MemStorage},
    session::builder::TorrentSessionBuilder,
    session::config::SessionConfig,
    storage::disk_cache::CachedBlockStorage,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Mock underlying storage that counts individual write calls to verify coalescing.
struct CountingStorage {
    inner: MemStorage,
    write_call_count: AtomicUsize,
    bytes_written: AtomicUsize,
}

impl CountingStorage {
    fn new() -> Self {
        CountingStorage {
            inner: MemStorage::new(10 * 1024 * 1024),
            write_call_count: AtomicUsize::new(0),
            bytes_written: AtomicUsize::new(0),
        }
    }

    fn write_calls(&self) -> usize {
        self.write_call_count.load(Ordering::SeqCst)
    }

    fn total_bytes(&self) -> usize {
        self.bytes_written.load(Ordering::SeqCst)
    }
}

impl BlockReader for CountingStorage {
    fn read_block(
        &self,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<usize, bittorrent_rs::BitTorrentError> {
        self.inner.read_block(offset, buffer)
    }
}

impl BlockWriter for CountingStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), bittorrent_rs::BitTorrentError> {
        self.write_call_count.fetch_add(1, Ordering::SeqCst);
        self.bytes_written.fetch_add(data.len(), Ordering::SeqCst);
        self.inner.write_block(offset, data)
    }
}

#[test]
fn test_cached_storage_write_and_read_from_memory() {
    let mock = Arc::new(CountingStorage::new());
    let cache = CachedBlockStorage::new(mock.clone(), 1024 * 1024);

    let block_data = vec![0x42u8; 16384];
    cache.write_block(0, &block_data).expect("write failed");

    // Block was written to cache, so underlying storage has NOT received any write calls yet
    assert_eq!(mock.write_calls(), 0);

    // Read back block from cache
    let mut read_buf = vec![0u8; 16384];
    let n = cache.read_block(0, &mut read_buf).expect("read failed");
    assert_eq!(n, 16384);
    assert_eq!(read_buf, block_data);

    let metrics = cache.metrics();
    assert_eq!(metrics.cache_hits, 1);
    assert_eq!(metrics.cache_misses, 0);
    assert_eq!(metrics.current_cached_bytes, 16384);
    assert_eq!(metrics.current_dirty_bytes, 16384);
}

#[test]
fn test_write_coalescing_contiguous_blocks() {
    let mock = Arc::new(CountingStorage::new());
    let cache = CachedBlockStorage::new(mock.clone(), 1024 * 1024);

    // Write 4 contiguous 16 KiB blocks (total 64 KiB = 65536 bytes)
    for i in 0..4 {
        let offset = i as u64 * 16384;
        let data = vec![i as u8; 16384];
        cache.write_block(offset, &data).expect("write failed");
    }

    assert_eq!(mock.write_calls(), 0);
    let metrics = cache.metrics();
    assert_eq!(metrics.current_cached_bytes, 65536);
    assert_eq!(metrics.current_dirty_bytes, 65536);

    // Flush dirty blocks
    cache.flush_all().expect("flush failed");

    // All 4 contiguous blocks should have been coalesced into 1 single write call!
    assert_eq!(mock.write_calls(), 1);
    assert_eq!(mock.total_bytes(), 65536);

    let metrics = cache.metrics();
    assert_eq!(metrics.flushes_performed, 1);
    assert_eq!(metrics.coalesced_write_operations, 1);
    assert_eq!(metrics.bytes_flushed_to_storage, 65536);
    assert_eq!(metrics.current_dirty_bytes, 0);

    // Verify written contents in underlying storage
    for i in 0..4 {
        let offset = i as u64 * 16384;
        let mut buf = vec![0u8; 16384];
        mock.read_block(offset, &mut buf).expect("read failed");
        assert_eq!(buf, vec![i as u8; 16384]);
    }
}

#[test]
fn test_non_contiguous_blocks_flush_distinct_chunks() {
    let mock = Arc::new(CountingStorage::new());
    let cache = CachedBlockStorage::new(mock.clone(), 1024 * 1024);

    // Block 0 at offset 0, Block 1 at offset 65536 (gap between 16384..65536)
    cache.write_block(0, &[0xAA; 16384]).expect("write failed");
    cache
        .write_block(65536, &[0xBB; 16384])
        .expect("write failed");

    cache.flush_all().expect("flush failed");

    // Must execute 2 distinct write calls to storage because of the gap
    assert_eq!(mock.write_calls(), 2);
    assert_eq!(mock.total_bytes(), 32768);
}

#[test]
fn test_flush_piece_and_discard_piece() {
    let mock = Arc::new(CountingStorage::new());
    let cache = CachedBlockStorage::new(mock.clone(), 1024 * 1024);

    let piece_len = 32768u32; // 2 x 16 KiB blocks per piece

    // Piece 0: 2 blocks (offset 0 and 16384)
    cache.write_block(0, &[0x11; 16384]).unwrap();
    cache.write_block(16384, &[0x22; 16384]).unwrap();

    // Piece 1: 2 blocks (offset 32768 and 49152)
    cache.write_block(32768, &[0x33; 16384]).unwrap();
    cache.write_block(49152, &[0x44; 16384]).unwrap();

    // Flush ONLY piece 0
    cache.flush_piece(0, piece_len).unwrap();

    // Exactly 1 coalesced write for piece 0
    assert_eq!(mock.write_calls(), 1);
    assert_eq!(mock.total_bytes(), 32768);

    // Suppose Piece 1 failed hash verification: discard it from cache!
    cache.discard_piece(1, piece_len);

    // Flush all remaining
    cache.flush_all().unwrap();

    // No additional writes occurred because piece 1 was discarded before touching disk!
    assert_eq!(mock.write_calls(), 1);
    assert_eq!(mock.total_bytes(), 32768);
}

#[test]
fn test_cache_miss_reads_from_underlying_storage() {
    let mock = Arc::new(CountingStorage::new());
    // Pre-populate underlying storage with data
    mock.write_block(5000, &[0xEE; 100]).unwrap();
    mock.write_call_count.store(0, Ordering::SeqCst);

    let cache = CachedBlockStorage::new(mock.clone(), 1024 * 1024);

    let mut buf = vec![0u8; 100];
    let n = cache.read_block(5000, &mut buf).unwrap();
    assert_eq!(n, 100);
    assert_eq!(buf, vec![0xEE; 100]);

    let metrics = cache.metrics();
    assert_eq!(metrics.cache_hits, 0);
    assert_eq!(metrics.cache_misses, 1);
}

#[test]
fn test_cache_lru_eviction_when_capacity_exceeded() {
    let mock = Arc::new(CountingStorage::new());
    // Capacity of exactly 32 KiB
    let cache = CachedBlockStorage::new(mock.clone(), 32768);

    // Write 2 blocks of 16 KiB = 32 KiB
    cache.write_block(0, &[0x10; 16384]).unwrap();
    cache.write_block(16384, &[0x20; 16384]).unwrap();

    // Flush so blocks become clean
    cache.flush_all().unwrap();
    assert_eq!(mock.write_calls(), 1);

    // Read block 0 to refresh its last_accessed timestamp
    let mut buf = vec![0u8; 16384];
    cache.read_block(0, &mut buf).unwrap();

    // Write 3rd block (16 KiB at offset 32768) -> triggers eviction of least recently used block (block 1 at 16384)
    cache.write_block(32768, &[0x30; 16384]).unwrap();

    let metrics = cache.metrics();
    // Cache size must remain <= max_cache_bytes (32768)
    assert!(metrics.current_cached_bytes <= 32768);
}

#[test]
fn test_session_config_and_builder_disk_cache() {
    let config = SessionConfig::default().with_disk_cache_size_mb(64);
    assert_eq!(config.disk_cache_size_mb, 64);

    let builder = TorrentSessionBuilder::new("sample.torrent", ".").disk_cache_size_mb(128);
    assert_eq!(builder.get_config().disk_cache_size_mb, 128);
}
