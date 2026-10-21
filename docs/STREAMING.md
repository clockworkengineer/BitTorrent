# Media Streaming Architecture & Guide

This document describes the progressive, seekable media streaming architecture of **BitTorrent-rs**. It explains how the library adapts active peer swarms into standard Rust seekable streams, coordinates dynamic forward piece lookahead, runs embedded HTTP 206 range servers, and launches media players in both the Desktop GUI and Web UI.

---

## 1. Overview

Traditional BitTorrent downloaders utilize a **rarest-first** piece selection strategy to maximize swarm health and piece availability. However, rarest-first downloads pieces in random order across the file, making it impossible to begin playback until 100% of the torrent has finished downloading.

BitTorrent-rs introduces a **seekable media streaming engine** that combines:
1. **Dynamic Prioritization**: High-priority requesting of pieces immediately surrounding the playback cursor.
2. **Forward Lookahead Buffer**: Pre-fetching a configurable window of upcoming pieces.
3. **Swarm Health Fallback**: Downloading rarest-first pieces in the background when the streaming lookahead window is satisfied.
4. **Seekable Rust Trait Implementation**: `TorrentStream` implementing standard `std::io::Read + std::io::Seek`.
5. **Embedded HTTP Range Server**: Serving `206 Partial Content` byte ranges directly to external media players (VLC, MPV, Windows Media Player) and browser HTML5 video players.

```
[Media Player / VLC / Browser]
        │
        │ HTTP GET Range: bytes=10485760-20971520
        ▼
[Embedded HTTP Range Server (StreamServer)]
        │
        ▼
[TorrentStream (Read + Seek)]
        │
        ├─ Read Cursor: Byte 10,485,760 ──► Maps to Piece #20
        │
        ▼
[StreamingPieceSelector]
        │
        ├─ Priority #1: Piece #20 (Playback Head)
        ├─ Priority #2: Pieces #21..#25 (Forward Lookahead Window)
        └─ Priority #3: Rarest First (Remaining Swarm Pieces)
```

---

## 2. The `TorrentStream` API

The core streaming abstraction is implemented in [`library/src/storage/stream.rs`](file:///c:/Projects/BitTorrent/library/src/storage/stream.rs).

### 2.1 Standard Trait Implementation
`TorrentStream` implements standard Rust `Read` and `Seek`:

```rust
use bittorrent_rs::TorrentSession;
use std::io::{Read, Seek, SeekFrom};

// Create a stream for the primary media file in the session
let mut stream = session.create_file_stream(0)?;

// Seek to 100 MB into the video
stream.seek(SeekFrom::Start(104_857_600))?;

// Read bytes sequentially - blocks automatically until pieces arrive
let mut buffer = [0u8; 65536];
let bytes_read = stream.read(&mut buffer)?;
```

### 2.2 Timeouts & Non-Blocking Polling
- **`with_read_timeout(Duration)`**: Configures how long `read()` will block waiting for missing piece blocks to arrive from the swarm before returning `std::io::ErrorKind::TimedOut`.
- **`with_poll_interval(Duration)`**: Controls the sleep interval between checks for piece completion.
- **Non-Blocking Mode**: Setting `timeout = Duration::ZERO` causes missing pieces to immediately return `std::io::ErrorKind::WouldBlock`, ideal for non-blocking event loops.

---

## 3. Dynamic Piece Selection (`StreamingPieceSelector`)

When streaming is activated, the session attaches `StreamingPieceSelector`:
- **Atomic Cursor Tracking**: Tracks the active read position using a shared `Arc<AtomicU32>` representing the current piece index.
- **Lookahead Window**: Requests blocks in the forward lookahead window (typically 5 to 10 pieces ahead of the cursor).
- **Prioritized Sorting**: Blocks for the immediate playback head are requested before lookahead blocks.
- **Swarm Balance**: Once the lookahead window is downloaded, remaining requests revert to rarest-first, ensuring swarm health is maintained.

---

## 4. Embedded HTTP Range Streaming Server (`StreamServer`)

To allow external media players to stream video without custom plugins, both the Desktop GUI and server daemon embed a lightweight multi-threaded HTTP server:

- **HTTP `206 Partial Content`**: Handles standard HTTP range request headers (`Range: bytes=start-end`).
- **MIME Type Autodetection**: Automatically detects content types (`video/mp4`, `video/webm`, `video/x-matroska`, `audio/mpeg`, etc.) based on file extension.
- **Multi-File URL Routing**:
  - `/stream`: Streams the largest or primary media file in the torrent.
  - `/stream/:file_index`: Streams a specific file index from a multi-file torrent.
- **Cross-Origin Support**: Emits CORS headers allowing web browser frontends (like `bt_webui`) to stream video directly.

---

## 5. Desktop GUI Media Streaming (`torrent_client`)

In the desktop application:
1. **`▶ Stream` Button**: Available in both the main torrent item list and the individual files table in the `Files` detail tab.
2. **Top Playback Banner**: When streaming starts, an active banner appears at the top of the window displaying:
   - File name and stream URL (`http://127.0.0.1:<port>/stream`).
   - `📋 Copy URL`: Copies stream URL to the system clipboard for VLC or MPV.
   - `🚀 Open Player`: Automatically launches the default OS media player or web browser using the `webbrowser` crate.
   - `⏹ Stop`: Shuts down the loopback server and frees the streaming selector.

---

## 6. Web UI HTML5 Video Player (`bt_webui`)

In the modern Web UI:
- Clicking `▶ Stream` opens a glassmorphic **HTML5 Video Player Modal** (`#stream-modal`).
- Embeds a native `<video controls autoplay>` player connected to the daemon's streaming endpoint (`http://localhost:8081/stream/:info_hash/:file_index`).
- Features a file dropdown selector to switch audio/video tracks on the fly.
- Displays live buffering indicators and download speeds in real time.

---

## 7. Command-Line Streaming Example

You can run standalone streaming from the terminal using the `torrent_stream_example`:

```bash
# Start streaming a torrent file or magnet link on port 8080
cargo run -p torrent_stream_example --release -- ./movie.torrent ./downloads 8080 0
```

Then open VLC and select:
**Media → Open Network Stream → `http://127.0.0.1:8080/stream`**
