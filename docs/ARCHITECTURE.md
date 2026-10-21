# Architecture Overview

This document describes the design, key components, data flows, and multi-process architecture of the BitTorrent client library and application ecosystem.

---

## 1. Crate Layout & Workspace

The workspace is organized into a modular hierarchy:

```
BitTorrent/
├── library/                       # Core BitTorrent protocol library (optionally #![no_std])
│   ├── src/
│   │   ├── core/                  # Metadata, context, selectors, Merkle tree, builder & upgrader
│   │   ├── network/               # Peer wire, uTP, WebRTC, trackers, DHT, LSD, PEX, MSE, NAT
│   │   ├── session/               # Session orchestrator, worker loops, choking policies, WebSeed
│   │   ├── storage/               # Disk I/O, cache write coalescing, fast-resume, streams, verify pool
│   │   └── utils/                 # Bencode, I/O traits, error types, and static buffer pools
│   └── tests/                     # Comprehensive integration test suite (60+ test suites)
├── clients/                       # Applications built on top of the library
│   ├── torrent_client/            # Desktop GUI application (egui/eframe)
│   ├── bt_daemon/                 # Headless server daemon & bt-cli command-line management tool
│   ├── bt_webui/                  # Modern Web UI gateway and dashboard
│   ├── sync_hub/                  # Local-first encrypted LAN folder sync client
│   └── torrent_client_shared/     # Shared IPC data structures, formatters, and models
├── examples/                      # 8 standalone demonstration applications
├── packaging/                     # Inno Setup installer script and Windows release automation
└── docs/                          # Architecture, API references, BEP matrix, and user manuals
```

---

## 2. Core Subsystems

### 2.1 Session & Concurrency Architecture

- **`TorrentSession`**: High-level orchestrator encapsulating transfer lifecycles (`Initialised → Downloading → Paused → Seeding → Ended`). Coordinates tracker announce loops, DHT/LSD discovery, NAT-PMP/UPnP port mappings, disk cache flushes, and background workers.
- **`TorrentSessionBuilder`**: Builder pattern interface for configuring sessions with file paths, magnet URIs, fast-resume data, memory cache bounds, WebSeeds, and verification workers.
- **Async Event Loop & Thread Redirection**:
  - Peer session event handling runs on cooperative asynchronous tasks using `futures::executor::LocalPool`.
  - Blocking network operations (e.g. `TcpStream::connect_timeout` or OS socket handshakes) are offloaded to dedicated OS worker threads on the `std` target so the cooperative executor is never starved.

### 2.2 Torrent State & Context (`TorrentContext`)

- **`TorrentContext`**: Thread-safe central state container (`Arc<Mutex<TorrentContext>>`) managing:
  - **Swarm Management**: Active `peer_swarm` (`HashMap<String, Arc<Mutex<Peer>>>`), client software identification, and connection types (`TCP`, `uTP`, `WebRTC`).
  - **Piece Availability**: Local bitfield, piece completion tracking, piece peer availability frequencies (rarity matrix), and file priority filters.
  - **Block Assembly**: Tracks in-flight block reservations, request timeouts, and endgame duplicates.
  - **Private Torrent Enforcement (BEP 27)**: Disables DHT, LSD, and PEX when `is_private` is set.
  - **Bandwidth & Metrics**: Upload/download transfer rates, parts-per-ten-thousand progress (`progress_ppm`), and telemetry counters.

### 2.3 BitTorrent v2 & Merkle Hash Tree Engine (BEP 52)

- **`MerkleTree`**: Canonical binary SHA-256 Merkle tree calculation:
  - 16 KiB leaf blocks (`BLOCK_SIZE = 16384`) with power-of-two padding as required by BEP 52.
  - Minimal logarithmic audit paths (`MerkleProof`) enabling independent block verification.
  - Streaming generation via `from_reader` and `from_file` for constant-memory processing of arbitrary file sizes.
- **`PieceLayersDiskCache`**: Structured binary `.layers` disk cache with `BT2L` magic header, two-tier LRU memory cache, and SHA-256 integrity checksums.
- **Peer Wire Hash Transfer**: Serves and verifies BEP 52 wire messages: `HashRequest` (ID 21), `Hashes` (ID 22 with uncle proof validation), and `HashReject` (ID 23).
- **`TorrentBuilder` & `TorrentUpgrader`**: Programmatic creation of pure v1, pure v2, or dual-compatible Hybrid `.torrent` files with pad file alignment (`.pad/<length>`).

### 2.4 Transport Layer & Protocol Extensions

- **Micro Transport Protocol (uTP — BEP 29)**:
  - `UtpSocketAdapter` wrapping UDP sockets as `AsyncSocket` transports.
  - **RFC 6817 LEDBAT**: Congestion window scaling driven by one-way delay tracking via sliding-window `DelayTracker`.
  - **SACK Extension**: Selective acknowledgment bitmask parsing, circular 16-bit sequence gap detection, and bounded out-of-order reassembly buffers (`BTreeMap<u16, Vec<u8>>`).
  - **RFC 6298 Fast Retransmission**: Microsecond RTT/RTO tracking and exponential timeout backoff.
