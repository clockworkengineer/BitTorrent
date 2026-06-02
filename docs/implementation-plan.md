# Implementation Plan: Missing BitTorrent Functionality

## Summary
The core download path (handshake → piece request → block assembly → disk write) works. The gaps fall into four phases: correctness issues that break BEP 3 compliance, performance improvements, resilience improvements, and dead-code cleanup.

---

## Phase 1 — Critical correctness gaps

### 1. Have message broadcast
**File:** `library/src/torrent_context.rs`, `library/src/peer.rs`

**Gap:** After a piece is written to disk, no `Have(piece_index)` message is broadcast to connected peers. Peers therefore never learn what we have, which violates BEP 3 and prevents seeding.

**Change:** In `TorrentContext::process_piece_block`, after `mark_piece_local` succeeds, iterate `peer_swarm` and call `peer.send_have(piece_index)` for each connected peer.

---

### 2. Upload / serve incoming `Request` messages
**Files:** `library/src/peer.rs:277`, `library/src/disk_io.rs`

**Gap:** `Peer::handle_peer_message` silently drops `PeerMessage::Request { .. }`. A seeder (or a leecher that has completed pieces) cannot serve data to other peers.

**Changes:**
- Add `DiskIO::read_piece_block(&self, tc: &TorrentContext, piece: u32, begin: u32, length: u32) -> Result<Vec<u8>, BitTorrentError>` — mirrors `write_piece` but reads and returns a slice.
- In the `PeerMessage::Request { index, begin, length }` match arm in `peer.rs`:
  1. If `am_choking == true`, ignore.
  2. Call `disk_io.read_piece_block(tc, index, begin, length)`.
  3. Send `PeerMessage::Piece { index, begin, block }`.

---

### 3. Send initial `Bitfield` after handshake
**File:** `library/src/session.rs`

**Gap:** `connect_and_download_peer` sends `Interested` after handshake but never sends our own `Bitfield`. Remote peers therefore don't know what we have and cannot request pieces from us.

**Change:** After `handshake()` succeeds, call `peer.send_bitfield(context.bitfield.clone())`.

---

### 4. Unchoke interested peers
**Files:** `library/src/peer.rs`, `library/src/session.rs`

**Gap:** `Peer::am_choking` is always `true`. The `Interested` handler sets `peer_interested = true` but never sends `Unchoke` back. We will never serve data even once we implement request handling.

**Changes:**
- After handshake, send `PeerMessage::Unchoke` to signal we are willing to serve.
- In the `Interested` match arm, send `PeerMessage::Unchoke` and set `am_choking = false`.

---

## Phase 2 — Performance and protocol completeness

### 5. Endgame mode
**Files:** `library/src/torrent_context.rs`, `library/src/constants.rs`

**Gap:** When only a few pieces remain, a single slow peer becomes a bottleneck. The standard fix is to request the same outstanding blocks from multiple peers simultaneously, then cancel duplicates when the first one arrives.

**Changes:**
- Add `ENDGAME_THRESHOLD: u32 = 5` to `constants.rs`.
- In `TorrentContext::next_block_request_for_peer`, if `pieces_missing <= ENDGAME_THRESHOLD`, allow returning blocks already in `requested_blocks`.
- When a block is received in `handle_peer_message`, if in endgame, iterate `peer_swarm` and send `Cancel { index, begin, length }` to peers that were sent the same request.

---

### 6. Periodic tracker re-announce
**File:** `library/src/session.rs`

**Gap:** `Tracker` stores `interval` from the announce response but never re-announces. Most trackers drop peers after 2–3 missed intervals.

**Change:** After `download_from_peers`, spawn a background thread that sleeps for `announce_response.interval` seconds, calls `tracker.announce_once(TrackerEvent::None)`, feeds any new peers into `connect_and_download_peer`, and exits when `context.status == TorrentStatus::Ended`.

---

### 7. Announce `completed` event
**File:** `library/src/session.rs`

**Gap:** The tracker never receives a `completed` event. This is required by BEP 3.

**Change:** In `TorrentSession`, after `download_finished.set()`, call `tracker.announce_once(TrackerEvent::Completed)`.

---

## Phase 3 — Resilience

### 8. Dead peer list expiry
**Files:** `library/src/manager.rs`, `library/src/constants.rs`

**Gap:** `dead_peers` is a `HashSet<String>` with no expiry. A transiently unreachable peer is blacklisted forever.

**Changes:**
- Replace `dead_peers: RwLock<HashSet<String>>` with `dead_peers: RwLock<HashMap<String, Instant>>`.
- In `is_peer_dead`, return `false` once `elapsed() > DEAD_PEER_TTL`.
- Add `const DEAD_PEER_TTL: Duration = Duration::from_secs(600)` to `constants.rs`.

---

### 9. UDP tracker connection-ID expiry
**File:** `library/src/announcer.rs`

**Gap:** `UdpAnnouncer` reuses `connection_id` indefinitely. The UDP tracker protocol requires re-connecting if the ID is more than 60 seconds old.

**Change:** Add `connected_at: Option<Instant>` to `UdpAnnouncer`. In `announce`, if `connected && connected_at.unwrap().elapsed() > Duration::from_secs(60)`, reset `connected = false` before calling connect.

---

## Phase 4 — Cleanup

### 10. Remove `place_block_into_piece` (dead code)
**File:** `library/src/peer.rs:291–314`

This method references a removed `read_buffer()` path and is never called. Remove it.

### 11. Remove unused tracker callback field
**File:** `library/src/tracker.rs`

`Tracker.callback: Option<TrackerCallback>` is declared but never set or invoked. Remove it.

### 12. Remove `TorrentSession::process_peer_message`
**File:** `library/src/session.rs:153–172`

All message processing runs through `Peer::handle_peer_message`. This method is never called. Remove it.

---

## Files to modify

| File | Phase | Changes |
|------|-------|---------|
| `library/src/peer.rs` | 1, 4 | Handle `Request`, send `Unchoke`, remove dead code |
| `library/src/disk_io.rs` | 1 | Add `read_piece_block()` |
| `library/src/torrent_context.rs` | 1, 2 | Broadcast `Have`; endgame in `next_block_request_for_peer` |
| `library/src/session.rs` | 1, 2, 4 | Send `Bitfield` after handshake; re-announce thread; `completed` event; remove dead method |
| `library/src/manager.rs` | 3 | Timestamp-based dead-peer expiry |
| `library/src/announcer.rs` | 3 | UDP connection-ID TTL |
| `library/src/tracker.rs` | 4 | Remove unused `callback` field |
| `library/src/constants.rs` | 2, 3 | `ENDGAME_THRESHOLD`, `DEAD_PEER_TTL` |

---

## Out of scope
- DHT / PEX peer discovery
- Magnet links / metadata extension protocol
- Tit-for-tat choking algorithm (requires per-peer upload rate metering)
- Multi-tracker parallel announce (BEP 12)

---

## Verification
1. Run `cargo test` — all existing tests must continue to pass.
2. Add a unit test that delivers a `Request` message to a `Peer` and asserts a `Piece` response is sent.
3. Add a session test that confirms `Bitfield` is sent after handshake.
4. Integration test: extend `integration_tests.rs` to verify the tracker receives a `completed` event.
5. Manual smoke test with the `torrent_client` binary against a small torrent — confirm hashes pass, tracker events fire, and a second client can download from us.
