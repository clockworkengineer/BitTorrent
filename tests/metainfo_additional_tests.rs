use bittorrent_rs::MetaInfoFile;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn test_parse_missing_announce_fails() {
    let path = std::env::temp_dir().join("bittorrent_missing_announce_test.torrent");
    let contents = b"d4:infod4:name4:test12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";
    fs::write(&path, contents).expect("Unable to write temp torrent file");

    let mut torrent = MetaInfoFile::new(&path).expect("Failed to open temp torrent file");
    assert!(torrent.parse().is_err());

    fs::remove_file(&path).expect("Unable to delete temp torrent file");
}

#[test]
fn test_local_files_to_download_list_single_file() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join("singlefile.torrent");
    let mut torrent = MetaInfoFile::new(&path).expect("Failed to open singlefile.torrent");
    torrent.parse().expect("Failed to parse singlefile.torrent");

    let download_path = Path::new(".");
    let (total, files) = torrent
        .local_files_to_download_list(download_path)
        .expect("Failed to get local files list");

    assert_eq!(files.len(), 1);
    assert_eq!(total, files[0].length);
    assert!(files[0].length > 0);
    assert_eq!(files[0].offset, 0);
}
