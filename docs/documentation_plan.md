# Comprehensive Documentation Modernization Plan

## Executive Summary

The **BitTorrent-rs** codebase has evolved into an enterprise-grade, `#![no_std]`-capable BitTorrent ecosystem spanning 44 implementation phases, 6 client applications/tools, 8 runnable examples, and complete support for 24 BitTorrent Enhancement Proposals (BEPs).

However, while `docs/roadmap.md` and test suites rigorously capture modern capabilities, several existing documentation files (`README.md`, `docs/architecture.md`, `docs/api-reference.md`, `docs/utp.md`, `docs/nat-pmp.md`, `clients/README.md`) contain obsolete sections, inaccurate "future work" claims for already-completed features, outdated API signatures, or broken file paths. Furthermore, several critical subsystems (BEP compliance matrix, seekable media streaming, BitTorrent v2 Merkle verification, WebTorrent WebRTC bridge, CLI reference manual, daemon/WebUI operational guide, and packaging/installer documentation) lack dedicated documentation files.

This plan details the audit findings and provides an actionable blueprint to **modify existing documents**, **author new technical and user-facing documents**, and **enhance code docstrings** across the workspace.

---

## 1. Source Analysis & Documentation Audit Findings

### 1.1 Inaccuracies & Outdated Sections in Existing Docs

