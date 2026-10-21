# Headless Daemon (`bt_daemon`) & Web UI (`bt_webui`) Operational Guide

This document describes how to deploy, configure, and operate **`bt_daemon`** (the headless BitTorrent server daemon) and **`bt_webui`** (the modern glassmorphic web dashboard gateway).

---

## 1. System Architecture

```
                               ┌─────────────────────────────┐
                               │     Web Browser Client      │
                               │  (http://localhost:8080)    │
                               └─────────────────────────────┘
                                              ▲
                                              │ HTTP / JSON API / HTML5 Video
                                              ▼
                               ┌─────────────────────────────┐
                               │   Web UI Gateway (bt_webui) │
                               │  Serves assets on port 8080 │
                               └─────────────────────────────┘
                                              ▲
                                              │ Local IPC Protocol
                                              │ (Unix Domain Socket / Named Pipe)
                                              ▼
┌────────────────────────┐     ┌─────────────────────────────┐
│  CLI Tool (bt-cli)     │────►│    Server Daemon (bt_daemon) │
│  Scripts / Crontab     │     │  - Swarm Manager            │
└────────────────────────┘     │  - FastResume Persistence   │
                               │  - Media Server (:8081)     │
                               └─────────────────────────────┘
```

---

## 2. Headless Daemon (`bt_daemon`)

`bt_daemon` is a lightweight, zero-GUI background service designed for Linux/macOS servers, Docker containers, and home NAS appliances.

### 2.1 Starting the Daemon
```bash
# Launch with default download directory
bt_daemon --dir /var/lib/bittorrent/downloads
```

### Command-Line Arguments
| Option | Default | Description |
| :--- | :--- | :--- |
| `--dir <path>` | `./downloads` | Default target directory for downloaded files. |
| `--data-dir <path>` | `~/.bittorrent` | Directory where `.fastresume` state files are persisted. |
| `--ipc <path>` | `/tmp/bt_daemon.sock` (Unix) or `\\.\pipe\bt_daemon_ipc` (Windows) | IPC socket endpoint. |
| `--stream-port <port>` | `8081` | Port for the embedded HTTP media streaming server. |
| `--port <port>` | `6881` | TCP/UDP listening port for peer connections. |

### 2.2 systemd Service Setup (Linux)
To run `bt_daemon` as a persistent system service on Ubuntu, Debian, or CentOS:

1. Create `/etc/systemd/system/bt-daemon.service`:
```ini
[Unit]
Description=BitTorrent Headless Server Daemon
After=network.target

[Service]
Type=simple
User=bittorrent
Group=bittorrent
ExecStart=/usr/local/bin/bt_daemon --dir /var/torrents/downloads --data-dir /var/torrents/data
Restart=on-failure
RestartSec=5s
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

2. Enable and start the service:
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now bt-daemon
```

---

## 3. Web UI Gateway (`bt_webui`)

`bt_webui` serves the modern glassmorphic web control interface and acts as a REST gateway translating browser HTTP requests into IPC calls for `bt_daemon`.

### 3.1 Starting the Web UI
```bash
bt_webui --port 8080
```

Open `http://localhost:8080` in your web browser.

---

## 4. REST API Reference

`bt_webui` exposes a complete JSON REST API:

### 4.1 Swarm Management

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/api/torrents` | Returns list of all active torrents with progress, speeds, and state. |
| `POST` | `/api/torrents` | Adds a torrent (JSON payload: `{"path": "...", "dir": "..."}`). |
| `POST` | `/api/torrents/:hash/pause` | Pauses the specified torrent. |
| `POST` | `/api/torrents/:hash/resume` | Resumes the specified torrent. |
| `DELETE` | `/api/torrents/:hash` | Removes the torrent (supports `?purge=true` to delete files). |
| `POST` | `/api/torrents/:hash/super-seeding` | Toggles BEP 16 Super-Seeding (`{"enabled": true|false}`). |

### 4.2 Swarm Visualizer & Piece Heatmap

| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/api/torrents/:hash/files` | Returns array of files, sizes, and priorities. |
| `POST` | `/api/torrents/:hash/files/:index/priority` | Updates priority: `{"priority": "skip"\|"low"\|"normal"\|"high"}`. |
| `GET` | `/api/torrents/:hash/peers` | Returns connected peers, client software tags, speeds, and transport (`TCP`, `uTP`, `WebRTC`). |
| `GET` | `/api/torrents/:hash/pieces` | Returns piece completion bitmask and swarm rarity counts for the heatmap. |

### 4.3 Media Streaming Endpoint
| Method | Endpoint | Description |
| :--- | :--- | :--- |
| `GET` | `/stream/:hash/:file_index` | Streams media using HTTP `206 Partial Content` with range seeking support. |

---

## 5. Web UI Features

- **Glassmorphic Responsive Dashboard**: Dark-mode aesthetic tailored for high-DPI displays.
- **Real-Time Swarm & Piece Visualizer**: Dynamic grid canvas color-coding pieces in emerald green (verified local), sky blue (swarm availability), or slate grey (missing).
- **Client Identification**: Displays parsed client badges (`qBittorrent`, `Transmission`, `Deluge`, `WebTorrent`).
- **HTML5 In-Browser Video Player**: Interactive video modal allowing users to stream downloading video files before download finishes.
