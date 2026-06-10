# Implementation Plan: Advanced Missing Features (Choking, Keep-Alives, & PEX)

This document presents the analysis, concrete design, and proposed code changes to add three advanced protocol features to the BitTorrent client library:
1. **Tit-for-Tat Choking & Optimistic Unchoking (BEP 3)**
2. **Connection Keep-Alives (BEP 3)**
3. **Peer Exchange (PEX - BEP 11)**

---

## 1. Tit-for-Tat Choking & Optimistic Unchoking (BEP 3)

### Current State
Currently, the library immediately unchokes any peer that sends an `Interested` message (i.e. `am_choking` is set to `false`). This permits unrestricted upload connections, which leads to bandwidth dilution, resource contention, and vulnerability to free-riding peers.

### Proposed Design
Implement the standard BitTorrent choking algorithm:
1. **Upload Slots Limit**: Limit the number of concurrently unchoked peers to a fixed count (e.g., `max_upload_slots = 4`).
2. **Bandwidth Tracking**: Monitor download rates from each peer over a sliding window (e.g., rolling average) to identify who uploads to us fastest.
3. **Periodic Review Loop**: Run a recurring task every 10 seconds:
   - Sort interested peers by their rolling average download rate (for active downloading) or upload rate (for seeding).
   - Unchoke the top `max_upload_slots - 1` (e.g., top 3) peers (Tit-for-Tat).
   - Choke any other previously unchoked peers that did not make the cut.
4. **Optimistic Unchoke**: Every 30 seconds (every third loop), select one random interested peer that is currently choked and unchoke it to explore its upload potential.

### Proposed Code Changes
- **`library/src/peer.rs`**:
  - Add fields `rolling_download_rate: Average` and `rolling_upload_rate: Average`.
  - Update rates whenever `PeerMessage::Piece` (download) or `PeerMessage::Request` (upload) is processed.
- **`library/src/torrent_context.rs`**:
  - Add `max_upload_slots: usize` field.
  - Implement a method `recalculate_choking_states(&self)` to select peers for unchoking/choking based on rate statistics and return a list of `PeerAction` commands.
- **`library/src/session.rs`**:
  - Spawn a recurring background task in the executor that locks `TorrentContext` every 10 seconds and triggers `recalculate_choking_states()`.

---

## 2. Connection Keep-Alives (BEP 3)

### Current State
The library decodes incoming `KeepAlive` frames but does not send them. The worker thread loops will timeout and drop connections if there is no socket activity for 30 seconds, which is overly aggressive and can drop healthy connections during idle periods.

### Proposed Design
Implement standard keep-alive handling:
1. **Keep-Alive Transmissions**: Send a `KeepAlive` message (length prefix `[0, 0, 0, 0]`) to all connected peers if no messages have been sent to them for 120 seconds.
2. **Idle Timeouts**: Close peer connections and mark them as dead only if no packet has been received from them for more than 120 seconds.

### Proposed Code Changes
- **`library/src/peer.rs`**:
  - Add fields `last_message_sent: Instant` and `last_message_received: Instant`.
  - Update `last_message_sent` on any `send_message()` call, and `last_message_received` in the worker read loop.
- **`library/src/session/worker.rs`**:
  - Inside the main read loop, check if `last_message_sent.elapsed() > Duration::from_secs(120)`. If so, send a `PeerMessage::KeepAlive`.
  - Modify the timeout check to drop connections only if `last_message_received.elapsed() > Duration::from_secs(120)`.

---

## 3. Peer Exchange (PEX - BEP 11)

### Current State
The client relies on tracker announcements and DHT queries for peer discovery. It does not trade peer lists directly with connected peers, which places unnecessary load on trackers.

### Proposed Design
Implement Peer Exchange (PEX) via the Extension Protocol:
1. **Handshake Negotiation**: Advertise support for PEX by mapping `"ut_pex"` to a local ID (e.g. `2`) in the extended handshake.
2. **PEX Messages**: Send periodic (e.g., once per minute) PEX updates to connected peers containing:
   - `added`: a compact string representation of peer IPv4 addresses and ports recently discovered.
   - `dropped`: a compact string representation of peers recently closed or marked dead.
3. **PEX Reception**: Parse incoming `"ut_pex"` messages, extract peer IP/ports, and push them to the session's programmatic discovery queue.

### Proposed Code Changes
- **`library/src/peer.rs`**:
  - Handle `PeerMessage::Extended` for `ut_pex` local extension IDs.
  - Decode the bencoded PEX payload to extract `added` and `dropped` IP/port lists.
- **`library/src/torrent_context.rs`**:
  - Maintain a list of recently added and dropped peers since the last PEX broadcast.
- **`library/src/session/worker.rs`**:
  - Periodically construct and send PEX dictionaries to peers that advertised `ut_pex` support.
  - Push newly received PEX peers to the `Manager` discovery queues.

---

## 4. Verification Plan

### Automated Testing
1. **Choking Algorithm Unit Tests**: Inject mock peers with varying rolling download rates and assert that the unchoke review loop unchokes the top-performing peers and correctly picks a random optimistic unchoke target.
2. **Keep-Alive Tests**: Verify via `MockSocket` that the client transmits keep-alive packets during idle periods.
3. **PEX Integration Tests**: Mock PEX message exchanges and verify that newly introduced peers are discovered and connected.