- **WebTorrent WebRTC Bridge**:
  - `WebRtcSocketAdapter` message-oriented framing for WebRTC Data Channels.
  - `WebTorrentAnnouncer` handling JSON WebSocket announce and scrape protocols (`ws://` and `wss://`).
  - `WebTorrentBridge` coordinating cross-swarm bridging: pieces downloaded from TCP/uTP swarms are verified and served to browser WebRTC peers, and vice-versa.
- **Peer Exchange (PEX — BEP 11)**:
  - Differential swarm generation broadcasting compact IPv4 (`added`/`dropped`) and IPv6 (`added6`/`dropped6`) peer lists.
  - Flag support: `PREF_ENCRYPT`, `SEED`, `UTP`, `HOLEPUNCH`, and `OUTGOING`.
- **BEP 55 Holepunch Extension (`ut_holepunch`)**:
  - Binary packet framing for `Rendezvous`, `Connect`, and `Error` messages.
  - Intermediate relay peer coordination for NAT/firewall traversal.
- **Message Stream Encryption (MSE / PE)**:
  - 128-bit Diffie-Hellman key exchange with Russian Peasant `mulmod` modular exponentiation.
  - Bi-directional RC4 stream ciphers transparently applied by `PeerNetwork`.
- **Port Mapping**:
  - `FallbackPortMapper` coordinating dual-protocol router traversal: NAT-PMP (RFC 6886) and UPnP (SSDP/IGD SOAP).

### 2.5 Storage, Caching, & Verification Architecture

```
Incoming Block (16 KiB)
        │
        ▼
CachedBlockStorage (In-Memory Buffer)
        │
        ├─ Contiguous blocks merged? ──► Sequential Coalesced Write to DiskIO (>95% syscall reduction)
        │
        ▼
Piece Assembly Completed
        │
        ▼
HashVerificationPool (Multi-Threaded Worker Pool)
        │
        ├─ SHA-1 / SHA-256 Merkle Check Passes?
        │    ├─ YES ──► Mark Piece Local ──► Flush Coalesced Cache Piece ──► Broadcast Have
        │    └─ NO  ──► Discard Corrupt Blocks from Cache ──► Penalize Peer Score
        │
        ▼
FastResumeData (.fastresume Persistence)
```

- **`CachedBlockStorage`**: Transparent `BlockStorage` caching layer buffering dirty blocks up to a configurable MB limit. Coalesces contiguous sub-blocks into single sequential disk writes on flush, reducing write syscalls by >95%. Automatically discards dirty blocks if a piece fails cryptographic verification.
- **`HashVerificationPool`**: Multi-threaded worker pool offloading CPU-intensive SHA-1 and SHA-256 Merkle root checks from the cooperative async loop. Piece selectors incorporate in-flight guards (`pieces_verifying`) to prevent duplicate block requests while verification is underway.
- **Fast Resume Engine (`FastResumeData`)**: Canonical BEP 47 / libtorrent Bencode serialization, on-disk file length validation, and sub-millisecond session startup without full disk re-hashing.

### 2.6 Seekable Media Streaming Engine

- **`TorrentStream`**: Adapts downloading transfers or specific files into standard Rust `std::io::Read + std::io::Seek` streams.
- **`StreamingPieceSelector`**: Dynamic piece selector prioritizing pieces around an atomic playback cursor (`Arc<AtomicU32>`) with a configurable forward lookahead window. Missing blocks are fetched on demand with configurable timeouts or non-blocking `WouldBlock` behavior.
- **Embedded HTTP Streaming Server (`StreamServer`)**: Multi-threaded HTTP server bound to loopback or port 8081 serving `206 Partial Content`, byte ranges (`bytes=start-end`), and MIME autodetection for desktop video players and browser Web UI.

### 2.7 Swarm Intelligence & Endgame Optimization

- **Super-Seeding (BEP 16 & BEP 6)**: Hides full bitfield upon handshake, offering rarest unseeded pieces via `Have`, `Suggest Piece`, and `Allowed Fast`. Enforces single-piece verification before allocating subsequent pieces, and automatically reverts to standard seeding if a second seed joins.
- **Adaptive Endgame Mode**: Multi-source block request staging, RFC 6298 swarm latency profiling (EMA smoothed RTT and expected completion times), straggler stall preemption, and redundant late block absorption without disk writes.
- **Choking Policy**: Tit-for-tat upload bandwidth distribution, 30-second round-robin optimistic unchoking, and anti-snubbing detection.
- **Dynamic WebSeed Failover (BEP 38)**: Multi-mirror HTTP/HTTPS downloads with EMA latency scoring, circuit-breaker backoff cooldowns, and automatic canary probing.

---

## 3. End-to-End Download Data Flow

