# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- Implemented BEP 38 / WebTorrent Dynamic WebSeed Failover & Mirror Health Tracker (`WebSeedMirrorTracker`, `MirrorHealthProfile`, `MirrorHealthSummary`), providing multi-source HTTP fallback routing, exponential circuit-breaker cooldown backoff, real-time EMA latency smoothing, and dynamic health-based mirror ranking across WebSeed endpoints.
- Integrated dynamic mirror selection and runtime fallback in `webseed::start_webseed_loop`, automatically routing requests to fastest healthy mirrors, bypassing failing endpoints, and probing recovering endpoints.
- Added WebSeed health tracker configuration in `SessionConfig` and `TorrentSessionBuilder` (`enable_webseed_health_tracker`, `webseed_failure_threshold`, `webseed_base_cooldown_secs`, `webseed_max_cooldown_secs`, `webseed_timeout_secs`) and runtime management APIs in `TorrentSession` (`webseed_mirror_summary`, `add_webseed`, `remove_webseed`).
- Created dedicated integration test suite `library/tests/webseed_failover_tests.rs` (6/6 tests passing).
- Implemented Endgame Mode Multi-Source Adaptive Erasure Caching (`AdaptiveErasureStaging`, `StagedBlock`, `BlockArrivalOutcome`, `EndgameStagingMetrics`, `SwarmLatencyProfiler`, `PeerLatencyProfile`, `PeerLatencySummary`) providing predictive erasure block staging, RFC 6298 EMA RTT/jitter latency profiling, and Expected Completion Time (ECT) tracking to eliminate tail completion stalls on slow peer pipelines.
- Integrated straggler detection and predictive preemption in `TorrentContext::next_block_request_for_peer`, proactively detecting in-flight blocks exceeding their completion window and cloning them to faster peers.
- Added in-memory redundant duplicate block absorption in `TorrentContext::process_piece_block` and targeted `Cancel` broadcasting, preventing redundant disk writes and hash verifications when late duplicate blocks arrive from preempted peers.
- Added configuration options in `SessionConfig` and `TorrentSessionBuilder` (`enable_adaptive_endgame`, `endgame_straggler_ratio`, `endgame_max_clones_per_block`) and runtime telemetry APIs in `TorrentSession` (`endgame_metrics`, `swarm_latency_summary`).
- Created dedicated integration test suite `library/tests/endgame_erasure_tests.rs` (6/6 tests passing).
- Implemented BEP 53 Selective Magnet Download & Range-Based Metadata Fetching (`MagnetLink::select_only`, `parse_bep53_select_only`, `format_bep53_select_only`, `MetadataRangeFetcher`, `PartialMetadataManifest`, `BEP53SelectionEvaluation`), enabling selective downloading via `so=` query parameters, canonical contiguous range compression serialization, multi-peer metadata range pipelining, expedited partial file manifest parsing, and comprehensive byte/piece savings evaluation.
- Added selective download configuration and runtime methods in `TorrentSessionBuilder` (`select_files`, `select_file_range`, `metadata_pipeline_depth`, `metadata_request_timeout`) and `TorrentSession` (`selected_files`, `select_files`, `bep53_evaluation`).
- Integrated BEP 53 selection filtering into `TorrentContext` (`selected_file_indices`, `set_selected_files`, `apply_bep53_file_selection`), ensuring unselected files are marked `FilePriority::Skip` and non-overlapping pieces are skipped in piece selectors (`RarestFirstSelector`, `SequentialSelector`, `StreamingPieceSelector`).
- Extended CLI companion command `bt-cli magnet-info <magnet-uri> [--select <indices>]` to inspect BEP 53 file selection and rewrite updated Magnet URIs.
- Implemented Asynchronous Piece Hash Verification Worker Pool (`HashVerificationPool`, `PieceVerificationJob`, `PieceVerificationResult`, `VerificationPoolMetrics`, `RecheckProgress`) providing multi-threaded background SHA-1 and v2 SHA-256 Merkle root verification offloaded from the cooperative async event loop, bounded queue depth tracking, telemetry counters, and thread-safe worker shutdown.
- Added parallel whole-torrent piece rechecking (`recheck_pieces_parallel`, `recheck_pieces_blocking`, `create_torrent_bitfield_parallel`) with real-time atomic progress monitoring and throughput reporting.
- Integrated in-flight piece verification guard set (`pieces_verifying`, `is_piece_verifying`, `mark_piece_verifying`) in `TorrentContext` and piece selectors (`RarestFirstSelector`, `SequentialSelector`, `StreamingPieceSelector`) to prevent redundant duplicate block requests and downloads while pieces are in-flight in verification worker threads.
- Added verification pool configuration options in `SessionConfig` and `TorrentSessionBuilder` (`enable_async_verification`, `verification_workers`, `verification_queue_capacity`) and session APIs in `TorrentSession` (`drain_verification_results`, `verification_metrics`, `recheck_pieces`).
- Added CLI companion command `bt-cli recheck <torrent-path> <data-dir> [--workers <N>]` with parallel hashing, real-time progress bars, and throughput reporting.
- Implemented BitTorrent v2 Piece Layers Disk Cache & Peer Wire Serving (`PieceLayersDiskCache`, `PieceLayersCacheMetrics`, `encode_layers`, `decode_layers`, `reconstruct_tree_from_piece_layer`) providing two-tier LRU memory and disk-backed caching with binary `.layers` format and SHA-256 integrity checksums, automatic ancestor layer reconstruction from piece layers, and direct BEP 52 `hash_request` peer wire serving.
- Extended `SessionConfig` and `TorrentSessionBuilder` with `v2_piece_layers_cache_enabled`, `v2_piece_layers_cache_dir`, and `v2_piece_layers_cache_max_mb`.
- Integrated `handle_hash_request`, `register_file_merkle_tree`, and `register_piece_layer` into `TorrentContext`.
- Added layer disk cache inspection and summary telemetry to `bt-cli v2-info`.
- Implemented WebTorrent Magnet URI Web-Seed & WebSocket Tracker Injector (`WebTorrentMagnetInjector`, `MagnetUriBuilder`, `TrackerExtractionReport`, `InjectionSummary`) supporting automated extraction of BEP 19 WebSeeds (`ws=`), exact sources (`xs=`), and WebSocket trackers (`wss://`/`ws://`), canonical RFC 3986 percent-encoding serialization, BEP 52 v1/v2 dual-hash parsing in hybrid swarms, and automatic public WebTorrent signaling injection with instant bridge pre-offer generation.
- Added CLI companion command `bt-cli magnet-info <magnet-uri> [--inject-webtorrent] [--web-seed <url>]` for inspecting, analyzing, and rewriting Magnet URIs with injected WebSeeds and WebSocket trackers.
- Implemented uTP SACK Dynamic Out-of-Order Packet Pipeline & Burst Loss Recovery (`UtpSocketAdapter`, `LedbatCongestionController`, `UtpPipelineMetrics`, `LossRecoveryState`, `detect_receive_gaps`) featuring SACK-driven fast loss detection without RTO delay, RFC 6298 microsecond RTT/RTO tracking with exponential backoff, burst loss scaling (`0.65 * cwnd`), and bounded out-of-order reassembly buffers protecting against memory exhaustion attacks.
- Implemented BitTorrent v2 Hash-Tree Torrent Exporter & Infohash Upgrade Tool (`TorrentUpgrader`, `TorrentExportFormat`, `TorrentUpgradeReport`) with BEP 52 Merkle tree generation, piece layers calculation, dual Magnet URI generation, and `TorrentSession::export_torrent` direct session export.
- Added CLI companion commands `bt-cli upgrade <input.torrent> <data-dir>` and `bt-cli v2-info <torrent-path>` for inspecting and upgrading torrent files offline.
- Implemented full WebTorrent WebSocket tracker signaling (`wss://`/`ws://`) and cross-swarm bridge mode (`WebTorrentBridge`) connecting desktop BitTorrent swarms with browser WebRTC Data Channels.
- Added Holepunch Extension (BEP 55: `ut_holepunch`) for peer-to-peer NAT traversal via intermediate rendezvous relay peers.
- Created `docs/api-stability.md` to document the stable public API versus internal API surfaces.
- Comprehensive inline Rustdoc comments across `TorrentSession`, `TorrentSessionBuilder`, `TorrentContext`, and `Tracker`, including `# Examples`, `# Errors`, and `# Panics` sections.

