use bittorrent_rs::{TorrentSession, TorrentStatus};
use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let torrent_path = args
        .next()
        .expect("Usage: torrent_session_example <torrent-file> <download-dir>");
    let download_dir = args
        .next()
        .expect("Usage: torrent_session_example <torrent-file> <download-dir>");

    let torrent_path = PathBuf::from(torrent_path);
    let download_dir = PathBuf::from(download_dir);

    let mut session = TorrentSession::new(&torrent_path, &download_dir, false)
        .expect("Failed to create torrent session");

    println!("Loaded torrent from: {}", torrent_path.display());
    println!("Download directory: {}", download_dir.display());
    println!("Initial status: {:?}", session.status());

    session.start_download().expect("Failed to start download");
    println!("Download started.");
    println!("Current progress: {:.2}%", session.progress());

    if session.status() == TorrentStatus::Downloading {
        println!("Torrent is ready for peer download operations.");
    }

    session.pause().expect("Failed to pause");
    println!("Paused. Status={:?}", session.status());

    session.resume().expect("Failed to resume");
    println!("Resumed. Status={:?}", session.status());

    session.stop().expect("Failed to stop");
    println!("Stopped. Final status={:?}", session.status());
}
