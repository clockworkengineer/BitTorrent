# Architecture Overview

## Current architecture

### Crate layout
- `library/src/`: library code
- `library/tests/`: test suite
- `examples/`: placeholder for future sample applications

### Key components
- `MetaInfoFile` — parses `.torrent` metadata and extracts file layout, piece length, tracker URL, and info hash
- `TorrentContext` — holds torrent state, bitfield, file list, peer swarm, and assembly metadata
- `DiskIO` — creates local files and computes bitfield from existing disk data
- `Tracker` — performs tracker announces and translates peer lists into `PeerDetails`
- `Peer` / `PeerNetwork` — peer state holder and low-level socket wrapper
- `Selector` — chooses pieces and peers for download
- `Manager` — registry for active torrents and dead peers

## What is missing

### Peer wire protocol
- No real BitTorrent handshake implementation
- No message parser for `bitfield`, `have`, `request`, `piece`, or `choke`
- `PeerNetwork::start_reads()` is a placeholder

### Download orchestration
- No high-level download controller
- No peer download loop, message scheduling, or request retry logic
- No endgame / rarest-first selection strategy beyond random missing-piece search

### Disk flow
- `DiskIO` currently preallocates files and scans existing content only
- Background disk queue is present but unused
- No safe write path from assembled pieces back to disk

### Resilience
- No peer liveness checks, timeout handling, or retry policies
- No support for torrent pause/resume or graceful stop

## Suggested architecture improvements

### Add a `TorrentSession`
A high-level controller that owns:
- `TorrentContext`
- `DiskIO`
- `Tracker`
- peer swarm and request pipeline
- session configuration

### Separate concerns clearly
- `torrent_context.rs`: immutable torrent metadata and local state
- `tracker.rs`: announce/event lifecycle and peer discovery
- `peer.rs` + `peer_network.rs`: wire protocol and peer state
- `disk_io.rs`: all disk reads/writes and file management
- `selector.rs`: piece/peer selection strategies

### Data flow
1. `TorrentSession` loads metadata and local file structure
2. `Tracker` announces and returns peers
3. `Manager` or session adds peers to swarm
4. `Peer` establishes handshake, exchanges bitfields
5. `Selector` chooses piece/block requests
6. `PeerNetwork` sends requests to peers
7. `AssemblerData` assembles blocks and validates hashes
8. `DiskIO` writes completed pieces to disk
9. `TorrentContext` updates progress and status
