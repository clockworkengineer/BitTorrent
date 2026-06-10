# Future Missing Features Implementation Plan: bittorrent-rs

This plan outlines the analysis and concrete design for adding five additional standard BitTorrent protocol features to the `bittorrent-rs` library:
1. **Local Service Discovery (LSD / LPD - BEP 14)**
2. **Fast Extension (BEP 6)**
3. **WebSeeding (HTTP Seeding - BEP 17 & BEP 19)**
4. **Dual-Stack IPv6 Support & IPv6 PEX (BEP 32 & BEP 11)**
5. **Tracker Scrape Support (BEP 48)**

---

## 1. Local Service Discovery (LSD / LPD - BEP 14)

### Purpose
Allows discovering peers on the same local area network (LAN) without querying trackers or traversing the global DHT. This reduces external ISP bandwidth and increases local download rates.

### Design Details
- **Multicast Configuration**:
  - IPv4 Multicast Address: `239.192.152.143`
  - Port: `6771`
  - Announcement Interval: Every 5 minutes (300 seconds)
- **Announcement Format**: An HTTP-like multicast packet:
  ```http
  BT-SEARCH * HTTP/1.1
  Host: 239.192.152.143:6771
  Port: <local-peer-listen-port>
  Infohash: <hex-encoded-infohash>
  cookie: <opaque-session-id>
  ```
- **Proposed Code Changes**:
  - **`library/src/lsd.rs` [NEW]**:
    - Implement an `LsdListener` that binds to UDP port `6771` and joins the multicast group.
    - Parse incoming messages, verify the `Infohash`, and enqueue peer IP/ports directly to the session's discovery channel.
    - Spawn a periodic task that broadcasts the multicast search packet to advertise local existence.

---

## 2. Fast Extension (BEP 6)

### Purpose
Improves startup speeds and bandwidth utilization, particularly when a peer is choked, by allowing them to download selected blocks from an "allowed fast" set.

### Design Details
- **New Peer Message IDs**:
  - `HaveAll` (ID `14`): Announces possession of all pieces (sent instead of a large bitfield).
  - `HaveNone` (ID `15`): Announces possession of zero pieces.
  - `Suggest` (ID `13`): Suggests a piece index the receiver should request next.
  - `Reject` (ID `16`): Rejects a block request (prevents hanging requests on choke).
  - `AllowedFast` (ID `17`): Exposes a piece index that is allowed to be requested even when choked.
- **Proposed Code Changes**:
  - **`library/src/peer_message.rs`**:
    - Add variants to the `PeerMessage` enum representing `HaveAll`, `HaveNone`, `Suggest`, `Reject`, and `AllowedFast` messages.
    - Implement encoding and decoding logic matching the wire formats.
  - **`library/src/peer.rs`**:
    - Add `supports_fast_extension` property initialized during the reserved-byte handshake analysis (bit `2` of byte `7`).
    - Handle `Reject` to immediately release block requests and prevent timeouts.
  - **`library/src/selector.rs`**:
    - Update `PieceSelector` to maintain a set of "allowed fast" pieces when choked by a peer.

---

## 3. WebSeeding (HTTP Seeding - BEP 17 & BEP 19)

### Purpose
Allows clients to download piece blocks directly from web servers (CDNs/HTTP mirrors) when swarm peer speeds are insufficient or when no seeders are online.

### Design Details
- **BEP 19 (GetRight-style)**: Uses `url-list` in the metainfo dictionary containing URL endpoints.
- **HTTP Range Requests**: Fetch blocks using the `Range: bytes=start-end` HTTP header.
- **Proposed Code Changes**:
  - **`library/src/metainfo.rs`**:
    - Parse the `url-list` key (which can be a single string or a list of strings) from the metainfo dictionary.
  - **`library/src/webseed.rs` [NEW]**:
    - Model a `WebSeedWorker` that acts as a mock peer.
    - Translate block request indexes into absolute file offsets.
    - Use `HttpClient` to submit HTTP GET queries with `Range` headers to the mirrors.
    - Pass downloaded bytes to `TorrentContext` to assemble and verify pieces on disk.

---

## 4. Dual-Stack IPv6 Support & IPv6 PEX (BEP 32 & BEP 11)

### Purpose
Allows the library to work seamlessly over IPv6, query IPv6 trackers/DHT, and exchange IPv6 peer lists using Peer Exchange.

### Design Details
- **PEX Keys**:
  - `added6`: Compact byte string representation of IPv6 peers (18 bytes per peer: 16-byte IP + 2-byte port).
  - `dropped6`: Compact representation of dropped IPv6 peers.
- **Proposed Code Changes**:
  - **`library/src/peer_network.rs`**:
    - Update socket resolution and creation to handle dual-stack IPv6 endpoints.
  - **`library/src/peer.rs`**:
    - Extend PEX parsing under `Extended` message types to extract and validate IPv6 chunks from `added6` and `dropped6` payloads.
  - **`library/src/session/worker.rs`**:
    - Extend the periodic PEX broadcast task to partition the swarm into IPv4 and IPv6 swarms, encoding both `added`/`dropped` and `added6`/`dropped6` dictionaries.

---

## 5. Tracker Scrape Support (BEP 48)

### Purpose
Enables querying tracker statistics (seed count, leecher count, download completion stats) for one or more info-hashes in a single transaction without making a full announce request.

### Design Details
- **URL Mapping**: Convert announcer endpoint `/announce` to `/scrape`.
- **Response Format**: Parse a bencoded dictionary of the form:
  `d5:filesd20:<info-hash>d8:completei5e10:downloadedi100e10:incompletei10eeee`
- **Proposed Code Changes**:
  - **`library/src/tracker.rs`**:
    - Add a `scrape(&self, info_hashes: &[Vec<u8>])` method to query UDP and HTTP trackers.
    - Decode the stats and expose them to the client interface to display health summaries before launching download threads.
