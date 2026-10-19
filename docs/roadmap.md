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

### Phase 23: Peer Swarm Visualizer & Piece Heatmap (Completed)
- **Peer Client Software Identification**: Implemented `parse_peer_client` to detect and decode client identity and version strings from Azureus-style (`-qB4600-`, `-TR4000-`, `-UT3550-`, `-DE2100-`), Shadow-style (`M4-2-0-`), and vendor-specific Peer IDs.
- **Piece Availability & Swarm Rarity Matrix**: Added `TorrentContext::get_piece_availability` returning local piece completion bitmasks alongside swarm-wide piece rarity counts (frequency of each piece held by connected peers).
- **Desktop GUI Piece Heatmap & Swarm Table**:
  - Added **"🧩 Pieces"** detail tab with interactive canvas grid rendering pieces in emerald green (local verified), sky blue / turquoise (swarm copies), or slate dark grey (missing), with hover tooltips detailing piece offsets and availability.
  - Enhanced **"👥 Peers"** detail tab with client software badges, progress bars, colored upload/download speed indicators, and protocol flag tooltips.
- **CLI & WebUI Swarm Visualizer**:
  - Added `bt-cli peers <hash>` and `bt-cli pieces <hash>` commands with visual ASCII progress and rarity bars.
  - Extended IPC with `IpcMessage::Peers` and `IpcMessage::Pieces` / `IpcReply::PeerList` and `IpcReply::PieceStates`.
  - Added `/api/torrents/:info_hash/peers` and `/api/torrents/:info_hash/pieces` endpoints in `bt_webui`.
  - Added glassmorphic **Swarm & Piece Visualizer** modal with live piece tile heatmap, peer connection tables, and automatic 1.5s background polling.

---

### Phase 24: Selective File Downloading & File Priorities (Completed)
- **File Priority Levels (`FilePriority`)**: Added `Skip` (0), `Low` (1), `Normal` (4, default), and `High` (7) priority levels with parsing and string representation methods.
- **Dynamic Piece Priority Mapping & Boundary Resolution**:
  - Automatically calculates piece priority dynamically from all overlapping files: `piece_priority(p) = max_{f overlaps p} f.priority`.
  - Boundary pieces shared between skipped and wanted files are safely retained at the highest overlapping priority, preventing data corruption.
  - `is_piece_wanted(p)` determines if any wanted file requires piece `p`.
- **Selector & Block Requester Integration**:
  - `RarestFirstSelector`, `SequentialSelector`, and `StreamingPieceSelector` filter out skipped pieces.
  - `TorrentContext::next_block_request_for_peer` sorts missing pieces by priority (`Reverse(get_piece_priority)`), dispatching blocks for `High` priority files before `Normal` and `Low`.
  - Selective download completion check (`is_download_complete`) transitions to `Seeding` when all wanted pieces are downloaded.
- **Dynamic Priority Modification**:
  - `TorrentContext::set_file_priority` and `TorrentSession::set_file_priority` allow updating file priorities on the fly.
  - Automatically clears pending block reservations for pieces that transition to `Skip`, and re-seeds the piece priority queue when files are re-enabled.
- **Full Client Ecosystem Support**:
  - **IPC & Daemon**: Added `IpcMessage::SetFilePriority { info_hash, file_index, priority }` and `priority` field to `FileInfo`.
  - **CLI (`bt-cli`)**: Added `bt-cli set-priority <hash> <index> <priority>` and `PRIORITY` column to `bt-cli files <hash>`.
  - **WebUI (`bt_webui`)**: Added `POST /api/torrents/:info_hash/files/:file_index/priority` endpoint, a dedicated `📁 Files` tab in the visualizer modal, and interactive priority dropdown `<select>` controls per file.
  - **Desktop GUI (`torrent_client`)**: Added interactive `egui::ComboBox` priority dropdown per file in the `Files` detail tab.

---

### Phase 25: Super-Seeding Mode (BEP 16)

Implemented complete BEP 16 Super-Seeding (Initial Seeding) mode across the core protocol library, daemon, CLI, WebUI, and Desktop GUI:
- **Wire Protocol & Seeder Emulation**:
  - In super-seeding mode, the seeder hides its complete bitfield on handshake and masquerades as an incomplete peer with no pieces.
  - Offers unseeded pieces exclusively via `PeerMessage::Have(piece_index)` based on rarest swarm distribution and piece offering history.
- **Swarm Validation & Propagation Enforcement**:
  - The seeder enforces a single-piece-at-a-time offer per peer until another peer in the swarm announces a `Have` message for that piece, validating that the recipient successfully shared it with the swarm.
  - Upload gatekeeper (`TorrentContext::can_upload_piece_to_peer`): Only blocks belonging to pieces offered to that specific peer are served, preventing greedy peers from leeching unauthorized pieces.
  - Automatic peer disconnect recovery: If a peer drops before the swarm validates the offered piece, the piece is immediately unreserved and recycled to other connected peers.
- **Automatic Exit Condition**:
  - Automatically detects when another complete seed joins or appears in the swarm (via `Bitfield` or `Have` messages covering all pieces), exiting super-seeding mode and reverting to standard seeding behavior.
