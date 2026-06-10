# BitTorrent Rust Library

A full-featured BitTorrent client library implemented in Rust with:

- `.torrent` metadata parsing via `MetaInfoFile` (v1 SHA-1 and v2 SHA-256)
- Tracker announce and peer discovery via `Tracker` (HTTP BEP 3, UDP BEP 15)
- Kademlia DHT peer discovery (BEP 5)
- Local Service Discovery (LSD — BEP 14)
- Peer Exchange (PEX — BEP 11) with IPv6 support
- Magnet link parsing and metadata bootstrap
- WebSeed HTTP seeding (BEP 17 / BEP 19)
- Peer wire protocol framing with Fast Extension (BEP 6) and Extension Protocol (BEP 10)
- Message Stream Encryption (MSE/PE) via RC4 + Diffie-Hellman
- uTorrent Transport Protocol framing (uTP — BEP 29)
- NAT-PMP auto port forwarding
- Private torrent enforcement (BEP 27)
- Piece selection, block request management, and disk-backed assembly
- Download lifecycle management with pause/resume and seeding support
- `#![no_std]` compatible core with hardware-agnostic I/O traits

## Getting Started

Build the library and run the test suite:

```bash
cargo test
```

Run the example downloader:

```bash
cargo run -p torrent_session_example --release -- <torrent-file> <download-dir>
```

Run the desktop client UI:

```bash
cargo run -p torrent_client --release -- <torrent-file> <download-dir>
```

Replace `<torrent-file>` with the path to a `.torrent` file and `<download-dir>` with the local directory where files should be created.

## Project Structure

- `library/` — core BitTorrent library crate
- `examples/` — workspace example app showing how to instantiate and use `TorrentSession`
- `clients/` — interactive desktop GUI client (`egui`/`eframe`)
- `docs/` — roadmap, architecture, and API documentation

## Documentation

- [`docs/architecture.md`](docs/architecture.md) — module layout, components, and data flow
- [`docs/api-reference.md`](docs/api-reference.md) — full public API reference
- [`docs/roadmap.md`](docs/roadmap.md) — implementation phases and future work
- [`docs/dht.md`](docs/dht.md) — DHT peer discovery internals
- [`docs/encryption.md`](docs/encryption.md) — MSE/RC4 encryption protocol
- [`docs/utp.md`](docs/utp.md) — uTP transport protocol
- [`docs/nat-pmp.md`](docs/nat-pmp.md) — NAT-PMP port forwarding
