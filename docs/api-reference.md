# API Reference

This document describes the public Rust API of the `bittorrent-rs` library.

---

## 1. Torrent Session Management

Transfers are orchestrated through `TorrentSession`, constructed via `TorrentSessionBuilder`.

### 1.1 `TorrentSessionBuilder`

Builder pattern for configuring and instantiating `TorrentSession`.

```rust
pub struct TorrentSessionBuilder { /* private fields */ }

impl TorrentSessionBuilder {
    /// Creates a builder with mandatory torrent file and download directory paths.
    pub fn new(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> Self;

    /// Initializes a session from a Magnet URI string.
    pub fn from_magnet(magnet_uri: &str, download_path: impl AsRef<Path>) -> Result<Self, BitTorrentError>;

    /// Configures the session for seeding (skips starting download checks).
    pub fn seeding(self, seeding: bool) -> Self;

    /// Injects a shared peer discovery/blacklist Manager registry.
    pub fn manager(self, manager: Arc<Manager>) -> Self;

    /// Injects a custom SessionConfig.
    pub fn config(self, config: SessionConfig) -> Self;

    /// Injects a pluggable piece selection strategy.
    pub fn selector(self, selector: Arc<dyn PieceSelector>) -> Self;

    /// Attaches verified FastResumeData to restore session state instantly.
    pub fn fast_resume(self, data: FastResumeData) -> Self;

    /// Loads fast-resume data from a .fastresume file on disk.
    pub fn fast_resume_file(self, path: impl AsRef<Path>) -> Self;

    /// Enables or disables Super-Seeding mode (BEP 16).
    pub fn super_seeding(self, enable: bool) -> Self;

    /// Configures IPv6 listening address and enables dual-stack mode.
    pub fn ipv6(self, addr: impl Into<String>) -> Self;
    pub fn enable_ipv6(self, enable: bool) -> Self;

    /// Enables or disables Peer Exchange (PEX — BEP 11).
    pub fn enable_pex(self, enable: bool) -> Self;

    /// Sets the maximum capacity (in MB) for the in-memory write cache.
    pub fn disk_cache_size_mb(self, mb: usize) -> Self;

    /// Enables or disables the BEP 55 Holepunch extension.
    pub fn enable_holepunch(self, enable: bool) -> Self;

    /// Configures WebTorrent WebRTC bridge support.
    pub fn enable_webtorrent(self, enable: bool) -> Self;
    pub fn webtorrent_bridge_mode(self, bridge: bool) -> Self;
    pub fn webtorrent_max_web_peers(self, max: usize) -> Self;

    /// Configures BitTorrent v2 piece layers persistent disk caching.
    pub fn enable_piece_layers_cache(self, enable: bool) -> Self;
    pub fn piece_layers_cache_dir(self, dir: impl AsRef<Path>) -> Self;
    pub fn piece_layers_cache_max_mb(self, mb: usize) -> Self;

    /// Configures the multi-threaded asynchronous hash verification pool.
    pub fn enable_async_verification(self, enable: bool) -> Self;
    pub fn verification_workers(self, workers: usize) -> Self;
    pub fn verification_queue_capacity(self, capacity: usize) -> Self;

    /// Configures adaptive endgame mode with erasure block staging.
    pub fn enable_adaptive_endgame(self, enable: bool) -> Self;
    pub fn endgame_straggler_ratio(self, ratio: f64) -> Self;
    pub fn endgame_max_clones_per_block(self, max: usize) -> Self;

    /// Configures WebSeed mirror latency tracking and circuit breaking.
    pub fn enable_webseed_health_tracker(self, enable: bool) -> Self;
    pub fn webseed_failure_threshold(self, threshold: usize) -> Self;
    pub fn webseed_cooldowns(self, base_secs: u64, max_secs: u64) -> Self;
    pub fn webseed_timeout(self, timeout_secs: u64) -> Self;

    /// Attaches additional WebSeed mirror URLs.
    pub fn with_web_seed(self, url: impl Into<String>) -> Self;
    pub fn with_web_seeds(self, urls: Vec<String>) -> Self;

    /// Builds the configured TorrentSession.
    pub fn build(self) -> Result<TorrentSession, BitTorrentError>;
}
```

### 1.2 `TorrentSession`

