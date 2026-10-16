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
- **NAT-PMP & UPnP (SSDP/IGD) Auto Port Forwarding**: `NatPmpClient` and `UpnpClient` in `nat.rs` automatically map TCP and UDP port 6881 on `start_download()` and release the mappings on `stop()`. UPnP uses SSDP multicast (`239.255.255.250:1900`) discovery and XML device description parsing to send SOAP requests to `WANIPConnection` / `WANPPPConnection`.

### Phase 17: Seekable Streaming Reader API (Completed)
- **`TorrentStream` (`std::io::Read + std::io::Seek`)**: Exposes active torrent transfers or specific files as standard Rust seekable streams in `storage::stream`.
- **Dynamic `StreamingPieceSelector`**: Pluggable selector tracking an atomic playback cursor (`Arc<AtomicU32>`) and forward lookahead window. Requests prioritize pieces around the playback head, falling back smoothly to swarm health.
- **Whole-Torrent & Per-File Streams**: `TorrentSession::create_stream()` and `TorrentSession::create_file_stream(index)` automatically wire the streaming piece selector and cursor into the session context.
- **Configurable Polling & Timeouts**: `.with_read_timeout(Duration)` and `.with_poll_interval(Duration)` allow callers to configure blocking behavior when awaiting missing pieces, or return `WouldBlock` with `Duration::ZERO`.


### Phase 18: HTTP Range Media Streaming Server & HTML5 WebUI Player (Completed)
- **Embedded HTTP Streaming Server (`bt_daemon`)**: Multi-threaded embedded HTTP server on port 8081 serving `206 Partial Content`, byte ranges (`bytes=start-end`), MIME detection, and multi-file routing.
- **IPC Protocol & CLI File Inspection**: Added `IpcMessage::Files` and `IpcReply::FileList` to query metadata and file lists, with `bt-cli files <hash>` command.
- **Glassmorphic WebUI Video Player**: Interactive modal `#stream-modal` in WebUI with HTML5 `<video>`, live stream stats, file selector dropdown, and `▶ Stream` buttons.

### Phase 19: uTP LEDBAT Congestion Control (RFC 6817 & BEP 29) (Completed)
- **Sliding-Window `DelayTracker`**: 4 circular time buckets tracking base delays across 4 minutes to account for clock drift and route switching.
- **`LedbatCongestionController`**: Linear congestion controller scaling `cwnd` based on normalized delay offset `(TARGET - queuing_delay) / TARGET` with default 100ms threshold.
- **Packet Loss & Fast Retransmit**: Multiplicative decrease (`cwnd = max(cwnd / 2, MIN_CWND)`) triggered on timeout and 3 duplicate ACKs.
- **Bidirectional Delay Measurement**: Accurately reflects remote timestamps in `timestamp_us` and `timestamp_difference_us` headers without requiring synchronized clocks.
- **`UtpSocketAdapter`**: Enforces congestion window during transmission and dynamically adjusts window size upon packet acknowledgment.

### Phase 20: BitTorrent v2 Merkle Tree Block Validation (BEP 52) (Completed)
- **Binary SHA-256 Merkle Tree (`MerkleTree`)**: Chunks files into 16 KiB leaf blocks, zero-pads odd leaves to power-of-two boundaries with `[0u8; 32]`, and generates `pieces_root` Merkle roots.
- **Audit Path Proofs (`MerkleProof`)**: Extracts and verifies logarithmic sibling proof paths up the tree to validate individual blocks independently.
- **Piece Layer Generation**: Generates and reconstructs piece layers for arbitrary power-of-two piece sizes (e.g. 32 KiB, 64 KiB, 128 KiB).
- **Metadata Integration**: Parsed and propagated per-file `pieces_root` in `FileDetails` from the metadata `file tree` dictionary.
- **Immediate Block Assembly Rejection**: Verified blocks directly in `TorrentContext::add_block_to_storage`, immediately rejecting and discarding corrupt or poisoned blocks from bad peers.

### Phase 21: Desktop GUI Video Streaming & In-App Player Launcher (Completed)
- **Embedded Loopback `StreamServer`**: Embedded multi-threaded HTTP server in `torrent_client` bound to an ephemeral loopback port (`127.0.0.1:0`) supporting HTTP Range requests (`206 Partial Content`), seek operations, and MIME type autodetection.
- **Dynamic Prioritization & Lookahead**: Integrates with `TorrentStream` and `StreamingPieceSelector`, dynamically prioritizing pieces around playback cursor and read positions.
- **Dual UI Access**: Provides `▶ Stream` buttons in both the torrent list items (streaming the primary/largest media file) and individual media files in the `Files` detail tab.
- **Active Streaming Top Banner**: Live playback banner with progress bar, `⏹ Stop`, `📋 Copy URL`, and `🚀 Open Player` to launch the stream directly in the default OS media player / browser via `webbrowser`.

### Phase 22: uTP Selective Acknowledgement (SACK Extension Header — BEP 29) (Completed)
- **Extension Header Parsing & Serialization**: Fully parsed and formatted chained uTP extension headers (`UtpExtension::SelectiveAck`, `UtpExtension::Unknown`), encoding dynamic extension type headers with big-endian wire alignment.
- **BEP 29 SACK Bitmask Arithmetic**: Reverse bitmask generation (`build_sack_bitmask`) and sequence checking (`is_seq_sacked`, `parse_sacked_seqs`) strictly adhering to BEP 29 bitmask standards (bit 0 = `ack_nr + 2`, multiples of 4 bytes padding, handling 16-bit sequence wraparound).
- **Out-of-Order Reassembly Buffer**: `UtpSocketAdapter` receiver buffers out-of-order packets in `BTreeMap<u16, Vec<u8>>` and emits SACK extension headers on `ST_STATE` responses. Contiguous sequences are stitched sequentially into the user receive buffer as missing holes are filled.
- **Sender Selective Retransmission & In-Flight Tracking**: Sender inspects incoming SACK extension headers to mark packets as acknowledged without retransmission, adjusting `bytes_in_flight` and crediting the LEDBAT congestion controller window.

---

## Future Roadmap

The following enhancements are proposed for future development:

1. **Peer Swarm Visualizer & Piece Heatmap**: Visual real-time map of connected peers, availability piece grids, piece frequency heatmaps, and country flags in GUI and WebUI.

2. **Selective File Downloading & File Priorities**: Ability to mark specific files within multi-file torrents as `Skip`, `Low`, `Normal`, or `High` priority, updating piece selection masks accordingly.

3. **Super-Seeding Mode (BEP 16)**: Algorithm to optimize initial seeding bandwidth by advertising pieces only after existing peers have uploaded them to others.
