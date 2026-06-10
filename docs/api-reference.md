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

    /// Configures timeouts, intervals, and feature flags.
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

    /// Commences tracker announcing, DHT bootstrapping, NAT-PMP mapping, and active peer connections.
    pub fn start_download(&mut self) -> Result<(), BitTorrentError>;

    /// Temporarily pauses peer wire block requests.
    pub fn pause(&mut self) -> Result<(), BitTorrentError>;

    /// Resumes block downloading/uploading.
    pub fn resume(&mut self) -> Result<(), BitTorrentError>;

    /// Halts all active network sockets, releases NAT-PMP port mappings, and stops background tasks.
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

Timeout boundaries, feature flags, and custom network socket drivers are managed via `SessionConfig`.

```rust
pub struct SessionConfig {
    /// Maximum time to wait for a TCP/UDP connection to succeed.
    pub connect_timeout: Duration,
    /// Maximum time to wait for a socket read before timing out.
    pub read_timeout: Duration,
    /// Maximum time to wait for a socket write before timing out.
    pub write_timeout: Duration,
    /// Minimum re-announce interval in seconds (tracker minimum is honoured if higher).
    pub min_reannounce_interval: u32,
    /// Pluggable socket factory (default: TcpSocketFactory).
    pub socket_factory: Arc<dyn SocketFactory>,
    /// Injected HTTP client (feature = "http-tracker").
    #[cfg(feature = "http-tracker")]
    pub http_client: Arc<dyn HttpClient>,
    /// Enable Kademlia DHT peer discovery (default: true).
    pub dht_enabled: bool,
    /// UDP port for the DHT listener (default: 6881).
    pub dht_port: u16,
    /// Enable Message Stream Encryption / Protocol Encryption handshake obfuscation (default: false).
    pub mse_enabled: bool,
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
            mse_enabled: false,
        }
    }
}
```

---

## Torrent Metadata

### `MetaInfoFile`
Parses `.torrent` file content and provides typed access to torrent properties.

