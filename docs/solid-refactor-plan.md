# SOLID Refactoring Plan: BitTorrent Rust Library & Clients

This document outlines a concrete, phased refactoring plan to transform the BitTorrent Rust library and client suite to strictly follow SOLID software design principles.

---

## Executive Summary & Objectives

While the library currently features high performance and extensive protocol support (v1/v2 metainfo, DHT, LSD, PEX, MSE, uTP, NAT-PMP, WebSeed), several core components suffer from monolithic struct design, tight coupling to concrete types, static enum dispatch, and broad mutable state sharing.

### Key Refactoring Goals:
1. **S - Single Responsibility Principle (SRP)**: Decompose monolithic structs (`TorrentContext`, `TorrentSession`, `MetaInfoFile`, `worker.rs`) into single-responsibility, cohesive components.
2. **O - Open/Closed Principle (OCP)**: Replace static `enum` dispatches (`AnnouncerEnum`) and hardcoded background routines with trait-based extensions (`Announcer`, `PeerDiscovery`, `StorageDriver`).
3. **L - Liskov Substitution Principle (LSP)**: Establish formal behavioral contracts and assertion suites for core abstractions (`AsyncSocket`, `BlockStorage`), ensuring mocks (`MockSocket`, `MemStorage`) and real implementations (`TcpStream`, `UtpSocketAdapter`, `DiskIO`) are 100% interchangeable.
4. **I - Interface Segregation Principle (ISP)**: Split monolithic traits and heavy state references into narrow, role-focused interfaces (`BlockReader`, `BlockWriter`, `SwarmReadAccess`, `PieceWriteAccess`, `SessionStatusQuery`).
5. **D - Dependency Inversion Principle (DIP)**: Invert high-level dependencies (`TorrentSession`, `PeerWorker`) to depend strictly on injected trait abstractions (`Arc<dyn PeerDiscovery>`, `Arc<dyn BlockStorage>`, `Arc<dyn PortMapper>`) rather than concrete structs.

---

## 1. Single Responsibility Principle (SRP) Refactoring

