# API Proposal for a Full Torrent Downloader

## Top-level API

### `TorrentSession`
A new high-level entry point for download logic.

```rust
pub struct TorrentSession {
    pub context: TorrentContext,
    pub disk_io: DiskIO,
    pub tracker: Tracker,
    pub manager: Manager,
}

impl TorrentSession {
    pub fn new(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> Result<Self, BitTorrentError>;
    pub fn start_download(&mut self) -> Result<(), BitTorrentError>;
    pub fn stop_download(&mut self);
    pub fn pause(&mut self);
    pub fn resume(&mut self) -> Result<(), BitTorrentError>;
    pub fn status(&self) -> TorrentStatus;
    pub fn progress(&self) -> f32;
}
```

## Peer API

### `Peer::handshake`
- perform BitTorrent handshake
- verify remote peer ID / info hash
- exchange bitfield

### `PeerNetwork`
- `fn read_messages(&self) -> Result<Vec<PeerMessage>, IoError>`
- `fn write_message(&self, message: PeerMessage) -> Result<(), IoError>`
- background worker for incoming wire messages

## Disk API

### `DiskIO`
Add methods:
- `fn write_piece(&self, piece_number: u32, piece_data: &[u8]) -> Result<(), BitTorrentError>`
- `fn read_piece(&self, piece_number: u32) -> Result<Vec<u8>, BitTorrentError>`
- `fn request_piece(&self, piece_number: u32, offset: u32, length: u32) -> Result<Vec<u8>, BitTorrentError>`

## Tracker API

### `Tracker`
- `fn announce_started(&mut self) -> Result<AnnounceResponse, BitTorrentError>`
- `fn announce_stopped(&mut self)`
- `fn announce_completed(&mut self) -> Result<AnnounceResponse, BitTorrentError>`
- `fn get_peers(&self) -> Vec<PeerDetails>`

## Example usage

```rust
let mut session = TorrentSession::new("files/example.torrent", "downloads/")?;
session.start_download()?;
while session.status() != TorrentStatus::Ended {
    println!("progress: {:.2}%", session.progress());
    std::thread::sleep(std::time::Duration::from_secs(1));
}
```

## Documentation deliverables
- `docs/roadmap.md` — step-by-step implementation plan
- `docs/architecture.md` — current architecture and missing components
- `docs/api-proposal.md` — public API design for the download engine
- `examples/` — runnable client examples once the download engine is implemented
