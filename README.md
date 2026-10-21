# BitTorrent-rs

[![CI](https://github.com/clockworkengineer/BitTorrent/actions/workflows/ci.yml/badge.svg)](https://github.com/clockworkengineer/BitTorrent/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](https://www.rust-lang.org/)
[![Version: 0.1.1](https://img.shields.io/badge/Version-0.1.1-blue.svg)](CHANGELOG.md)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE-MIT)
[![Windows Setup](https://img.shields.io/badge/Windows-Installer%20v0.1.1-blueviolet.svg)](dist/BitTorrent-Setup-v0.1.1.exe)
[![Buy Me a Coffee](https://img.shields.io/badge/Donate-Buy%20Me%20A%20Coffee-orange.svg)](https://www.buymeacoffee.com/clockworkengineer)

An enterprise-grade, `#![no_std]`-capable BitTorrent client, daemon, Web UI, and protocol library implemented in modern Rust. Features comprehensive support for 24 BitTorrent Enhancement Proposals (BEPs), BitTorrent v2 (BEP 52) SHA-256 Merkle trees, WebTorrent WebRTC browser bridging, uTP LEDBAT congestion control with SACK loss recovery, seekable progressive media streaming, and high-throughput disk write coalescing.

---

## Key Features

- **BitTorrent v1, v2 (BEP 52), & Hybrid Swarms**: Full `.torrent` and Magnet metadata parsing, power-of-two padded SHA-256 Merkle trees, on-the-fly 16 KiB block proof verification (`MerkleProof`), `.layers` disk cache, pad file insertion, and legacy v1-to-v2 upgrading via `TorrentUpgrader`.
- **WebTorrent WebRTC Browser Bridging**: Direct browser-to-desktop cross-swarm bridging via WebRTC Data Channels (`WebRtcSocketAdapter`), WebSocket tracker signaling (`ws://` and `wss://`), and automatic Magnet URI mirror injection.
- **Micro Transport Protocol (uTP — BEP 29)**: Complete UDP-based framing adapter, RFC 6817 LEDBAT delay-based congestion control, SACK (Selective Acknowledgment) out-of-order reassembly buffers, and RFC 6298 microsecond RTT/RTO fast loss recovery.
- **High-Throughput Storage & Disk Cache**: In-memory write cache with sequential block coalescing (reducing disk write syscalls by >95%), corrupted block discard on hash failure, and persistent BEP 47 fast-resume state (`.fastresume`).
- **Parallel Hash Verification Pool**: Multi-threaded worker pool (`HashVerificationPool`) offloading CPU-intensive SHA-1 and SHA-256 block verifications from the async event loop, with piece selector in-flight guards.
- **Seekable Media Streaming Engine**: `TorrentStream` implementing standard Rust `Read + Seek` traits with dynamic cursor lookahead piece selection (`StreamingPieceSelector`), paired with an embedded HTTP 206 `Range` server and media player launchers for Desktop GUI and Web UI.
- **Super-Seeding Mode (BEP 16 & BEP 6)**: Rarest piece allocation, single-piece offer tracking, upload gatekeeping, and synergistic integration with BEP 6 `Suggest Piece` and `Allowed Fast` grants.
- **Adaptive Endgame Mode**: Multi-source block request staging, RFC 6298 swarm latency profiling, and redundant late block absorption.
- **Dynamic WebSeed Failover (BEP 17 / BEP 19 / BEP 38)**: Multi-mirror HTTP/HTTPS downloads with smoothed latency scoring, circuit-breaker backoff cooldowns, and automatic canary probing.
- **NAT & Firewall Traversal**: Dual-mode UPnP (SSDP/IGD SOAP) and NAT-PMP (RFC 6886) port mapping, alongside BEP 55 `ut_holepunch` peer rendezvous traversal.
- **Decentralized Discovery**: Kademlia DHT (BEP 5, BEP 32 IPv6, BEP 51 scrape), Local Service Discovery (LSD — BEP 14), and dual-stack Peer Exchange (PEX — BEP 11) with IPv6 support.
- **Privacy & Traffic Obfuscation**: Message Stream Encryption (MSE / PE) via Diffie-Hellman + RC4 ciphers, and strict Private Torrent enforcement (BEP 27).
- **Bare-Metal Portability**: Core protocol logic, Bencode parsing, and message framing compile cleanly under `#![no_std]` targets using hardware-agnostic `AsyncSocket` and `BlockStorage` abstractions.

---

## Applications & Ecosystem

The workspace includes a complete suite of desktop, server, and command-line clients:

| Application | Path | Description |
| :--- | :--- | :--- |
| **Desktop Client** | [`clients/torrent_client`](clients/torrent_client) | Interactive GUI (`egui`/`eframe`) featuring live piece heatmaps, peer badges, integrated media streaming, and settings. |
| **Server Daemon** | [`clients/bt_daemon`](clients/bt_daemon) | Headless background daemon with IPC socket service, embedded HTTP streaming server, and `.fastresume` state persistence. |
| **CLI Tool** | [`clients/bt_cli`](clients/bt_daemon) | Command-line management utility (`bt-cli`) for querying, adding, inspecting, and upgrading torrent swarms. |
| **Web Dashboard** | [`clients/bt_webui`](clients/bt_webui) | Modern glassmorphic Web UI with interactive swarm visualizer, file priority controls, and HTML5 video player modal. |
| **LAN Sync Hub** | [`clients/sync_hub`](clients/sync_hub) | Local-first, decentralized folder synchronization client using private torrents, LSD, and MSE encryption. |

---

## Windows Installation & Packaging

Pre-compiled Windows release artifacts are available directly in the repository:

- **Windows Setup Installer**: [`dist/BitTorrent-Setup-v0.1.1.exe`](dist/BitTorrent-Setup-v0.1.1.exe) (Compiled with Inno Setup 6, featuring file associations for `.torrent` and `magnet:`, Start Menu shortcuts, and clean uninstaller).
- **Portable ZIP Package**: [`dist/BitTorrent-v0.1.1-windows-x86_64-portable.zip`](dist/BitTorrent-v0.1.1-windows-x86_64-portable.zip) (Zero-install standalone archive containing all binaries, icons, and licenses).

To build the installer and release package from source on Windows:
```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\windows\package_release.ps1
```

---

## Getting Started

### 1. Build and Test the Workspace

```bash
# Run the entire test suite (unit, integration, and doctests)
cargo test --workspace

# Run clippy linter across all targets
cargo clippy --workspace --all-targets -- -D warnings
```

### 2. Launch the Desktop GUI Client

```bash
cargo run -p torrent_client --release -- [torrent-file] [download-dir]
```

### 3. Run the Headless Daemon & Web UI

```bash
# Terminal 1: Start the server daemon (runs IPC on port/socket, stream server on :8081)
cargo run -p bt_daemon --release -- --dir ./downloads

# Terminal 2: Start the Web UI gateway (serves dashboard on http://localhost:8080)
cargo run -p bt_webui --release -- --port 8080
```

### 4. Manage Transfers with the CLI (`bt-cli`)

```bash
# Add a torrent
cargo run -p bt_daemon --bin bt-cli -- add linux.torrent ./downloads

# Inspect live peer swarm and client software versions
cargo run -p bt_daemon --bin bt-cli -- peers <info-hash>

# View interactive piece availability heatmap
cargo run -p bt_daemon --bin bt-cli -- pieces <info-hash>

# Upgrade legacy v1 torrent to BitTorrent v2 / Hybrid format
cargo run -p bt_daemon --bin bt-cli -- upgrade input.torrent ./downloads --format hybrid
```

---

## Workspace Structure

- `library/` — Core BitTorrent protocol library crate (optionally `#![no_std]`).
- `clients/` — Workspace client applications (`torrent_client`, `bt_daemon`, `bt_webui`, `sync_hub`, `torrent_client_shared`).
- `examples/` — 8 demonstration programs showcasing library APIs (sessions, creator, streaming, fast-resume, selective download, port mapping).
- `packaging/` — Windows Inno Setup installer script and packaging automation tools.
- `docs/` — Architecture, API reference, BEP compliance matrix, user guides, and protocol specifications.

---

## Documentation

| Document | Purpose |
| :--- | :--- |
| [`docs/BEP-COMPLIANCE.md`](docs/BEP-COMPLIANCE.md) | **BEP Compliance Matrix**: Full breakdown of all 24 supported BitTorrent specifications. |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | **System Architecture**: High-level subsystems, concurrency model, data flow, and IPC layout. |
| [`docs/API-REFERENCE.md`](docs/API-REFERENCE.md) | **Public API Reference**: `TorrentSession`, builder patterns, configuration, and data structures. |
| [`docs/USER-GUIDE.md`](docs/USER-GUIDE.md) | **End-User Guide**: Instructions for GUI client, daemon, Web UI, and settings. |
| [`docs/CLI-REFERENCE.md`](docs/CLI-REFERENCE.md) | **CLI Reference**: Complete manual for the `bt-cli` command-line tool. |
| [`docs/DAEMON-AND-WEBUI.md`](docs/DAEMON-AND-WEBUI.md) | **Daemon & Web UI**: Headless server installation, REST endpoints, and systemd deployment. |
| [`docs/STREAMING.md`](docs/STREAMING.md) | **Media Streaming Guide**: Seekable `TorrentStream`, HTTP 206 range server, and player integration. |
| [`docs/V2-MERKLE.md`](docs/V2-MERKLE.md) | **BitTorrent v2 Guide**: SHA-256 Merkle trees, piece layers, pad files, and upgrader tooling. |
| [`docs/WEBTORRENT-BRIDGE.md`](docs/WEBTORRENT-BRIDGE.md) | **WebTorrent Guide**: WebRTC Data Channels, WebSocket signaling, and browser cross-swarming. |
| [`docs/UTP.md`](docs/UTP.md) | **uTP Transport**: Packet framing, LEDBAT congestion control, and SACK loss recovery. |
| [`docs/NAT-PMP.md`](docs/NAT-PMP.md) | **Port Forwarding**: UPnP (SSDP/IGD SOAP) and NAT-PMP automated mapping protocols. |
| [`docs/ENCRYPTION.md`](docs/ENCRYPTION.md) | **Encryption**: Message Stream Encryption (MSE / PE) handshake and RC4 ciphers. |
| [`docs/DHT.md`](docs/DHT.md) | **Kademlia DHT**: Distributed Hash Table, IPv6 DHT, and trackerless discovery. |
| [`docs/SYNC-HUB.md`](docs/SYNC-HUB.md) | **Sync Hub**: Local-first encrypted folder synchronization guide. |
| [`docs/PACKAGING-AND-DISTRIBUTION.md`](docs/PACKAGING-AND-DISTRIBUTION.md) | **Packaging Guide**: Inno Setup installer compilation, icon assets, and portable packaging. |
| [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) | **Performance**: Zero-copy slice parsing, buffer pooling, and integer math optimizations. |
| [`docs/PORTABILITY.md`](docs/PORTABILITY.md) | **Portability**: `#![no_std]` bare-metal support, hardware traits, and mock testing. |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | **Milestone Roadmap**: Detailed chronological record of all 44 development phases. |

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
