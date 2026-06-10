# Torrent Client Roadmap

This document outlines the milestones and phases of development for the BitTorrent library, showing what has been achieved and what lies ahead.

---

## Complete Milestones

All milestones through Phase 16 have been fully implemented, tested, and integrated.

### Phase 1: Core Torrent State (Completed)
- Defined `TorrentSession` and `TorrentSessionBuilder` APIs.
- Established context tracking and lifecycle transitions (`Initialised`, `Downloading`, `Paused`, `Seeding`, `Ended`).
- Created robust `.torrent` metadata validators.

### Phase 2: Tracker Announcement & Discovery (Completed)
- Hardened HTTP tracker announcement requests and error handling.
- Supported tracker status notifications (`started`, `stopped`, `completed`).
- Exposed compact peer list parsing into programmatic discovery queues.

### Phase 3: Peer Wire Protocol Integration (Completed)
- Implemented protocol handshake and peer ID exchanges.
- Managed choke/unchoke and interest signals.
- Spawns peer messaging loops and unchokes interested peers natively.

### Phase 4: Piece Selection & Requests (Completed)
- Designed block request window reservation management.
- Implemented pluggable piece selection (`RarestFirstSelector`, `SequentialSelector`).
- Added endgame duplicate request handling and cancellation broadcasts.

### Phase 5: Piece Assembly & Verification (Completed)
- Handled incoming blocks writing directly to disk or memory storage.
- Implemented streaming hash validation (block-by-block hash verification).
- Updated local bitfield states and download speeds dynamically.

### Phase 6: Session Management & Resilience (Completed)
- Spawned parallel peer sessions on background threads.
- Blacklisted non-responsive/dead peers with temporary TTLs.
- Handled pause/resume loops and clean stop/shutdown sequences.
- Added full support for seeding torrents.

### Phase 7: Verification & Testing (Completed)
- Built comprehensive workspace test coverage (63 unit/integration tests).
- Created a runnable interactive desktop GUI client (`torrent_client` binary via `egui`).
- Documented public usage APIs.

### Phase 8: `#![no_std]` Core Extraction (Completed)
- Gated OS-dependent modules and dependencies behind target features in `Cargo.toml`.
- Configured core parsing, encoding, message framing, and selection algorithms to compile cleanly under bare-metal targets (`#![no_std]`).
- Abstracted OS-specific filesystem and socket calls into hardware-agnostic traits (`AsyncSocket`, `BlockStorage`).

### Phase 9: Cooperative Async Multitasking & Executor (Completed)
- Re-architected peer session and logger loops to run asynchronously on a single cooperative background executor (`futures::executor::LocalPool`).
- Implemented non-blocking UDP/TCP state socket handling.
- Added thread redirection triggers for blocking connection setups under the `std` target.

### Phase 10: Zero-Copy & Memory Optimizations (Completed)
- Replaced heap piece buffers with streaming verification, saving 100% of piece assembly RAM overhead.
- Implemented zero-copy Bencode slice parsing (`BNode<'a>`) and network frame parsing (`PeerMessage<'a>`) to eliminate runtime allocations.
- Created static socket buffer pools (`StaticBufferPool`) to reduce heap churn.
- Removed floating-point calculations to avoid FPU dependency emulation on low-end hardware.

### Phase 11: DHT & UDP Tracker Announcing (Completed)
- Implemented Kademlia DHT peer discovery (BEP 5) with XOR distance metrics and a 160-bucket routing table.
- Added KRPC parsing and UDP bootstrap seeding.
- Added integration test coverage for the UDP tracker announcer protocol (BEP 15) using mock server UDP threads.

### Phase 12: Advanced Peer Discovery (Completed)
- **Local Service Discovery (LSD — BEP 14)**: `LsdAnnouncer` and `LsdListener` broadcast and receive `BT-SEARCH` multicast announcements over UDP on `239.192.152.143:6771`.
- **Peer Exchange (PEX — BEP 11)**: Periodic `ut_pex` messages exchange compact peer lists with all Extension Protocol-capable peers, including IPv6 addresses (`added6`/`dropped6`).
- **Tracker Scrape (BEP 48)**: `Tracker::scrape()` queries the `/scrape` endpoint and returns seeder, leecher, and download count statistics.
- **Keep-Alive Timers**: Peers with no message activity for > 120 seconds receive a KeepAlive, and stale peers are disconnected.

### Phase 13: WebSeed Support (Completed)
- **BEP 17 / BEP 19 HTTP Seeding**: `webseed.rs` implements range-based HTTP GET downloads from web seed mirror URLs parsed from the `"url-list"` torrent metadata key.
- The WebSeed loop runs concurrently alongside peer wire connections during `start_download()`.

### Phase 14: Private Torrents & BitTorrent v2 (Completed)
- **Private Torrents (BEP 27)**: `MetaInfoFile::is_private()` parses the `"private": 1` flag. DHT, LSD, and PEX are disabled when the flag is set.
- **BitTorrent v2 (BEP 52)**: `MetaInfoFile` detects `"meta version": 2`, parses the `"file tree"` directory structure, and computes 32-byte SHA-256 info-hashes using the `sha2` crate.

### Phase 15: Message Stream Encryption (Completed)
- Pure-Rust **RC4 stream cipher** and **Diffie-Hellman** key exchange in `mse.rs`.
- `PeerNetwork` transparently applies RC4 ciphers when they are installed.
- MSE negotiation is opt-in via `SessionConfig.mse_enabled` (default: `false`) for backward compatibility.
- Overflow-safe modular exponentiation via `mulmod` (Russian Peasant algorithm).

### Phase 16: uTP & NAT-PMP (Completed)
- **uTorrent Transport Protocol (uTP — BEP 29)**: `UtpSocketAdapter` in `utp.rs` wraps a `UdpSocket` as an `AsyncSocket`-compatible peer transport with full uTP header framing (SYN/DATA/ACK/RESET state machine).
- **NAT-PMP Auto Port Forwarding**: `NatPmpClient` in `nat.rs` automatically maps TCP and UDP port 6881 on `start_download()` and releases the mappings on `stop()`. Gateway discovery is performed via local IP heuristic.

---

## Future Roadmap

The following enhancements are proposed for future development:

1. **LEDBAT Congestion Control for uTP**: Implement the full Low Extra Delay Background Transport algorithm (RFC 6817) — one-way delay measurement, dynamic window sizing, and packet retransmission — in `UtpSocketAdapter`.

2. **UPnP / SSDP Port Mapping**: Complement NAT-PMP with UPnP (SOAP over SSDP) for compatibility with older routers that do not support NAT-PMP. See [nat-pmp.md](file:///c:/Projects/BitTorrent/docs/nat-pmp.md) for current scope.

3. **BEP 52 Merkle Tree Block Validation**: Extend the v2 implementation to verify individual blocks against per-file `pieces_root` Merkle tree hashes, enabling block-level verification without downloading an entire piece.

4. **Tit-for-Tat Upload Choking**: Optimize upload slot allocation by measuring rolling per-peer upload rates and preferentially unchoking the peers contributing the most data to us.

5. **Optimistic Unchoking**: Periodically unchoke a randomly-selected interested peer to discover unused upload capacity and give new peers a chance to contribute.
