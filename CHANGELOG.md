# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- Implemented full WebTorrent WebSocket tracker signaling (`wss://`/`ws://`) and cross-swarm bridge mode (`WebTorrentBridge`) connecting desktop BitTorrent swarms with browser WebRTC Data Channels.
- Added Holepunch Extension (BEP 55: `ut_holepunch`) for peer-to-peer NAT traversal via intermediate rendezvous relay peers.
- Created `docs/api-stability.md` to document the stable public API versus internal API surfaces.
- Comprehensive inline Rustdoc comments across `TorrentSession`, `TorrentSessionBuilder`, `TorrentContext`, and `Tracker`, including `# Examples`, `# Errors`, and `# Panics` sections.

### Refactored
- Added segregated `BlockReader` and `BlockWriter` traits to `io_traits.rs` to adhere to the Interface Segregation Principle (ISP).
- Introduced `SwarmReadAccess`, `PieceWriteAccess`, and `SessionStatusQuery` trait views on `TorrentContext`.
- Extracted `Announcer` trait to support extensible tracker announcer implementations (OCP).
- Added comprehensive SOLID refactoring plan in `docs/solid-refactor-plan.md`.