### 1.1 Decompose Monolithic `TorrentContext`
* **File:** [`library/src/core/torrent_context.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_context.rs) (48 KB)
* **Current Violation:** `TorrentContext` mixes peer swarm tracking, piece bitfields, missing piece heaps, file descriptors, disk storage handles, upload/download speed counters, pause/resume signaling events, and bad peer scoring.
* **Refactoring Strategy:**
  - `SwarmManager`: Manages connected peers map, maximum swarm size, peer blacklisting/scoring.
  - `PieceProgressTracker`: Manages completed bitfields, missing piece bitfields, rarity heap, piece data verification.
  - `DownloadStatsCounter`: Manages thread-safe atomic counters (`bytes_downloaded`, `bytes_uploaded`, instant rates).
  - `TransferControlState`: Manages status transitions (`Initialised`, `Downloading`, `Paused`, `Seeding`, `Ended`) and manual reset signaling events.

### 1.2 Modularize `MetaInfoFile`
* **File:** [`library/src/core/metainfo.rs`](file:///c:/Projects/BitTorrent/library/src/core/metainfo.rs) (43 KB)
* **Current Violation:** Parses Bencode, calculates SHA-1/SHA-256 info-hashes, validates file names against path traversal, extracts multi-file paths (v1) and nested file trees (v2), and builds local disk layouts.
* **Refactoring Strategy:**
  - `BencodeParser`: Low-level dictionary and list extraction.
  - `InfoHashCalculator`: Pure SHA-1 / SHA-256 info hash derivation for v1 and v2 torrents.
  - `TorrentFileTree`: Tree structure representing files, sizes, and relative paths.
  - `PathSanitizer`: Validates relative paths against directory traversal (`..`, absolute paths, illegal Windows/Unix characters).

### 1.3 Split Monolithic Worker Loop (`worker.rs`)
* **File:** [`library/src/session/worker.rs`](file:///c:/Projects/BitTorrent/library/src/session/worker.rs) (27 KB)
* **Current Violation:** Combines peer handshake execution, block request scheduling, endgame cancellation broadcasting, PEX timer ticks, timeout monitoring, and GUI message dispatch.
* **Refactoring Strategy:**
  - `PeerHandshakeHandler`: Negotiates protocol extensions, MSE encryption, and Bitfield payload exchanges.
  - `BlockRequestHandler`: Calculates piece/block request queues and handles request timeout cancellations.
  - `PexExtensionHandler`: Periodically formats and handles IPv4/IPv6 PEX peer lists.
  - `EndgameHandler`: Manages duplicate block requests and broadcast cancellations when nearing completion.

---

## 2. Open/Closed Principle (OCP) Refactoring

### 2.1 Trait-Based Announcer Protocol (`Announcer`)
* **Files:** [`library/src/network/announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs), [`library/src/network/tracker.rs`](file:///c:/Projects/BitTorrent/library/src/network/tracker.rs)
* **Current Violation:** Uses `AnnouncerEnum` (`Http(HttpAnnouncer)` / `Udp(UdpAnnouncer)`) with exhaustive `match` statements across tracker client methods. Adding WebSocket or gRPC trackers requires modifying existing code.
* **Refactoring Strategy:**
  ```rust
  #[async_trait]
  pub trait Announcer: Send + Sync {
      async fn announce(&self, request: &AnnounceRequest) -> Result<AnnounceResponse, BitTorrentError>;
      async fn scrape(&self, info_hash: &[u8]) -> Result<ScrapeResponse, BitTorrentError>;
  }
  ```
  - `HttpAnnouncer` and `UdpAnnouncer` implement `Announcer`.
  - `AnnouncerFactory` parses scheme (`http://`, `https://`, `udp://`, `wss://`) and registers new protocols without modifying tracker core logic.

### 2.2 Extensible Peer Discovery Framework (`PeerDiscovery`)
* **Files:** [`library/src/session/session.rs`](file:///c:/Projects/BitTorrent/library/src/session/session.rs), [`library/src/network/dht.rs`](file:///c:/Projects/BitTorrent/library/src/network/dht.rs), [`library/src/network/lsd.rs`](file:///c:/Projects/BitTorrent/library/src/network/lsd.rs)
* **Current Violation:** `TorrentSession` explicitly instantiates, holds options for, and manages lifetime for DHT, LSD, Trackers, and WebSeeds.
* **Refactoring Strategy:**
  ```rust
  #[async_trait]
  pub trait PeerDiscovery: Send + Sync {
      async fn start_discovery(&self, info_hash: &[u8], tx: mpsc::Sender<PeerDetails>) -> Result<(), BitTorrentError>;
      async fn stop_discovery(&self) -> Result<(), BitTorrentError>;
  }
  ```
  - Implement `TrackerDiscovery`, `DhtDiscovery`, `LsdDiscovery`, `PexDiscovery`, `WebSeedDiscovery`.
  - `TorrentSession` maintains a vector `Vec<Box<dyn PeerDiscovery>>`, enabling new discovery mechanisms to be added dynamically.

---

## 3. Liskov Substitution Principle (LSP) Refactoring

### 3.1 Network Transport Invariants & Behavioral Test Suite
* **Files:** [`library/src/utils/io_traits.rs`](file:///c:/Projects/BitTorrent/library/src/utils/io_traits.rs), [`library/src/network/utp.rs`](file:///c:/Projects/BitTorrent/library/src/network/utp.rs)
* **Current Violation:** `UtpSocketAdapter`, `TcpStream`, and `MockSocket` have slight behavioral differences in timeout handling, zero-byte reads, and shutdown semantics.
* **Refactoring Strategy:**
  - Define strict invariant specifications for `AsyncSocket` (e.g. non-blocking return contracts, EOF behaviors, timeout error types).
  - Create a generic compliance test suite `test_async_socket_compliance<S: AsyncSocket>(factory: F)` run against `TcpSocket`, `UtpSocketAdapter`, and `MockSocket`.

### 3.2 Storage Provider Invariants
* **Files:** [`library/src/storage/disk_io.rs`](file:///c:/Projects/BitTorrent/library/src/storage/disk_io.rs), [`library/src/utils/io_traits.rs`](file:///c:/Projects/BitTorrent/library/src/utils/io_traits.rs)
* **Current Violation:** `DiskIO` performs synchronous file I/O operations directly, whereas `MemStorage` acts in-memory. Bypassing storage traits leads to inconsistent error codes on permission errors or missing disk space.
* **Refactoring Strategy:**
  - Ensure all storage drivers (`DiskIO`, `MemStorage`, `SparseFileStorage`) pass the unified `BlockStorage` compliance test suite.

---

## 4. Interface Segregation Principle (ISP) Refactoring

### 4.1 Role-Based Trait Views for `TorrentContext`
* **File:** [`library/src/core/torrent_context.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_context.rs)
* **Current Violation:** All components (peer loop, stats recorder, webseed manager, disk assembler) hold an `Arc<Mutex<TorrentContext>>`, granting complete read/write access to unrelated state.
* **Refactoring Strategy:**
  - Define fine-grained, segregated traits:
    ```rust
    pub trait SwarmReadAccess: Send + Sync {
        fn connected_peer_count(&self) -> usize;
        fn contains_peer(&self, peer_id: &str) -> bool;
    }

    pub trait PieceWriteAccess: Send + Sync {
        fn mark_piece_completed(&self, index: u32) -> Result<(), BitTorrentError>;
        fn is_piece_needed(&self, index: u32) -> bool;
    }

    pub trait SessionStatusQuery: Send + Sync {
        fn get_stats(&self) -> SessionStats;
        fn status(&self) -> TorrentStatus;
    }
    ```

### 4.2 Segregate Storage Interfaces
* **File:** [`library/src/utils/io_traits.rs`](file:///c:/Projects/BitTorrent/library/src/utils/io_traits.rs)
* **Current Violation:** `BlockStorage` combines reading blocks, writing blocks, creating directory trees, and verifying piece checksums.
* **Refactoring Strategy:**
  - Segregate into `BlockReader`, `BlockWriter`, and `StorageVerifier`.
  - Components requiring read-only access (e.g. upload choked peer worker) only take `&dyn BlockReader`.

---

## 5. Dependency Inversion Principle (DIP) Refactoring

### 5.1 Invert `TorrentSession` & `TorrentSessionBuilder` Dependencies
* **Files:** [`library/src/session/session.rs`](file:///c:/Projects/BitTorrent/library/src/session/session.rs), [`library/src/session/builder.rs`](file:///c:/Projects/BitTorrent/library/src/session/builder.rs)
* **Current Violation:** `TorrentSession` directly constructs concrete `DiskIO`, `TcpStream`, `Dht`, and `NatPmpClient` structs.
* **Refactoring Strategy:**
  - Inject abstractions via builder configuration:
    - `with_storage(storage: Arc<dyn BlockStorage>)`
    - `with_port_mapper(mapper: Arc<dyn PortMapper>)`
    - `with_discovery(discovery: Arc<dyn PeerDiscovery>)`
  - High-level orchestrators depend strictly on interface abstractions, making unit testing completely headless and dependency-mockable.

### 5.2 Decouple Encryption via Decorator Pattern
* **File:** [`library/src/network/peer_network.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer_network.rs)
* **Current Violation:** `PeerNetwork` inspects `#[cfg(feature = "mse")]` and directly locks internal RC4 mutexes on read/write.
* **Refactoring Strategy:**
  - Implement an `EncryptedSocket` decorator wrapping `Arc<dyn AsyncSocket>`.
  - `PeerNetwork` interacts only with `Arc<dyn AsyncSocket>`, remaining completely agnostic to whether encryption, plain TCP, uTP, or a mock transport is underlying the connection.

---

## Implementation Roadmap & Verification

| Phase | Target Components | Major Refactoring | Status |
| :--- | :--- | :--- | :--- |
| **Phase 1** | `io_traits.rs`, `peer_network.rs` | ISP & DIP: Segregate `BlockStorage` & implement `EncryptedSocket` decorator. | **Completed** (`EncryptedSocket` decorator + `Socket::Encrypted`) |
| **Phase 2** | `announcer.rs`, `tracker.rs` | OCP: Introduce `Announcer` trait, `set_announcer`, & dynamic injection. | **Completed** (`AnnouncerFactory::create_announcer` + `Tracker::set_announcer`) |
| **Phase 3** | `torrent_context.rs` | SRP & ISP: Decompose into `SwarmManager`, `PieceProgressTracker`, `DownloadStatsCounter`, `TransferControlState`. Role-based traits (`SwarmReadAccess`, `PieceWriteAccess`, `SessionStatusQuery`) implemented. | In Progress |
| **Phase 4** | `metainfo.rs`, `worker.rs` | SRP: Extract `BencodeParser`, `InfoHashCalculator`, `PeerHandshakeHandler`, `BlockRequestHandler`. | Planned |
| **Phase 5** | `session.rs`, `builder.rs` | DIP & OCP: Dependency injection for `PeerDiscovery`, `PortMapper`, `BlockStorage`. | Planned |

---
