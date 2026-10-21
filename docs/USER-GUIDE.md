# BitTorrent-rs User Guide

Welcome to **BitTorrent-rs**, an enterprise-grade, `#![no_std]`-capable BitTorrent client and library written in pure Rust.

This guide covers how to install, configure, and use the desktop client (`torrent_client`), command-line interface (`bt-cli`), headless server daemon (`bt_daemon`), and modern Web UI (`bt_webui`).

---

## Table of Contents
- [Installation & Binaries](#installation--binaries)
- [Desktop Client (`torrent_client`)](#desktop-client-torrent_client)
  - [Interface Overview](#interface-overview)
  - [Preferences & Settings](#preferences--settings)
  - [Keyboard Shortcuts](#keyboard-shortcuts)
  - [Integrated Media Streaming](#integrated-media-streaming)
- [Headless Daemon (`bt_daemon`)](#headless-daemon-bt_daemon)
  - [Running the Daemon](#running-the-daemon)
  - [systemd Service Installation](#systemd-service-installation)
- [Command-Line Client (`bt-cli`)](#command-line-client-bt-cli)
  - [Commands Reference](#commands-reference)
- [Web UI Dashboard (`bt_webui`)](#web-ui-dashboard-bt_webui)
- [Configuration & State Files](#configuration--state-files)
- [Troubleshooting & FAQ](#troubleshooting--faq)

---

## Installation & Binaries

Pre-compiled release binaries are published automatically for Windows, Linux, and macOS on the [GitHub Releases page](https://github.com/clockworkengineer/BitTorrent/releases).

### Building from Source

Ensure you have Rust 1.80+ installed:

```bash
git clone https://github.com/clockworkengineer/BitTorrent.git
cd BitTorrent
cargo build --release
```

Compiled executables will be located in `target/release/`:
- `torrent_client` (Desktop GUI)
- `bt_daemon` (Headless daemon)
- `bt-cli` (CLI management tool)
- `bt_webui` (Web UI gateway)

---

## Desktop Client (`torrent_client`)

Launch the graphical interface:
```bash
./target/release/torrent_client
```

### Interface Overview
- **Sidebar Status Filters**: Quickly filter swarms by `All`, `📥 Downloading`, `📤 Seeding`, `⏸ Paused`, or `✅ Completed`.
- **Interactive Sort Columns**: Click headers (`Name`, `Progress`, `Size`, `Status`, `↓ Speed`, `↑ Speed`, `Peers`) to toggle ascending/descending order.
- **Details Panel**: Select any torrent to view:
  - **Overview**: Info hash, save folder, piece length, uploaded/downloaded ratios, and time active.
  - **Files**: Inspect multi-file structures, file sizes, and priority toggles.
  - **Peers**: Live list of connected swarm peers, IP addresses, client software strings, and transfer rates.
  - **Pieces**: Color-coded piece verification map indicating downloaded, verified, and missing pieces.
  - **Trackers**: Announce URLs, tier indices, status messages, and seed/leech counts.
  - **Logs**: Real-time swarm diagnostic messages and search filter.
- **Safe Removal**: Clicking `🗑 Delete` brings up a confirmation dialog with an option to preserve or permanently delete downloaded data from disk.

### Preferences & Settings
Access the Settings dialog by clicking `⚙ Settings` in the top toolbar or pressing `Ctrl+,` (`Cmd+,` on macOS):
- **Storage & Downloads**: Customize default download path and maximum concurrent active sessions.
- **Network & Protocols**: Configure listening port (default `6881`), enable/disable Kademlia DHT (BEP 5), Local Service Discovery (BEP 14), and uTP transport protocol (BEP 29).
- **Privacy & Encryption**: Toggle Message Stream Encryption (MSE / PE) for obfuscating traffic against ISP bandwidth throttling.

### Keyboard Shortcuts
| Shortcut | Action |
| :--- | :--- |
| `Ctrl+O` / `Cmd+O` | Open file browser to select a `.torrent` file |
| `Ctrl+,` / `Cmd+,` | Open Preferences / Settings dialog |
| `Space` | Pause or Resume the currently highlighted torrent |
| `Delete` | Open Remove confirmation modal for the selected torrent |

### Integrated Media Streaming
Watch audio/video files before the download finishes:
1. Click `▶ Stream` on any downloading torrent.
2. The built-in streaming server will prioritize initial and end pieces and launch your system default media player (VLC, MPV, Windows Media Player).
3. Copy the stream URL at any time to stream to another machine on your local network.

---

## Headless Daemon (`bt_daemon`)

`bt_daemon` is a lightweight, headless daemon designed for remote servers, seedboxes, and home NAS devices. It maintains swarm states, verifies chunks asynchronously, and handles local IPC requests over Unix sockets or Windows Named Pipes.

### Running the Daemon

```bash
# Launch with default download directory
bt_daemon --dir /path/to/downloads
```

### systemd Service Installation (Linux)

Install the unit file provided in the repository:

```bash
sudo cp clients/bt_daemon/bt-daemon.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now bt-daemon
```

Verify service status:
```bash
sudo systemctl status bt-daemon
```

---

## Command-Line Client (`bt-cli`)

`bt-cli` communicates directly with the running `bt_daemon` instance.

### Commands Reference

- **Add a Torrent / Magnet link**:
  ```bash
  bt-cli add "magnet:?xt=urn:btih:..." --dir /downloads
  bt-cli add ubuntu.torrent
  ```
- **List All Torrents**:
  ```bash
  bt-cli list
  ```
- **Inspect Detailed Swarm Status**:
  ```bash
  bt-cli status <info_hash>
  ```
- **Pause & Resume**:
  ```bash
  bt-cli pause <info_hash>
  bt-cli resume <info_hash>
  ```
- **List Torrent Files**:
  ```bash
  bt-cli files <info_hash>
  ```
- **Set File Download Priority**:
  ```bash
  bt-cli priority <info_hash> <file_index> high|normal|low|skip
  ```
- **Toggle Super-Seeding (Initial Seeding / BEP 16)**:
  ```bash
  bt-cli super-seed <info_hash> on|off
  ```
- **Remove a Torrent**:
  ```bash
  bt-cli remove <info_hash>
  # To also delete files on disk:
  bt-cli remove <info_hash> --purge
  ```

---

## Web UI Dashboard (`bt_webui`)

`bt_webui` provides an embedded single-page application dashboard for controlling `bt_daemon` from any desktop or mobile browser.

### Starting the Web UI

```bash
# Default: listens on 127.0.0.1:8080
bt_webui

# Custom port and external binding:
bt_webui --host 0.0.0.0 --port 8080 --stream-port 8081
```

Open `http://localhost:8080` in your browser.

- **Real-Time Swarm Metrics**: WebSocket feeds push download/upload rates, ETA, seed/leech swarms, and piece maps live without page reloads.
- **Health Check Endpoint**: `/api/health` returns JSON `{"status": "ok", "service": "bt_webui"}` for load balancers and container orchestrators.

---

## Configuration & State Files

Configuration and fast-resume states are saved automatically:
- **Windows**: `%LOCALAPPDATA%\BitTorrent-rs\client_state.json`
- **Linux / macOS**: `~/.local/share/BitTorrent-rs/client_state.json`

To reset the client to default state, remove the `client_state.json` file when the client is closed.

---

## Troubleshooting & FAQ

### Zero Peers Found
1. Ensure UDP and TCP traffic on your chosen listening port (default `6881`) is allowed by your firewall.
2. In Settings, confirm **Enable Kademlia DHT (BEP 5)** and **Enable Local Service Discovery (BEP 14)** are checked.
3. For corporate or restricted networks, enable **Message Stream Encryption (MSE)** to prevent protocol filtering.

### Verifying Local Files (Fast Resume)
BitTorrent-rs caches piece verification bitfields in `client_state.json`. When restarting the app, files will not undergo slow full re-hashing unless missing piece data is detected.
