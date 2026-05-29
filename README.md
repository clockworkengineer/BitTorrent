# BitTorrent Rust Library

A BitTorrent client library implemented in Rust with:

- torrent metadata parsing via `MetaInfoFile`
- tracker announce and peer discovery via `Tracker`
- peer wire protocol framing and handshake support
- piece selection, block request management, and disk-backed assembly
- download lifecycle management with pause/resume and seeding support

## Getting started

Build the library and run the test suite:

```bash
cargo test
```

Run the example downloader:

```bash
cargo run -p torrent_session_example --release -- <torrent-file> <download-dir>
```

Replace `<torrent-file>` with the path to a `.torrent` file and `<download-dir>` with the local directory where files should be created.

## Project structure

- `library/` — core BitTorrent library crate
- `examples/` — workspace example app showing how to instantiate and use `TorrentSession`
- `docs/` — roadmap, architecture, and API documentation

## Documentation

See `docs/roadmap.md` for implementation phases and `docs/api-proposal.md` for the public API design.
