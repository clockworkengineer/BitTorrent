# Architecture Overview

## Crate layout
- `library/src/` — library code
- `library/tests/` — test suite
- `clients/torrent_client/` — CLI client binary
- `examples/torrent_session_example/` — example application

## Key components

### Session layer
- `TorrentSession` — high-level download orchestrator; owns `TorrentContext`, `DiskIO`, and a pool of per-peer worker threads; exposes `start_download()`, `pause()`, `resume()`, `stop()`, and `wait_for_download_finished()`
- `Manager` — registry for active torrents (keyed by info hash) and a blacklist of dead peers; routes newly discovered peers via an MPSC sender

### Torrent state
- `TorrentContext` — central mutable state: info hash, piece metadata, local bitfield, peer swarm (`HashMap<String, Arc<Mutex<Peer>>>`), in-progress piece buffers (`AssemblerData`), and outstanding block requests; exposes `next_block_request_for_peer()` (rarest-first selection) and `process_piece_block()` (assembly + SHA1 validation)
- `TorrentStatus` enum — `Initialised → Downloading → Paused → Seeding → Ended`
- `MetaInfoFile` — parses `.torrent` metadata: info hash (SHA1 of info dict), piece hashes, piece length, file layout, and tracker URL list (primary + backup)

### Peer layer
- `Peer` — per-peer state: remote bitfield, choke/interest flags, outstanding request count, reserved block list; performs handshake and drives the message loop
- `PeerNetwork` — raw TCP socket wrapper; encodes and decodes handshake and length-prefixed messages
- `PeerMessage` — enum covering the full peer wire protocol: `KeepAlive`, `Choke`, `Unchoke`, `Interested`, `NotInterested`, `Have`, `Bitfield`, `Request`, `Piece`, `Cancel`, `Port`; implements `encode()`/`decode()`

### Tracker layer
- `Tracker` — manages announce lifecycle (Started, Completed, Stopped, None); parses compact peer lists into `PeerDetails`
- `Announcer` trait — implemented by `HttpAnnouncer` (bencode response) and `UdpAnnouncer` (binary connection/announce protocol); `AnnouncerFactory` selects implementation by URL scheme

### Disk layer
- `DiskIO` — creates local file/directory structure, scans existing data to build the initial bitfield, writes assembled pieces to the correct file offsets (handles pieces that span file boundaries), and exposes a background write queue (MPSC channel)
- `PieceBuffer` — assembles incoming blocks into a complete piece; tracks per-block completion with a countdown; signals readiness via `all_blocks_there()`
- `PieceRequest` — message type carried on the disk I/O queue (info hash, peer IP, piece number, block offset, block size)

### Selection
- `Selector` — rarest-first piece selection and peer ranking by average latency

### Utilities
- `Bencode` / `BNode` — recursive bencode parser and encoder for torrent files and tracker responses
- `ManualResetEvent` — thread synchronization primitive (set/reset/wait) used for pause and download-complete signals
- `Average` — running mean for latency/bandwidth metrics
- `peer_id` — generates a random peer ID (`-AZ1000-{12-char random}`)
- `host` — resolves the local IP address via a non-sending UDP probe
- `constants` — `BLOCK_SIZE` (16 KiB), `HASH_LENGTH` (20), `PEER_ID_LENGTH` (20), `INITIAL_HANDSHAKE_LENGTH` (68), `MAXIMUM_SWARM_SIZE` (100)
- `util` — big-endian pack/unpack helpers and info-hash hex encoding
- `error` — `BitTorrentError` enum (`Io`, `InvalidBencode`, `MissingField`, `Parse`, `NotParsed`)

## Data flow

```
TorrentSession::start_download()
  │
  ├─ DiskIO::create_local_torrent_structure()
  ├─ DiskIO::create_torrent_bitfield()     ← resume support
  ├─ Tracker::announce_started()           ← discovers PeerDetails
  │
  └─ per peer: spawn connect_and_download_peer() thread
       │
       ├─ Peer::handshake()                ← TCP connect + protocol handshake
       ├─ send/receive Bitfield            ← update remote_piece_bitfield
       ├─ send Interested / receive Unchoke
       │
       └─ message loop
            ├─ TorrentContext::next_block_request_for_peer()  ← rarest-first
            ├─ Peer::send_message(Request)
            ├─ Peer::read_message() → PeerMessage::Piece
            ├─ TorrentContext::process_piece_block()
            │    └─ PieceBuffer::add_block()
            │         └─ all blocks present → SHA1 check
            │              └─ DiskIO::write_piece() → filesystem
            └─ TorrentContext::mark_piece_local()
```

## Known gaps and future work

- **Endgame mode** — no special handling for the final few pieces
- **Upload / seeding** — `TorrentStatus::Seeding` exists but upload logic is not implemented
- **Peer liveness** — no keepalive timeout or reconnect logic
- **Choking algorithm** — no tit-for-tat or optimistic unchoke; peers are unchoked unconditionally
- **DHT / PEX** — no peer discovery beyond the tracker announce
- **Magnet links** — metadata extension not implemented
