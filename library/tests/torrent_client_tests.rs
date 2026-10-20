mod common;

use bittorrent_rs::{TorrentClient, TorrentStatus};
use common::sample_file;

#[test]
fn test_torrent_client_lifecycle() {
    let download_path = std::env::temp_dir().join("torrent_client_test");
    let _ = std::fs::remove_dir_all(&download_path);
    std::fs::create_dir_all(&download_path).unwrap();

    let torrent_file = sample_file("singlefile.torrent");
    let mut client = TorrentClient::new(&torrent_file, &download_path).unwrap();

    assert_eq!(client.status(), TorrentStatus::Initialised);
    assert_eq!(client.progress(), 0.0);

    // Stop client
    client.stop().unwrap();
    assert_eq!(client.status(), TorrentStatus::Ended);

    let _ = std::fs::remove_dir_all(&download_path);
}
