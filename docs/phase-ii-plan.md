# Phase II Implementation Plan: bittorrent-rs

> [!NOTE]
> **Status: All items fully implemented and tested as of Phase 16.**
> This document is preserved as a historical record of the Phase II design decisions.
> See [roadmap.md](file:///c:/Projects/BitTorrent/docs/roadmap.md) for the current state.

This document outlines the analysis and concrete design for five next-generation standard BitTorrent protocol features added to the `bittorrent-rs` library:

1. **Private Torrents (BEP 27)** — ✅ Complete
2. **Handshake Encryption / Message Stream Encryption (MSE)** — ✅ Complete
3. **uTorrent Transport Protocol (uTP - BEP 29)** — ✅ Complete
4. **Auto Port Forwarding via NAT-PMP** — ✅ Complete
5. **BitTorrent v2 Support (BEP 52)** — ✅ Complete

---

## 1. Private Torrents (BEP 27) — ✅ Implemented

### Purpose
Restricts peer discovery exclusively to the trackers specified in the `.torrent` metadata for torrents flagged as private. This is vital for private trackers to enforce ratio limits and swarm security.

### Design Details
- **Flag Parsing**: Look for the `"private"` integer key inside the `"info"` dictionary of the torrent metadata.
- **Enforcement Rules**: If `"private": 1`:
  - **Disable DHT**: Do not query or announce to Kademlia DHT routers.
  - **Disable LSD**: Suppress local multicast search queries and ignore incoming local searches for this torrent.
  - **Disable PEX**: Do not serialize PEX messages (`added`/`dropped`/`added6`/`dropped6`) or process incoming PEX updates.

### Implemented In
- **[metainfo.rs](file:///c:/Projects/BitTorrent/library/src/metainfo.rs)**: `is_private()` helper
- **[torrent_context.rs](file:///c:/Projects/BitTorrent/library/src/torrent_context.rs)**: `pub is_private: bool` field
- **[session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)** & **[session/worker.rs](file:///c:/Projects/BitTorrent/library/src/session/worker.rs)**: Guards before DHT, LSD, and PEX

---

## 2. Handshake Encryption / Message Stream Encryption (MSE) — ✅ Implemented

### Purpose
Obfuscates the protocol header and payload to prevent passive traffic analysis, helping bypass ISP traffic shaping and throttling of BitTorrent streams.

### Design Details
- **Negotiation Cryptography**:
  - Diffie-Hellman (DH) key exchange with a 128-bit safe prime to establish a shared secret.
  - RC4 stream cipher to wrap the payload stream.
- **Opt-in Flag**: `SessionConfig.mse_enabled` (default `false` for backward compatibility).

### Implemented In
- **[mse.rs](file:///c:/Projects/BitTorrent/library/src/mse.rs)**: Pure-Rust `Rc4`, `DiffieHellman`, `mod_pow`, `mulmod`
- **[peer_network.rs](file:///c:/Projects/BitTorrent/library/src/peer_network.rs)**: `set_mse_ciphers()` transparent encryption
- **[session/worker.rs](file:///c:/Projects/BitTorrent/library/src/session/worker.rs)**: DH exchange + SHA-1 key derivation

See [encryption.md](file:///c:/Projects/BitTorrent/docs/encryption.md) for the full protocol walkthrough.

---

## 3. uTorrent Transport Protocol (uTP - BEP 29) — ✅ Implemented

### Purpose
Implements a UDP-based congestion control protocol (using the LEDBAT algorithm) to throttle transfer speed based on latency cues, preventing the torrent client from overloading local home routers and choking domestic traffic.

### Design Details
- **Frame Packaging**: Fixed 20-byte header (Type, Version, Extension, Connection ID, Timestamps, Window Size, Seq/Ack numbers).
- **Current Scope**: Framing and connection state machine (SYN/DATA/ACK/RESET). Full LEDBAT congestion control is a future enhancement.

### Implemented In
- **[utp.rs](file:///c:/Projects/BitTorrent/library/src/utp.rs)**: `UtpSocketAdapter`, `UtpHeader`, `UtpPacketType`
- **[lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs)**: Module registration and `UtpSocketAdapter` re-export

See [utp.md](file:///c:/Projects/BitTorrent/docs/utp.md) for the header layout and state machine.

---

## 4. Auto Port Forwarding via NAT-PMP — ✅ Implemented

### Purpose
Automatically opens ports on home NAT routers, allowing the client to receive incoming TCP/UDP connections from remote peers, significantly enhancing peer discovery and transfer speeds.

### Design Details
- **NAT-PMP / PCP**: Query router gateway IP on port `5351` using UDP to map ports.
- **Lifecycle**: Map on `start_download()`, release on `stop()`.

### Implemented In
- **[nat.rs](file:///c:/Projects/BitTorrent/library/src/nat.rs)**: `NatPmpClient`, `get_default_gateway()`
- **[session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs)**: `nat_pmp` field, lifecycle hooks

See [nat-pmp.md](file:///c:/Projects/BitTorrent/docs/nat-pmp.md) for packet formats and details.

---

## 5. BitTorrent v2 Support (BEP 52) — ✅ Implemented

### Purpose
Transitions metadata parsing and verification systems to BitTorrent v2 guidelines, addressing security risks in SHA-1 and enabling more efficient file verification.

### Design Details
- **Hash Functions**: SHA-256 (32-byte) info-hashes for v2 torrents (SHA-1 retained for v1).
- **File Trees**: Parses the `"file tree"` directory dictionary structure via recursive traversal.

### Implemented In
- **[metainfo.rs](file:///c:/Projects/BitTorrent/library/src/metainfo.rs)**: v2 detection, `file tree` traversal, SHA-256 hash computation
- **[Cargo.toml](file:///c:/Projects/BitTorrent/library/Cargo.toml)**: `sha2` crate dependency
