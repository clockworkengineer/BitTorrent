# Future Missing Features Implementation Plan: bittorrent-rs (Phase II)

This document outlines the analysis and concrete design for adding five next-generation standard BitTorrent protocol features to the `bittorrent-rs` library:
1. **Private Torrents (BEP 27)**
2. **Handshake Encryption / Message Stream Encryption (MSE)**
3. **uTorrent Transport Protocol (uTP - BEP 29)**
4. **Auto Port Forwarding via UPnP & NAT-PMP**
5. **BitTorrent v2 Support (BEP 52)**

---

## 1. Private Torrents (BEP 27)

### Purpose
Restricts peer discovery exclusively to the trackers specified in the `.torrent` metadata for torrents flagged as private. This is vital for private trackers to enforce ratio limits and swarm security.

### Design Details
- **Flag Parsing**: Look for the `"private"` integer key inside the `"info"` dictionary of the torrent metadata.
- **Enforcement Rules**: If `"private": 1`:
  - **Disable DHT**: Do not query or announce to Kademlia DHT routers.
  - **Disable LSD**: Suppress local multicast search queries and ignore incoming local searches for this torrent.
  - **Disable PEX**: Do not serialize PEX messages (`added`/`dropped`/`added6`/`dropped6`) or process incoming PEX updates.

### Proposed Code Changes
- **[metainfo.rs](file:///c:/Projects/BitTorrent/library/src/metainfo.rs)**:
  - Add `is_private()` helper parsing the `"private"` entry from the `"info"` dictionary.
- **[torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs)**:
  - Expose a `pub is_private: bool` field on the `TorrentContext` struct.
- **[session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)** & **[session/worker.rs](file:///c:/Projects/BitTorrent/library/src/session/worker.rs)**:
  - Check `is_private` before spinning up LSD announcer/listener, bootstrap DHT, or broadcasting/processing PEX lists.

---

## 2. Handshake Encryption / Message Stream Encryption (MSE)

### Purpose
Obfuscates the protocol header and payload to prevent passive traffic analysis, helping bypass ISP traffic shaping and throttling of BitTorrent streams.

### Design Details
- **Negotiation Cryptography**:
  - Diffie-Hellman (DH) key exchange with a 768-bit or 1024-bit prime to establish a shared secret.
  - RC4 stream cipher to wrap the payload stream.
- **Fallback Configurations**:
  - Support configuration profiles: Plaintext only, Encryption preferred, Encryption required.

### Proposed Code Changes
- **[peer_network.rs](file:///c:/Projects/BitTorrent/library/src/peer_network.rs)** [NEW helper mod / traits]:
  - Implement DH key negotiation and RC4 cipher wrapping layers.
- **[session/worker.rs](file:///c:/Projects/BitTorrent/library/src/session/worker.rs)**:
  - Insert cryptographic negotiation phase prior to executing standard handshake writes.

---

## 3. uTorrent Transport Protocol (uTP - BEP 29)

### Purpose
Implements a UDP-based congestion control protocol (using the LEDBAT algorithm) to throttle transfer speed based on latency cues, preventing the torrent client from overloading local home routers and choking domestic traffic.

### Design Details
- **Frame Packaging**: Custom header structure (Type, Version, Extension, Connection ID, Timestamp, Delay Difference, Ack Number).
- **Delay-Based Control**: Measure one-way delay differences to determine network queuing delay. If queuing delay exceeds 100ms, decrease congestion window size.

### Proposed Code Changes
- **`library/src/utp.rs` [NEW]**:
  - Implement the LEDBAT algorithm, connection state machines (SYN, ACK, DATA, FIN), and frame parsing.
- **[peer_network.rs](file:///c:/Projects/BitTorrent/library/src/peer_network.rs)**:
  - Support binding UDP sockets as a socket factory source for standard peer sessions.

---

## 4. Port Forwarding via UPnP & NAT-PMP

### Purpose
Automatically opens ports on home NAT routers, allowing the client to receive incoming TCP/UDP connections from remote peers, which significantly enhances peer discovery and transfer speeds.

### Design Details
- **UPnP (Universal Plug and Play)**:
  - Broadcast SSDP M-SEARCH queries over UDP `239.255.255.250:1900` to locate router IGD (Internet Gateway Device).
  - Use SOAP calls over HTTP to map ports.
- **NAT-PMP / PCP (Port Control Protocol)**:
  - Query router gateway IP on port `5351` using UDP to map ports.

### Proposed Code Changes
- **`library/src/nat.rs` [NEW]**:
  - Implement SSDP discovery, SOAP request builder, and NAT-PMP packet layout.
- **[session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)**:
  - Map external ports on session instantiation, and clean up mappings (delete port map actions) on session exit.

---

## 5. BitTorrent v2 Support (BEP 52)

### Purpose
Transitions metadata parsing and verification systems to BitTorrent v2 guidelines, addressing security risks in SHA-1 and enabling more efficient file verification.

### Design Details
- **Hash Functions**: Upgrades piece validation and info-hash computation from SHA-1 (20-byte) to SHA-256 (32-byte).
- **File Trees**: Replaces the flat `"files"` dictionary with a `"file tree"` directory dictionary structure.
- **Merkle Trees**: Validates piece block lists via per-file Merkle trees rather than a global flat hash block array, allowing verification of individual blocks/files on demand.

### Proposed Code Changes
- **[metainfo.rs](file:///c:/Projects/BitTorrent/library/src/metainfo.rs)**:
  - Support parsing v2 structures and calculating SHA-256 info-hashes.
- **[torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs)** & **[disk_io.rs](file:///c:/Projects/BitTorrent/library/src/disk_io.rs)**:
  - Implement Merkle path validation and store 32-byte piece verification hashes.
