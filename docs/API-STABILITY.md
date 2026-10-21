# bittorrent-rs API Stability and Guidelines

This document details the stable API surface of the `bittorrent-rs` library. Callers should rely only on public types documented here as stable.

## Stable API Surface

The following modules and types comprise the official, public stable API of the `bittorrent-rs` library:

- **`TorrentSession`**: The primary coordinator for a BitTorrent transfer session. Exposes state controls like download commencement, pausing, resuming, and termination.
- **`TorrentSessionBuilder`**: Fluent builder pattern configured via `TorrentSession::builder()` or `TorrentSession::from_magnet()` to bootstrap sessions.
- **`SessionConfig`**: Struct holding all tunable session-wide options (ports, timeouts, injectable factories).
- **`TorrentClient`**: High-level consumer interface for managing multiple torrent sessions concurrently.
- **`MagnetLink`**: Parser and validation structure for `magnet:?` format URIs.
- **`MetaInfoFile`**: Decoder and parser for standard bencoded `.torrent` metainfo files.
- **`PeerMessage`**: Enumerated variant encoding/decoding low-level BitTorrent peer protocol wire packets.
- **`HttpClient` / `SocketFactory`**: Injectable traits permitting customization of HTTP communication and raw sockets.

## Internal APIs (Unstable)

The following components are internal details and are not part of the stable API surface. Callers must not rely directly on their signatures or internals, as they are subject to change in minor/patch releases:

- **`TorrentContext`** (lives under `internals` module): The raw synchronized state keeper of a single torrent transfer. Exposes lock guards and bitfield statuses.
- **`Peer`**: Local struct representation of a single connected peer.
- **`PeerNetwork`**: Transport wrapper around raw TCP/uTP streams.
- **`DiskIO`**: Disk-backed block storage scheduler.
- **`Assembler`**: Piece/block presence assembler.
