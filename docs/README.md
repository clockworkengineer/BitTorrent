# Project Documentation Hub

This directory contains the comprehensive architectural specifications, public API reference, standards compliance matrices, protocol deep dives, and operator guides for the **BitTorrent-rs** ecosystem.

---

## 1. Core Architecture & Standards

- [`BEP-COMPLIANCE.md`](BEP-COMPLIANCE.md) — **BEP Compliance & Standards Matrix**: Authoritative breakdown of all 24 supported BitTorrent specifications.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — **System Architecture**: High-level subsystems, concurrency model, data flows, and IPC topology.
- [`API-REFERENCE.md`](API-REFERENCE.md) — **Public API Reference**: `TorrentSession`, `TorrentSessionBuilder`, `SessionConfig`, and core types.
- [`ROADMAP.md`](ROADMAP.md) — **Chronological Roadmap**: Detailed milestones and achievement log across all 44 implementation phases.

---

## 2. Client & Operator Guides

- [`USER-GUIDE.md`](USER-GUIDE.md) — **End-User Guide**: Installation, Desktop GUI usage, preferences, keyboard shortcuts, and streaming.
- [`CLI-REFERENCE.md`](CLI-REFERENCE.md) — **CLI Reference Manual**: Complete command documentation for `bt-cli`.
- [`DAEMON-AND-WEBUI.md`](DAEMON-AND-WEBUI.md) — **Headless Daemon & Web UI**: Server installation, IPC sockets, REST API endpoints, and systemd units.
- [`SYNC-HUB.md`](SYNC-HUB.md) — **Sync Hub Manual**: Decentralized local-first LAN encrypted folder synchronization.
- [`PACKAGING-AND-DISTRIBUTION.md`](PACKAGING-AND-DISTRIBUTION.md) — **Packaging & Distribution**: Inno Setup Windows installer, portable archives, and icon generation.

---

## 3. Protocol Deep Dives

- [`V2-MERKLE.md`](V2-MERKLE.md) — **BitTorrent v2 (BEP 52)**: SHA-256 Merkle trees, piece layers, pad files, and legacy upgrader.
- [`WEBTORRENT-BRIDGE.md`](WEBTORRENT-BRIDGE.md) — **WebTorrent & WebRTC**: Data channels, WebSocket tracker signaling (`ws://`/`wss://`), and browser bridging.
- [`STREAMING.md`](STREAMING.md) — **Seekable Media Streaming**: `TorrentStream`, dynamic lookahead piece selection, and HTTP 206 range server.
- [`UTP.md`](UTP.md) — **Micro Transport Protocol (BEP 29)**: UDP framing, RFC 6817 LEDBAT congestion control, and SACK loss recovery.
- [`NAT-PMP.md`](NAT-PMP.md) — **Port Forwarding Protocols**: UPnP (SSDP/IGD SOAP) and NAT-PMP (RFC 6886) automated port mapping.
- [`ENCRYPTION.md`](ENCRYPTION.md) — **Traffic Encryption (MSE / PE)**: Diffie-Hellman handshake obfuscation and RC4 stream ciphers.
- [`DHT.md`](DHT.md) — **Kademlia DHT (BEP 5 & BEP 32)**: Trackerless peer discovery, dual-stack IPv6 DHT, and BEP 51 scrape.

---

## 4. Developer, Portability, & Refactoring

- [`REFACTOR-LIBRARY-ATTRIBUTES-PLAN.md`](REFACTOR-LIBRARY-ATTRIBUTES-PLAN.md) — **10 Attributes Refactoring Plan**: Concrete roadmap implementing the 10 quality attributes from `notes/ATTRIBUTES.md`.
- [`PERFORMANCE.md`](PERFORMANCE.md) — **Performance Optimizations**: Zero-copy slice decoders, static buffer pools, and integer arithmetic.
- [`PORTABILITY.md`](PORTABILITY.md) — **Portability & `#![no_std]`**: Bare-metal compilation, hardware I/O traits, and port-free mock testing.
- [`UI-LAYOUT.md`](UI-LAYOUT.md) — **GUI Layout Notes**: Layout tokens, color themes, and egui component patterns.
- [`USE-CASES.md`](USE-CASES.md) — **Architecture Use Cases**: Server distributions, edge mesh, and private sync topologies.