The runtime orchestrator for an active torrent transfer.

```rust
pub struct TorrentSession { /* private fields */ }

impl TorrentSession {
    /// Creates a builder instance.
    pub fn builder(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> TorrentSessionBuilder;

    /// Starts tracker announcing, DHT discovery, port mapping, and peer workers.
    pub fn start_download(&mut self) -> Result<(), BitTorrentError>;

    /// Temporarily pauses peer wire block requests.
    pub fn pause(&mut self) -> Result<(), BitTorrentError>;

    /// Resumes block downloading/uploading.
    pub fn resume(&mut self) -> Result<(), BitTorrentError>;

    /// Safely halts all active sockets, flushes disk cache, and releases port mappings.
    pub fn stop(&mut self) -> Result<(), BitTorrentError>;

    /// Returns the current lifecycle state (Initialised, Downloading, Paused, Seeding, Ended).
    pub fn status(&self) -> TorrentStatus;

    /// Returns download completion percentage (0.0 to 100.0).
    pub fn progress(&self) -> f32;

    /// Returns a thread-safe handle to the internal TorrentContext.
    pub fn context(&self) -> Arc<Mutex<TorrentContext>>;

    /// Returns the root download directory path.
    pub fn download_path(&self) -> &Path;

    // --- Media Streaming ---

    /// Creates a seekable stream across the entire torrent (Read + Seek).
    pub fn create_stream(&self) -> Result<TorrentStream, BitTorrentError>;

    /// Creates a seekable stream for a specific file index (Read + Seek).
    pub fn create_file_stream(&self, file_index: usize) -> Result<TorrentStream, BitTorrentError>;

    // --- File Priorities & Selective Download ---

    /// Dynamically sets the download priority for a specific file index.
    pub fn set_file_priority(&mut self, file_index: usize, priority: FilePriority) -> Result<(), BitTorrentError>;

    /// Returns the list of currently selected file indices (BEP 53).
    pub fn selected_files(&self) -> Vec<usize>;

    /// Sets selective file indices and evaluates bandwidth/piece savings.
    pub fn select_files(&mut self, file_indices: &[usize]) -> Result<(), BitTorrentError>;
    pub fn bep53_evaluation(&self) -> Option<BEP53SelectionEvaluation>;

    // --- Super-Seeding (BEP 16) ---

    /// Checks if the session is currently in super-seeding mode.
    pub fn is_super_seeding(&self) -> bool;

    /// Toggles super-seeding mode on or off dynamically.
    pub fn set_super_seeding(&mut self, enabled: bool);

    // --- Fast Resume Persistence (BEP 47) ---

    /// Generates FastResumeData capturing current download state.
    pub fn generate_fast_resume(&self) -> Result<FastResumeData, BitTorrentError>;

    /// Persists .fastresume data to disk.
    pub fn save_fast_resume(&self, path: impl AsRef<Path>) -> Result<(), BitTorrentError>;

    // --- BitTorrent v2 & Torrent Export ---

    /// Exports the current session state into v1, pure v2, or hybrid .torrent bytes.
    pub fn export_torrent(&self, format: TorrentExportFormat) -> Result<(Vec<u8>, TorrentUpgradeReport), BitTorrentError>;

    // --- Verification & Telemetry ---

    /// Triggers parallel re-verification of all downloaded pieces.
    pub fn recheck_pieces(&self) -> Result<(), BitTorrentError>;

    /// Returns metrics from the background hash verification worker pool.
    pub fn verification_metrics(&self) -> Option<VerificationPoolMetrics>;

    /// Returns adaptive endgame staging metrics.
    pub fn endgame_metrics(&self) -> Option<EndgameStagingMetrics>;

    /// Returns telemetry summaries of all configured WebSeed mirror endpoints.
    pub fn webseed_mirror_summary(&self) -> Vec<MirrorHealthSummary>;

    /// Dynamically adds or removes a WebSeed mirror URL at runtime.
    pub fn add_webseed(&self, url: &str);
    pub fn remove_webseed(&self, url: &str);

    /// Flushes pending dirty blocks in the in-memory write cache to disk.
    pub fn flush_disk_cache(&self) -> Result<(), BitTorrentError>;
}
```

---

## 2. Session Configuration (`SessionConfig`)