```rust
impl MetaInfoFile {
    /// Creates a MetaInfoFile from raw `.torrent` bytes (no std::fs access needed).
    pub fn from_bytes(data: &[u8]) -> Self;

    /// Parses the bencoded metadata; must be called before any accessor methods.
    pub fn parse(&mut self) -> Result<(), BitTorrentError>;

    /// Returns the SHA-1 (v1) or SHA-256 (v2) info-hash bytes.
    pub fn get_info_hash(&self) -> Result<Vec<u8>, BitTorrentError>;

    /// Returns true if the `"private": 1` flag is set in the info dictionary.
    pub fn is_private(&self) -> bool;

    /// Returns true if this is a BitTorrent v2 torrent (`"meta version": 2`).
    pub fn is_v2(&self) -> bool;

    /// Returns HTTP web seed mirror URLs from the `"url-list"` key.
    pub fn get_web_seeds(&self) -> Vec<String>;

    /// Returns the announce URL (first tracker).
    pub fn get_announce(&self) -> Result<String, BitTorrentError>;

    /// Returns all tracker announce URLs (including backup tiers).
    pub fn get_announce_list(&self) -> Vec<String>;

    /// Returns piece count, piece length, and total torrent length.
    pub fn get_piece_info(&self) -> Result<(u32, u32, u64), BitTorrentError>;

    /// Returns the list of files to be downloaded with their lengths and paths.
    pub fn get_files_to_download(&self) -> Result<Vec<FileInfo>, BitTorrentError>;
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
Framer for all peer wire protocol messages.

```rust
pub enum PeerMessage<'a> {
    // Core messages (BEP 3)
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

    // Fast Extension messages (BEP 6)
    HaveAll,                                          // ID 14
    HaveNone,                                         // ID 15
    Suggest(u32),                                     // ID 13
    AllowedFast(u32),                                 // ID 17
    Reject { index: u32, begin: u32, length: u32 },  // ID 16

    // Extension Protocol (BEP 10)
    Extended { ext_id: u8, payload: &'a [u8] },      // ID 20
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

---

## Tracker Scrape (BEP 48)

```rust
pub struct ScrapeResponse {
    /// Number of peers currently seeding (have a complete copy).
    pub complete: u32,
    /// Total number of times the torrent has been fully downloaded.
    pub downloaded: u32,
    /// Number of peers currently downloading (leeching).
    pub incomplete: u32,
}

impl Tracker {
    /// Queries the tracker scrape endpoint and returns swarm statistics.
    pub fn scrape(&mut self) -> Result<ScrapeResponse, BitTorrentError>;
}
```

---

## Message Stream Encryption (`mse` module)

Provides a pure-Rust RC4 stream cipher and Diffie-Hellman key exchange. See [encryption.md](file:///c:/Projects/BitTorrent/docs/encryption.md) for the full protocol walkthrough.

```rust
pub struct Rc4 {
    /* private state */
}
impl Rc4 {
    /// Creates a new RC4 cipher initialized with the given secret key.
    pub fn new(key: &[u8]) -> Self;
    /// Encrypts (or decrypts) the buffer in place using the keystream.
    pub fn encrypt(&mut self, data: &mut [u8]);
}

pub struct DiffieHellman {
    pub public_key: u64,  // Share this with the remote peer
    /* private_key: u64 */
}
impl DiffieHellman {
    /// Generates a new random private/public key pair.
    pub fn new() -> Self;
    /// Computes the 8-byte shared secret from the remote party's public key.
    pub fn compute_shared_secret(&self, remote_public_key: u64) -> [u8; 8];
}

/// Overflow-safe 128-bit modular multiplication (Russian Peasant algorithm).
pub fn mulmod(a: u128, b: u128, m: u128) -> u128;

/// Binary modular exponentiation: returns (base^exp) % modulus.
pub fn mod_pow(base: u128, exp: u128, modulus: u128) -> u128;
```

---

## uTorrent Transport Protocol (`utp` module)

Provides uTP packet framing and a `AsyncSocket`-compatible adapter over UDP. See [utp.md](file:///c:/Projects/BitTorrent/docs/utp.md) for the full header layout and state machine.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtpPacketType {
    Data = 0, Ack = 1, Syn = 2, Reset = 3, State = 4,
}

pub struct UtpHeader {
    pub packet_type: UtpPacketType,
    pub version: u8,
    pub extension: u8,
    pub connection_id: u16,
    pub timestamp_us: u32,
    pub timestamp_difference_us: u32,
    pub wnd_size: u32,
    pub seq_nr: u16,
    pub ack_nr: u16,
}
impl UtpHeader {
    pub fn encode(&self) -> Vec<u8>;             // Serializes to 20 bytes
    pub fn decode(buf: &[u8]) -> Result<Self, BitTorrentError>;
}

pub struct UtpSocketAdapter { /* private */ }
impl AsyncSocket for UtpSocketAdapter { /* ... */ }
```

---

## NAT-PMP Port Mapping (`nat` module)

Provides automatic port forwarding via NAT-PMP. See [nat-pmp.md](file:///c:/Projects/BitTorrent/docs/nat-pmp.md) for packet formats and lifecycle details.

```rust
pub struct NatPmpClient { /* private: gateway: Ipv4Addr */ }

impl NatPmpClient {
    pub fn new(gateway: Ipv4Addr) -> Self;
    pub fn request_mapping(
        &self, is_tcp: bool, internal_port: u16,
        external_port: u16, lifetime_secs: u32,
    ) -> Result<u16, BitTorrentError>;
    pub fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError>;
    pub fn build_mapping_request(
        &self, is_tcp: bool, internal_port: u16,
        external_port: u16, lifetime_secs: u32,
    ) -> Vec<u8>;
    pub fn parse_mapping_response(buf: &[u8]) -> Result<(u16, u16, u32), BitTorrentError>;
}

/// Infers the default gateway IP from the local IP address.
pub fn get_default_gateway() -> Ipv4Addr;
```

---

## Local Service Discovery (`lsd` module)

Discovers peers on the same local network without a tracker.

```rust
pub struct LsdAnnouncer { /* private */ }
impl LsdAnnouncer {
    /// Starts periodic BT-SEARCH multicast broadcasts on 239.192.152.143:6771.
    pub fn start(info_hash: &[u8], port: u16, peer_tx: Sender<PeerDetails>);
}

pub struct LsdListener { /* private */ }
impl LsdListener {
    /// Binds a UDP multicast socket and processes incoming BT-SEARCH announcements.
    pub fn start(info_hash: &[u8], peer_tx: Sender<PeerDetails>);
}
```

---

## Hardware Abstractions

### `AsyncSocket`
Defines asynchronous non-blocking stream interactions.

```rust
pub trait AsyncSocket: Send + Sync {
    fn read<'a>(&'a self, buf: &'a mut [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>>;
    fn write<'a>(&'a self, buf: &'a [u8])
        -> Pin<Box<dyn Future<Output = Result<(), BitTorrentError>> + Send + 'a>>;
}
```

### `BlockStorage`
Defines block-level reads and writes.

```rust
pub trait BlockStorage: Send + Sync {
    fn read_block(&self, offset: u64, buf: &mut [u8]) -> Result<usize, BitTorrentError>;
    fn write_block(&self, offset: u64, buf: &[u8]) -> Result<(), BitTorrentError>;
}
```

### Test Implementations
- **`MockSocket`** — in-memory channel-based socket for port-free testing.
- **`MemStorage`** — heap-backed `BlockStorage` for filesystem-free testing.
