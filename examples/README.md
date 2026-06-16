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
