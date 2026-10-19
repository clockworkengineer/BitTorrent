# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
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