- **Full Client Ecosystem & Management**:
  - **Core & Session**: Configurable via `SessionConfig.super_seeding`, `TorrentSessionBuilder::super_seeding(bool)`, `TorrentSession::is_super_seeding()`, and `TorrentSession::set_super_seeding(bool)`.
  - **IPC & Daemon**: Added `IpcMessage::SetSuperSeeding { info_hash, enabled }` and added `is_super_seeding` boolean to `TorrentStatusInfo`.
  - **CLI (`bt-cli`)**: Added `bt-cli super-seed <hash> <on|off>` command and `[SS]` indicator in the status table.
  - **WebUI (`bt_webui`)**: Added `POST /api/torrents/:info_hash/super-seeding` API endpoint, glowing `Super-Seed` badge, and one-click `⚡ SS On / SS Off` toggle button in the torrent list.
  - **Desktop GUI (`torrent_client`)**: Added interactive `Super-Seeding (BEP 16)` checkbox toggle in the `Overview` detail tab.
  - **Test Suite**: Dedicated integration tests in `library/tests/super_seeding_tests.rs` verifying piece distribution, swarm have validation, upload filtering, disconnect recovery, and auto-exit upon detecting a second seed.

---

### Phase 26: WebRTC / WebTorrent Peer Transport

Implemented complete WebTorrent and WebRTC Data Channel peer transport support across the library and client ecosystem:
- **WebRTC Data Channel Socket Adapter (`WebRtcSocketAdapter`)**:
  - Implemented `AsyncSocket` adapter for WebRTC Data Channels (e.g., standard `"webtorrent"` labeled channel).
  - Handles message-oriented binary packet framing, thread-safe asynchronous buffering, channel lifecycle states (`Connecting`, `Open`, `Closing`, `Closed`), and data transfer metrics (`bytes_sent`, `bytes_received`, `messages_sent`, `messages_received`).
  - Supports loopback pair creation (`create_pair`) for cross-peer protocol validation.
- **WebTorrent WebSocket Tracker Signaling**:
  - Dedicated `WebTorrentAnnouncer` supporting both `ws://` and `wss://` tracker protocols.
  - Generates JSON announce requests containing WebRTC SDP offers (`offers`), peer identification, download statistics, and left byte counts.
  - Parses incoming JSON announce responses routing SDP answers, peer contacts, and swarm statistics.
  - Helper functions for standard WebRTC Session Description Protocol (SDP) offer and answer generation (`create_webtorrent_offer`, `create_webtorrent_answer`).
  - Integrated into `AnnouncerFactory` and `AnnouncerEnum` with automatic protocol detection for WebSocket URLs.
- **Wire Protocol & Swarm Visualizer Integration**:
  - `Socket::WebRtc` variant in `bittorrent-rs::Socket` enum with full async read/write/close dispatch.
  - `Peer::new_webrtc` constructor and dynamic `connection_type` tracking (`"WebRTC"`, `"uTP"`, or `"TCP"`).
  - `extract_peer_infos` propagates connection transport to client shared models.
  - **WebUI & Desktop Visualizer**: Added dedicated transport badges in the Swarm peers list:
    - Glowing violet `<span class="status-pill">WebRTC</span>` badge for browser/WebTorrent peers.
    - Blue `<span class="status-pill">uTP</span>` badge for Micro Transport Protocol peers.
    - Emerald `<span class="status-pill">TCP</span>` badge for standard TCP peers.
- **Test Suite**: Comprehensive test suite in `library/tests/webrtc_tests.rs` verifying adapter lifecycle, loopback bidirectional byte and message transfer, SDP offer/answer generation, JSON announce request and response handling, WebSocket tracker detection in `AnnouncerFactory`, and full BitTorrent wire message exchange over WebRTC Data Channels.

---

### Phase 27: Fast Resume File Serialization (BEP 47)

Implemented native binary / bencode serialization, on-disk file verification, and session restoration adhering to BEP 47 and libtorrent `.fastresume` conventions:
- **Fast Resume Data Structure (`FastResumeData`)**:
  - Encapsulates 20-byte info-hash, target save path, piece count, piece length, packed bitfield, byte-per-piece array (`pieces`), total uploaded and downloaded bytes, active/seeding durations, seeding state, file entry array (`FastResumeFileEntry`), and known peer addresses (`"ip:port"`).
  - Piece helper utilities: `has_piece`, `set_piece`, `pieces_have_count`, and `progress`.
- **BEP 47 / libtorrent Bencode Serialization**:
  - `to_bencode`: Serializes dictionary keys in strict lexicographical canonical Bencode order (`"file-format"`, `"file-version"`, `"info-hash"`, `"save_path"`, `"num_pieces"`, `"piece_length"`, `"bitfield"`, `"pieces"`, `"total_uploaded"`, `"total_downloaded"`, `"active_time"`, `"seeding_time"`, `"is_seeding"`, `"file sizes"`, `"file_paths"`, `"file_priorities"`, `"peers"`).
  - `from_bencode`: Robust decoding with backward and cross-client compatibility: supports both packed `"bitfield"` and byte-per-piece `"pieces"` strings, multi-format `[size, mtime]` pairs or single sizes, and compact or string peer representations.
  - Disk persistence: `save_to_file` and `load_from_file` for native `.fastresume` or `.resume` files.
- **Fast On-Disk Validation (`verify_on_disk`)**:
  - Instant disk verification without rehashing every block from disk: verifies file existence and exact byte length matching across all torrent files.
  - Strict timestamp verification (`verify_on_disk_strict`): verifies filesystem modification timestamps (`mtime`) with tolerance for FAT32 / filesystem clock precision.
- **Session & Builder Integration**:
  - `TorrentSessionBuilder::fast_resume(data)`, `TorrentSessionBuilder::fast_resume_data(data)`, and `TorrentSessionBuilder::fast_resume_file(path)`.
  - In `build()`, verified fast-resume data restores piece bitfields, recalculates piece lengths, sets download metrics, applies file priorities, and marks completion directly without disk re-hashing. If verification fails (e.g. truncated or missing file), safely falls back to standard hash verification.
  - `TorrentSession::generate_fast_resume()` and `TorrentSession::save_fast_resume(path)` for saving state on exit or periodically.
