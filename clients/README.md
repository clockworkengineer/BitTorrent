# Client Applications & Ecosystem

This directory contains executable applications and client utilities built on top of the core `bittorrent-rs` library.

---

## Workspace Projects

### 1. Desktop GUI Client (`torrent_client`)
A cross-platform graphical client implemented with `egui` and `eframe`.
- **Features**: Visual piece heatmap grid, live peer swarm table with client badges, integrated media streaming player launcher, multi-file priority toggles, and settings modal.
- **Run from repository root**:
  ```bash
  cargo run -p torrent_client --release -- [torrent-file] [download-dir]
  ```

### 2. Headless Server Daemon (`bt_daemon`)
A lightweight, headless background service designed for remote servers, seedboxes, and home NAS appliances.
- **Features**: Multi-torrent session management, high-performance local IPC service (Unix domain sockets / Windows Named Pipes), embedded HTTP streaming server on port 8081 (`206 Partial Content`), and automatic `.fastresume` state persistence.
- **Run daemon**:
  ```bash
  cargo run -p bt_daemon --release -- --dir ./downloads
  ```

### 3. Command-Line Management Tool (`bt-cli`)
The companion CLI binary packaged alongside `bt_daemon`.
- **Features**: Querying live swarm stats, file listings, priority management, piece heatmaps, super-seeding control, legacy v1 to v2 upgrading, and parallel rechecking.
- **Run CLI commands**:
  ```bash
  cargo run -p bt_daemon --bin bt-cli -- list
  cargo run -p bt_daemon --bin bt-cli -- peers <info-hash>
  cargo run -p bt_daemon --bin bt-cli -- pieces <info-hash>
  cargo run -p bt_daemon --bin bt-cli -- upgrade input.torrent ./downloads --format hybrid
  ```

### 4. Web UI Dashboard Gateway (`bt_webui`)
A modern glassmorphic web dashboard providing remote browser management for `bt_daemon`.
- **Features**: Real-time status cards, interactive piece visualizer modal, file selection & priority controls, and HTML5 embedded video player.
- **Run Web UI**:
  ```bash
  cargo run -p bt_webui --release -- --port 8080
  ```
  Open `http://localhost:8080` in your web browser.

### 5. Local-First Encrypted Sync Hub (`sync_hub`)
A decentralized folder synchronization client designed for local LAN environments.
- **Features**: Filesystem change watching, dynamic private `.torrent` generation, LAN-only LSD discovery (BEP 14), and mandatory Message Stream Encryption (MSE / PE).
- **Run Sync Hub**:
  ```bash
  cargo run -p sync_hub --release -- ./my_folder [port]
  ```

### 6. Shared Client Library (`torrent_client_shared`)
Common data types, IPC protocol messages (`IpcMessage` / `IpcReply`), formatters, and telemetry models shared across the GUI, CLI, daemon, and Web UI.
