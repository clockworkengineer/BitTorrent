# Library Attributes Refactoring & Improvement Plan - Phase 2

This document outlines an updated, concrete, and actionable plan to further elevate the `bittorrent-rs` library in alignment with the 10 core attributes of high-quality software libraries detailed in [attributes.md](file:///c:/Projects/BitTorrent/notes/attributes.md).

---

## 1. Intuitive API Design
### Current Assessment
- We introduced `TorrentSessionBuilder` and encapsulated internal context/worker fields.
### Gaps
- `TorrentSessionBuilder` does not permit direct customization of the piece selection strategy (it defaults to `RarestFirstSelector`).
- The public API of `Tracker` requires consumers to pass `Arc<Mutex<TorrentContext>>` directly, which leaks concurrency synchronization details.
### Proposed Changes
- **Target File**: [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)
- Add a `.selector(Arc<dyn PieceSelector>)` setter to `TorrentSessionBuilder`.
- Add a `.config(SessionConfig)` setter to `TorrentSessionBuilder`.
- Introduce a high-level wrapper on `TorrentSession` to announcement/peer discovery without exposing context locks.

---

## 2. Comprehensive Documentation
### Current Assessment
- Crate-level docs and peer wire state machine ASCII diagrams have been added.
### Gaps
- There is no explanation or code examples on how to configure and use `MemStorage` and `MockSocket` for testing or embedded environments.
### Proposed Changes
- **Target File**: [lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs)
- Add a dedicated `### Portability and Testing` section to the crate-level documentation illustrating how to boot the client in-memory via `MemStorage` and mock peer connections with `MockSocket`.

---

## 3. High Reliability
### Current Assessment
- Timeout configs and exponential announcement retries have been implemented.
### Gaps
- When a peer connection drops or halts block transfers, requested blocks remain in the "pending" state until a timeout occurs, stalling the download speed.
- If a peer sends a corrupted block hash, the client discards the entire piece, but does not identify or ban/choke the bad peer.
### Proposed Changes
- **Target File**: [peer.rs](file:///c:/Projects/BitTorrent/library/src/peer.rs) and [torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs)
- Implement request cancellation and timeout fallback: if a peer fails to fulfill a block request within 2 seconds, cancel the request and re-request from a different peer.
- Implement bad peer blacklisting: track hash verification failures and disconnect/blacklist peers that repeatedly supply corrupted blocks.

---

## 4. Performance and Efficiency
### Current Assessment
- Zero-copy wire parsing, single-pass piece selection, and static buffer pools are implemented.
### Gaps
- The client requests only one block at a time per peer connection. On high-bandwidth, high-latency connections, this suffers from round-trip time (RTT) starvation.
### Proposed Changes
- **Target File**: [peer.rs](file:///c:/Projects/BitTorrent/library/src/peer.rs)
- Implement block request pipelining (queueing up to 5 concurrent block requests per peer connection) to saturate the network pipe and maximize downloading throughput.

---

## 5. Maintainability
### Current Assessment
- Peer action runner and `Assembler` block tracker have been modularized.
### Gaps
- `session.rs` is still quite long and coordinates thread workers, local thread pools, and re-announcing cycles.
### Proposed Changes
- **Target Files**: [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)
- Extract the thread coordination logic and background task scheduling from `session.rs` into a dedicated worker module `session/worker.rs`.

---

## 6. Flexibility and Customization
### Current Assessment
- Pluggable `PieceSelector` trait and dynamic configuration options exist.
### Gaps
- Peer network connections strictly invoke `TcpStream::connect` directly. Users cannot customize socket creation parameters or route connections through proxy layers (like SOCKS5 or Tor/I2P).
### Proposed Changes
- **Target File**: [io_traits.rs](file:///c:/Projects/BitTorrent/library/src/io_traits.rs)
- Introduce a `SocketFactory` trait:
  ```rust
  pub trait SocketFactory: Send + Sync {
      fn connect(&self, ip: &str, port: u16) -> Result<Arc<dyn AsyncSocket>, BitTorrentError>;
  }
  ```
- Store `Arc<dyn SocketFactory>` in the session configuration, permitting complete transport customization.

---

## 7. Strong Security
### Current Assessment
- Path traversal protections are active for multi-file outputs.
### Gaps
- The client does not validate incoming piece block offsets and lengths against the active torrent piece metadata, which could allow a malicious peer to overflow buffers or write blocks outside expected ranges.
### Proposed Changes
- **Target File**: [torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs)
- Add strict boundary checks in `process_piece_block` to verify that `begin + block_len <= piece_length` before executing write commands.

---

## 8. High Testability
### Current Assessment
- Mock socket and in-memory storage implementations are present.
### Gaps
- Testing the `Tracker` announce loop requires binding local TCP listeners and spinning up local HTTP servers, which is resource-intensive and prone to flakiness.
### Proposed Changes
- **Target File**: [tracker.rs](file:///c:/Projects/BitTorrent/library/src/tracker.rs)
- Abstract HTTP client operations under a pluggable `HttpClient` trait, enabling in-memory mocking of tracker announcements without binding ports.

---

## 9. Compatibility and Portability
### Current Assessment
- `#![no_std]` core target support and `MemStorage` are active.
### Gaps
- Time metrics and latency calculations in `Average` and timeout trackers rely on `Instant::now` and `Duration`, which do not compile on platforms without standard system clocks.
### Proposed Changes
- **Target File**: [util.rs](file:///c:/Projects/BitTorrent/library/src/util.rs)
- Gate dynamic statistics tracking and performance metrics calculation under standard target features, or inject a clock interface to maintain portability.

---

## 10. Low Dependency Footprint
### Current Assessment
- Splitting the HTTP tracker feature allows compiling standard library targets without HTTP dependencies.
### Gaps
- The `futures` library is still required even for core target builds.
### Proposed Changes
- **Target File**: [Cargo.toml](file:///c:/Projects/BitTorrent/library/Cargo.toml)
- Move futures dependency entirely into the `std` target feature configuration, leaving the core `#![no_std]` parser completely dependency-free.