- **Daemon & CLI Companion Tools**:
  - `bt_daemon`: Saves `.fastresume` files in the data directory upon daemon state save, restoring sessions instantly upon restart using `.fast_resume_file(...)`.
  - `bt_cli`: Added `bt-cli resume-info <path>` command to inspect and validate `.fastresume` files directly from the command line.
  - `torrent_fast_resume_example`: Updated example demonstrating standard session creation, `.fastresume` export, and instant resumption.
- **Test Suite**: Dedicated integration test suite in `library/tests/fast_resume_tests.rs` verifying bencode roundtripping, piece progress calculations, on-disk verification passes and rejections, context restoration, file I/O, and session builder roundtrips with real `.torrent` files.

---

### Phase 28: IPv6 Compact Tracker & Dual-Stack Peer Wire Enhancements (BEP 7 & BEP 15)

Implemented native dual-stack (IPv4 / IPv6) peer discovery and connectivity across HTTP trackers, UDP trackers, DHT, and peer connections:
- **BEP 7 IPv6 Compact Tracker Encoding & Decoding**:
  - Implemented `decode_compact_ipv6_peers` and `encode_compact_ipv6_peers` parsing 18-byte peer records (16-byte IPv6 address + 2-byte big-endian port).
  - Handles incomplete slices and arbitrary buffer offsets without panics.
- **Dual-Stack HTTP Tracker Announce Support**:
  - `HttpAnnouncer::decode_announce_response`: Concurrently decodes both `peers` (IPv4) and `peers6` (IPv6) compact lists, seamlessly merging discovered peers into `peer_list`.
  - Added support for dictionary peer lists with IPv6 addresses and bracketed strings (`[2001:db8::1]`), correctly preserving colons in IPv6 addresses while stripping IPv4-mapped prefixes (`::ffff:`).
  - `HttpAnnouncer::build_announce_url`: Appends `&ipv6=...` URL parameter to announce requests when an IPv6 address is configured in `TrackerAnnounceContext`.
- **BEP 15 UDP Tracker IPv6 Support**:
  - `UdpAnnouncer::announce`: Detects 18-byte compact peer records (or explicit IPv6 socket connections) and routes parsing to `get_compact_ipv6_peer_list`.
  - Automatically binds to `[::]:0` when connecting to IPv6 remote tracker endpoints.
- **Wire Protocol & Socket Formatting**:
  - Added `format_socket_addr(ip, port)` to format socket addresses as `[ipv6]:port` or `ipv4:port`, ensuring standard `SocketAddr` parsing succeeds across `peer_network`, `utp`, and `dht`.
  - Updated `PeerDetails` with `endpoint()`, `is_ipv6()`, and `is_ipv4()` helpers.
  - `TrackerAnnounceContext::get_compact_ipv6_peer_list` filters out both client IPv4 and IPv6 addresses to prevent self-connection swarms.
- **DHT (BEP 32) IPv6 Integration**:
  - Parsed 18-byte compact IPv6 peer entries from `values` list in `get_peers` DHT query responses.
  - Parsed 38-byte compact IPv6 node records (`nodes6`) in DHT node discovery.
  - `encode_compact_peers` encodes both IPv4 (6-byte) and IPv6 (18-byte) compact peer records.
- **Session & Builder Configuration**:
  - Extended `SessionConfig` with `pub ipv6: Option<String>` and `pub enable_ipv6: bool`, along with `.with_ipv6(...)` and `.with_enable_ipv6(...)`.
  - Added `.ipv6(...)` and `.enable_ipv6(...)` configuration methods to `TorrentSessionBuilder`.
- **Test Suite**: Dedicated integration tests in `library/tests/ipv6_tracker_tests.rs` verifying IPv6 socket address formatting, roundtrip compact encoding/decoding, dual-stack `peers` + `peers6` HTTP announce responses, dictionary-based IPv6 parsing, URL parameter generation, and session builder configuration.

---

### Phase 29: Disk Cache & Asynchronous Write Coalescing

Implemented high-throughput in-memory block caching, asynchronous write coalescing, and disk protection against corrupted blocks:
- **Cached Block Storage Wrapper (`CachedBlockStorage`)**:
  - Implements `BlockStorage` (both `BlockReader` and `BlockWriter`) transparently wrapping any underlying storage (e.g. `DiskIO`, `MemStorage`).
  - Buffers dirty 16 KiB blocks in memory up to a configurable capacity (`max_cache_bytes`).
- **Sequential Write Coalescing**:
  - Automatically merges contiguous incoming blocks (e.g. 32 x 16 KiB blocks across a 512 KiB piece) into large sequential disk writes on flush, reducing disk write syscalls and seek overhead by over 95%.
- **Read-Through In-Memory Serving**:
  - `read_block` serves data directly from in-memory cache blocks without issuing disk read syscalls, accelerating piece hash verification and seeding upload requests.
  - Automatically fetches un-cached intervals from underlying storage when needed.
- **Corrupted Block Poisoning Protection & Piece-Level Flush/Discard**:
  - Discards dirty blocks in memory (`discard_piece`, `discard_range`) if a piece fails SHA-1 hash verification, ensuring corrupt blocks from bad peers never reach disk.
  - `flush_piece` immediately flushes verified coalesced pieces to disk.
