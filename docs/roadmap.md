# Torrent Client Roadmap

This document outlines the milestones and phases of development for the BitTorrent library, showing what has been achieved and what lies ahead.

---

## Complete Milestones

All original core milestones have been fully implemented, tested, and integrated:

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
- Built comprehensive workspace test coverage (~47 unit/integration tests).
- Created a runnable interactive desktop GUI client (`torrent_client` binary via `egui`).
- Documented public usage APIs.

---

## Advanced Feature Milestones

The following phases represent the advanced system design improvements implemented in later development cycles:

### Phase 8: `#![no_std]` Core Extraction
- Gated OS-dependent modules and dependencies behind target features in `Cargo.toml`.
- Configured core parsing, encoding, message framing, and selection algorithms to compile cleanly under bare-metal targets (`#![no_std]`).
- Abstracted OS-specific filesystem and socket calls into hardware-agnostic traits (`AsyncSocket`, `BlockStorage`).

### Phase 9: Cooperative Async Multitasking & Executor
- Re-architected peer session and logger loops to run asynchronously on a single cooperative background executor (`futures::executor::LocalPool`).
- Implemented non-blocking UDP/TCP state socket handling.
- Added thread redirection triggers for blocking connection setups under the `std` target.

### Phase 10: Zero-Copy & Memory Optimizations
- Replaced heap piece buffers with streaming verification, saving 100% of piece assembly RAM overhead.
- Implemented zero-copy Bencode slice parsing (`BNode<'a>`) and network frame parsing (`PeerMessage<'a>`) to eliminate runtime allocations.
- Created static socket buffer pools (`StaticBufferPool`) to reduce heap churn.
- Removed floating-point calculations to avoid FPU dependency emulation on low-end hardware.

### Phase 11: DHT & UDP Tracker Announcing
- Implemented Kademlia DHT peer discovery (BEP 5) with XOR distance metrics and a 160-bucket routing table.
- Added KRPC parsing and UDP bootstrap seeding.
- Added integration test coverage for the UDP tracker announcer protocol (BEP 15) using mock server UDP threads.

---

## Future Roadmap

The following design enhancements are proposed for future development:

1. **Magnet Link Support (BEP 9)**: Add support for parsing magnet links and downloading the metainfo file using the metadata extension protocol.
2. **Tit-for-Tat Upload Choking**: Prioritize uploads to peers that upload to us at the highest rates.
3. **Optimistic Unchoking**: Periodically unchoke random interested peers to discover unused upload capacity.
4. **Peer Exchange (PEX - BEP 11)**: Add local peer list exchange between connected peers to reduce tracker load.
