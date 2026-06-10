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
- **`TorrentSession`**: The high-level entry point orchestrating transfers. It encapsulates download paths, running threads, peer connection setups, NAT-PMP lifecycle, and DHT/LSD/tracker background tasks.
- **`TorrentSessionBuilder`**: Implements the builder pattern to construct a session configuration, including file validation and manager injection.
- **`Manager`**: Serves as a global registry for torrents (keyed by info-hash) and aggregates a shared blacklist of slow or dead peers.
- **Background Tasks & Thread Redirection**: Standard tasks (stats logging, tracker announcements) run as asynchronous tasks on a cooperative single-threaded `futures::executor::LocalPool` executor. Network connection attempts (`TcpStream::connect_timeout`), which are blocking, are redirected to dedicated OS worker threads on the `std` target so they do not block the shared executor loop.

### Torrent State & Metadata
- **`TorrentContext`**: The core thread-safe state container. It manages the active `peer_swarm`, piece bitfield progress, missing pieces tracking, and private-torrent flags, and delegates piece block assembly tasks.
- **`TorrentStatus`**: Represents the transfer lifecycle: `Initialised → Downloading → Paused → Seeding → Ended`.
- **`MetaInfoFile`**: Decodes `.torrent` metainfo dicts (v1 and v2), computes SHA-1 or SHA-256 info-hashes depending on format version, validates file names against path traversal, and returns details like piece counts and announce URLs.

### Peer Wire Protocol
- **`Peer`**: Manages remote peer state (choke/interest flags, outstanding requests count, timestamps for rate limiting) and processes peer wire packets.
- **`PeerNetwork`**: Wraps the network socket transport layer, framing length-prefixed bytes into structured messages. Transparently applies RC4 encryption/decryption when MSE ciphers are installed.
- **`PeerMessage<'a>`**: Zero-copy enum framing all wire messages including core messages (`Choke`, `Unchoke`, `Interested`, `NotInterested`, `Have`, `Bitfield`, `Request`, `Piece`, `Cancel`, `Port`) and Fast Extension messages (`HaveAll`, `HaveNone`, `Suggest`, `AllowedFast`, `Reject`) and Extension Protocol messages (`Extended`). Payload buffers slice directly from read buffers.

### DHT Discovery (BEP 5)
- **`Dht`**: Implements Kademlia Distributed Hash Table peer discovery.
- **`RoutingTable`**: Composed of 160 routing buckets (8 nodes each), indexed using the leading zeros of XOR distances between node IDs.
- **`KRPC`**: Zero-allocation Bencode query/response parser for UDP discovery.

### Local Service Discovery (LSD — BEP 14)
- **`LsdAnnouncer`**: Periodically broadcasts `BT-SEARCH` multicast announcements to `239.192.152.143:6771` over UDP, advertising the torrent info-hash and local listening port to peers on the same subnet.
- **`LsdListener`**: Binds a UDP socket to the multicast group and listens for incoming `BT-SEARCH` announcements from other local clients. Discovered peers are submitted to the session downloader.
- Both LSD components are disabled when a torrent's `is_private` flag is set to `true`.

### Tracker Announcement
- **`Tracker`**: Drives start, complete, and stop events. Implements exponential backoff with retries for request fallbacks. Also supports tracker scrape (BEP 48) via `Tracker::scrape()`.
- **`AnnouncerEnum`**: Static dispatch enum wrapper representing either `HttpAnnouncer` (BEP 3) or `UdpAnnouncer` (BEP 15), completely avoiding dynamic dispatch vtable allocations.
- **`ScrapeResponse`**: Contains seeder count (`complete`), download count (`downloaded`), and leecher count (`incomplete`) returned by the tracker scrape endpoint.

### Peer Exchange (PEX — BEP 11)
- Implemented within the peer message handler and the session worker's periodic broadcast loop.
- Each connected peer that supports the Extension Protocol (BEP 10) receives periodic `ut_pex` messages containing compact peer lists for IPv4 (`added`/`dropped`) and IPv6 (`added6`/`dropped6`).
- PEX is disabled when `is_private` is `true`.

### WebSeed (HTTP Seeding — BEP 17 / BEP 19)
- **`webseed.rs`**: Implements the WebSeed download loop, which identifies missing pieces in the torrent context, reserves block requests, and fetches piece data from HTTP mirror URLs using range-based `GET` requests.
- Web seed URLs are parsed from the `"url-list"` key in the torrent metadata and stored in `TorrentContext.web_seeds`.
- The WebSeed loop runs concurrently alongside peer wire connections during `start_download()`.

