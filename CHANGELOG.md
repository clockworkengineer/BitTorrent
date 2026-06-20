# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- Created `docs/api-stability.md` to document the stable public API versus internal API surfaces.
- Comprehensive inline Rustdoc comments across `TorrentSession`, `TorrentSessionBuilder`, `TorrentContext`, and `Tracker`, including `# Examples`, `# Errors`, and `# Panics` sections.

### Changed
- Added a `5`-second connection and read timeout to `UreqHttpClient` to prevent indefinite blocking/hangs on offline trackers.
- Refactored `session_tests.rs` to mock the HTTP tracker announcer (`DummyHttpClient`), eliminating external network calls in unit tests and accelerating test execution.
- Conserved consistency by replacing standard `println!` logging in `Tracker::log_announce` with `log_debug!`.