- **LRU Memory Eviction**:
  - Evicts clean (already flushed) blocks based on least-recently-used access timestamps (`last_accessed`) when cache capacity is reached.
  - Automatically flushes and evicts if dirty blocks overflow capacity.
- **Session & Builder Configuration**:
  - Added `pub disk_cache_size_mb: usize` to `SessionConfig` (default: 32 MB) with `.with_disk_cache_size_mb(usize)`.
  - Added `.disk_cache_size_mb(usize)` to `TorrentSessionBuilder`.
  - Added `TorrentSession::flush_disk_cache()` and automatic flushing on session stop and drop.
- **Test Suite**: Dedicated integration test suite in `library/tests/disk_cache_tests.rs` verifying in-memory read/write roundtrips, multi-block contiguous coalescing into single storage write calls, non-contiguous multi-chunk flushing, piece-level discard and flush, LRU eviction under capacity constraints, and cache miss fallbacks.

---

### Phase 30: Fast Extensions (BEP 6)

Implemented the full BitTorrent Fast Extension (BEP 6) specification, optimizing peer bootstrap speeds, choke/unchoke turnaround, and wire protocol efficiency:
- **Canonical Allowed Fast Set Algorithm (`generate_allowed_fast_set`)**:
  - Implemented the official CacheLogic canonical algorithm in `utils::util`.
  - Applies IPv4 `/24` masking (`ip & 0xffffff00`) and IPv6 `/48` prefix masking to ensure IP subnet invariance.
  - Generates deterministic pseudo-random piece indices via cyclic SHA-1 hashing of state buffer concatenated with the torrent info-hash modulo piece count.
  - Validated against official BEP 6 test vectors ($k=7$ and $k=9$ for 1313 pieces, info-hash `0xAA`, IP `80.4.4.200`).
- **Handshake Negotiation (`reserved[7] |= 0x04`)**:
  - Advertises BEP 6 support via bit 0x04 in `reserved[7]` during peer handshakes.
  - Automatically negotiates Fast Extension support with remote peers upon handshake completion.
- **Wire Protocol Fast Extension Messages**:
  - `HaveAll` (ID 14, 0x0E) & `HaveNone` (ID 15, 0x0F): Sent immediately after handshake when a peer possesses all or no pieces, replacing cumbersome bitfields and conserving bandwidth.
  - `Suggest Piece` (ID 13, 0x0D): Advisory message informing connected peers of optimal pieces to request (used for super-seeding and sequential seeding).
  - `Allowed Fast` (ID 17, 0x11): Announces piece indices that a peer is permitted to request and download even while choked.
  - `Reject Request` (ID 16, 0x10): Sent when choked and a request cannot be satisfied, eliminating race conditions and ambiguously stalled blocks.
- **Peer State Machine & Action Integration**:
  - Added `SendReject`, `SendAllowedFast`, `SendSuggest`, `SendHaveAll`, and `SendHaveNone` to `PeerAction` and `Peer::execute_actions`.
  - Choked incoming requests for non-allowed pieces automatically trigger `PeerAction::SendReject`.
  - Choked incoming requests for pieces in the remote peer's canonical Allowed Fast set are served normally without waiting for an unchoke round.
  - Receiving `Reject Request` immediately releases reserved blocks in `TorrentContext`, allowing immediate re-reservation or request dispatch to other peers.
- **Choked Fast Downloading**:
  - Implemented `TorrentContext::next_allowed_fast_block_request_for_peer(&peer)` to pipeline requests for Allowed Fast pieces even while choked by the remote peer.
  - `send_block_requests` and `run_peer_loop` trigger immediate block requesting for Allowed Fast pieces when choked.
- **Test Suite**: Dedicated integration test suite in `library/tests/fast_extension_tests.rs` covering official CacheLogic test vectors, subnet masking invariance, wire message framing/roundtrip, handshake bit negotiation, `HaveAll`/`HaveNone` bitfield updates, `Suggest` and `AllowedFast` tracking, choked request rejection and allowed-fast fulfillment, block release on rejection, and choked fast downloading.

---

### Phase 31: BitTorrent v2 Merkle Hash Tree File Verification & Hybrid Torrents (BEP 52)

Implemented the full BitTorrent v2 (BEP 52) specification, supporting pure v2 torrents, canonical SHA-256 Merkle hash tree verification, top-level `piece layers`, dual-stack Hybrid torrents (v1 + v2), and programmatic torrent construction:
- **Canonical SHA-256 Merkle Hash Tree Implementation (`MerkleTree`)**:
  - Implements canonical SHA-256 Merkle trees with power-of-two padding (leaf nodes and intermediate nodes zero-padded as defined in BEP 52).
  - Leaf hash calculation over 16 KiB blocks (`BLOCK_SIZE = 16384`) with canonical internal node hashing: `SHA256(left_child || right_child)`.
  - Added streaming generation via `MerkleTree::from_reader<R: Read>(...)` and `MerkleTree::from_file<P: AsRef<Path>>(...)`, allowing files of arbitrary size to be hashed in constant memory without reading whole files into RAM.
  - Implemented `MerkleTree::proof_for_block(block_index)` to generate minimal compact logarithmic audit paths (`MerkleProof`) for block verification.
- **Top-Level `piece layers` Dictionary Support**:
  - Parsed and extracted the top-level `piece layers` dictionary from torrent metadata into `MetaInfoFile::piece_layers: PieceLayersMap`.
  - Maps 32-byte file `pieces root` to its concatenated 32-byte layer hashes for files larger than the piece length.
  - Added `MetaInfoFile::get_piece_layers()` accessor.