Fine-grained network, protocol, and buffer configuration options:

```rust
pub struct SessionConfig {
    // Network Timeouts
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub min_reannounce_interval: u32,
    pub listen_port: u16,

    // Sockets & HTTP
    pub socket_factory: Arc<dyn SocketFactory>,
    #[cfg(feature = "http-tracker")]
    pub http_client: Arc<dyn HttpClient>,

    // Protocol Flags
    pub dht_enabled: bool,
    pub dht_port: u16,
    pub mse_enabled: bool,
    pub enable_pex: bool,
    pub enable_holepunch: bool,

    // Dual-Stack IPv6
    pub enable_ipv6: bool,
    pub ipv6: Option<String>,

    // WebTorrent WebRTC Bridge
    pub enable_webtorrent: bool,
    pub webtorrent_bridge_mode: bool,
    pub webtorrent_max_web_peers: usize,

    // Storage & Caching
    pub disk_cache_size_mb: usize,
    pub v2_piece_layers_cache_enabled: bool,
    pub v2_piece_layers_cache_dir: Option<PathBuf>,
    pub v2_piece_layers_cache_max_mb: usize,

    // Async Hash Verification Pool
    pub enable_async_verification: bool,
    pub verification_workers: usize,
    pub verification_queue_capacity: usize,

    // Adaptive Endgame & WebSeeds
    pub enable_adaptive_endgame: bool,
    pub endgame_straggler_ratio: f64,
    pub endgame_max_clones_per_block: usize,
    pub enable_webseed_health_tracker: bool,
    pub webseed_failure_threshold: usize,
    pub webseed_base_cooldown_secs: u64,
    pub webseed_max_cooldown_secs: u64,
    pub webseed_timeout_secs: u64,

    // Super-Seeding
    pub super_seeding: bool,
}
```

---

## 3. Metadata & Torrent Construction

### 3.1 `MetaInfoFile`

Parses `.torrent` metainfo buffers and extracts v1/v2 metadata:

```rust
impl MetaInfoFile {
    pub fn from_bytes(data: &[u8]) -> Self;
    pub fn parse(&mut self) -> Result<(), BitTorrentError>;

    pub fn get_info_hash(&self) -> Result<Vec<u8>, BitTorrentError>;
    pub fn get_v1_info_hash(&self) -> Option<[u8; 20]>;
    pub fn get_v2_info_hash(&self) -> Option<[u8; 32]>;

    pub fn is_v1(&self) -> bool;
    pub fn is_v2(&self) -> bool;
    pub fn is_hybrid(&self) -> bool;
    pub fn is_private(&self) -> bool;

    pub fn get_announce(&self) -> Result<String, BitTorrentError>;
    pub fn get_announce_list(&self) -> Vec<String>;
    pub fn get_web_seeds(&self) -> Vec<String>;
    pub fn get_files_to_download(&self) -> Result<Vec<FileInfo>, BitTorrentError>;
}
```

### 3.2 `TorrentBuilder`

Constructs new BitTorrent v1, v2, and Hybrid `.torrent` files:

```rust
pub struct TorrentBuilder { /* private fields */ }

impl TorrentBuilder {
    pub fn new(source_path: impl AsRef<Path>) -> Self;
    pub fn piece_length(self, length: u32) -> Self;
    pub fn comment(self, comment: impl Into<String>) -> Self;
    pub fn created_by(self, creator: impl Into<String>) -> Self;
    pub fn is_private(self, private: bool) -> Self;
    pub fn add_tracker(self, announce_url: impl Into<String>) -> Self;
    pub fn add_web_seed(self, url: impl Into<String>) -> Self;

    /// Builds a legacy BitTorrent v1 .torrent file.
    pub fn build_v1(&self) -> Result<Vec<u8>, BitTorrentError>;

    /// Builds a pure BitTorrent v2 .torrent file with Merkle trees.
    pub fn build_v2(&self) -> Result<Vec<u8>, BitTorrentError>;

    /// Builds a dual-compatible Hybrid (v1 + v2) .torrent file with pad files.
    pub fn build_hybrid(&self) -> Result<Vec<u8>, BitTorrentError>;
}
```

### 3.3 `TorrentUpgrader`