### Message Stream Encryption (MSE — PE)
- **`mse.rs`**: Provides a pure-Rust RC4 stream cipher and Diffie-Hellman key exchange for obfuscating peer connection traffic.
- **`PeerNetwork`** transparently applies RC4 ciphers when they are installed via `set_mse_ciphers()`.
- MSE negotiation is opt-in via `SessionConfig.mse_enabled` (default: `false`). When enabled, the worker performs the DH exchange and derives RC4 keys before the standard BitTorrent handshake.
- See [encryption.md](file:///c:/Projects/BitTorrent/docs/encryption.md) for a full protocol walkthrough.

### uTorrent Transport Protocol (uTP — BEP 29)
- **`utp.rs`**: Implements `UtpSocketAdapter`, which wraps a `UdpSocket` as an `AsyncSocket`-compatible connection, supporting the uTP framing state machine (SYN, DATA, ACK, RESET).
- The adapter implements uTP header encode/decode (20-byte fixed header), sequence numbering, and ACK responses.
- Current scope: framing adapter only. Full LEDBAT congestion control is a future enhancement.
- See [utp.md](file:///c:/Projects/BitTorrent/docs/utp.md) for the header layout and state machine details.

### NAT-PMP Port Mapping
- **`nat.rs`**: Implements a UDP NAT-PMP client (`NatPmpClient`) that sends port mapping requests to the local router gateway.
- `get_default_gateway()` infers the gateway IP from the local IP address.
- `TorrentSession` maps TCP and UDP port 6881 on `start_download()` and releases mappings on `stop()`.
- See [nat-pmp.md](file:///c:/Projects/BitTorrent/docs/nat-pmp.md) for request/response packet formats and lifecycle details.

### Private Torrents (BEP 27)
- `MetaInfoFile::is_private()` parses the `"private": 1` flag from the info dictionary.
- `TorrentContext.is_private` stores the flag and is checked before starting DHT, LSD, and PEX.
- When `is_private = true`: DHT is not started, LSD announcements are suppressed, PEX messages are neither sent nor processed.

### BitTorrent v2 (BEP 52)
- `MetaInfoFile` detects `"meta version": 2` and parses the `"file tree"` nested dictionary structure.
- Info-hash computation upgrades from SHA-1 (20 bytes) to SHA-256 (32 bytes) for v2 torrents.
- `MetaInfoFile::is_v2()` returns `true` for v2 torrents.
- Merkle tree block-level validation (per-file `pieces_root`) is a planned future enhancement.

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
  ├─ if mse_enabled:
  │    └─ DiffieHellman exchange → RC4 ciphers installed on PeerNetwork
  │
  ├─ if not is_private:
  │    ├─ Dht::start() & Dht::bootstrap()      [DHT peer discovery]
  │    └─ LsdAnnouncer::start()                [LSD local multicast]
  │
  ├─ NatPmpClient::request_mapping()           [Open port 6881 TCP+UDP]
  ├─ Tracker::start_announcing()               [Discover tracker peers]
  │
  ├─ if web_seeds present:
  │    └─ webseed::start_webseed_loop()        [HTTP mirror downloads]
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
  │         ├─ check_piece_hash_streaming()    [Verifies SHA-1 or SHA-256 checksum]
  │         ├─ if verification passes:
  │         │    ├─ TorrentContext::mark_piece_local()
  │         │    ├─ Peer::broadcast_cancel()   [Endgame cancellations]
  │         │    ├─ Peer::send_have()          [Announces to active swarm]
  │         │    └─ try_complete_download()    [Transition to Seeding]
  │         │
  │         └─ check_request_timeouts()        [Cancels and re-requests stale blocks]
  │
TorrentSession::stop()
  ├─ NatPmpClient::release_mapping()           [Remove port 6881 TCP+UDP]
  └─ Dht::stop() / LsdListener::stop()
```

---

## Known Gaps & Future Work

- **Full LEDBAT Congestion Control**: The uTP adapter handles framing but does not implement the delay-based window sizing algorithm (LEDBAT — RFC 6817).
- **UPnP / SSDP Port Mapping**: NAT-PMP is implemented; UPnP (SOAP over SSDP) would cover older routers that do not support NAT-PMP.
- **BEP 52 Merkle Block Validation**: v2 torrent parsing is supported; per-file Merkle tree verification of individual blocks (`pieces_root`) is a future enhancement.
- **Tit-for-Tat Upload Choking**: Optimizing upload distribution by prioritizing peers that upload to us at the highest rate.
- **Optimistic Unchoking**: Periodically unchoke a random peer to discover unused upload capacity in the swarm.
