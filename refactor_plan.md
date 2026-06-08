# Concrete Refactoring Plan: BitTorrent Library Size & Performance Optimization

This document outlines a concrete plan to optimize the performance and reduce the binary size of the `bittorrent-rs` library.

---

## 1. High-Performance Zero-Copy Bencode Parser

### Current Issue
The `BNode` enum in [bencode.rs](file:///c:/Projects/BitTorrent/library/src/bencode.rs) holds owned data types:
```rust
pub enum BNode {
    Dictionary(Vec<(Vec<u8>, BNode)>),
    List(Vec<BNode>),
    Number(Vec<u8>),
    String(Vec<u8>),
}
```
During torrent metainfo and tracker response decoding, the parser allocates a `Vec<u8>` for every dictionary key, string, and integer. This generates thousands of small heap allocations, impacting CPU cache lines and memory layout.

### Proposed Refactoring
Refactor `BNode` to be a zero-copy structure referencing the original buffer lifetime `'a`:
```rust
pub enum BNode<'a> {
    Dictionary(Vec<(&'a [u8], BNode<'a>)>),
    List(Vec<BNode<'a>>),
    Number(&'a [u8]),
    String(&'a [u8]),
}
```
- **Performance**: Eliminates all string and number allocation overhead during parsing.
- **Size**: Decreases memory allocation code path instantiation in the binary.

---

## 2. Zero-Copy Peer Wire Protocol Messages (`PeerMessage<'a>`)

### Current Issue
The `PeerMessage::Piece` variant holds an owned buffer:
```rust
pub enum PeerMessage {
    ...
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    ...
}
```
For every 16 KB block received from the network, `PeerNetwork::read_message` allocates a fresh `Vec<u8>` and copies the payload. This payload is copied again when written to the file and then discarded, creating significant memory copy overhead.

### Proposed Refactoring
Introduce a lifetime variable to `PeerMessage` to borrow slices directly from `PeerNetwork::read_buffer`:
```rust
pub enum PeerMessage<'a> {
    ...
    Bitfield(&'a [u8]),
    Piece {
        index: u32,
        begin: u32,
        block: &'a [u8],
    },
    ...
}
```
- **Performance**: Zero heap allocations and zero memory copies when parsing incoming blocks from the network.
- **Size**: Simplifies serialization/deserialization code paths.

---

## 3. Static Enum Dispatch for Announcer

### Current Issue
The `Tracker` struct in [tracker.rs](file:///c:/Projects/BitTorrent/library/src/tracker.rs) uses dynamic dispatch for protocols:
```rust
announcer: Box<dyn Announcer>
```
This requires virtual table (vtable) lookups at runtime and forces a heap allocation (`Box`) upon initialization.

### Proposed Refactoring
Replace the trait object with a static dispatch enum:
```rust
pub enum AnnouncerType {
    Http(HttpAnnouncer),
    Udp(UdpAnnouncer),
}
```
- **Performance**: Avoids dynamic dispatch vtable overhead and heap allocation. Enables compiler inlining optimizations.
- **Size**: Eliminates vtable structure generation in the binary.

---

## 4. Eliminate Unused Crate Dependencies

### Current Issue
The library's [Cargo.toml](file:///c:/Projects/BitTorrent/library/Cargo.toml) lists unused crates or crates that can be easily replaced:
1. `hex = "0.4"`: Entirely unused in the library (hex formatting is manually implemented in [util.rs](file:///c:/Projects/BitTorrent/library/src/util.rs)).
2. `base64 = "0.21"`: Only used to encode random suffixes for peer IDs in [peer_id.rs](file:///c:/Projects/BitTorrent/library/src/peer_id.rs).

### Proposed Refactoring
1. Remove `hex` from [Cargo.toml](file:///c:/Projects/BitTorrent/library/Cargo.toml).
2. Replace `base64` in [peer_id.rs](file:///c:/Projects/BitTorrent/library/src/peer_id.rs) by generating alphanumeric random suffixes directly using the `rand` crate:
```rust
pub fn get() -> String {
    let mut rng = rand::thread_rng();
    let chars: String = (0..12)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            let char_list = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            char_list[idx] as char
        })
        .collect();
    format!("-AZ1000-{}", chars)
}
```
- **Size**: Decreases compiled binary size by removing deep dependencies from the build graph.
- **Speed**: Speeds up compilation times.

---

## 5. Remove Dead Code (Selector & DiskIO Channels)

### Current Issue
1. [selector.rs](file:///c:/Projects/BitTorrent/library/src/selector.rs) contains unused methods:
   - `next_piece`: Never called.
   - `local_piece_suggestions`: Never called.
   - `get_list_of_peers`: Never called.
2. [disk_io.rs](file:///c:/Projects/BitTorrent/library/src/disk_io.rs) defines channels and a background thread:
   - `piece_write_queue` and `piece_request_queue` are created but never written to or read from.
   - `DiskIO::new` spawns a background thread that immediately exits.

### Proposed Refactoring
- Clean up unused methods in `Selector`.
- Remove dead channels and the unused background thread from `DiskIO`.
- **Size**: Directly reduces the instruction count and size of the library.

---

## 6. Optimize Rarest-Piece Selection Algorithm

### Current Issue
In [torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs), the block request loop calls `get_sorted_missing_pieces_for_peer`:
```rust
    fn get_sorted_missing_pieces_for_peer(&self, peer: &Peer) -> Vec<(usize, u32)> {
        let mut candidates: Vec<(usize, u32)> = (0..self.number_of_pieces as u32)
            .filter(|&piece| !self.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
            .map(|piece| (self.piece_data[piece as usize].peer_count, piece))
            .collect();
        candidates.sort_by_key(|(count, piece)| (*count, *piece));
        candidates
    }
```
Sorting the list of all missing pieces (potentially thousands of elements) on every 16 KB block request is extremely CPU intensive.

### Proposed Refactoring
- Optimize this to find the minimum element in a single pass (using `min_by_key`) instead of doing a full sort when requesting one block at a time.
- Alternatively, cache the rarest piece rankings and update them incrementally when pieces are completed, avoiding re-scanning the entire piece list on every request.
- **Performance**: Drastically reduces CPU utilization in the peer worker threads.

---

## 7. Fine-Grained Locking & Atomic State Metrics

### Current Issue
The entire `TorrentContext` is protected by a single monolithic Mutex lock, serializing all updates and stat queries across all peer worker threads and the GUI main thread.

### Proposed Refactoring
Separate hot metrics (such as `total_bytes_downloaded` and `total_bytes_uploaded`) out of `TorrentContext` or make them atomic values:
```rust
pub struct TorrentContext {
    pub total_bytes_downloaded: Arc<AtomicU64>,
    pub total_bytes_uploaded: Arc<AtomicU64>,
    ...
}
```
- **Performance**: Allows lock-free reads of progress statistics by the GUI and background logger threads, minimizing lock contention.
