# Project Documentation

This folder contains the architecture, API reference, roadmap, and design documents for the BitTorrent Rust library.

## Core Design & Reference
- [architecture.md](file:///c:/Projects/BitTorrent/docs/architecture.md) — System architecture, module layouts, and core data flow.
- [api-reference.md](file:///c:/Projects/BitTorrent/docs/api-reference.md) — Public API reference, builder configurations, and usage patterns.
- [ui-layout.md](file:///c:/Projects/BitTorrent/docs/ui-layout.md) — Recommended desktop client layout and egui framework notes.

## Technical Details & Features
- [dht.md](file:///c:/Projects/BitTorrent/docs/dht.md) — Kademlia DHT peer discovery (BEP 5) and KRPC wire protocols.
- [portability.md](file:///c:/Projects/BitTorrent/docs/portability.md) — `#![no_std]` bare-metal support, hardware-agnostic traits, and port-free mock testing.
- [performance.md](file:///c:/Projects/BitTorrent/docs/performance.md) — Zero-copy slice parsing, streaming hash validation, and static buffer pools.

## History & Planning
- [roadmap.md](file:///c:/Projects/BitTorrent/docs/roadmap.md) — Implementation milestones, completed phases, and future improvements.

---

## Getting Started

For usage and runnable code, see the root [README.md](file:///c:/Projects/BitTorrent/README.md) and the [examples/torrent_session_example](file:///c:/Projects/BitTorrent/examples/torrent_session_example/src/main.rs).
