# Local-First Encrypted File-Syncing Hub (`sync_hub`)

The `sync_hub` application is a decentralized, local-first folder synchronization client built on top of the core BitTorrent library. It enables computers on the same local network to securely sync folders without relying on external cloud storage or public trackers.

---

## Key Features

* **Directory Watching:** Automatically detects file changes, additions, and deletions in real-time.
* **Dynamic Torrent Generation:** Converts local directory structures on-the-fly into private `.torrent` files.
* **Private LSD Discovery:** Bypasses standard BEP 27 restrictions to allow Local Service Discovery (LSD) on private torrents, enabling nodes to find each other on the LAN without leaking information to the internet.
* **MSE Transport Encryption:** Mandates Message Stream Encryption (MSE/PE) to secure peer-to-peer data transfers.

---

## How It Works

```
[Local Folder] ──(File Watcher)──> [Detect Change] ──> [Rebuild Private Torrent]
                                                               │
                                                               v
[LAN Peer Swarm] <──(MSE Encrypted Sync) <──(LSD Peer Discovery) <── [Start TorrentSession]
```

1. **Watch:** The application monitors a specified directory for any file write or deletion events.
2. **Rebuild:** Upon detection, the hub generates a Bencoded `.torrent` file representing the current state of files. The torrent is marked as `"private": 1`.
3. **Announce:** The node binds to a specified port and starts the `TorrentSession` using the generated torrent metadata.
4. **Discover:** Using local multicast (LSD), it advertises its info-hash to other clients on the same subnet.
5. **Sync:** Discovered peers connect to the node, verify handshake credentials, negotiate an encrypted RC4 channel (MSE), and transfer the missing pieces directly.

---

## Getting Started

### 1. Build the Sync Hub Crate
Compile the executable target from the root directory:

```bash
cargo build -p sync_hub --release
```

### 2. Run a Synchronization Instance
Start the daemon pointing to your target folder and define a listening port (defaults to `downloads/sync_folder` on port `6881` if omitted):

```bash
cargo run -p sync_hub --release -- <sync-directory> [port]
```

Example:
```bash
cargo run -p sync_hub --release -- ./my_shared_files 6882
```

### 3. Testing Local Sync Between Two Folders
To test the synchronization flow locally on the same machine:

1. Open a terminal and run the first instance pointing to folder `A` on port `6882`:
   ```bash
   cargo run -p sync_hub --release -- ./downloads/sync_a 6882
   ```
2. Open a second terminal and run the second instance pointing to folder `B` on port `6883`:
   ```bash
   cargo run -p sync_hub --release -- ./downloads/sync_b 6883
   ```
3. Add a file to `downloads/sync_a`. The first hub will rebuild the sync session. 
4. The second hub will automatically discover the peer via **LSD**, establish an **MSE encrypted connection**, and sync the newly added files to `downloads/sync_b`.
