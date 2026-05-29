use bittorrent_rs::{TorrentSession, TorrentStatus};
use std::fs;
use std::path::PathBuf;

fn sample_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}

fn cleanup_download_path(download_path: &PathBuf) {
    let _ = fs::remove_dir_all(download_path);
}

#[test]
fn test_create_session_for_download() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_download");
    cleanup_download_path(&download_path);

    let mut session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, false)
        .expect("Failed to create torrent session");

    assert_eq!(session.status(), TorrentStatus::Initialised);
    assert!(session.context.files_to_download.len() >= 1);
    assert!(
        session
            .context
            .files_to_download
            .iter()
            .all(|f| std::path::Path::new(&f.name).exists())
    );

    session.start_download().expect("Failed to start download");
    assert_eq!(session.status(), TorrentStatus::Downloading);
    assert!(session.progress() >= 0.0);

    session.pause().expect("Failed to pause download");
    assert_eq!(session.status(), TorrentStatus::Paused);

    session.resume().expect("Failed to resume download");
    assert_eq!(session.status(), TorrentStatus::Downloading);

    session.stop().expect("Failed to stop download");
    assert_eq!(session.status(), TorrentStatus::Ended);

    cleanup_download_path(&download_path);
}

#[test]
fn test_create_session_for_seeding() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("session_seeding");
    cleanup_download_path(&download_path);

    let session = TorrentSession::new(sample_file("singlefile.torrent"), &download_path, true)
        .expect("Failed to create seeding torrent session");

    assert_eq!(session.status(), TorrentStatus::Seeding);
    assert_eq!(session.progress(), 100.0);
    assert!(session.validate().is_ok());

    cleanup_download_path(&download_path);
}
