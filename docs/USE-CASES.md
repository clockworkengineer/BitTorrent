# Potential Use Cases & Application Architecture

This document describes how the BitTorrent client library's modular architecture can be leveraged to build diverse, high-performance, and decentralized applications.

---

## Key Architectural Strengths

The library was designed from the ground up to separate protocol logic from side effects:
* **Strict Engine/Side-Effect Separation:** The core BitTorrent state machine, peer swarm coordination, and protocol parsing do not interact with the operating system directly. Instead, they depend on the abstract traits `BlockStorage` and `AsyncSocket`.
* **`no_std` Compatibility:** The core library does not require standard library utilities (like `std::fs`, `std::net`, or `std::thread`). It can run in any environment providing a heap allocator (`alloc`).
* **Pluggable Piece Selection:** Via the `PieceSelector` trait, developers can swap between sequential and rarest-first downloading strategies.
* **Network & Discovery Autonomy:** Embedded support for HTTP/UDP Trackers, Kademlia DHT, Local Service Discovery (LSD), Peer Exchange (PEX), and NAT-PMP port mapping.

---

## Application Ideas

### 1. Peer-to-Peer Media Streaming Player
Create an application that streams video or audio files directly from a magnet link or `.torrent` file in real-time.
* **Piece Selection:** Use `SequentialSelector` to prioritize downloading the earliest pieces of the file first to ensure fast buffering.
* **Data Flow:** Run a virtual local HTTP server within the client application. As the media player requests byte ranges via standard HTTP `GET` requests, the server reads the corresponding blocks from `BlockStorage` (buffering/waiting for missing blocks if necessary) and streams them back to the player.
* **WebSeed Fallback:** Configure the session to use WebSeeds (BEP 17/19) so that streaming can start instantly from high-bandwidth HTTP mirrors before peer connections are established.

### 2. Decentralized Game Launcher & Patcher
Build a game launcher that distributes updates and installations via peer-to-peer sharing, cutting CDN hosting costs.
* **Swarm Syncing:** Game assets and patch files are packaged into v1 or v2 torrent files.
* **Local Speedups:** Utilizing **LSD (BEP 14)** and **PEX (BEP 11)**, launchers on the same local area network (such as gaming centers, LAN parties, or university networks) share downloaded game blocks at maximum speed.
* **Background Seeding:** The launcher can run as a low-priority system tray process, seeding downloaded patches to other users.

### 3. Local-First Encrypted File-Syncing Hub
A private, secure cloud-storage alternative for teams that synchronizes files across machines on local networks.
* **Private Swarms:** Enforce **Private Torrents (BEP 27)** and turn off public DHT/trackers to keep network traffic fully local.
* **Transport Encryption:** Enable **Message Stream Encryption (MSE)** to obfuscate and secure all peer connections on the network.
* **LAN Discovery:** Rely on **LSD** to automatically detect sync nodes on the subnet, allowing high-bandwidth file syncing over local ethernet or Wi-Fi without uploading data to external cloud servers.

### 4. Browser-Based Client (WebAssembly)
Run a fully sandboxed torrent downloader directly in web browsers.
* **Wasm Compilation:** Build the library using its `#![no_std]` configuration for the `wasm32-unknown-unknown` target.
* **Custom I/O Implementation:**
  * Implement the `AsyncSocket` trait using browser WebRTC DataChannels or WebSockets (connected to signaling servers/TCP proxies).
  * Implement the `BlockStorage` trait using the browser's native **Origin Private File System (OPFS)** or **IndexedDB** for persistent sandboxed storage.
* **Zero Installation:** Users can download and upload files entirely from a webpage.

### 5. Lightweight Home NAS / Raspberry Pi Seedbox
A headless seedbox daemon with a web dashboard designed to run 24/7 on low-power devices.
* **Daemon Integration:** Run the client as a background system service (daemon) using a minimal footprint.
* **API layer:** Expose a lightweight HTTP JSON-RPC or WebSocket server from the daemon.
* **Web UI Dashboard:** Build a modern, mobile-friendly web dashboard (React, Vue, etc.) that connects to the daemon to manage downloads, limit upload/download bandwidth, and configure schedules.
