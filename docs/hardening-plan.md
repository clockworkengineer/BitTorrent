# Concrete Hardening Plan: BitTorrent Rust Library

This document outlines a concrete plan to harden the `bittorrent-rs` library against security vulnerabilities, resource exhaustion, denial-of-service (DoS) attacks, and cryptographic weaknesses.

---

## 1. Cryptographic Handshake Hardening (MSE/PE)
*Obfuscating peer connections using strong and secure cryptography to bypass traffic shaping.*

### Current Weakness
- `DiffieHellman` in `mse.rs` uses a 128-bit safe prime and a 64-bit private key size. A 64-bit key size is easily brute-forced on consumer hardware, compromising connection secrecy.
- Uses `rand::random()` directly, which does not guarantee cryptographic randomness (CSPRNG) on all platforms.

### Concrete Hardening Steps
- **Upgrade DH Key Size:**
  - Increase the Diffie-Hellman prime parameters to use the standard 768-bit or 1024-bit primes specified in the BitTorrent MSE specification.
  - Implement or integrate a lightweight multi-precision integer (BigInt) module compatible with `#![no_std]` targets.
- **Ensure CSPRNG Usage:**
  - Update `rand` calls to utilize `rand::rngs::StdRng` or the `getrandom` crate to guarantee cryptographically secure random private keys across platforms.

---

## 2. Frame-Size and Payload Validation
*Protecting the peer message framer from buffer overflows, memory exhaustion, and fake message floods.*

### Current Weakness
- `PeerNetwork::read_message` reads any message length prefix up to the size of the receive buffer.
- Malicious peers can flood the connection with invalid, oversized length prefixes, causing parsing overhead and socket disconnections.

### Concrete Hardening Steps
- **Per-Message Type Length Limits:**
  - Enforce maximum expected length checks depending on the message ID byte:
    - Control messages (`Choke`, `Unchoke`, `Interested`, `NotInterested`, `HaveAll`, `HaveNone`): exactly 1 or 5 bytes.
    - `Have`, `Suggest`, `AllowedFast`: exactly 5 bytes.
    - `Request`, `Cancel`, `Reject`: exactly 13 bytes.
    - `Piece` message: maximum of 16 KiB (standard block size) + 9 bytes header overhead.
  - Instantly close the connection and temporarily blacklist any peer sending a message length outside these strict boundaries.

---

## 3. Strict Path Traversal and Device Name Validation
*Preventing malicious torrent metainfo files from writing files outside the download directory.*

### Current Weakness
- `validate_relative_path` checks for standard relative segments (`.` and `..`), but does not check for Windows-specific reserved file names (e.g. `CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`, `LPT1` to `LPT9`) or NTFS alternative data streams (`filename:stream_name`), which could bypass checks and write to special device handles.

### Concrete Hardening Steps
- **Reserved Names Blacklist:**
  - Filter out filenames matching Windows reserved device names (case-insensitively).
- **Alternative Data Stream Protection:**
  - Reject any path segment containing the `:` character to prevent writing into alternative NTFS data streams.
- **Path Canonicalization Check:**
  - After joining the download directory with the relative torrent file paths, canonicalize the path and verify it starts with the canonicalized download directory path, preventing any directory escape.

---

## 4. Resource Allocation & Connection Throttling
*Preventing thread, socket, and memory exhaustion under high swarm sizes or active floods.*

### Current Weakness
- Spawns an OS thread for every peer session connection. Swarms with hundreds of peers can cause thread exhaustion and crash the client.
- The peer discovery queues can grow without bounds if trackers or DHT return millions of records.

### Concrete Hardening Steps
- **Thread Pools or Async Event Loops:**
  - Transition from spawning raw OS threads per peer connection to executing connections asynchronously on a bounded thread pool or single-threaded async event loop using cooperative tasks.
- **Connection Rate Limiter:**
  - Impose a global limit on the number of concurrent connections (e.g. max 200) and connection attempts per second (e.g. max 5 connection attempts per second).
- **Peer List Bounding:**
  - Cap the internal peer swarm and discovery queue size. Drop new peer announcements if the queue exceeds a safe capacity (e.g., 1000 candidates).

---

## 5. Swarm Contribution Enforcement (Tit-for-Tat Choking)
*Preventing leeching and defending against slow-sender or bad-faith peer behaviors.*

### Current Weakness
- The choking algorithm unchokes the top uploading peers, but lacks a strict tit-for-tat enforcement threshold.
- Peers can reserve slots without contributing or trickle byte transmissions to keep slots open.

### Concrete Hardening Steps
- **Strict Contribution Thresholds:**
  - Require peers in active slots to upload at a minimum rate (e.g., > 1 KiB/s) to keep their unchoked status.
- **Bad Behavior Scoring:**
  - Implement a reputation score for each peer. Deduct points for requesting blocks repeatedly and timing out, sending corrupted block hashes, or failing to respond to KeepAlives. Blacklist peers when their score falls below a threshold.
