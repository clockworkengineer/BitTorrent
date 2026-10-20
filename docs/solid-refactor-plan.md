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

### Phase 2: Inverting Peer Coupling via Event Sinks (DIP & SRP)

#### Problem Statement
In [`library/src/network/peer.rs`](file:///c:/Projects/BitTorrent/library/src/network/peer.rs), each `Peer` maintains `pub tc: Option<Arc<Mutex<TorrentContext>>>`. This creates:
1. **Cyclic Reference**: `TorrentContext` -> `Peer` -> `TorrentContext`.
2. **Tight Coupling**: `Peer` directly locks the 80-field context to record stats, verify blocks, and update bitfields.
3. **Untestability**: Unit tests cannot instantiate a `Peer` in isolation without constructing a full `TorrentContext`.

#### Target Architecture
Replace `pub tc: Option<Arc<Mutex<TorrentContext>>>` with an abstract event listener trait `PeerEventListener`.

#### Implementation Specifications

```rust
// library/src/network/peer_event.rs

/// Decoupled event notification sink emitted by a peer connection.
pub trait PeerEventListener: Send + Sync {
    /// Notifies that a remote peer has completed a block upload.
    fn on_block_received(&self, peer_ip: &str, piece_index: u32, begin: u32, block: &[u8]);
    
    /// Notifies that a remote peer announced possession of a piece.
    fn on_peer_have(&self, peer_ip: &str, piece_index: u32);
    
    /// Notifies that a remote peer updated its bitfield.
    fn on_peer_bitfield(&self, peer_ip: &str, bitfield: &[u8]);
    
    /// Notifies that a peer disconnected or timed out.
    fn on_peer_disconnected(&self, peer_ip: &str);
}
```

```rust
// library/src/network/peer.rs

pub struct Peer {
    pub network: Option<PeerNetwork>,
    pub ip: String,
    pub port: u16,
    pub choke_state: PeerChokeState,
    pub scheduler: PeerBlockScheduler,
    pub metrics: PeerRateEstimator,
    // INVERTED DEPENDENCY: Peer communicates through abstract sink, not TorrentContext!
    pub event_listener: Option<Arc<dyn PeerEventListener>>,
}
```

#### Affected Files to Update
- Create `library/src/network/peer_event.rs`.
- Update `library/src/network/peer.rs` (remove `tc`, add `event_listener`).
- Update `library/src/session/worker.rs` (forward peer events to context through listener adapter).

---

### Phase 3: Interface Segregation via Role-Based Views (ISP)

#### Problem Statement
Components across the workspace receive `&TorrentContext` when they only require a minute fraction of its capabilities:
- `PieceSelector::select_piece` receives full `&TorrentContext` just to check `is_piece_local` and `is_piece_wanted`.
- `WebSeedTracker` receives full `&TorrentContext` just to check piece lengths and missing status.
- `ChokingStrategy` receives full `Arc<Mutex<TorrentContext>>` just to read peer rates and interest flags.

#### Target Architecture
Introduce lean, role-focused trait views in `library/src/core/views.rs`. `TorrentContext` implements these views. Components accept the minimal trait interface.

#### Implementation Specifications

```rust
// library/src/core/views.rs

/// Read-only view for piece availability and priority requirements.
pub trait PieceAvailabilityView: Send + Sync {
    fn number_of_pieces(&self) -> usize;
    fn piece_length(&self, piece: u32) -> u32;
    fn is_piece_local(&self, piece: u32) -> bool;
    fn is_piece_wanted(&self, piece: u32) -> bool;
    fn is_piece_verifying(&self, piece: u32) -> bool;
    fn peer_count_for_piece(&self, piece: u32) -> usize;
}

/// Read-only snapshot view of the swarm for choking and unchoking decisions.
pub trait SwarmView: Send + Sync {
    fn connected_peers_count(&self) -> usize;
    fn unchoked_peers_count(&self) -> usize;
    fn for_each_peer(&self, f: &mut dyn FnMut(&str, &PeerSnapshot));
}

/// Read-only snapshot view of session bandwidth statistics.
pub trait TransferStatsView: Send + Sync {
    fn total_downloaded(&self) -> u64;
    fn total_uploaded(&self) -> u64;
    fn current_download_rate(&self) -> u64;
    fn current_upload_rate(&self) -> u64;
}

// Implement traits on TorrentContext
impl PieceAvailabilityView for TorrentContext { ... }
impl SwarmView for TorrentContext { ... }
impl TransferStatsView for TorrentContext { ... }
```

Update `PieceSelector`:
```rust
// library/src/core/selector.rs
pub trait PieceSelector: std::fmt::Debug + Send + Sync {
    /// Selects the next piece index using segregated availability views.
    fn select_piece(
        &self,
        availability: &dyn PieceAvailabilityView,
        remote_bitfield: &[u8],
    ) -> Option<u32>;
}
```

#### Affected Files to Update
- Create `library/src/core/views.rs`.
- Update `library/src/core/selector.rs` to accept `&dyn PieceAvailabilityView`.
- Update `library/src/core/torrent_context.rs` to implement view traits.

---

### Phase 4: Open/Closed Announcer Protocol Registry (OCP)

#### Problem Statement
In [`library/src/network/announcer.rs`](file:///c:/Projects/BitTorrent/library/src/network/announcer.rs) (lines 607–637), `AnnouncerFactory::create_announcer` relies on a hardcoded chain of string prefix comparisons (`http://`, `udp://`, `wss://`). Adding new announcer protocols or mock test announcers requires modifying existing library code.

#### Target Architecture
Replace the static if-else factory with an extensible `AnnouncerRegistry` supporting open registration of scheme builders.

#### Implementation Specifications

```rust
// library/src/network/announcer_registry.rs

pub trait AnnouncerBuilder: Send + Sync {
    fn build(&self, url: &str) -> Result<Box<dyn Announcer>, BitTorrentError>;
}

pub struct AnnouncerRegistry {
    builders: Vec<(String, Box<dyn AnnouncerBuilder>)>,
}

impl AnnouncerRegistry {
    pub fn new_with_defaults() -> Self {
        let mut registry = Self { builders: Vec::new() };
        #[cfg(feature = "http-tracker")]
        {
            registry.register("http://", HttpAnnouncerBuilder);
            registry.register("https://", HttpAnnouncerBuilder);
        }
        registry.register("udp://", UdpAnnouncerBuilder);
        #[cfg(feature = "webrtc")]
        {
            registry.register("ws://", WebTorrentAnnouncerBuilder);
            registry.register("wss://", WebTorrentAnnouncerBuilder);
        }
        registry
    }

    pub fn register(&mut self, prefix: &str, builder: impl AnnouncerBuilder + 'static) {
        self.builders.push((prefix.to_string(), Box::new(builder)));
    }

    pub fn create(&self, url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        for (prefix, builder) in &self.builders {
            if url.starts_with(prefix) {
                return builder.build(url);
            }
        }
        Err(BitTorrentError::Parse(format!("Unsupported tracker scheme: {}", url)))
    }
}
```

#### Affected Files to Update
- Create `library/src/network/announcer_registry.rs`.
- Update `library/src/network/announcer.rs` to delegate `create_announcer` to default registry.

---

### Phase 5: Storage Subsystem Separation & Invariant Enforcement (SRP & LSP)

#### Problem Statement
[`library/src/storage/disk_io.rs`](file:///c:/Projects/BitTorrent/library/src/storage/disk_io.rs) conflates 4 distinct responsibilities:
1. Block read/write implementation (`BlockReader`, `BlockWriter`).
2. Local filesystem directory tree creation and file preallocation (`create_local_torrent_structure`).
3. Sequential file reading for bitfield generation (`create_torrent_bitfield`).
4. Direct mutation of `TorrentContext` fields during parallel verification (`create_torrent_bitfield_parallel`).

Furthermore, `MemStorage` with 0 capacity violates LSP when used as a drop-in `BlockStorage` placeholder in magnet links bootstrap.

#### Target Architecture
1. **SRP**: Decompose `DiskIO` into:
   - `DiskBlockStorage`: Pure implementation of `BlockReader` and `BlockWriter`.
   - `FileAllocationManager`: Pre-creates directories and sparse/preallocated files.
   - `DiskBitfieldScanner`: Scans disk files and outputs a bitfield without mutating context.
2. **LSP**: Standardize `BlockStorage` behavioral contracts and create `library/tests/common/storage_contract.rs` ensuring consistent bounds checking and idempotent flushing.

#### Implementation Specifications

```rust
// library/src/storage/file_allocation.rs
pub struct FileAllocationManager;

impl FileAllocationManager {
    pub fn allocate_torrent_structure(
        download_path: &Path,
        files: &[FileDetails],
    ) -> Result<(), BitTorrentError> {
        for file in files {
            let path = Path::new(&file.name);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            if !path.exists() {
                let handle = std::fs::File::create(path)?;
                handle.set_len(file.length)?;
            }
        }
        std::fs::create_dir_all(download_path)?;
        Ok(())
    }
}
```

```rust
// library/src/storage/bitfield_scanner.rs
pub struct BitfieldScanner;

impl BitfieldScanner {
    /// Scans disk files and returns the detected bitfield without mutating the context directly.
    pub fn scan_local_files(
        files: &[FileDetails],
        piece_length: u32,
        expected_hashes: &[u8],
    ) -> Result<Vec<u8>, BitTorrentError> {
        // Pure calculation returning bitfield vector
    }
}
```

#### Affected Files to Update
- Create:
  - `library/src/storage/file_allocation.rs`
  - `library/src/storage/bitfield_scanner.rs`
- Modify:
  - `library/src/storage/disk_io.rs` (extract helper responsibilities; delegate).
  - `library/src/utils/io_traits.rs` (ensure `MemStorage` honors bounds-checking contract cleanly).

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