- **Dual Info-Hash & Backward Compatibility**:
  - Dual info-hash computation: calculates both 20-byte SHA-1 info-hash (`"v1 info hash"`) and 32-byte SHA-256 info-hash (`"v2 info hash"`).
  - Hybrid Torrents (`meta.is_hybrid()`): seamlessly sets primary `"info hash"` to the 20-byte SHA-1 hash, ensuring backward-compatible peer wire handshakes and tracker announces work with legacy v1 swarms.
  - Pure v2 Torrents (`meta.is_v2()`): uses 32-byte SHA-256 info-hash.
  - Added `get_v1_info_hash()` and `get_v2_info_hash()` accessors to `MetaInfoFile`.
- **Hybrid Piece Alignment & Pad Files (BEP 52)**:
  - Supports automatic piece boundary alignment for hybrid torrents by inserting `.pad/<length>` padding files in the v1 `files` dictionary.
  - Added `FileDetails::is_pad_file()` helper to identify and handle padding files.
  - Seamlessly reconciles v1 `files` list and v2 `file tree`, associating 32-byte `pieces_root` to downloaded files both by relative normalized path and index.
- **Block-Level Merkle Verification (`TorrentContext`)**:
  - Extended `TorrentContext` with `file_merkle_proofs: HashMap<[u8; 32], Vec<MerkleProof>>` and `piece_layers: PieceLayersMap`.
  - Added `register_merkle_proof`, `register_piece_layer`, and `get_file_pieces_root`.
  - Implemented `verify_v2_block`: validates incoming 16 KiB data blocks directly against registered `MerkleProof` audit paths or canonical single-block pieces roots, discarding corrupt blocks before assembly.
- **Programmatic Torrent Construction (`TorrentBuilder`)**:
  - Implemented `TorrentBuilder` (`library/src/core/torrent_builder.rs`) supporting:
    - `.build_v1()`: Standard BitTorrent v1 `.torrent` files with SHA-1 piece hashes.
    - `.build_v2()`: Pure BitTorrent v2 files with hierarchical `file tree` dictionaries, `pieces root`, and `piece layers`.
    - `.build_hybrid()`: Dual-stack Hybrid files containing both piece-aligned v1 `pieces` (with pad files) and v2 `file tree` + `piece layers`.
- **Test Suite**: Dedicated integration test suite in `library/tests/bep52_v2_tests.rs` covering pure v2 torrent generation and parsing, hybrid generation and dual info-hashes, pad file insertion and detection, incremental streaming hashing from reader and disk, piece layer registration and hash consistency, and Merkle audit path proof verification.

---

### Phase 32: PEX (Peer Exchange — BEP 11) IPv6 (`added6` / `dropped6`) Support

Implemented full BitTorrent Peer Exchange (BEP 11 / `ut_pex`) with comprehensive dual-stack IPv4 and IPv6 support, peer flags, private torrent guards, and swarm differential generation:
- **BEP 11 Dual-Stack PEX Protocol Module (`network::pex`)**:
  - Implemented `PexMessage` handling `added` (IPv4 6-byte records), `added.f` (flags), `dropped` (IPv4), `added6` (IPv6 18-byte records), `added6.f` (flags), and `dropped6` (IPv6).
  - Enforced strict BEP 11 lexicographical bencode dictionary key ordering (`added` < `added.f` < `added6` < `added6.f` < `dropped` < `dropped6`).
  - Added `PexFlags` support for bit flags:
    - `0x01` (`PREF_ENCRYPT`): encryption preferred.
    - `0x02` (`SEED`): peer is a seed / upload only.
    - `0x04` (`UTP`): peer supports uTP (BEP 29).
    - `0x08` (`HOLEPUNCH`): peer supports holepunching (BEP 55).
    - `0x10` (`OUTGOING`): connection was initiated outgoing.
  - Implemented `parse_ipv4_octets` and `parse_ipv6_octets` supporting bracketed and plain IPv6 addresses.
- **Swarm Differential Generation (`TorrentContext::generate_pex_message`)**:
  - Compares the active `peer_swarm` against `sent_peers` per remote connection.
  - Generates incremental added and dropped candidate lists with automatic flag calculation.
  - Excludes the recipient peer from its own announcement.
  - Enforces BEP 11 recommendation of at most 50 added and 50 dropped peers per transmission.
- **Session & Peer Wire Integration**:
  - `run_peer_loop`: Periodically broadcasts differential dual-stack PEX messages to connected extension-supporting peers.
  - Incoming PEX handling (`Peer::handle_peer_message`): Decodes `ut_pex` payloads, filters out seeds when the local client is already a seed (`tc.status == TorrentStatus::Seeding` or complete), and dispatches `PeerAction::DiscoverPeers`.
  - Records dropped peers in `TorrentContext::pex_dropped_peers` (capped circular queue) and tracks statistics (`pex_added_peers_count`, `pex_dropped_peers_count`).
- **Privacy & Feature Gating**:
  - Private torrents (`is_private == true`): Omit `ut_pex` from extension handshake advertising, disable periodic PEX broadcasts, and ignore all incoming PEX messages (strict BEP 27 enforcement).
  - IPv6 gating: When `enable_ipv6 == false`, skips `added6` and `dropped6` generation and suppresses outgoing connections to discovered IPv6 endpoints.
- **Configuration & Builder**:
  - Added `pub enable_pex: bool` to `SessionConfig` (default: `true`) with `.with_enable_pex(bool)`.
  - Added `.enable_pex(bool)` to `TorrentSessionBuilder`.
