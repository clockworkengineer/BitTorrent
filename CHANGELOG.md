# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- Created `docs/api-stability.md` to document the stable public API versus internal API surfaces.
- Comprehensive inline Rustdoc comments across `TorrentSession`, `TorrentSessionBuilder`, `TorrentContext`, and `Tracker`, including `# Examples`, `# Errors`, and `# Panics` sections.

### Refactored
- Added segregated `BlockReader` and `BlockWriter` traits to `io_traits.rs` to adhere to the Interface Segregation Principle (ISP).
- Introduced `SwarmReadAccess`, `PieceWriteAccess`, and `SessionStatusQuery` trait views on `TorrentContext`.
- Extracted `Announcer` trait to support extensible tracker announcer implementations (OCP).
- Added comprehensive SOLID refactoring plan in `docs/solid-refactor-plan.md`.