```
TorrentSessionBuilder::build()
  │
  ├─ FastResumeData::load_from_file() [Instant bitfield restoration if valid]
  └─ If no fast-resume: DiskIO::create_torrent_bitfield() [HashVerificationPool parallel scan]
  │
TorrentSession::start_download()
  │
  ├─ FallbackPortMapper::request_mapping()     [Open port via NAT-PMP or UPnP SOAP]
  ├─ If not private:
  │    ├─ Dht::start() & Dht::bootstrap()      [Kademlia UDP peer discovery]
  │    └─ LsdAnnouncer::start()                [Local multicast announcements]
  ├─ Tracker::start_announcing()               [HTTP, UDP, and WebSocket tracker announces]
  ├─ If web_seeds present:
  │    └─ WebSeedMirrorTracker::start()        [Ranked HTTP range mirror downloads]
  │
  ├─ Worker Event Loop (Cooperative Async Tasks):
  │    │
  │    ├─ Accept Inbound / Connect Outbound Peers:
  │    │    ├─ If mse_enabled: DH Exchange ──► RC4 ciphers installed on PeerNetwork
  │    │    ├─ Handshake Exchange (negotiates Fast Extension, BEP 10, v2 hashes)
  │    │    └─ Wire loop: send Bitfield / HaveAll / Interested
  │    │
  │    ├─ Block Request Dispatch:
  │    │    ├─ PieceSelector (RarestFirst / Sequential / Streaming lookahead)
  │    │    ├─ Filter skipped files (FilePriority) and in-flight verifying pieces
  │    │    └─ Peer::send_message(Request)
  │    │
  │    ├─ Receive PeerMessage::Piece:
  │    │    ├─ CachedBlockStorage::write_block() [Buffered in memory]
  │    │    └─ When piece completes:
  │    │         └─ Submit PieceVerificationJob to HashVerificationPool
  │    │
  │    ├─ Verification Results Drain:
  │    │    ├─ If valid: Mark piece local ──► Flush coalesced piece to disk ──► Broadcast Have
  │    │    └─ If corrupt: Discard dirty piece from cache ──► Report bad peer
  │    │
  │    └─ Download Completion:
  │         └─ All wanted pieces verified ──► Transition to TorrentStatus::Seeding
  │
TorrentSession::stop()
  ├─ Flush CachedBlockStorage to disk
  ├─ FastResumeData::save_to_file()
  ├─ Disconnect all peers safely without deadlocks
  └─ Release NAT-PMP / UPnP port mappings
```

---

## 4. Multi-Process Client Architecture

```
┌────────────────────────────────────────────────────────┐
│                   Desktop GUI Client                   │
│                     (torrent_client)                   │
│         [egui/eframe] + [Embedded StreamServer]        │
└────────────────────────────────────────────────────────┘
                            ▲
                            │ Direct In-Process Library API
                            ▼
┌────────────────────────────────────────────────────────┐
│                 BitTorrent Core Library                │
│                     (bittorrent-rs)                    │
└────────────────────────────────────────────────────────┘
                            ▲
                            │ In-Process
                            ▼
┌────────────────────────────────────────────────────────┐
│                  Server Daemon (bt_daemon)             │
│   ├── Multi-Torrent Session Manager                    │
│   ├── Embedded HTTP Streaming Server (:8081)           │
│   └── IPC Socket Server (Unix Domain / Named Pipe)     │
└────────────────────────────────────────────────────────┘
           ▲                                  ▲
           │ IPC Protocol                     │ IPC Protocol
           ▼                                  ▼
┌───────────────────────┐          ┌───────────────────────┐
│       CLI Tool        │          │    Web UI Gateway     │
│       (bt-cli)        │          │      (bt_webui)       │
│  Management/Scripts   │          │  REST API + Dashboard │
└───────────────────────┘          └───────────────────────┘
```

- **In-Process GUI Model (`torrent_client`)**: The desktop application embeds `bittorrent-rs` directly, running the UI thread on `eframe` and background download sessions on cooperative worker threads.
- **Client-Server Headless Model (`bt_daemon` + `bt-cli` / `bt_webui`)**:
  - `bt_daemon` acts as a multi-torrent server managing swarms, disk I/O, and fast-resume persistence.
  - `bt-cli` and `bt_webui` communicate with `bt_daemon` via a high-performance IPC protocol (Unix domain sockets on Linux/macOS, Windows Named Pipes on Windows).
  - `bt_webui` acts as a reverse gateway, translating browser HTTP REST and WebSocket requests into IPC commands for the daemon.

---

## 5. Security & Robustness Considerations

- **Deadlock-Free Lock Hierarchy**:
  - Event listeners (`ContextEventListener`) utilize `try_lock()` to prevent re-entrant mutex deadlocks during peer disconnection cascades.
  - `TorrentContext::disconnect_all_peers` drops `peer_swarm` write locks before closing peer sockets.
  - Relay rendezvous operations in holepunching release internal state locks before socket dispatch.
- **Memory Safety & Denial-of-Service Defense**:
  - Out-of-order reassembly buffers in uTP are bounded (`max_out_of_order_packets = 128`), discarding flood packets.
  - Path traversal validation in `MetaInfoFile` sanitizes all relative file paths from untrusted torrent metadata.
  - Zero-copy Bencode slice decoders enforce maximum depth recursion limits to prevent stack overflow.
