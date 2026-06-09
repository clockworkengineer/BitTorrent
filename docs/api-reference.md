# API Reference

This document describes the public API of the BitTorrent Rust library.

---

## Torrent Session Management

The library uses a builder pattern to instantiate and configure transfer sessions.

### `TorrentSessionBuilder`
Builder helper to configure `TorrentSession` attributes.

```rust
pub struct TorrentSessionBuilder {
    // Private configuration details
}

impl TorrentSessionBuilder {
    /// Creates a builder with mandatory torrent file and target download directory paths.
    pub fn new(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> Self;

    /// Configures the session for seeding (skips starting download checks).
    pub fn seeding(self, seeding: bool) -> Self;

    /// Injects a shared peer discovery/blacklist Manager registry.
    pub fn manager(self, manager: Arc<Manager>) -> Self;

    /// Configures timeouts and intervals.
    pub fn config(self, config: SessionConfig) -> Self;

    /// Injects a pluggable piece selection strategy.
    pub fn selector(self, selector: Arc<dyn PieceSelector>) -> Self;

    /// Builds the configured TorrentSession.
    pub fn build(self) -> Result<TorrentSession, BitTorrentError>;
}
```

### `TorrentSession`
Handles starting, pausing, resuming, and stopping downloads, and tracks overall progress.

```rust
pub struct TorrentSession {
    // Private implementation details
}

impl TorrentSession {
    /// Creates a builder instance.
    pub fn builder(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> TorrentSessionBuilder;

    /// Creates a session with default settings.
    pub fn new(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
        seeding: bool,
    ) -> Result<Self, BitTorrentError>;

    /// Commences tracker announcing, DHT bootstrapping, and active peer connections.
    pub fn start_download(&mut self) -> Result<(), BitTorrentError>;

    /// Temporarily pauses peer wire block requests.
    pub fn pause(&mut self) -> Result<(), BitTorrentError>;

    /// Resumes block downloading/uploading.
    pub fn resume(&mut self) -> Result<(), BitTorrentError>;

    /// Halts all active network sockets and stops background logging/announcements.
    pub fn stop(&mut self) -> Result<(), BitTorrentError>;

    /// Returns the current lifecycle state of the transfer.
    pub fn status(&self) -> TorrentStatus;

    /// Returns download completion percentage (0.0 to 100.0).
    pub fn progress(&self) -> f32;

    /// Validates verified file presence and exact sizes on disk.
    pub fn validate(&self) -> Result<(), BitTorrentError>;

    /// Returns a thread-safe handle to the internal TorrentContext.
    pub fn context(&self) -> Arc<Mutex<TorrentContext>>;

    /// Returns the root download directory path.
    pub fn download_path(&self) -> &Path;

    /// Spawns re-announce loop thread reporting events to the tracker.
    pub fn start_reannounce_loop(&self, tracker: Tracker) -> JoinHandle<()>;

    /// Cleanly joins and terminates all active peer worker threads.
    pub fn join_peer_workers(&mut self);
}
```

---

## Session Configuration

Timeout boundaries and custom network socket drivers are managed via `SessionConfig`.

```rust
pub struct SessionConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub write_timeout: Duration,
    pub min_reannounce_interval: u32,
    pub socket_factory: Arc<dyn SocketFactory>,
    #[cfg(feature = "http-tracker")]
    pub http_client: Arc<dyn HttpClient>,
    pub dht_enabled: bool,
    pub dht_port: u16,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            min_reannounce_interval: 60,
            socket_factory: Arc::new(TcpSocketFactory),
            #[cfg(feature = "http-tracker")]
            http_client: Arc::new(UreqHttpClient),
            dht_enabled: true,
            dht_port: 6881,
        }
    }
}
```

---

## Zero-Copy Lifetime Structures

For optimized memory efficiency under `#![no_std]`, Bencode keys and peer payloads borrow slices directly from lifetime-bound buffers.

### `BNode<'a>`
A parsed Bencode syntax node.

```rust
pub enum BNode<'a> {
    Int(i64),
    Str(&'a [u8]),
    List(Vec<BNode<'a>>),
    Dict(BTreeMap<&'a str, BNode<'a>>),
}
```

### `PeerMessage<'a>`
Framer for structured peer wire messages.

```rust
pub enum PeerMessage<'a> {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(&'a [u8]),
    Request { index: u32, begin: u32, length: u32 },
    Piece { index: u32, begin: u32, block: &'a [u8] },
    Cancel { index: u32, begin: u32, length: u32 },
    Port(u16),
}
```

---

## Pluggable Piece Selection

Downloads can leverage custom selection algorithms by implementing `PieceSelector`.

```rust
pub trait PieceSelector: Send + Sync {
    /// Returns the next piece index to request from the remote peer, or None if no pieces are ready.
    fn select_piece(&self, context: &TorrentContext, remote_peer: &Peer) -> Option<u32>;
}
```

### Provided Implementations
- **`RarestFirstSelector`**: Prioritizes downloading pieces that have the lowest availability in the connected peer swarm.
- **`SequentialSelector`**: Downloads pieces sequentially in linear order (ideal for streaming media playback).
