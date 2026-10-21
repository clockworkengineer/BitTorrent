# Command-Line Interface Reference (`bt-cli`)

This document provides a comprehensive command-line reference for `bt-cli`, the companion CLI management and inspection utility for **BitTorrent-rs**.

---

## 1. Overview & Connection Model

`bt-cli` operates in two modes:
1. **Daemon Management Mode**: Connects to an active `bt_daemon` process via local IPC (Unix Domain Socket on Linux/macOS, Windows Named Pipe `\\.\pipe\bt_daemon_ipc` on Windows) to manage active transfers, query live peer tables, and adjust priorities on the fly.
2. **Offline Inspection & Upgrade Mode**: Directly inspects `.torrent` files, `.fastresume` state files, and Magnet URIs without requiring a running daemon.

### Basic Syntax
```bash
bt-cli <command> [arguments...] [options...]
```

When building or running from the cargo workspace root:
```bash
cargo run -p bt_daemon --bin bt-cli -- <command> [arguments...]
```

---

## 2. Daemon Transfer Management Commands

### 2.1 `add` — Add a Torrent or Magnet Link
Adds a `.torrent` file or Magnet URI to the running daemon.

```bash
bt-cli add <torrent-path-or-magnet-uri> [--dir <download-dir>]
```

**Options**:
- `--dir <path>`: Specifies custom download root directory. Defaults to daemon's default download folder if omitted.

**Examples**:
```bash
bt-cli add linux-mint.torrent --dir /storage/torrents
bt-cli add "magnet:?xt=urn:btih:d2bde92211d4de9f32b2a99e46f6c7dd29f32ba4&dn=Ubuntu"
```

---

### 2.2 `status` / `list` — List Active Swarms
Displays a formatted status table of all torrents managed by the daemon.

```bash
bt-cli status
```

**Sample Output**:
```
INFO-HASH                                 NAME             STATUS       PROGRESS   SPEED (DN/UP)   PEERS   SS
-----------------------------------------------------------------------------------------------------------------
d2bde92211d4de9f32b2a99e46f6c7dd29f32ba4  Ubuntu 24.04     Downloading   45.20%     12.4M / 1.2M    42      -
a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2  Debian 12 ISO    Seeding      100.00%      0.0B / 4.8M    18     [SS]
```

---

### 2.3 `pause` & `resume` — Pause or Resume Transfers
Pauses or resumes peer wire downloading and uploading for a specific torrent.

```bash
bt-cli pause <info-hash>
bt-cli resume <info-hash>
```

---

### 2.4 `remove` — Remove a Torrent Session
Removes a transfer session from the daemon.

```bash
bt-cli remove <info-hash> [--purge]
```

**Options**:
- `--purge`: Deletes all downloaded payload files from disk in addition to removing session state.

---

### 2.5 `shutdown` — Gracefully Stop Daemon
Flushes all dirty blocks in the disk cache, saves `.fastresume` state files, and cleanly terminates the background `bt_daemon` process.

```bash
bt-cli shutdown
```

---

## 3. Swarm & File Inspection Commands

### 3.1 `files` — Inspect Torrent File Hierarchy
Lists all individual files inside a multi-file torrent, showing file indices, file sizes, and current download priorities.

```bash
bt-cli files <info-hash>
```

**Sample Output**:
```
INDEX  SIZE (BYTES)  PRIORITY  PATH
------------------------------------------------------------
0      15,482        Normal    read_me.txt
1      1,428,576,000 High      video_track.mp4
2      450,220,128   Skip      bonus_features.iso
```

---

### 3.2 `set-priority` — Configure File Download Priority
Dynamically sets the download priority for a specific file index within an active transfer.

```bash
bt-cli set-priority <info-hash> <file-index> <skip|low|normal|high>
```

**Priority Levels**:
- `skip`: Skips downloading this file. Boundary pieces shared with wanted files are safely retained without corruption.
- `low`: Downloads blocks only when higher-priority files are not pending.
- `normal`: Standard rarest-first download priority (default).
- `high`: Prioritizes piece block requests before normal-priority files.

---

### 3.3 `peers` — Live Swarm Peer Table
Queries connected peers for a specific swarm, displaying remote IP addresses, client software strings (parsed Azureus/Shadow/uTorrent client tags), connection transport, and transfer rates.

```bash
bt-cli peers <info-hash>
```

**Sample Output**:
```
PEER IP              CLIENT IDENTIFICATION    TRANSPORT  PROGRESS  SPEED (DN/UP)
------------------------------------------------------------------------------------
192.168.1.105:6881   qBittorrent 4.6.0        uTP         98.2%     2.4 MB/s / 420 KB/s
203.0.113.42:51413   Transmission 4.0.0       TCP        100.0%     4.1 MB/s / 120 KB/s
webrtc:browser_9a2f  WebTorrent / Brave       WebRTC      12.4%     850 KB/s / 1.1 MB/s
```

---

### 3.4 `super-seed` — Control Initial Seeding (BEP 16)
Toggles BEP 16 Super-Seeding mode on or off.

```bash
bt-cli super-seed <info-hash> <on|off>
```

---

## 4. Offline Inspection, Conversion, & Utility Commands

### 4.1 `v2-info` — Inspect BitTorrent v2 Metadata
Inspects a `.torrent` file and reports whether it is v1, pure v2, or Hybrid, alongside piece lengths, SHA-1 and SHA-256 info-hashes, piece layers count, and per-file Merkle roots (`pieces_root`).

```bash
bt-cli v2-info <torrent-path>
```

---

### 4.2 `upgrade` — Upgrade Legacy v1 Torrent to v2 / Hybrid
Upgrades a legacy BitTorrent v1 `.torrent` file into a BitTorrent v2 or Hybrid file by verifying and hashing downloaded payload files on disk.

```bash
bt-cli upgrade <input.torrent> <data-dir> [--format <hybrid|v2>] [--output <output.torrent>]
```

**Options**:
- `--format <hybrid|v2>`: Target metadata format (defaults to `hybrid`).
- `--output <path>`: Output `.torrent` file destination.

---

### 4.3 `magnet-info` — Parse & Rewrite Magnet URIs
Inspects a Magnet URI, extracting info-hashes, display name, trackers, and WebSeeds. Can also inject WebTorrent signaling and BEP 53 file selections.

```bash
bt-cli magnet-info "<magnet-uri>" [--inject-webtorrent] [--web-seed <url>] [--select <indices>]
```

**Options**:
- `--inject-webtorrent`: Injects public WebTorrent WebSocket trackers for instant browser bridging.
- `--web-seed <url>`: Appends custom BEP 19 HTTP mirror URLs.
- `--select <indices>`: Appends BEP 53 `so=` file selection filters.

---

### 4.4 `resume-info` — Inspect Fast Resume State
Parses and displays contents of a `.fastresume` file (save path, verified bitfield progress, uploaded/downloaded metrics, active seeding durations).

```bash
bt-cli resume-info <path-to-fastresume-file>
```

---

### 4.5 `recheck` — Multi-Threaded Hash Verification
Executes parallel multi-threaded SHA-1 and SHA-256 Merkle root re-verification of downloaded content on disk using `HashVerificationPool`.

```bash
bt-cli recheck <torrent-path> <data-dir> [--workers <N>]
```

**Options**:
- `--workers <N>`: Number of parallel hashing threads (defaults to logical CPU cores). Displays a real-time progress bar and throughput calculation (MB/s).