### Refactored
- Added segregated `BlockReader` and `BlockWriter` traits to `io_traits.rs` to adhere to the Interface Segregation Principle (ISP).
- Introduced `SwarmReadAccess`, `PieceWriteAccess`, and `SessionStatusQuery` trait views on `TorrentContext`.
- Extracted `Announcer` trait to support extensible tracker announcer implementations (OCP).
- Added comprehensive SOLID refactoring plan in `docs/solid-refactor-plan.md`.

### Fixed
- Fixed transfer rate flickering to 0 B/s in `TorrentContext` by applying Exponential Moving Average (EMA) smoothing (70% previous, 30% sample) and graceful rate decay rather than instantaneous resets when no block completed in an update interval.
- Fixed peer discovery staying at 0 peers:
  - Fixed premature state check in `Tracker::queue_new_peers` that was silently discarding all peers discovered on initial announce because `TorrentSession::start_download` had not yet transitioned `TorrentContext` status from `Initialised` to `Downloading`.
  - Moved `context.start_downloading()` to the start of `TorrentSession::start_download()` so the session is in active downloading state before peer worker receivers, DHT, and tracker announcers start.
  - Relaxed worker candidate receiver in `session.rs` to accept peers unless the session is explicitly paused or ended.
  - Enhanced `Tracker::announce_with_fallback` to accumulate and deduplicate peer candidates across all configured tier trackers instead of stopping prematurely at the first tracker if it returned 0 peers.
  - Fixed `TorrentContext::add_peer` return value when inserting into `peer_swarm` and added handshake read retries in `peer_network.rs`.