Converts legacy v1 `.torrent` files into v2 or Hybrid formats:

```rust
pub enum TorrentExportFormat { V1, V2, Hybrid }

pub struct TorrentUpgrader;

impl TorrentUpgrader {
    /// Upgrades a parsed MetaInfoFile and raw file buffers.
    pub fn upgrade_metainfo(
        meta: &MetaInfoFile,
        files_data: &HashMap<PathBuf, Vec<u8>>,
        format: TorrentExportFormat,
    ) -> Result<(Vec<u8>, TorrentUpgradeReport), BitTorrentError>;

    /// Upgrades a .torrent file by hashing content files directly on disk.
    pub fn upgrade_from_file(
        torrent_path: impl AsRef<Path>,
        data_dir: impl AsRef<Path>,
        format: TorrentExportFormat,
    ) -> Result<(Vec<u8>, TorrentUpgradeReport), BitTorrentError>;

    /// Generates a canonical Magnet URI featuring dual v1/v2 info-hashes.
    pub fn generate_magnet_uri(report: &TorrentUpgradeReport, trackers: &[String]) -> String;
}
```

---

## 4. Seekable Streaming (`TorrentStream`)

Implements standard Rust I/O traits over an active torrent transfer:

```rust
pub struct TorrentStream { /* private fields */ }

impl std::io::Read for TorrentStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}

impl std::io::Seek for TorrentStream {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64>;
}

impl TorrentStream {
    /// Configures blocking timeout when awaiting missing piece blocks.
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self;

    /// Configures polling sleep interval while waiting for piece arrival.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self;
}
```

---

## 5. Merkle Tree & Audit Proofs (`MerkleTree`)

Cryptographic SHA-256 Merkle tree calculation conforming to BEP 52:

```rust
pub struct MerkleProof {
    pub leaf_index: usize,
    pub sibling_hashes: Vec<[u8; 32]>,
}

pub struct MerkleTree { /* private fields */ }

impl MerkleTree {
    pub const BLOCK_SIZE: usize = 16384; // 16 KiB

    /// Computes Merkle tree from in-memory bytes.
    pub fn from_bytes(data: &[u8]) -> Self;

    /// Computes Merkle tree incrementally from any Read stream.
    pub fn from_reader<R: std::io::Read>(reader: R) -> Result<Self, std::io::Error>;

    /// Computes Merkle tree directly from a file on disk.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, std::io::Error>;

    /// Returns the 32-byte root hash (pieces_root).
    pub fn root(&self) -> [u8; 32];

    /// Generates a minimal logarithmic audit proof path for a 16 KiB block.
    pub fn proof_for_block(&self, block_index: usize) -> Option<MerkleProof>;

    /// Verifies a 16 KiB block hash against root using an audit proof path.
    pub fn verify_block_hash(
        root: &[u8; 32],
        block_hash: &[u8; 32],
        proof: &MerkleProof,
    ) -> bool;
}
```

---

## 6. Magnet URIs (`MagnetLink` & `MagnetUriBuilder`)

Canonical parser and builder supporting dual info-hashes and BEP 53 file selection:

```rust
pub struct MagnetLink {
    pub info_hash: Vec<u8>,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
    pub web_seeds: Vec<String>,
    pub v2_info_hash: Option<[u8; 32]>,
    pub selected_files: Vec<usize>,
}

impl MagnetLink {
    pub fn from_uri(uri: &str) -> Result<Self, BitTorrentError>;
    pub fn to_uri(&self) -> String;
    pub fn is_hybrid(&self) -> bool;
    pub fn is_v2(&self) -> bool;
    pub fn is_webtorrent(&self) -> bool;
}

pub struct MagnetUriBuilder { /* private fields */ }

impl MagnetUriBuilder {
    pub fn new(info_hash: &[u8]) -> Self;
    pub fn display_name(self, name: impl Into<String>) -> Self;
    pub fn add_tracker(self, tracker: impl Into<String>) -> Self;
    pub fn add_web_seed(self, seed: impl Into<String>) -> Self;
    pub fn v2_info_hash(self, hash: [u8; 32]) -> Self;
    pub fn select_file(self, index: usize) -> Self;
    pub fn with_webtorrent_defaults(self) -> Self;
    pub fn build(self) -> String;
}
```
