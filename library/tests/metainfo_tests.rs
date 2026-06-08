use bittorrent_rs::MetaInfoFile;
use std::path::PathBuf;

fn sample_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}

#[test]
fn test_exception_on_file_not_existing() {
    let path = sample_file("does_not_exist.torrent");
    assert!(MetaInfoFile::new(path).is_err());
}

#[test]
fn test_single_file_torrent_contains_valid_keys() {
    let mut torrent = MetaInfoFile::new(sample_file("singlefile.torrent"))
        .expect("Failed to open single file torrent");
    torrent
        .parse()
        .expect("Failed to parse single file torrent");
    assert_eq!(
        torrent.get_tracker().unwrap(),
        "http://192.168.1.215:9005/announce"
    );
    assert_eq!(torrent.get_piece_length().unwrap().to_string(), "16384");
    assert_eq!(
        bittorrent_rs::util::info_hash_to_string(&torrent.get_info_hash().unwrap()),
        "7fd1a2631b385a4cc68bf15040fa375c8e68cb7e"
    );
}

#[test]
fn test_multi_file_torrent_contains_valid_keys() {
    let mut torrent = MetaInfoFile::new(sample_file("multifile.torrent"))
        .expect("Failed to open multi file torrent");
    torrent.parse().expect("Failed to parse multi file torrent");
    assert_eq!(
        torrent.get_tracker().unwrap(),
        "http://192.168.1.215:9005/announce"
    );
    assert_eq!(
        bittorrent_rs::util::info_hash_to_string(&torrent.get_info_hash().unwrap()),
        "c28bf4c5ab095923eecad46701d09408912928e7"
    );
}
