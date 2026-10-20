# Project Documentation Hub

This directory contains the comprehensive architectural specifications, public API reference, standards compliance matrices, protocol deep dives, and operator guides for the **BitTorrent-rs** ecosystem.

---

## 1. Core Architecture & Standards

- [`bep-compliance.md`](bep-compliance.md) — **BEP Compliance & Standards Matrix**: Authoritative breakdown of all 24 supported BitTorrent specifications.
- [`architecture.md`](architecture.md) — **System Architecture**: High-level subsystems, concurrency model, data flows, and IPC topology.
- [`api-reference.md`](api-reference.md) — **Public API Reference**: `TorrentSession`, `TorrentSessionBuilder`, `SessionConfig`, and core types.
- [`roadmap.md`](roadmap.md) — **Chronological Roadmap**: Detailed milestones and achievement log across all 44 implementation phases.

---

## 2. Client & Operator Guides

- [`user-guide.md`](user-guide.md) — **End-User Guide**: Installation, Desktop GUI usage, preferences, keyboard shortcuts, and streaming.
- [`cli-reference.md`](cli-reference.md) — **CLI Reference Manual**: Complete command documentation for `bt-cli`.
- [`daemon-and-webui.md`](daemon-and-webui.md) — **Headless Daemon & Web UI**: Server installation, IPC sockets, REST API endpoints, and systemd units.
- [`sync-hub.md`](sync-hub.md) — **Sync Hub Manual**: Decentralized local-first LAN encrypted folder synchronization.
- [`packaging-and-distribution.md`](packaging-and-distribution.md) — **Packaging & Distribution**: Inno Setup Windows installer, portable archives, and icon generation.

---

## 3. Protocol Deep Dives

- [`v2-merkle.md`](v2-merkle.md) — **BitTorrent v2 (BEP 52)**: SHA-256 Merkle trees, piece layers, pad files, and legacy upgrader.
- [`webtorrent-bridge.md`](webtorrent-bridge.md) — **WebTorrent & WebRTC**: Data channels, WebSocket tracker signaling (`ws://`/`wss://`), and browser bridging.
- [`streaming.md`](streaming.md) — **Seekable Media Streaming**: `TorrentStream`, dynamic lookahead piece selection, and HTTP 206 range server.
- [`utp.md`](utp.md) — **Micro Transport Protocol (BEP 29)**: UDP framing, RFC 6817 LEDBAT congestion control, and SACK loss recovery.
- [`nat-pmp.md`](nat-pmp.md) — **Port Forwarding Protocols**: UPnP (SSDP/IGD SOAP) and NAT-PMP (RFC 6886) automated port mapping.
- [`encryption.md`](encryption.md) — **Traffic Encryption (MSE / PE)**: Diffie-Hellman handshake obfuscation and RC4 stream ciphers.
- [`dht.md`](dht.md) — **Kademlia DHT (BEP 5 & BEP 32)**: Trackerless peer discovery, dual-stack IPv6 DHT, and BEP 51 scrape.

---

## 4. Developer, Portability, & Refactoring

- [`performance.md`](performance.md) — **Performance Optimizations**: Zero-copy slice decoders, static buffer pools, and integer arithmetic.
- [`portability.md`](portability.md) — **Portability & `#![no_std]`**: Bare-metal compilation, hardware I/O traits, and port-free mock testing.
- [`ui-layout.md`](ui-layout.md) — **GUI Layout Notes**: Layout tokens, color themes, and egui component patterns.
- [`use-cases.md`](use-cases.md) — **Architecture Use Cases**: Server distributions, edge mesh, and private sync topologies.
- [`documentation_plan.md`](documentation_plan.md) — **Documentation Modernization Plan**: Source audit findings and execution blueprint.
- [`solid-refactor-plan.md`](solid-refactor-plan.md) — **SOLID Refactoring Plan**: Architectural refactoring plan for SOLID principles.
- [`dry-refactor-plan.md`](dry-refactor-plan.md) — **DRY Refactoring Plan**: Code deduplication roadmap across workspace crates.
