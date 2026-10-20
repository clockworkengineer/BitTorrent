use bittorrent_rs::TorrentBuilder;
use std::path::Path;

/// Generates `.torrent` bytes for a directory using the core library's `TorrentBuilder`.
pub fn generate_torrent_bytes(
    sync_dir: &Path,
    name: &str,
    piece_length: usize,
) -> Result<Vec<u8>, std::io::Error> {
    let builder = TorrentBuilder::new(name)
        .tracker("http://127.0.0.1:6881/announce")
        .piece_length(piece_length)
        .private(true)
        .add_dir(sync_dir)?;

    builder
        .build_v1()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}
