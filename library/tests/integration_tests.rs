use bittorrent_rs::announcer::Announcer;
use bittorrent_rs::disk_io::DiskIO;
use bittorrent_rs::metainfo::MetaInfoFile;
use bittorrent_rs::selector::RarestFirstSelector;
use bittorrent_rs::session::SessionConfig;
use bittorrent_rs::torrent_context::TorrentContext;
use bittorrent_rs::tracker::{PeerDetails, Tracker};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct FakeAnnouncer;

impl Announcer for FakeAnnouncer {
    fn announce(
        &mut self,
        tracker: &bittorrent_rs::tracker::TrackerAnnounceContext,
    ) -> Result<bittorrent_rs::tracker::AnnounceResponse, bittorrent_rs::error::BitTorrentError> {
        let mut response = bittorrent_rs::tracker::AnnounceResponse::default();
        response.status_message = format!("event={}", tracker.event.as_str());
        response.interval = 30;
        response.peer_list.push(PeerDetails {
            info_hash: tracker.info_hash.clone(),
            peer_id: Some("FAKE_PEER".to_string()),
            ip: "127.0.0.1".to_string(),
            port: 6881,
        });
        Ok(response)
    }
}

fn sample_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}

#[test]
fn test_meta_info_parsing_and_validation() {
    let torrent_path = sample_file("singlefile.torrent");
    let mut meta_info = MetaInfoFile::new(&torrent_path).expect("Failed to load torrent");
    meta_info.parse().expect("Failed to parse torrent");
    meta_info.validate().expect("Torrent validation failed");

    let piece_length = meta_info.get_piece_length().expect("Failed to get piece length");
    assert!(piece_length > 0);
    let pieces = meta_info.get_pieces_info_hash().expect("Failed to get piece hashes");
    assert_eq!(pieces.len() % bittorrent_rs::constants::HASH_LENGTH, 0);
}

#[test]
fn test_local_file_structure_creation() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("integration_local_file_structure");
    let _ = fs::remove_dir_all(&download_path);
    fs::create_dir_all(&download_path).unwrap();

    let torrent_path = sample_file("singlefile.torrent");
    let mut meta_info = MetaInfoFile::new(&torrent_path).expect("Failed to load torrent");
    meta_info.parse().expect("Failed to parse torrent");
    meta_info.validate().expect("Torrent validation failed");

    let piece_length = meta_info.get_piece_length().expect("Failed to get piece length");
    let (_, files_to_download) = meta_info.local_files_to_download_list(&download_path).expect("Failed to get files list");
    let disk_io = Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().expect("Failed to create file structure");
    let selector = Arc::new(RarestFirstSelector);
    let mut context = TorrentContext::new(&meta_info, selector, disk_io.clone(), &download_path, false, SessionConfig::default())
        .expect("Failed to create torrent context");
    disk_io.create_torrent_bitfield(&mut context).expect("Failed to create torrent bitfield");

    for file in &context.files_to_download {
        assert!(std::path::Path::new(&file.name).exists());
        assert_eq!(fs::metadata(&file.name).unwrap().len(), file.length);
    }

    let _ = fs::remove_dir_all(&download_path);
}

#[test]
fn test_tracker_announce_with_fake_announcer() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("integration_tracker_announce");
    let _ = fs::remove_dir_all(&download_path);

    let torrent_path = sample_file("singlefile.torrent");
    let mut meta_info = MetaInfoFile::new(&torrent_path).expect("Failed to load torrent");
    meta_info.parse().expect("Failed to parse torrent");
    meta_info.validate().expect("Torrent validation failed");

    let piece_length = meta_info.get_piece_length().expect("Failed to get piece length");
    let (_, files_to_download) = meta_info.local_files_to_download_list(&download_path).expect("Failed to get files list");
    let disk_io = Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().expect("Failed to create file structure");
    let selector = Arc::new(RarestFirstSelector);
    let mut context = TorrentContext::new(&meta_info, selector, disk_io.clone(), &download_path, false, SessionConfig::default())
        .expect("Failed to create torrent context");
    disk_io.create_torrent_bitfield(&mut context).expect("Failed to create torrent bitfield");
    let context = Arc::new(Mutex::new(context));

    let mut tracker = Tracker::new_with_announcer(context.clone(), Box::new(FakeAnnouncer {}))
        .expect("Failed to create tracker");

    let response = tracker.announce_started().expect("Tracker announce failed");
    assert_eq!(response.status_message, "event=started");
    assert!(!response.peer_list.is_empty());
    assert_eq!(response.peer_list[0].ip, "127.0.0.1");

    let _ = fs::remove_dir_all(&download_path);
}
