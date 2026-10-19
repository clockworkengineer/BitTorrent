# Examples

This folder is reserved for example projects and demonstration code that use the `library` crate.

## Available Examples

### 1. Torrent Session Example (`torrent_session_example`)
Demonstrates the basic `TorrentSession` lifecycle API (loading, starting, pausing, resuming, and stopping).
```bash
cargo run -p torrent_session_example --release -- <torrent-file> <download-dir>
```

### 2. Torrent Info Example (`torrent_info_example`)
Demonstrates loading and parsing a standard BitTorrent `.torrent` (metainfo) file and extracting its metadata, trackers, web seeds, and file list.
```bash
cargo run -p torrent_info_example --release -- <torrent-file-path>
```

### 3. Magnet DHT Example (`magnet_dht_example`)
Demonstrates parsing magnet URIs and bootstrapping a DHT session to fetch torrent metadata.
```bash
cargo run -p magnet_dht_example --release -- "<magnet-uri>" [download-dir]
```

### 4. Fast Resume Example (`torrent_fast_resume_example`)
Demonstrates how to initialize a `TorrentSession` with `skip_hash_check: true` to bypass full disk verification of existing files on startup (significantly reducing startup times).
```bash
cargo run -p torrent_fast_resume_example --release -- <torrent-file> <download-dir>
```

### 5. Port Mapping Example (`port_mapping_example`)
Demonstrates automated port mapping configuration using `FallbackPortMapper` to request and release TCP/UDP port mapping leases on a local gateway router via NAT-PMP or UPnP SOAP protocols.
```bash
cargo run -p port_mapping_example --release
```

### 6. Torrent Creator Example (`torrent_creator_example`)
Demonstrates programmatic `.torrent` creation from a file or directory using `TorrentBuilder`. Supports BitTorrent v1, v2 (BEP 52 Merkle trees & piece layers), and Hybrid torrents with custom piece lengths, announce tracker tiers, WebSeed mirrors, and automatic Magnet URI generation.
```bash
# Package a file or directory into a hybrid torrent with 64 KiB pieces
cargo run -p torrent_creator_example --release -- ./input_dir ./output.torrent hybrid 64
```

### 7. Torrent Stream Example (`torrent_stream_example`)
Demonstrates sequential seekable media streaming using `TorrentStream` and `StreamingPieceSelector`. Spins up a lightweight local HTTP server supporting HTTP 206 `Range` requests, allowing video players (such as VLC, mpv, or browsers) to stream media files progressively while downloading and seeking within uncompleted torrents.
```bash
cargo run -p torrent_stream_example --release -- <torrent-file-or-magnet> [download-dir] [port] [file-index]

# Stream to media player via:
# VLC -> Media -> Open Network Stream -> http://127.0.0.1:8080/stream
```

### 8. Selective Download Example (`selective_download_example`)
Demonstrates BEP 53 (`so=`) selective file/range filtering and dynamic WebSeed HTTP mirror tracking. Shows how to configure `TorrentSession` to only fetch specific file indices, prints detailed bandwidth and piece reduction metrics via `session.bep53_evaluation()`, and manages multi-mirror WebSeed endpoints with dynamic latency scoring and circuit breaking.
```bash
cargo run -p selective_download_example --release -- <torrent-file-or-magnet> <download-dir> [selected-indices]

# Example: only download file indices 0 and 2
cargo run -p selective_download_example --release -- my_archive.torrent ./downloads 0,2
```