- **Test Suite**: Dedicated integration test suite in `library/tests/pex_ipv6_tests.rs` covering dual-stack IPv4/IPv6 encode/decode roundtrips, canonical dictionary key order verification, BEP 11 flag bit operations, seed-to-seed connection filtering, IPv6 disabled suppression, private torrent enforcement, and differential swarm generation with capping.

---

### Phase 33: Super-Seeding Optimization with Suggest Piece (BEP 6 & BEP 16)

Implemented synergistic integration of BEP 6 Fast Extension (`Suggest Piece` and `Allowed Fast`) with BEP 16 Super-Seeding, dramatically accelerating initial piece distribution and eliminating stalling during unchoke intervals:
- **Fast Extension Super-Seeding Integration**:
  - Automatically identifies remote peers supporting BEP 6 Fast Extensions during super-seeding allocations.
  - When offering pieces (`offer_super_seed_piece`): Emits `PeerAction::SendHave(piece)`, `PeerAction::SendSuggest(piece)`, and `PeerAction::SendAllowedFast(piece)`.
  - The suggested piece allows BEP 6 clients to request the exact optimal piece advised by the super-seeder rather than picking randomly or waiting.
  - The `Allowed Fast` grant enables peers to immediately request and download their assigned piece even while choked, bypassing unchoke wait times.
  - Legacy peers (without BEP 6) continue to receive standard BEP 16 `Have` messages seamlessly.
- **Swarm Propagation Routing**:
  - When a piece is verified as propagated across the swarm via another peer's `Have` announcement (`on_peer_have`):
    - Credits the original peer and calculates their next optimal piece.
    - Routes `SendHaveToPeer`, `SendSuggestToPeer`, and `SendAllowedFastToPeer` directly to the credited peer.
- **Downloader Request Prioritization**:
  - `next_block_request_for_peer`: In standard downloading mode, prioritizes unrequested blocks from pieces suggested by the remote peer (`peer.suggested_pieces`) before general rarest-first selection.
  - `next_allowed_fast_block_request_for_peer`: When choked, prioritizes pieces that are both suggested and in the `allowed_fast_pieces` set.
  - Automatic cleanup: When a piece completes download and assembly, clears it from `suggested_pieces` and `allowed_fast_pieces`.
- **Metrics & Advisory Helpers**:
  - Added `total_pieces_offered` and `total_pieces_propagated` tracking in `SuperSeedingState`.
  - Added `TorrentContext::super_seed_stats()` returning `(total_pieces_offered, total_pieces_propagated)`.
  - Added `TorrentContext::suggest_next_piece_for_peer(&self, peer: &Peer)` helper for sequential streaming or advisory seeding.
- **Test Suite**: Dedicated integration test suite in `library/tests/super_seeding_suggest_tests.rs` covering BEP 6 `Suggest` and `AllowedFast` emissions during initial interest, multi-peer propagation routing with `SendSuggestToPeer`, downloader block request prioritization of suggested pieces, choked request prioritization, super-seeding metrics tracking, and advisory piece suggestion.

---

### Phase 34: BitTorrent v2 Peer Wire Messages (BEP 52: hash_request, hashes, hash_reject)

Implemented the full BEP 52 Hash Transfer Protocol for dynamic, on-demand exchange and cryptographic verification of SHA-256 Merkle tree layers between peers:
- **Wire Message Framing & Serialization (`PeerMessage`)**:
  - `HashRequest` (ID `21`): 48-byte payload containing `pieces_root` (`[u8; 32]`), `base_layer` (`u32`), `index` (`u32`), `length` (`u32`), and `proof_layers` (`u32`).
  - `Hashes` (ID `22`): 48-byte header followed by concatenated 32-byte SHA-256 hashes (`length` base hashes + ancestor uncle hashes).
  - `HashReject` (ID `23`): 48-byte payload matching the rejected `HashRequest` structure.
  - Zero-copy framing and decoding validation (detecting payload truncation and 32-byte alignment).
- **Cryptographic Hash Generation & Uncle Proof Verification (`MerkleTree`)**:
  - `MerkleTree::generate_hashes_response`: Gathers `length` base layer hashes and constructs uncle proof nodes towards the root. Omits the first $\log_2(\text{length}) - 1$ internal proof layers per BEP 52 specification.
  - `MerkleTree::verify_hashes_response`: Cryptographically verifies received base hashes by hashing pairwise up $\log_2(\text{length})$ levels, combining with subsequent uncle proof hashes, and authenticating against `pieces_root`.
- **Torrent Context & Block Verification Synergy (`TorrentContext`)**:
  - `handle_hash_request`: Resolves requested Merkle trees by `pieces_root` and generates `hashes` responses; returns rejection if unavailable.
  - `handle_hashes_response`: Authenticates incoming proof layers and registers verified base layer hashes in `verified_v2_hashes`.
  - `verify_v2_block`: Automatically references cached verified leaf hashes from `verified_v2_hashes` to authenticate incoming 16 KiB blocks instantly on-the-fly.
  - Metrics tracking: `v2_hash_stats()` tracks requests sent/received, hashes sent/received, and rejects sent/received.
- **Peer Wire Dispatch & Action Execution (`Peer`)**:
  - `handle_peer_message`: Automatically answers incoming `HashRequest` messages with `SendHashes` (when data is available) or `SendHashReject`.
  - `request_v2_hashes`: Helper method to initiate hash requests to remote peers.
  - `execute_actions`: Asynchronously serializes and writes `SendHashRequest`, `SendHashes`, and `SendHashReject` messages to the peer wire network.