| Document | Current Inaccuracy / Gap | Correct Current State in Code |
| :--- | :--- | :--- |
| [`README.md`](file:///c:/Projects/BitTorrent/README.md) | Feature list stops at Phase 16; mentions only basic v2 parsing and basic uTP; lacks WebTorrent, super-seeding, streaming, disk cache, verification pool, installer, daemon, CLI, and WebUI. | Full ecosystem implemented (Phases 1–44); 5 binaries in `clients/`; Windows installer & portable zip; dual v1/v2 hybrid support; seekable media streaming. |
| [`docs/architecture.md`](file:///c:/Projects/BitTorrent/docs/architecture.md) | Section *"Known Gaps & Future Work"* (lines 157–164) claims LEDBAT, UPnP/SSDP, BEP 52 Merkle validation, Tit-for-tat, and Optimistic unchoking are not implemented. | **All five are implemented and fully tested**: LEDBAT in `utp.rs`, UPnP in `nat.rs`, Merkle validation in `merkle.rs`, and choking strategies in `choking_policy.rs`. |
| [`docs/architecture.md`](file:///c:/Projects/BitTorrent/docs/architecture.md) | Missing architectural diagrams and component descriptions for v2 Merkle engine, disk cache write coalescing, async verification pool, WebTorrent bridge, and IPC architecture. | Need to document `MerkleTree`, `CachedBlockStorage`, `HashVerificationPool`, `WebTorrentBridge`, and IPC architecture. |
| [`docs/api-reference.md`](file:///c:/Projects/BitTorrent/docs/api-reference.md) | `SessionConfig` shows only 10 fields; `TorrentSessionBuilder` and `TorrentSession` show minimal Phase 1–7 APIs. | `SessionConfig` contains >25 configuration options; `TorrentSessionBuilder` and `TorrentSession` have extensive runtime control and telemetry APIs. |
| [`docs/utp.md`](file:///c:/Projects/BitTorrent/docs/utp.md) | Section *"Future Work"* claims SACK (Selective Acknowledgment) is unimplemented; references obsolete file path `library/src/utp.rs`. | SACK (BEP 29), out-of-order reassembly, and burst loss recovery are fully implemented; path is `library/src/network/utp.rs`. |
| [`docs/nat-pmp.md`](file:///c:/Projects/BitTorrent/docs/nat-pmp.md) | File references point to `library/src/nat.rs` and `library/src/session.rs` instead of submodules. | Source files are at `library/src/network/nat.rs` and `library/src/session/session.rs`. |
| [`docs/dht.md`](file:///c:/Projects/BitTorrent/docs/dht.md) | Lacks coverage of BEP 32 IPv6 compact node/peer parsing (`nodes6`, `values`) and DHT scrape (`BEP 51`). | Implemented in `library/src/network/dht.rs`. |
| [`clients/README.md`](file:///c:/Projects/BitTorrent/clients/README.md) | Only documents `torrent_client`. | Workspace contains `torrent_client`, `bt_daemon`, `bt_webui`, `sync_hub`, and `torrent_client_shared`. |
| [`docs/README.md`](file:///c:/Projects/BitTorrent/docs/README.md) | Index does not list `user-guide.md`, `sync-hub.md`, or the new architecture documents. | Needs complete reorganization and updated sitemap. |

---

## 2. Action Plan: Modifications to Existing Documents

### 2.1 Update [`README.md`](file:///c:/Projects/BitTorrent/README.md)
- **Badges**: Update version badge and documentation links.
- **Comprehensive Feature Matrix**:
  - Full BitTorrent v1, v2 (BEP 52), and Hybrid swarms with SHA-256 Merkle block validation.
  - WebTorrent WebRTC data channel bridge and WebSocket tracker signaling (`ws://`/`wss://`).
  - uTP (BEP 29) with LEDBAT congestion control (RFC 6817), SACK out-of-order reassembly, and fast loss recovery.
  - Multi-tier disk write cache with contiguous block coalescing and corrupt block discarding.
  - Multi-threaded cryptographic hash verification pool (`HashVerificationPool`).
  - Seekable media streaming reader (`TorrentStream`) and embedded HTTP 206 partial content server.
  - Super-seeding initial distribution mode (BEP 16) with BEP 6 fast extension synergy.
  - Adaptive endgame mode with erasure block staging and RFC 6298 latency profiling.
  - BEP 38 WebSeed dynamic failover with circuit-breaker health tracking.
  - Dual-stack IPv4/IPv6 support across trackers, DHT, PEX, and wire sockets.
  - UPnP (SSDP/IGD) and NAT-PMP automated port mapping.
- **Getting Started Section**:
  - Add instructions for Desktop GUI (`torrent_client`), Headless Daemon (`bt_daemon`), Web UI (`bt_webui`), CLI tool (`bt-cli`), and Sync Hub (`sync_hub`).
  - Add download links and usage for pre-packaged Windows Setup Installer (`BitTorrent-Setup-v0.1.1.exe`) and Portable ZIP release.
- **Project Structure**:
  - Expand `clients/` subproject descriptions (`bt_daemon`, `bt_webui`, `sync_hub`, `torrent_client`, `torrent_client_shared`).
  - Expand `examples/` listing all 8 runnable examples.

### 2.2 Modernize [`docs/architecture.md`](file:///c:/Projects/BitTorrent/docs/architecture.md)
- **Remove Obsolete "Known Gaps"**:
  - Delete claims that LEDBAT, UPnP, BEP 52 Merkle trees, and Tit-for-tat choking are unimplemented.
  - Replace with current architecture descriptions of these subsystems.
- **Add Key Architecture Components**:
  - **Storage Architecture**: `CachedBlockStorage` write coalescing, `PieceLayersDiskCache` `.layers` file format, and `HashVerificationPool` worker architecture.
  - **Streaming Architecture**: `TorrentStream`, `StreamingPieceSelector`, cursor lookahead window, and embedded HTTP 206 server.
  - **BitTorrent v2 Merkle Engine**: Power-of-two padded binary Merkle tree, leaf verification, uncle proof paths, and `TorrentUpgrader`.
  - **Transport & WebTorrent Layer**: `UtpSocketAdapter` with SACK and LEDBAT, `WebRtcSocketAdapter`, and cross-swarm TCP/WebRTC bridge.
  - **IPC & Multi-Process Architecture**: Domain socket / Named Pipe protocol between `bt_daemon`, `bt-cli`, and `bt_webui`.
- **Update End-to-End Data Flow Diagram**:
  - Incorporate fast-resume loading, async verification, disk cache flushing, WebTorrent signaling, and super-seeding lifecycle.

### 2.3 Overhaul [`docs/api-reference.md`](file:///c:/Projects/BitTorrent/docs/api-reference.md)
- **`SessionConfig`**: Document all configuration fields including IPv6, disk cache size, super-seeding, holepunching, WebTorrent bridge, BEP 52 piece layers cache, async verification workers, adaptive endgame, and WebSeed health tracker.
- **`TorrentSessionBuilder`**: Document all builder methods (`from_magnet`, `fast_resume`, `super_seeding`, `enable_webtorrent`, `disk_cache_size_mb`, `enable_async_verification`, `enable_adaptive_endgame`, `with_web_seeds`, etc.).
- **`TorrentSession`**: Document all session control and inspection methods (`export_torrent`, `create_stream`, `create_file_stream`, `set_file_priority`, `recheck_pieces`, `generate_fast_resume`, `save_fast_resume`, `endgame_metrics`, `webseed_mirror_summary`, etc.).
- **Add Missing Core Types**:
  - `TorrentBuilder` and `TorrentUpgrader`
  - `FastResumeData` and `FastResumeFileEntry`
  - `MerkleTree`, `MerkleProof`, and `PieceLayersDiskCache`
  - `HashVerificationPool` and `VerificationPoolMetrics`
  - `SwarmLatencyProfiler`, `AdaptiveErasureStaging`, and `EndgameStagingMetrics`
  - `WebSeedMirrorTracker` and `MirrorHealthSummary`
  - `MagnetLink` and `MagnetUriBuilder` (with BEP 53 file selection)

### 2.4 Update [`docs/utp.md`](file:///c:/Projects/BitTorrent/docs/utp.md)
- Correct source file link to [`library/src/network/utp.rs`](file:///c:/Projects/BitTorrent/library/src/network/utp.rs).
- Document SACK (Selective Acknowledgment) extension header parsing and wire format.
- Document circular 16-bit sequence number gap detection (`detect_receive_gaps`) and out-of-order reassembly queue (`BTreeMap<u16, Vec<u8>>`).
- Document RFC 6298 microsecond RTT/RTO tracking and exponential timeout backoff.
- Update "Future Work" to reflect that SACK and LEDBAT are complete.

### 2.5 Update [`docs/nat-pmp.md`](file:///c:/Projects/BitTorrent/docs/nat-pmp.md)
- Fix relative file links to point to [`library/src/network/nat.rs`](file:///c:/Projects/BitTorrent/library/src/network/nat.rs) and [`library/src/session/session.rs`](file:///c:/Projects/BitTorrent/library/src/session/session.rs).
- Expand the UPnP (SSDP/IGD) section with details on the fallback mechanism (`FallbackPortMapper`).

### 2.6 Update [`docs/dht.md`](file:///c:/Projects/BitTorrent/docs/dht.md)
- Add documentation for BEP 32 IPv6 compact node/peer parsing (`nodes6`, `values`).
- Add documentation for BEP 51 DHT scrape protocol.
- Fix any outdated source file links.

### 2.7 Update [`clients/README.md`](file:///c:/Projects/BitTorrent/clients/README.md)
- Provide a summary and build/run command for all 5 client projects:
  - `torrent_client`: Graphical desktop client (`egui`/`eframe`) with streaming player and swarm visualizer.
  - `bt_daemon`: Headless server daemon with IPC service, embedded HTTP streaming server, and auto-state persistence.
  - `bt_webui`: Glassmorphic web dashboard gateway connecting to `bt_daemon`.
  - `sync_hub`: Decentralized local-first LAN folder synchronization daemon.
  - `torrent_client_shared`: Shared telemetry models, IPC wire protocols, and formatting utilities.

### 2.8 Update [`docs/README.md`](file:///c:/Projects/BitTorrent/docs/README.md)
- Reorganize into clear categories:
  - **Architecture & Specifications**
  - **Client & Tool Guides**
  - **Protocol Technical Deep Dives**
  - **Developer, Portability & Performance References**
- Add clickable links to all existing and new documentation files.

---

## 3. Action Plan: New Documents to Add

### 3.1 [`docs/bep-compliance.md`](file:///c:/Projects/BitTorrent/docs/bep-compliance.md) — BEP Compliance & Standards Matrix
- **Purpose**: Authoritative reference of all BitTorrent Enhancement Proposals (BEPs) implemented in the library.
- **Contents**:
  - Detailed matrix of 24 BEPs (BEP 3, 5, 6, 7, 9, 10, 11, 14, 15, 16, 17, 19, 21, 24, 27, 29, 32, 38, 47, 48, 51, 52, 53, 55).
  - Status for each BEP: Full, Partial, or Extended.
  - Code reference links to modules implementing each BEP.
  - Technical notes on protocol conformance, edge cases, and interoperability testing.

### 3.2 [`docs/streaming.md`](file:///c:/Projects/BitTorrent/docs/streaming.md) — Media Streaming Architecture & Guide
- **Purpose**: Comprehensive guide to seekable media streaming within BitTorrent-rs.
- **Contents**:
  - `TorrentStream` architecture (`Read + Seek` traits).
  - `StreamingPieceSelector` dynamic lookahead window and atomic playback cursor tracking.
  - Embedded loopback HTTP streaming server (`StreamServer`) supporting `206 Partial Content` and MIME autodetection.
  - Desktop client player launcher (`webbrowser` integration) and WebUI HTML5 video modal.
  - Command-line streaming via `torrent_stream_example`.

### 3.3 [`docs/v2-merkle.md`](file:///c:/Projects/BitTorrent/docs/v2-merkle.md) — BitTorrent v2 & Hybrid Specification Guide
- **Purpose**: Technical deep dive into BEP 52 v2 and Hybrid torrent implementation.
- **Contents**:
  - Canonical binary SHA-256 Merkle tree calculation and power-of-two padding.
  - Merkle audit paths (`MerkleProof`) and on-the-fly 16 KiB block verification.
  - Top-level `piece layers` and `.layers` persistent disk cache format (`PieceLayersDiskCache`).
  - Dual info-hash handling: 20-byte SHA-1 and 32-byte SHA-256 in Hybrid swarms.
  - Piece boundary alignment and `.pad/<length>` file insertion.
  - Hash transfer protocol over peer wire (`hash_request`, `hashes`, `hash_reject`).
  - Torrent construction and upgrading via `TorrentBuilder` and `TorrentUpgrader`.

### 3.4 [`docs/webtorrent-bridge.md`](file:///c:/Projects/BitTorrent/docs/webtorrent-bridge.md) — WebTorrent & WebRTC Bridging Guide
- **Purpose**: Guide to browser-to-desktop bridging via WebTorrent protocols.
- **Contents**:
  - `WebRtcSocketAdapter`: Message-oriented framing over WebRTC Data Channels.
  - WebTorrent WebSocket tracker signaling protocol (`ws://` and `wss://`).
  - JSON announce/scrape payloads with SDP offer/answer exchanges.
  - `WebTorrentBridge` coordinator: bridging pieces between TCP/uTP swarms and browser clients.
  - Magnet URI auto-injection: extracting and injecting WebSeeds and WebSocket trackers.

### 3.5 [`docs/cli-reference.md`](file:///c:/Projects/BitTorrent/docs/cli-reference.md) — Command-Line Management Tool (`bt-cli`)
- **Purpose**: Complete manual for the `bt-cli` command-line utility.
- **Contents**:
  - Global flags (`--socket`, `--timeout`, `--json`).
  - Command reference with syntax, options, and sample output:
    - `list`, `add`, `pause`, `resume`, `remove`
    - `files`, `set-priority`
    - `peers`, `pieces`
    - `super-seed`
    - `v2-info`, `upgrade`
    - `recheck`
    - `magnet-info`, `resume-info`

### 3.6 [`docs/daemon-and-webui.md`](file:///c:/Projects/BitTorrent/docs/daemon-and-webui.md) — Headless Daemon & Web UI Operational Guide
- **Purpose**: Operational manual for server, NAS, and headless deployments.
- **Contents**:
  - `bt_daemon` startup flags, directory layout, and state auto-persistence (`.fastresume`).
  - IPC protocol architecture (Unix Domain Sockets on Linux/macOS, Named Pipes on Windows).
  - systemd service configuration unit file.
  - `bt_webui` gateway architecture, REST API documentation (`/api/torrents`, `/api/torrents/:hash/files`, etc.).
  - Web UI features: glassmorphic dashboard, piece heatmap, peer client badges, live video streaming player.

### 3.7 [`docs/packaging-and-distribution.md`](file:///c:/Projects/BitTorrent/docs/packaging-and-distribution.md) — Windows Installer & Packaging Guide
- **Purpose**: Developer guide for building installers and distribution packages.
- **Contents**:
  - Inno Setup 6 configuration (`packaging/windows/installer.iss`): registry entries, file associations (`.torrent`, `magnet:`), desktop/start-menu shortcuts.
  - Packaging script (`packaging/windows/package_release.ps1`): stages binaries, generates portable ZIP, compiles installer.
  - Console-less Windows executable configuration (`#![windows_subsystem = "windows"]`).
  - Application icon generation (`assets/icon.ico` multi-resolution icon and `assets/icon.png`).

---

## 4. Action Plan: Code Docstring & Rustdoc Enhancements

### 4.1 Root Library Documentation ([`library/src/lib.rs`](file:///c:/Projects/BitTorrent/library/src/lib.rs))
- Modernize crate-level doc comment:
  - Add quick references to BitTorrent v2, WebTorrent, streaming, disk caching, and fast resume.
  - Update runnable code examples demonstrating `TorrentSessionBuilder` with modern options.
  - Expand module re-export doc comments for easier discovery via `cargo doc`.

### 4.2 Public Struct and Trait Docstrings
- Ensure all public types in `core::views`, `storage::fast_resume`, `storage::disk_cache`, `storage::verification_pool`, `network::utp`, and `network::webrtc` have complete Rustdoc comments, `# Errors` sections, and code examples.

---

## 5. Implementation Roadmap & Execution Order

```
[Phase 1: Existing Docs Audit & Fixes]
  ├─ Update README.md (modern features, clients, installer, badges)
  ├─ Fix docs/architecture.md (remove obsolete gaps, add storage/v2/streaming architecture)
  ├─ Update docs/api-reference.md (expand SessionConfig, Builder, Session, and core types)
  ├─ Correct docs/utp.md (add SACK, fix file path, remove false future work)
  ├─ Fix docs/nat-pmp.md and docs/dht.md (correct paths and add IPv6/scrape)
  └─ Update clients/README.md and docs/README.md

[Phase 2: Add New Protocol & Architectural Documents]
  ├─ Create docs/bep-compliance.md (24 BEP compliance matrix)
  ├─ Create docs/v2-merkle.md (BEP 52 Merkle trees, piece layers, and upgrader)
  ├─ Create docs/streaming.md (seekable streaming reader & HTTP 206 server)
  └─ Create docs/webtorrent-bridge.md (WebRTC channels, WebSocket signaling & bridge)

[Phase 3: Add New User & Operator Guides]
  ├─ Create docs/cli-reference.md (bt-cli reference manual)
  ├─ Create docs/daemon-and-webui.md (bt_daemon & bt_webui operational guide)
  └─ Create docs/packaging-and-distribution.md (Windows installer & packaging guide)

[Phase 4: Code Docstrings & Verification]
  ├─ Enhance library/src/lib.rs top-level docstrings and examples
  ├─ Run `cargo doc --workspace --no-deps` to verify zero broken doc links
  └─ Run `cargo clippy --workspace --all-targets -- -D warnings`
```

---

## 6. Success Criteria & Verification

1. **Zero Outdated Claims**: No document claims that LEDBAT, SACK, UPnP, BEP 52 Merkle verification, or choking algorithms are "future work".
2. **100% Accurate File Paths**: All markdown links point to actual paths in the repository.
3. **Comprehensive Coverage**: Every crate in `clients/`, every example in `examples/`, and all 24 supported BEPs are documented.
4. **Clean Rustdoc**: `cargo doc --workspace --no-deps` compiles without warnings or unresolved intra-doc links.
5. **Clear Organization**: `docs/README.md` serves as a clean, navigable hub for all documentation.
