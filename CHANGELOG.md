# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
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
