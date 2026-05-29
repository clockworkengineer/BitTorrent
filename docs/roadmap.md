# Torrent Client Roadmap

## Goal
Make the current library a fully functional BitTorrent downloader with:
- complete torrent metadata handling
- tracker announce and peer discovery
- peer handshake and message exchange
- piece selection, request, assembly, and disk write
- download lifecycle, seeding, and error recovery

## Current state
- Torrent metadata parsing exists via `MetaInfoFile`
- Disk preallocation exists via `DiskIO::create_local_torrent_structure`
- Tracker announce support exists via `Tracker::announce_once` and `Tracker::start_announcing`
- Peer structure exists, but actual handshake/message processing and download logic are not implemented
- `DiskIO` background worker is currently a stub

## Phases

### Phase 1: Stabilize core torrent state
1. Define a `TorrentSession` or `TorrentClient` API around `TorrentContext`
2. Ensure `TorrentContext::new` can be created cleanly for both download and seeding
3. Add explicit state transitions: `Initialised`, `Downloading`, `Seeding`, `Paused`, `Ended`
4. Add robust validation for torrent files and local file structure

### Phase 2: Tracker and peer discovery
1. Harden tracker announce/response parsing and error handling
2. Support tracker `started`, `stopped`, and `completed` events
3. Expose peer list output from tracker announces
4. Add a configurable peer discovery queue for `Manager`

### Phase 3: Peer wire protocol
1. Implement BitTorrent handshake and peer ID exchange in `Peer`
2. Add message parsing and processing loop in `PeerNetwork`
3. Manage `choke/unchoke`, `interested/not interested`, `have`, and `bitfield` messages
4. Track peer state transitions and support optimistic unchoking

### Phase 4: Piece selection and requests
1. Improve `Selector` to choose rarest-first / endgame-safe pieces
2. Implement block request window management and rate limiting
3. Route piece requests into `DiskIO` and peer messaging
4. Update `TorrentContext` to manage outstanding requests and request retries

### Phase 5: Piece assembly and disk write
1. Implement block assembly in `AssemblerData` and `PieceBuffer`
2. Verify block hashes against torrent piece hashes
3. Write complete pieces to disk on success
4. Mark pieces as complete in local bitfield and update download progress

### Phase 6: Download lifecycle and resilience
1. Add download loops for multiple peers and parallelism
2. Detect slow/freezing peers and blacklist dead peers via `Manager`
3. Support pause/resume and graceful shutdown
4. Implement seeding logic once download completes

### Phase 7: Documentation, examples, and tests
1. Add integration tests for metadata parsing, tracker announce, and file creation
2. Add end-to-end example(s) under `examples/`
3. Document public API, architecture, and usage patterns
4. Create a root `README.md` or `docs/README.md`
