# Concrete SOLID Refactoring Plan for BitTorrent Workspace

> **Objective:** Systematically transform the `bittorrent-rs` library, client applications (`torrent_client`, `bt_daemon`, `bt_webui`, `sync_hub`), examples, and test suites into a clean, modular, and fully decoupled architecture strictly adhering to the **SOLID** principles (Single Responsibility, Open/Closed, Liskov Substitution, Interface Segregation, Dependency Inversion), while guaranteeing:
> 1. Complete backward compatibility with the stable public API surface in [`docs/api-stability.md`](file:///c:/Projects/BitTorrent/docs/api-stability.md).
> 2. Zero degradation of `#![no_std]` core crate compatibility.
> 3. Zero warnings on `cargo clippy --workspace --all-targets -- -D warnings`.
> 4. 100% test pass rate across unit, integration, and doc tests.

---

## 1. Executive Summary & SOLID Compliance Scorecard

A comprehensive static analysis and architectural audit across the BitTorrent codebase revealed severe coupling, bloated structs ("God Objects"), cyclic dependencies, and hardcoded dispatchers. 

### Architectural Diagnostic Scorecard

| Principle | Current Workspace Status | Primary Violation Sites | Target Architecture |
|:---|:---:|:---|:---|
| **S - Single Responsibility** | 🔴 Critical | • `TorrentContext` (80+ fields, 200+ methods, 115 KB)<br>• `Peer` (Framing, handshake, choking, rates, holepunch, and context locking in 1,330 lines)<br>• `DiskIO` (Block I/O, file preallocation, directory walking, bitfield hashing)<br>• `bt_daemon::Engine` (Session management, state persistence, HTTP range streaming, IPC dispatch) | Decompose God Objects into focused single-purpose aggregates and domain services. |
| **O - Open / Closed** | 🟡 Moderate | • `AnnouncerFactory::create_announcer` (hardcoded `if-else` on URL prefixes)<br>• Choking strategies (monolithic `spawn_choking_loop` coupling execution with policy)<br>• `PeerMessage` processing (monolithic message `match` chains) | Introduce pluggable registries (`AnnouncerRegistry`), policy traits (`ChokingPolicy`), and message middleware pipelines. |
| **L - Liskov Substitution** | 🟢 Minor | • `MemStorage` (capacity-0 allocation in magnet mode behaves inconsistently with disk)<br>• `PieceSelector` implementations (stateful binary heap mutation during query) | Formalize behavioral contracts and automated cross-implementation test suites (`contract_tests`). |
| **I - Interface Segregation** | 🔴 Critical | • `TorrentContext` passed monolithically to components needing only 1–2 fields (e.g. `latency_profiler`, `WebSeed`, `PieceSelector`)<br>• Monolithic `ChokingStrategy` forcing task channel and executor awareness | Define narrow, role-focused views (`PieceAvailabilityView`, `SwarmView`, `TransferStatsView`). |
| **D - Dependency Inversion** | 🔴 Critical | • Cyclic reference: `TorrentContext` owns `Peer`, while `Peer` retains `Arc<Mutex<TorrentContext>>`<br>• `TorrentSession` directly depends on OS threads, `std::fs`, and concrete `DiskIO`<br>• Announcers instantiating concrete networking primitives | Invert dependencies via event sinks (`PeerEventListener`), runtime task adapters, and dependency injection. |

---

## 2. Target SOLID Architecture Blueprint

```mermaid
graph TD
    subgraph "Public API Layer (Stable Surface)"
        TS[TorrentSession]
        TSB[TorrentSessionBuilder]
        SC[SessionConfig]
        TC[TorrentClient]
    end

    subgraph "Domain Views (Interface Segregation)"
        PAV[PieceAvailabilityView]
        SV[SwarmView]
        TSV[TransferStatsView]
        SSV[SuperSeedView]
    end

    subgraph "Core Aggregates (Single Responsibility)"
        TCtx[TorrentContext<br/><i>Coordinator Facade</i>]
        PStore[PieceStore<br/><i>Bitfields & Hashes</i>]
        STopo[SwarmTopology<br/><i>Peers & Scores</i>]
        TMetrics[TransferMetrics<br/><i>Rates & Speeds</i>]
        V2Store[V2MerkleStore<br/><i>BEP 52 Hash Trees</i>]
        MetaBoot[MetadataBootstrap<br/><i>BEP 9 State</i>]
        Frio[FilePriorityStore<br/><i>BEP 53 Selections</i>]
    end

    subgraph "Network & Peer Layer (Decoupled)"
        PeerConn[PeerConnection<br/><i>Framed I/O</i>]
        PeerChoke[PeerChokeState<br/><i>Choke Flags</i>]
        PeerSched[PeerBlockScheduler<br/><i>Reservations</i>]
        PEL[PeerEventListener<br/><i>Event Sink (DIP)</i>]
    end

    subgraph "Pluggable Infrastructure (Open/Closed)"
        AReg[AnnouncerRegistry]
        CPol[ChokingPolicy]
        BS[BlockStorage<br/><i>BlockReader + BlockWriter</i>]
    end

    TS --> TCtx
    TCtx --> PStore
    TCtx --> STopo
    TCtx --> TMetrics
    TCtx --> V2Store
    TCtx --> MetaBoot
    TCtx --> Frio

    TCtx -.implements.-> PAV
    TCtx -.implements.-> SV
    TCtx -.implements.-> TSV
    TCtx -.implements.-> SSV

    PeerConn --> PEL
    PEL -.notifies.-> TCtx
    CPol -.reads.-> SV
    AReg -.produces.-> Announcer
    PStore -.delegates to.-> BS
```

---

## 3. Concrete Phase-by-Phase Refactoring Roadmap

---

### Phase 1: `TorrentContext` God Object Decomposition (SRP) [COMPLETED]

#### Problem Statement
`TorrentContext` in [`library/src/core/torrent_context.rs`](file:///c:/Projects/BitTorrent/library/src/core/torrent_context.rs) is an anti-pattern God Object (2,800+ lines, ~80 fields, 200+ methods) holding state for multiple distinct domains simultaneously. Modifying any feature (e.g. BEP 52 v2 Merkle verification) touches the same struct that tracks peer connection limits, speed metrics, and file priority.

#### Target Architecture
Decompose `TorrentContext` internally into cohesive domain aggregates owned as sub-structs:

```
TorrentContext (Coordinator Facade)
├── pieces: PieceStore
├── swarm: SwarmTopology
├── metrics: TransferMetrics
├── metadata: MetadataBootstrap
├── v2_merkle: V2MerkleStore
└── priority: FilePriorityStore
```

#### Implementation Specifications

```rust
// library/src/core/piece_store.rs
pub struct PieceStore {
    pub number_of_pieces: usize,
    pub piece_length: u32,
    pub pieces_info_hash: Vec<u8>,
    pub bitfield: Vec<u8>,
    pub pieces_missing: Vec<u8>,
    pub piece_data: Vec<PieceInfo>,
    pub missing_pieces_count: usize,
    pub pieces_verifying: Mutex<std::collections::HashSet<u32>>,
    pub piece_priority_queue: Mutex<std::collections::BinaryHeap<PieceRarity>>,
}

// library/src/core/swarm_topology.rs
pub struct SwarmTopology {
    pub peers: RwLock<HashMap<String, Arc<Mutex<Peer>>>>,
    pub bad_peer_scores: Mutex<HashMap<String, usize>>,
    pub maximum_swarm_size: usize,
    pub unchoked_peers_count: AtomicUsize,
    pub pex_dropped_peers: Mutex<Vec<PeerDetails>>,
    pub pex_added_peers_count: AtomicUsize,
    pub pex_dropped_peers_count: AtomicUsize,
}

// library/src/core/transfer_metrics.rs
pub struct TransferMetrics {
    pub total_bytes_downloaded: Arc<AtomicU64>,
    pub initial_bytes_downloaded: u64,
    pub total_bytes_to_download: u64,
    pub total_bytes_uploaded: Arc<AtomicU64>,
    pub download_speed_tracker: Mutex<(std::time::Instant, u64, i64)>,
    pub upload_speed_tracker: Mutex<(std::time::Instant, u64, i64)>,
}
```

#### Affected Files to Update
- Create:
  - `library/src/core/piece_store.rs`
  - `library/src/core/swarm_topology.rs`
  - `library/src/core/transfer_metrics.rs`
  - `library/src/core/metadata_bootstrap.rs`
- Modify:
  - `library/src/core/torrent_context.rs` (embed sub-aggregates; delegate existing public methods to preserve complete compatibility with `internals`).

---

### Phase 2: Inverting Peer Coupling via Event Sinks (DIP & SRP) [COMPLETED]

#### Problem Statement
In [`library/src/network/peer.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer.rs), each `Peer` maintained `pub tc: Option<Arc<Mutex<TorrentContext>>>`. This created:
1. **Cyclic Reference**: `TorrentContext` -> `Peer` -> `TorrentContext`.
2. **Tight Coupling**: `Peer` directly locked the context to record stats, verify blocks, and update bitfields.
3. **Untestability**: Unit tests could not instantiate a `Peer` in isolation without constructing a full `TorrentContext`.

#### Completed Implementation
1. **`PeerEventListener` Trait & Implementations** (`library/src/network/peer_event.rs`):
   - Created `PeerEventListener` trait defining decoupled lifecycle hooks: `on_block_received`, `on_peer_have`, `on_peer_bitfield`, `on_peer_choked`, `on_peer_unchoked`, `on_peer_disconnected`.
   - Implemented `NullPeerEventListener` (no-op sink for headless/isolated peers), `RecordingPeerEventListener` (thread-safe event collector for testing), and `ContextEventListener` (adapter bridging peer disconnections and events to coordinator).
2. **Peer Decoupling & Independent Initialization** (`library/src/network/peer.rs`):
   - Added `pub event_listener: Option<Arc<dyn PeerEventListener>>` to `Peer`.
   - Added `pub fn init_bitfield(&mut self, number_of_pieces: usize, bitfield_len: usize)` allowing complete peer setup without `TorrentContext`.
   - Marked direct `tc` field as `#[deprecated]` while retaining non-breaking backward compatibility.
   - Dispatched all peer wire events (`Choke`, `Unchoke`, `Have`, `Bitfield`, `Piece`) and `close()` notifications to `event_listener`.
3. **Worker Integration & Verification**:
   - Wired `ContextEventListener` in `library/src/session/worker.rs`.
   - Created `library/tests/solid_phase2_tests.rs` with 4 comprehensive tests validating isolated instantiation, recording listeners, message dispatching, and context bridging.
   - 100% test pass rate across `cargo test --workspace` with 0 warnings on `cargo clippy --workspace --all-targets -- -D warnings` and `#![no_std]` core build verified.

---

### Phase 3: Interface Segregation via Role-Based Views (ISP) [COMPLETED]

#### Problem Statement
Components across the workspace previously received full `&TorrentContext` when they only required a minute fraction of its capabilities:
- `PieceSelector::select_piece` received full `&TorrentContext` just to check `is_piece_local` and `is_piece_wanted`.
- Unit tests could not exercise piece selection strategies without instantiating the entire 80-field coordinator context.

#### Completed Implementation
1. **Created Segregated Views Trait Module** (`library/src/core/views.rs`):
   - **`RemotePeerPieceView`**: Abstract view of remote peer piece possession (`is_piece_on_remote_peer(&self, piece: u32) -> bool`). Implemented for `Peer`, `[u8]`, `Vec<u8>`, references `&T`, and `MutexGuard<'a, T>`.
   - **`PieceAvailabilityView`**: Focused read-only view for piece availability, verification status, and priority (`number_of_pieces`, `piece_length`, `is_piece_local`, `is_piece_wanted`, `is_piece_verifying`, `peer_count_for_piece`, `pop_rarest_piece`). Implemented for `TorrentContext`, `MockPieceAvailabilityView`, references `&T`, and `MutexGuard<'a, T>`.
   - **`SwarmView` & `PeerSnapshot`**: Swarm snapshot view for connection tracking and choking logic (`connected_peers_count`, `unchoked_peers_count`, `for_each_peer`). Implemented for `TorrentContext`.
   - **`TransferStatsView`**: Bandwidth and transfer progress query view (`total_downloaded`, `total_uploaded`, `current_download_rate`, `current_upload_rate`, `progress_percent`). Implemented for `TorrentContext`.
   - **`MockPieceAvailabilityView`**: Isolated mock struct enabling piece selectors and algorithms to be tested with zero coordinator overhead.
2. **Refactored `PieceSelector` Hierarchy** (`library/src/core/selector.rs`):
   - Decoupled `PieceSelector` trait signature from concrete `TorrentContext` and `Peer`:
     ```rust
     pub trait PieceSelector: std::fmt::Debug + Send + Sync {
         fn select_piece(
             &self,
             availability: &dyn PieceAvailabilityView,
             peer: &dyn RemotePeerPieceView,
         ) -> Option<u32>;
     }
     ```
   - Updated `RarestFirstSelector`, `SequentialSelector`, and `StreamingPieceSelector` to depend strictly on segregated views.
   - Preserved 100% backward compatibility via Deref-coercing blanket implementations on references and `MutexGuard`.
3. **Comprehensive Test Suite & Verification**:
   - Created `library/tests/solid_phase3_tests.rs` with 6 unit/integration tests validating isolated selector operation on mock availability views with raw byte slices, swarm views, transfer stats views, and ISP consumer separation.
   - 100% test pass rate across `cargo test --workspace` with 0 warnings on `cargo clippy --workspace --all-targets -- -D warnings` and `#![no_std]` core build verified.

---

### Phase 4: Open/Closed Announcer Protocol Registry (OCP) [COMPLETED]

#### Problem Statement
In [`library/src/network/announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs), `AnnouncerFactory::create_announcer` relied on a hardcoded chain of string prefix comparisons (`http://`, `udp://`, `wss://`). Adding new announcer protocols or mock test announcers required modifying existing library code.

#### Target Architecture
Replace the static if-else factory with an extensible `AnnouncerRegistry` supporting open registration of scheme builders.

#### Completed Implementation Details
1. **Created `AnnouncerBuilder` & `AnnouncerRegistry` in [`library/src/network/announcer_registry.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer_registry.rs)**:
   - Defined `AnnouncerBuilder` trait supporting both trait-object construction (`build_announcer`) and enum-dispatch construction (`build_enum`).
   - Implemented built-in builders: `HttpAnnouncerBuilder`, `UdpAnnouncerBuilder`, and `WebTorrentAnnouncerBuilder`.
   - Implemented `AnnouncerRegistry` with prefix registration, scheme overriding (LIFO priority), unregistration, and instantiation methods (`create_announcer`, `create`).
   - Provided thread-safe global registry accessors: `global_registry()` and `register_global()`.
2. **Refactored `AnnouncerFactory` in [`library/src/network/announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs)**:
   - Delegated `AnnouncerFactory::create_announcer` and `AnnouncerFactory::create` directly to the `global_registry()`, preserving 100% backward compatibility for existing callers.
   - Added `AnnouncerFactory::register` for global protocol extension without modifying library source.
3. **Comprehensive Test Suite & Verification**:
   - Created `library/tests/solid_phase4_tests.rs` with 4 comprehensive tests validating default protocol resolution, custom scheme registration (`mock://`), overriding existing schemes (`http://`), and global registry extensions via `AnnouncerFactory::register`.
   - Verified 100% test pass rate across `cargo test --workspace` with 0 warnings on `cargo clippy --workspace --all-targets -- -D warnings` and `#![no_std]` core build verification passing.

---

### Phase 5: Storage Subsystem Separation & Invariant Enforcement (SRP & LSP) [COMPLETED]

#### Problem Statement
[`library/src/storage/disk_io.rs`](file:///c:/Projects/BitTorrent/library/src/storage/disk_io.rs) previously conflated 4 distinct responsibilities:
1. Block read/write implementation (`BlockReader`, `BlockWriter`).
2. Local filesystem directory tree creation and file preallocation (`create_local_torrent_structure`).
3. Sequential file reading for bitfield generation (`create_torrent_bitfield`).
4. Parallel verification dispatching and progress tracking.

Furthermore, placeholder storages used during magnet links bootstrap risked LSP contract violations on out-of-bounds reads/writes without explicit invariant protection.

#### Target Architecture
1. **SRP**: Decompose `DiskIO` into focused components:
   - `DiskBlockStorage`: Pure implementation of `BlockReader` and `BlockWriter` over target file slices.
   - `FileAllocationManager`: Pre-creates directory trees and pre-allocated files.
   - `BitfieldScanner`: Pure calculation scanning disk files or abstract `BlockStorage` to output verified bitfields.
2. **LSP**: Standardize `BlockStorage` behavioral contracts across all implementors (`MemStorage`, `DiskBlockStorage`, `CachedBlockStorage`, and `NullBlockStorage`), verifying uniform bounds checking, overflow arithmetic protection, and idempotent flushing.

#### Completed Implementation Details
1. **Decomposed Storage Subsystems (SRP)**:
   - **`FileAllocationManager`** in [`library/src/storage/file_allocation.rs`](file:///c:/Projects/BitTorrent/library/src/storage/file_allocation.rs): Handles filesystem directory tree creation, file sizing, and existence verification.
   - **`DiskBlockStorage`** in [`library/src/storage/disk_block_storage.rs`](file:///c:/Projects/BitTorrent/library/src/storage/disk_block_storage.rs): Pure `BlockReader` and `BlockWriter` resolving global torrent offsets to target file ranges.
   - **`BitfieldScanner`** in [`library/src/storage/bitfield_scanner.rs`](file:///c:/Projects/BitTorrent/library/src/storage/bitfield_scanner.rs): Pure calculation scanning files or abstract storage, returning [`ScanResult`](file:///c:/Projects/BitTorrent/library/src/storage/bitfield_scanner.rs#L25) with verified bitfield and piece statuses without mutating `TorrentContext`.
   - **Refactored `DiskIO`** in [`library/src/storage/disk_io.rs`](file:///c:/Projects/BitTorrent/library/src/storage/disk_io.rs): Delegates allocation to `FileAllocationManager`, bitfield verification to `BitfieldScanner`, and block read/write to `DiskBlockStorage` while maintaining 100% backward compatibility for all existing callers.
2. **Standardized Storage Behavioral Invariants (LSP)**:
   - Enhanced **`MemStorage`** in [`library/src/utils/io_traits.rs`](file:///c:/Projects/BitTorrent/library/src/utils/io_traits.rs) with safe zero-length I/O, `checked_add` arithmetic overflow protection, explicit capacity getters, and idempotent flushing.
   - Introduced **`NullBlockStorage`** in [`library/src/utils/io_traits.rs`](file:///c:/Projects/BitTorrent/library/src/utils/io_traits.rs) as an explicit placeholder for uninitialized / bootstrap sessions, replacing 0-capacity allocations in `TorrentContext::new_magnet_bootstrap`.
   - Hardened **`CachedBlockStorage`** in [`library/src/storage/disk_cache.rs`](file:///c:/Projects/BitTorrent/library/src/storage/disk_cache.rs) with arithmetic overflow validation on `read_block` and `write_block`.
3. **Comprehensive Test Suite & Verification**:
   - Created generic contract test harness in [`library/tests/common/storage_contract.rs`](file:///c:/Projects/BitTorrent/library/tests/common/storage_contract.rs) verifying zero-byte I/O safety, in-bounds roundtrip, out-of-bounds rejection, arithmetic overflow safety, and idempotent flushing.
   - Created test suite [`library/tests/solid_phase5_tests.rs`](file:///c:/Projects/BitTorrent/library/tests/solid_phase5_tests.rs) running the contract across `MemStorage`, `DiskBlockStorage`, `CachedBlockStorage`, and `NullBlockStorage`, along with isolated unit tests for `FileAllocationManager` and `BitfieldScanner`.
   - 100% test pass rate across `cargo test --workspace` with 0 warnings on `cargo clippy --workspace --all-targets -- -D warnings` and `#![no_std]` core build verification passing.

---

### Phase 6: Policy-Based Choking Engine (OCP & SRP)

#### Problem Statement
In [`library/src/session/config.rs`](file:///c:/Projects/BitTorrent/library/src/session/config.rs) (lines 105–114), `ChokingStrategy` requires:
```rust
pub trait ChokingStrategy: Send + Sync {
    fn spawn_choking_loop(
        &self,
        context: Arc<Mutex<TorrentContext>>,
        task_tx: Sender<Pin<Box<dyn Future...>>>,
        manager: Option<Arc<Manager>>,
    );
}
```
This forces any strategy implementation to handle thread channels, asynchronous timers, and context locking, making it impossible to cleanly write pure, testable choking algorithms (e.g. Tit-for-Tat, BitTyrant, Round-Robin).

#### Target Architecture
Separate the **choking algorithm policy** from the **loop execution runtime**:
- `ChokingPolicy`: Pure algorithmic evaluation (`fn evaluate(&self, swarm: &dyn SwarmView) -> ChokingDecisions`).
- `ChokingLoopRunner`: Handles asynchronous timers, task queues, and sends message actions.

#### Implementation Specifications

```rust
// library/src/session/choking_policy.rs

pub struct ChokingDecision {
    pub unchoked_peers: Vec<String>,
    pub optimistically_unchoked_peer: Option<String>,
    pub choked_peers: Vec<String>,
}

pub trait ChokingPolicy: Send + Sync + std::fmt::Debug {
    /// Evaluates connected peers and determines who should be choked/unchoked.
    fn evaluate(
        &self,
        swarm: &dyn SwarmView,
        max_upload_slots: usize,
    ) -> ChokingDecision;
}

#[derive(Debug, Default)]
pub struct StandardTitForTatPolicy;

impl ChokingPolicy for StandardTitForTatPolicy {
    fn evaluate(&self, swarm: &dyn SwarmView, max_upload_slots: usize) -> ChokingDecision {
        // Pure ranking based on download rate and reciprocal unchoking
    }
}
```

#### Affected Files to Update
- Create `library/src/session/choking_policy.rs`.
- Update `library/src/session/config.rs` (embed `Arc<dyn ChokingPolicy>`).
- Update `library/src/session/worker.rs` (generic runner invoking policy).

---

### Phase 7: Client Architecture Hardening (`bt_daemon` & `sync_hub`) (SRP & DIP)

#### Problem Statement
In [`clients/bt_daemon/src/bin/bt_daemon.rs`](file:///c:/Projects/BitTorrent/clients/bt_daemon/src/bin/bt_daemon.rs), struct `Engine` is 870 lines long and handles:
1. In-memory `TorrentSession` collection management.
2. State file JSON serialization & fastresume disk persistence.
3. HTTP range media streaming server and socket parsing.
4. IPC command routing and response mapping.

In `sync_hub/src/manager.rs`, `SyncManager` directly coordinates directory watchers, file generation, and session lifecycle.

#### Target Architecture
Decompose `bt_daemon` into single-responsibility services:
- `DaemonStateRepository`: Handles loading and persisting active torrents to JSON.
- `TorrentSessionPool`: Thread-safe container managing active `TorrentSession` handles.
- `StreamingServer`: HTTP range streaming server delegating to `torrent_client_shared`.
- `IpcCommandHandler`: Pure command handler processing `IpcMessage` against the pool.

#### Implementation Specifications

```rust
// clients/bt_daemon/src/repository.rs
pub struct DaemonStateRepository {
    state_file: PathBuf,
}

impl DaemonStateRepository {
    pub fn load(&self) -> Result<DaemonState, std::io::Error> { ... }
    pub fn save(&self, state: &DaemonState) -> Result<(), std::io::Error> { ... }
}

// clients/bt_daemon/src/session_pool.rs
pub struct TorrentSessionPool {
    sessions: Vec<SessionState>,
}

impl TorrentSessionPool {
    pub fn add(&mut self, session: TorrentSession) { ... }
    pub fn find_by_hash(&self, hash: &str) -> Option<&TorrentSession> { ... }
    pub fn remove(&mut self, hash: &str) -> Option<TorrentSession> { ... }
}
```

#### Affected Files to Update
- Refactor `clients/bt_daemon/src/bin/bt_daemon.rs` into modular sub-files:
  - `clients/bt_daemon/src/repository.rs`
  - `clients/bt_daemon/src/session_pool.rs`
  - `clients/bt_daemon/src/streaming.rs`

---

## 4. Implementation Order & Verification Quality Gates

| Phase | Focus Principle | Target Deliverable | Risk | Verification Command |
|:---:|:---:|:---|:---:|:---|
| **Phase 1** | **SRP** | Decompose `TorrentContext` into `PieceStore`, `SwarmTopology`, `TransferMetrics` | Medium | `cargo test -p bittorrent-rs -- context_tests` |
| **Phase 2** | **DIP / SRP** | Replace `Peer.tc` cyclic reference with `PeerEventListener` | Medium | `cargo test -p bittorrent-rs -- peer_tests mock_peer_tests` |
| **Phase 3** | **ISP** | Introduce `PieceAvailabilityView`, `SwarmView`, update `PieceSelector` | Low | `cargo test -p bittorrent-rs -- selection_tests` |
| **Phase 4** | **OCP** | Open `AnnouncerRegistry` supporting runtime protocol additions | Low | `cargo test -p bittorrent-rs -- tracker_tests` |
| **Phase 5** | **SRP / LSP** | Extract `FileAllocationManager` & enforce storage LSP contract tests | Low | `cargo test -p bittorrent-rs -- disk_io_tests` |
| **Phase 6** | **OCP / SRP** | Separate `ChokingPolicy` algorithmic decisions from execution runner | Low | `cargo test -p bittorrent-rs -- session_tests` |
| **Phase 7** | **SRP / DIP** | Modularize `bt_daemon::Engine` into repository, pool, and streaming | Low | `cargo check -p bt_daemon && cargo test --workspace` |

---

## 5. Non-Negotiable Architectural Constraints

1. **Strict Preservation of Public API Stability**:
   All public types and methods marked stable in [`docs/api-stability.md`](file:///c:/Projects/BitTorrent/docs/api-stability.md) (`TorrentSession`, `TorrentSessionBuilder`, `SessionConfig`, `TorrentClient`, `MagnetLink`, `MetaInfoFile`, `PeerMessage`) must preserve their exact signatures and behavioral contracts.
2. **`#![no_std]` Core Compatibility**:
   Any new traits or types introduced in `library/src/core/` and `library/src/utils/` must remain compatible with `#![no_std]` using `alloc::vec::Vec`, `alloc::string::String`, and `core::sync`.
3. **Clippy Cleanliness**:
   Every refactored component must compile with zero warnings under `cargo clippy --workspace --all-targets -- -D warnings`.
4. **Zero Regressions**:
   All existing unit, integration, and doc tests across the repository must pass without modification to test assertions.
