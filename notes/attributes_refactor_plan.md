# Library Attributes Refactoring & Improvement Plan

This document outlines a concrete, actionable plan to align the `bittorrent-rs` library with the 10 core attributes of high-quality software libraries, as detailed in [attributes.md](file:///c:/Projects/BitTorrent/notes/attributes.md).

---

## 1. Intuitive API Design
### Current Assessment
- `TorrentSession::new` requires concrete path inputs and a boolean seeding parameter.
- Internal structures like `context` (`Arc<Mutex<TorrentContext>>`) and `disk_io` are exposed directly, exposing implementation details.
- Several methods like `connect_and_download_peer` and `download_from_peers` require passing an external `Option<Arc<Manager>>` (dead peer tracker) which exposes internal peer-management details.

### Specific Gaps
- Leaked abstractions on [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs#L31-L39).
- Hard-to-read configuration parameters (e.g. `seeding: bool`) instead of descriptive config structures.

### Proposed Changes
- **Target File**: [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)
- Make `context` and `disk_io` private fields in `TorrentSession`.
- Introduce a builder pattern (`TorrentSessionBuilder`) to instantiate sessions cleanly.
- Encapsulate the `Manager` instance internally within `TorrentSession` so that public methods do not require passing it as an option.

---

## 2. Comprehensive Documentation
### Current Assessment
- The crate features comments on most methods, but does not provide complete code snippets or usage instructions in the library docs.
- State machines inside the peer communication loop lack high-level process documentation.

### Specific Gaps
- Missing documentation examples at the crate root in [lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs).
- Lack of sequence diagrams or protocol docs for peer connection handshakes.

### Proposed Changes
- **Target File**: [lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs)
- Add comprehensive doc examples at the top of [lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs) demonstrating how to parse a metainfo file and download a torrent.
- Document peer wire transitions (Choke, Unchoke, Interested, Request) as a state-diagram comment at the top of [peer.rs](file:///c:/Projects/BitTorrent/library/src/peer.rs).

---

## 3. High Reliability
### Current Assessment
- Network connection timeouts and file I/O operations are handled cleanly, but HTTP tracker connection drops or DNS failures in `tracker.rs` are not automatically retried, which can result in peer discovery stalling.

### Specific Gaps
- Single-point tracker query failure in [tracker.rs](file:///c:/Projects/BitTorrent/library/src/tracker.rs#L338-L350).

### Proposed Changes
- **Target File**: [tracker.rs](file:///c:/Projects/BitTorrent/library/src/tracker.rs)
- Introduce a retry mechanism with exponential backoff for tracker HTTP announcements.
- Add strict validation checks to verify DNS lookup results and handle tracker redirect URLs safely.

---

## 4. Performance and Efficiency
### Current Assessment
- The library uses static memory pools and streaming SHA-1 verification.
- However, when sockets block on standard targets, the cooperative read/write loop sleeps for a fixed `2ms` duration, which limits performance and throughput on gigabit networks.

### Specific Gaps
- Fixed sleep polling in `TcpSocket` on [peer_network.rs](file:///c:/Projects/BitTorrent/library/src/peer_network.rs#L156-L159).

### Proposed Changes
- **Target File**: [peer_network.rs](file:///c:/Projects/BitTorrent/library/src/peer_network.rs)
- Integrate event-driven wakers (using conditional compiling for `mio` or `tokio` on `std` target) to wake the cooperative tasks immediately when the socket is ready, instead of waiting for a timer ticks.

---

## 5. Maintainability
### Current Assessment
- The code is modular, but files like `session.rs` and `torrent_context.rs` contain too many responsibilities (combining networking, disk I/O coordination, stats logging, and peer worker threads).

### Specific Gaps
- Monolithic state management in [torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs) (~720 lines).

### Proposed Changes
- **Target Files**: [torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs) and [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)
- Extract peer message routing and processing out of `handle_peer_session` in `session.rs` and delegate it to a dedicated handler inside `peer.rs`.
- Extract the assembler and block storage tracking from `TorrentContext` to a separate `Assembler` struct.

---

## 6. Flexibility and Customization
### Current Assessment
- Swarm size, connect timeout, read timeout, block size, and piece selection logic (Rarest-First) are completely hardcoded.

### Specific Gaps
- Hardcoded constants in [constants.rs](file:///c:/Projects/BitTorrent/library/src/constants.rs) and [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs#L390-L406).

### Proposed Changes
- **Target Files**: [selector.rs](file:///c:/Projects/BitTorrent/library/src/selector.rs) and [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)
- Introduce a pluggable piece selection trait:
  ```rust
  pub trait PieceSelector {
      fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32>;
  }
  ```
  This allows users to choose between `RarestFirst` (default) and `Sequential` (optimal for streaming video).
- Expose connection timeouts, write timeouts, and re-announce floors dynamically in a `SessionConfig` struct.

---

## 7. Strong Security
### Current Assessment
- The library validates file details relative paths to prevent directory traversal.
- However, the root directory name (`root_name` or `name` field of the torrent `info` section) is not validated during multi-file parsing, allowing a malicious torrent to escape the download path.

### Specific Gaps
- Missing folder name validation in [metainfo.rs](file:///c:/Projects/BitTorrent/library/src/metainfo.rs#L281-L286).

### Proposed Changes
- **Target File**: [metainfo.rs](file:///c:/Projects/BitTorrent/library/src/metainfo.rs)
- Validate `root_name` in `local_files_to_download_list` using `validate_relative_path(&root_name)` to prevent directory traversals targeting parent directory folders.

---

## 8. High Testability
### Current Assessment
- Testing network messages currently requires binding to localhost TCP listener ports, which depends on OS resources and makes tests slow and flaky.

### Specific Gaps
- Binding dependencies in [tests/session_tests.rs](file:///c:/Projects/BitTorrent/library/tests/session_tests.rs#L33-L34).

### Proposed Changes
- **Target File**: [io_traits.rs](file:///c:/Projects/BitTorrent/library/src/io_traits.rs)
- Implement a `MockSocket` wrapper:
  ```rust
  pub struct MockSocket {
      pub incoming: std::sync::mpsc::Receiver<Vec<u8>>,
      pub outgoing: std::sync::mpsc::Sender<Vec<u8>>,
  }
  ```
  This enables unit tests to feed bytes directly into the protocol state machine without binding standard TCP ports.

---

## 9. Compatibility and Portability
### Current Assessment
- The core of the library supports `#![no_std]`, but a filesystem implementation does not exist for systems without directories.

### Specific Gaps
- Standard file reliance in [disk_io.rs](file:///c:/Projects/BitTorrent/library/src/disk_io.rs).

### Proposed Changes
- **Target File**: [io_traits.rs](file:///c:/Projects/BitTorrent/library/src/io_traits.rs)
- Introduce a default in-memory storage adapter `MemStorage` implementing `BlockStorage` to store block arrays in standard memory buffers, enabling portability onto targets without disk storage or partition drivers.

---

## 10. Low Dependency Footprint
### Current Assessment
- The library separates target environments cleanly, but pulling in `ureq`, `url`, and `urlencoding` is mandatory on all standard target targets.

### Specific Gaps
- Large standard dependency list in [Cargo.toml](file:///c:/Projects/BitTorrent/library/Cargo.toml#L8-L15).

### Proposed Changes
- **Target File**: [Cargo.toml](file:///c:/Projects/BitTorrent/library/Cargo.toml)
- Split the `std` feature: introduce a new `http-tracker` feature gating HTTP clients.
- Allow compiling a `std` client using only raw sockets (e.g. manual peer feeding or UDP tracker announcements) without pulling in HTTP request parsing engines.
