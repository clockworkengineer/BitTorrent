# Concrete Library Size & Performance Refactoring Plan

This document outlines the concrete steps to reduce the binary footprint and maximize the runtime performance of the core `bittorrent-rs` library.

---

## 1. Devirtualize Async Sockets via Native Async Traits
* **Problem:** [AsyncSocket](file:///c:/Projects/BitTorrent/library/src/utils/io_traits.rs) currently returns boxed futures:
  ```rust
  fn read<'a>(&'a self, buf: &'a mut [u8]) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>>;
  ```
  This incurs a heap allocation (`Box`) on **every single network read and write packet**, leading to memory fragmentation and overhead.
* **Refactor Plan:**
  * Since the project uses Rust 2024, leverage native `async fn` in traits:
    ```rust
    pub trait AsyncSocket: Send + Sync {
        async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError>;
        async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError>;
        fn close(&self);
    }
    ```
  * Refactor socket implementations in the network stack (including mock sockets and TCP sockets) to use native `async fn` implementations.

## 2. Zero-Allocation Bencode Tokenizer
* **Problem:** `BNode` (in [bencode.rs](file:///c:/Projects/BitTorrent/library/src/utils/bencode.rs)) parses bencoded bytes into heap-allocated dictionary and list arrays (`Vec<BNode>`). For large torrents (e.g. metadata with thousands of files), this causes significant heap bloat.
* **Refactor Plan:**
  * Introduce a flat, streaming parser iterator:
    ```rust
    pub struct BencodeTokenizer<'a> {
        buffer: &'a [u8],
        position: usize,
    }
    ```
  * The tokenizer yields tokens pointing to slices of the original buffer (e.g., integer values, string boundaries, dictionary keys) without allocating collections.
  * Update `MetaInfoFile` parsing to query token slices lazily.

## 3. Lock-Free Atomic Statistics
* **Problem:** Global download/upload counters use `Mutex<TorrentContext>` or Atomic fields inside Mutexes.
* **Refactor Plan:**
  * Migrate counters and speed-tracking metrics strictly to `AtomicU64` and `AtomicU32` structures, avoiding thread synchronization locks in fast peer worker paths.