- Fixed peer connections dropping and resetting:
  - Expanded `worker.rs` peer message buffer from 16 KiB to 128 KiB to support large bitfields and extended handshakes without parse errors.
  - Increased block request timeout from 2s to 15s to prevent premature cancellations and choking.
  - Fixed premature peer loop breakout in `worker.rs` when downloads complete, allowing seed connections to remain active.
  - Added partial-frame retry in `peer_network.rs` (`read_exact` and `read_message`) on `WouldBlock`/`TimedOut` to prevent stream framing desynchronization.
  - Added peer count retention grace period in `SessionState` and stabilized UI status bar speed aggregation in `app.rs`.
- Fixed 15-second delay when adding or restoring torrent connections:
  - Offloaded initial tracker announcement from `TorrentSession::start_download` into the background asynchronous executor task spawned by `start_reannounce_loop`, allowing session creation and UI addition to return instantly in < 1ms without blocking on external tracker network round-trips.
  - Streamlined `Tracker::announce_with_fallback` failover by avoiding redundant multi-attempt retry delays per tracker endpoint, failing over immediately to secondary tiers when a tracker is unreachable.
- Fixed 10-second hang when closing the client application window:
  - Replaced `Arc<Mutex<TcpStream>>` in `TcpSocket` with `Arc<TcpStream>`, allowing `close()` to trigger an immediate `shutdown(Shutdown::Both)` on the underlying OS socket. This unblocks active peer worker `read` calls immediately without waiting for mutex locks or 5-second socket read timeouts.
  - Added clean termination (`std::process::exit(0)`) in desktop client `on_exit` handler after stopping active sessions and saving application configuration, instantly halting daemon UDP/DHT listeners without delaying window closure.