- **Test Suite**: Dedicated integration test suite in `library/tests/bep52_peer_wire_tests.rs` covering wire format encode/decode roundtrips, malformed packet rejections, Merkle sub-tree proof generation with uncle validation, corruption detection, block verification integration, peer wire message handling, and stats tracking.

---

### Phase 35: Holepunch Extension (BEP 55: ut_holepunch NAT Traversal)

Implemented the complete BEP 55 `ut_holepunch` extension for peer-to-peer NAT and firewall traversal via intermediate rendezvous relay peers:
- **Binary Wire Framing & Serialization (`HolepunchMessage`)**:
  - Implemented BEP 55 binary packet encoding and decoding:
    - 1-byte `msg_type`: `Rendezvous` (`0x00`), `Connect` (`0x01`), `Error` (`0x02`).
    - 1-byte `addr_type`: `IPv4` (`0x00`, 4 octets) and `IPv6` (`0x01`, 16 octets).
    - 2-byte `port` (big-endian `u16`).
    - 4-byte `err_code` (big-endian `u32`): `None` (`0x00`), `NoSuchPeer` (`0x01`), `NotConnected` (`0x02`), `NoSupport` (`0x03`), `NoSelf` (`0x04`).
  - Total payload: 12 bytes for IPv4 endpoints, 24 bytes for IPv6 endpoints.
  - Strict input validation rejecting truncated payloads, unknown message types, invalid address types, and malformed error codes.
- **Relay Coordination & Validation (`TorrentContext`)**:
  - `handle_holepunch_rendezvous`: Validates rendezvous requests when acting as an intermediate relay peer:
    - `NoSelf`: Rejects requests where the target endpoint is the relay peer itself (local loopback/all-interfaces address with listen port).
    - `NotConnected`: Rejects requests if target peer is not currently connected in `peer_swarm`.
    - `NoSupport`: Rejects requests if connected target peer does not advertise `ut_holepunch` support.
    - On validation success: Generates dual `Connect` messages for both the initiator and the target peer with each other's endpoints.
  - Performance metrics: Added atomic counters `holepunch_rendezvous_sent`, `holepunch_rendezvous_received`, `holepunch_connect_sent`, `holepunch_connect_received`, `holepunch_error_sent`, and `holepunch_error_received`.
  - Added `TorrentContext::holepunch_stats()` returning snapshot of all holepunch telemetry.
- **Peer Wire Protocol & Worker Integration (`Peer`, `worker.rs`)**:
  - Advertises `"ut_holepunch": 3` in extension handshake dictionary `m` during connection negotiation (configurable via `enable_holepunch`).
  - Detects remote peer `ut_holepunch` support in incoming extension handshake dictionary.
  - Dispatches incoming `ext_id == 3` messages:
    - `Rendezvous`: Invokes relay logic and emits `PeerAction::SendHolepunch` containing dual `Connect` messages or an `Error` response.
    - `Connect`: Checks swarm presence and dispatches `PeerAction::HolepunchConnect(addr)` to asynchronously initiate connection if not already in swarm.
    - `Error`: Updates telemetry metrics.
  - Added `Peer::initiate_holepunch(target_addr, tc)` helper to request rendezvous through a connected peer.
  - Handled `PeerAction::SendHolepunch` and `PeerAction::HolepunchConnect` in `execute_actions`.
- **Configuration & Builder Options**:
  - Added `enable_holepunch: bool` (default `true`) to `SessionConfig` and `TorrentSessionBuilder::enable_holepunch(bool)`.
- **Test Suite**: Dedicated integration test suite in `library/tests/holepunch_tests.rs` (11/11 tests passing) covering dual-stack IPv4/IPv6 message encode/decode, malformed input rejection, relay rendezvous validation and error code handling, message dispatching and metrics tracking, duplicate connection suppression, and builder configuration.

---

### Phase 36: WebTorrent WebSocket Tracker Signaling & Bridge Mode

Implemented full browser-to-desktop bridging via WebTorrent signaling channels (`wss://` and `ws://` WebSocket tracker protocols) and WebRTC Data Channels:
- **WebSocket Signaling Protocol & Models (`webrtc.rs`)**:
  - `WebTorrentAnnounceRequest`: JSON announce payloads with multi-offers (`Vec<WebTorrentOffer>`), event signaling (`started`, `completed`, `stopped`), `uploaded`, `downloaded`, `left`, and `numwant`.
  - `WebTorrentAnswer`: Formats SDP answer objects with `to_peer_id` targeting specific browser peers.
  - `WebTorrentScrapeRequest` & `WebTorrentScrapeResponse`: Full JSON scrape protocol support for querying complete, incomplete, and downloaded swarm stats across multiple info-hashes on WebSocket trackers.
  - `WebTorrentIncomingMessage`: Clean demuxing of inbound WebSocket tracker frames into `PeerAnnouncement`, `InboundOffer`, `InboundAnswer`, and `Error`.
- **WebSocket Transport Abstraction (`WebSocketSignalingClient`)**:
  - Extensible `WebSocketSignalingClient` trait for dispatching WebSocket frames to trackers.
  - In-memory `MockWebSocketSignalingChannel` for deterministic testing and loopback announce/scrape verification.
  - `WebTorrentAnnouncer::with_client` and `set_signaling_client` integration with `Announcer` trait methods `announce` and `scrape`.
