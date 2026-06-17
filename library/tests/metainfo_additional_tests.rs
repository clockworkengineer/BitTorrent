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

#[test]
fn test_validate_relative_path_harden() {
    let path = std::env::temp_dir().join("bittorrent_bad_path_test.torrent");
    
    // Test reserved Windows device name CON
    let contents = b"d8:announce18:http://tracker.com4:infod4:name7:CON.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee";
    fs::write(&path, contents).unwrap();
    let mut torrent = MetaInfoFile::new(&path).unwrap();
    torrent.parse().unwrap();
    assert!(torrent.local_files_to_download_list(Path::new(".")).is_err());
    fs::remove_file(&path).unwrap();

    // Test colon in path
    let contents2 = b"d8:announce18:http://tracker.com4:infod4:name12:bad:file.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee";
    fs::write(&path, contents2).unwrap();
    let mut torrent2 = MetaInfoFile::new(&path).unwrap();
    torrent2.parse().unwrap();
    assert!(torrent2.local_files_to_download_list(Path::new(".")).is_err());
    fs::remove_file(&path).unwrap();
}

#[test]
fn test_validate_relative_path_traversal_and_reserved_names() {
    let path = std::env::temp_dir().join("bittorrent_bad_path_traversal.torrent");

    let test_cases = vec![
        b"d8:announce18:http://tracker.com4:infod4:name10:/absolutep12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
        b"d8:announce18:http://tracker.com4:infod4:name11:\\absolutep212:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
        b"d8:announce18:http://tracker.com4:infod4:name10:foo/../bar12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
        b"d8:announce18:http://tracker.com4:infod4:name10:foo//bar/b12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
        b"d8:announce18:http://tracker.com4:infod4:name9:foo/./bar12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
        b"d8:announce18:http://tracker.com4:infod4:name8:lpt3.txt12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
        b"d8:announce18:http://tracker.com4:infod4:name0:12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaa6:lengthi100eee".as_ref(),
    ];

    for contents in test_cases {
        fs::write(&path, contents).unwrap();
        let mut torrent = MetaInfoFile::new(&path).unwrap();
        torrent.parse().unwrap();
        assert!(torrent.local_files_to_download_list(Path::new(".")).is_err());
        let _ = fs::remove_file(&path);
    }
}

