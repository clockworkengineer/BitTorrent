# Architecture Overview

This document describes the design, key components, and data flows of the BitTorrent client library.

---

## Crate Layout & Workspace
- [library/src/](file:///c:/Projects/BitTorrent/library/src/) — Core BitTorrent library code (optionally `#![no_std]`).
- [library/tests/](file:///c:/Projects/BitTorrent/library/tests/) — Crate test suite (unit and integration tests).
- [clients/torrent_client/](file:///c:/Projects/BitTorrent/clients/torrent_client/src/main.rs) — Desktop client binary implementing an interactive UI using `egui`/`eframe`.
- [examples/torrent_session_example/](file:///c:/Projects/BitTorrent/examples/torrent_session_example/src/main.rs) — Simple CLI session runner demonstrating top-level programmatic usage.

---

## Key Components

### Session & Concurrency Layer
- **`TorrentSession`**: The high-level entry point orchestrating transfers. It encapsulates download paths, running threads, and peer connection setups.
- **`TorrentSessionBuilder`**: Implements the builder pattern to construct a session configuration, including file validation and manager injection.
- **`Manager`**: Serves as a global registry for torrents (keyed by info-hash) and aggregates a shared blacklist of slow or dead peers.
- **Background Tasks & Thread Redirection**: Standard tasks (stats logging, tracker announcements) run as asynchronous tasks on a cooperative single-threaded `futures::executor::LocalPool` executor. Network connection attempts (`TcpStream::connect_timeout`), which are blocking, are redirected to dedicated OS worker threads on the `std` target so they do not block the shared executor loop.

### Torrent State & Metadata
- **`TorrentContext`**: The core thread-safe state container. It manages the active `peer_swarm`, piece bitfield progress, and missing pieces tracking, and delegates piece block assembly tasks.
- **`TorrentStatus`**: Represents the transfer lifecycle: `Initialised → Downloading → Paused → Seeding → Ended`.
- **`MetaInfoFile`**: Decodes `.torrent` metainfo dicts, computes info-hashes, validates file names against path traversal, and returns details like piece counts and announce URLs.

### Peer Wire Protocol
- **`Peer`**: Manages remote peer state (choke/interest flags, outstanding requests count, timestamps for rate limiting) and processes peer wire packets.
- **`PeerNetwork`**: Wraps the network socket transport layer, framing length-prefixed bytes into structured messages.
- **`PeerMessage<'a>`**: Zero-copy enum framing all wire messages (`Choke`, `Unchoke`, `Interested`, `NotInterested`, `Have`, `Bitfield`, `Request`, `Piece`, `Cancel`, `Port`). Payload buffers slice directly from read buffers.

### DHT Discovery (BEP 5)
- **`Dht`**: Implements Kademlia Distributed Hash Table peer discovery.
- **`RoutingTable`**: Composed of 160 routing buckets (8 nodes each), indexed using the leading zeros of XOR distances between node IDs.
- **`KRPC`**: Zero-allocation Bencode query/response parser for UDP discovery.

### Tracker Announcement
- **`Tracker`**: Drives start, complete, and stop events. Implements exponential backoff with retries for request fallbacks.
- **`AnnouncerEnum`**: Static dispatch enum wrapper representing either `HttpAnnouncer` (BEP 3) or `UdpAnnouncer` (BEP 15), completely avoiding dynamic dispatch vtable allocations.

### Block Assembly & Disk I/O
- **`DiskIO`**: Manages filesystem layouts. Maps global linear torrent offsets into target files and handles piece writes across boundaries.
- **`Assembler`**: Tracks in-progress piece assemblies, blocks countdowns, and request timeouts.
- **`PieceBuffer`**: Lightweight metadata tracker indicating which blocks are present, storing data directly to storage to avoid RAM buffer exhaustion.

### Piece Selection
- **`PieceSelector`**: Pluggable trait for downloading strategies.
  - `RarestFirstSelector`: Prioritizes pieces with the lowest peer availability.
  - `SequentialSelector`: Downloads pieces sequentially (optimized for media streaming).

### Hardware Abstractions
- **`AsyncSocket`**: Abstract trait for reading and writing network packets asynchronously.
- **`BlockStorage`**: Abstract trait for reading and writing blocks of file contents.
- **`MemStorage`** & **`MockSocket`**: In-memory test implementations of hardware traits, allowing port-free and filesystem-free integration tests.

---

## Data Flow

The diagram below outlines the standard flow of operations when initiating a download session:

```
TorrentSessionBuilder::build() -> TorrentSession
  │
  ├─ DiskIO::create_local_torrent_structure()  [Initializes file tree]
  ├─ DiskIO::create_torrent_bitfield()         [Scans verified files on disk]
  │
TorrentSession::start_download()
  │
  ├─ Dht::start() & Dht::bootstrap()           [Initializes DHT discovery]
  ├─ Tracker::start_announcing()               [Discovers Tracker peers]
  │
  ├─ Spawn peer connection threads:
  │    │
  │    ├─ Peer::new_with_socket() -> Handshake exchange
  │    ├─ send Bitfield & Interested
  │    ├─ receive Unchoke
  │    │
  │    └─ Peer Message Loop (Cooperative Async tasks)
  │         ├─ TorrentContext::next_block_request_for_peer()
  │         ├─ Peer::send_message(Request)
  │         │
  │         ├─ receive PeerMessage::Piece
  │         ├─ BlockStorage::write_block()     [Write directly to disk/memory]
  │         ├─ check_piece_hash_streaming()    [Verifies checksum block-by-block]
  │         ├─ if verification passes:
  │         │    ├─ TorrentContext::mark_piece_local()
  │         │    ├─ Peer::broadcast_cancel()   [Endgame cancellations]
  │         │    ├─ Peer::send_have()          [Announces to active swarm]
  │         │    └─ try_complete_download()    [Transition to Seeding]
  │         │
  │         └─ check_request_timeouts()        [Cancels and re-requests stale blocks]
```

---

## Known Gaps & Future Work

- **Magnet Link Metadata Exchange**: Support parsing magnet links and downloading the metainfo dictionary via the extension protocol (BEP 9 / BEP 10).
- **Tit-for-Tat Choking Algorithm**: Optimize upload distribution using a rolling average of download bandwidth to prioritize peers that actively upload to us.
- **Optimistic Unchoking**: Periodically unchoke a random peer to discover unused upload capacity in the swarm.
- **Keep-Alive Reconnect Logic**: Detect dropped TCP sockets via regular KeepAlive timeouts and trigger reconnection attempts.