- **WebTorrent Swarm Bridge Coordinator (`WebTorrentBridge`)**:
  - `WebTorrentBridge`: Coordinates outgoing pending offers, resolves inbound answers to activate WebRTC sockets, and converts incoming browser offers into answers and active peer connections.
  - Seamless socket binding in `worker.rs`: `perform_handshake` automatically detects `peer_details.is_webrtc()` (`webrtc:<peer_id>`) and attaches the paired `Socket::WebRtc` data channel without attempting standard TCP connections.
  - Cross-swarm data bridging: Pieces downloaded from standard TCP/uTP peers are verified, recorded in the bitfield, and served to WebRTC browser peers; pieces received from browser peers are written to storage and advertised to TCP/uTP swarms via `Have` broadcasts.
  - Telemetry: Metrics for `offers_sent`, `offers_received`, `answers_sent`, `answers_received`, `tcp_to_webrtc_bytes`, `webrtc_to_tcp_bytes`, and active WebRTC peer count.
- **Session Configuration & Builder**:
  - `enable_webtorrent: bool` (default `true`) in `SessionConfig` and `TorrentSessionBuilder::enable_webtorrent(bool)`.
  - `webtorrent_bridge_mode: bool` (default `true`) in `SessionConfig` and `TorrentSessionBuilder::webtorrent_bridge_mode(bool)`.
  - `webtorrent_max_web_peers: usize` (default `25`) in `SessionConfig` and `TorrentSessionBuilder::webtorrent_max_web_peers(usize)`.
  - `TorrentContext::webtorrent_bridge_stats()` and `record_webrtc_bridge_transfer()` integration.
- **Test Suite**: Dedicated integration test suite in `library/tests/webtorrent_bridge_tests.rs` (8/8 tests passing) alongside `library/tests/webrtc_tests.rs` (7/7 tests passing) covering multi-offer announce construction, scrape exchange, incoming message demuxing, bridge offer/answer handshake, mock WebSocket tracker signaling, cross-swarm piece distribution, and builder configuration.

---

### Phase 37: BitTorrent v2 Hash-Tree Torrent Exporter & Infohash Upgrade Tool (BEP 52)

Implemented offline and session-level conversion and export tooling for upgrading legacy BitTorrent v1 `.torrent` files into BitTorrent v2 (pure v2) or dual-compatible Hybrid (v1 + v2) metadata:
- **Merkle Tree & File Tree Exporter (`TorrentUpgrader`)**:
  - `TorrentUpgrader::upgrade_metainfo`: Accepts parsed `MetaInfoFile` and in-memory files data, calculating SHA-256 Merkle trees (`MerkleTree`), piece layers, and canonical `file tree` structures per BEP 52.
  - Automatically aligns piece lengths to power-of-two values $\ge 16\text{ KiB}$ (`BLOCK_SIZE = 16384`) conforming to BEP 52 constraints.
  - Supports target formats (`TorrentExportFormat`): `V1`, `V2` (pure v2), and `Hybrid` (v1 + v2).
  - `TorrentUpgrader::upgrade_from_file`: Reads legacy `.torrent` metainfo and verifies content files directly on disk, upgrading them to hybrid or pure v2 format without manual hashing steps.
- **Detailed Upgrade & Export Report (`TorrentUpgradeReport`)**:
  - Returns structured telemetry: torrent name, export format, total combined file size, piece length, v1 SHA-1 info-hash (if hybrid/v1), 32-byte SHA-256 v2 info-hash (if hybrid/v2), 20-byte truncated v2 info-hash prefix for compact swarms, per-file `pieces root` Merkle hashes, and piece layers count.
  - `TorrentUpgrader::generate_magnet_uri`: Builds canonical Magnet URIs featuring dual-hash parameters when applicable (`xt=urn:btih:<v1_hex>` and `xt=urn:btmh:1220<v2_hex>`), display name (`dn`), and tracker endpoints (`tr`).
- **Session-Level Direct Torrent Exporter**:
  - `TorrentSession::export_torrent(&self, format: TorrentExportFormat)`: Exports an active, downloading, or seeding session directly from disk/block storage into pure v2, hybrid, or v1 bencoded `.torrent` bytes along with an upgrade report.
- **CLI Companion Tooling (`bt-cli`)**:
  - `bt-cli upgrade <input.torrent> <data-dir> [--format <hybrid|v2>] [--output <output.torrent>]`: Upgrades a legacy torrent file using downloaded data on disk and writes the output file with a complete summary table and canonical Magnet URI.
  - `bt-cli v2-info <torrent-path>`: Inspects any torrent file and displays format (v1, hybrid, pure v2), piece length, v1 info-hash, v2 info-hash, truncated v2 info-hash, piece layers count, and per-file Merkle `pieces root` hashes.
- **Test Suite**: Dedicated integration test suite in `library/tests/torrent_upgrader_tests.rs` (5/5 tests passing) covering single-file v1 to pure v2 upgrade, multi-file v1 to hybrid conversion with padding alignment, piece length clamping to BEP 52 powers of 2, missing file error handling, and `TorrentSession::export_torrent` direct export.

---

## Future Roadmap

The following enhancements are proposed for future development:

1. **uTP SACK Dynamic Out-of-Order Packet Pipeline**: Enhanced SACK bitmap reassembly optimization and burst loss recovery in high-latency network conditions.

2. **WebTorrent Magnet URI Web-Seed & WebSocket Tracker Injector**: Automated extraction of `ws=` and `wss=` trackers from Magnet links for instant browser bridge bootstrapping.

3. **BitTorrent v2 Piece Layers Disk Cache & Peer Wire Serving**: Persistent on-disk caching of intermediate Merkle tree layers to serve incoming BEP 52 `hash_request` messages with minimal I/O overhead.





