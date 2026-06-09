use bittorrent_rs::announcer::Announcer;
use bittorrent_rs::disk_io::DiskIO;
use bittorrent_rs::manager::Manager;
use bittorrent_rs::metainfo::MetaInfoFile;
use bittorrent_rs::selector::Selector;
use bittorrent_rs::torrent_context::{TorrentContext, TorrentStatus};
use bittorrent_rs::tracker::PeerDetails;
use bittorrent_rs::tracker::Tracker;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc::channel};
use std::time::Duration;

struct FakeAnnouncer;

impl Announcer for FakeAnnouncer {
    fn announce(
        &mut self,
        tracker: &bittorrent_rs::tracker::TrackerAnnounceContext,
    ) -> Result<bittorrent_rs::tracker::AnnounceResponse, bittorrent_rs::error::BitTorrentError>
    {
        let mut response = bittorrent_rs::tracker::AnnounceResponse::default();
        response.status_message = format!("event={}", tracker.event.as_str());
        response.interval = 30;
        response.peer_list.push(PeerDetails {
            info_hash: tracker.info_hash.clone(),
            peer_id: Some("FAKE_PEER".to_string()),
            ip: "1.2.3.4".to_string(),
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
fn test_tracker_started_and_peer_list() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("tracker_started_test");
    let _ = fs::remove_dir_all(&download_path);

    let mut meta =
        MetaInfoFile::new(sample_file("singlefile.torrent")).expect("Failed to load torrent");
    meta.parse().expect("Failed to parse torrent");
    let piece_length = meta.get_piece_length().expect("Failed to get piece length");
    let (_, files_to_download) = meta.local_files_to_download_list(&download_path).expect("Failed to get files list");
    let disk_io = Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().expect("Failed to create file structure");
    let selector = Selector::new();
    let mut context = TorrentContext::new(&meta, selector, disk_io.clone(), &download_path, false)
        .expect("Failed to create torrent context");
    disk_io.create_torrent_bitfield(&mut context).expect("Failed to create torrent bitfield");
    let context = Arc::new(Mutex::new(context));

    let mut tracker = Tracker::new_with_announcer(context.clone(), Box::new(FakeAnnouncer {}))
        .expect("Failed to create tracker");

    context.lock().unwrap().status = TorrentStatus::Downloading;

    let response = tracker
        .announce_started()
        .expect("Tracker announce started failed");
    assert_eq!(response.status_message, "event=started");
    assert_eq!(tracker.last_peer_list().len(), 1);
    assert_eq!(tracker.last_peer_list()[0].ip, "1.2.3.4");
    assert_eq!(
        tracker.last_peer_list()[0].peer_id.as_deref(),
        Some("FAKE_PEER")
    );

    let _ = fs::remove_dir_all(&download_path);
}

#[test]
fn test_tracker_completed_event() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("tracker_completed_event_test");
    let _ = fs::remove_dir_all(&download_path);

    let mut meta =
        MetaInfoFile::new(sample_file("singlefile.torrent")).expect("Failed to load torrent");
    meta.parse().expect("Failed to parse torrent");
    let piece_length = meta.get_piece_length().expect("Failed to get piece length");
    let (_, files_to_download) = meta.local_files_to_download_list(&download_path).expect("Failed to get files list");
    let disk_io = Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().expect("Failed to create file structure");
    let selector = Selector::new();
    let mut context = TorrentContext::new(&meta, selector, disk_io.clone(), &download_path, false)
        .expect("Failed to create torrent context");
    disk_io.create_torrent_bitfield(&mut context).expect("Failed to create torrent bitfield");
    let context = Arc::new(Mutex::new(context));

    let mut tracker = Tracker::new_with_announcer(context.clone(), Box::new(FakeAnnouncer {}))
        .expect("Failed to create tracker");

    let response = tracker
        .announce_completed()
        .expect("Tracker announce completed failed");
    assert_eq!(response.status_message, "event=completed");

    let _ = fs::remove_dir_all(&download_path);
}

#[test]
fn test_manager_peer_discovery_queue_receives_peers() {
    let manager = Manager::new();
    let (sender, receiver) = channel();
    manager.set_peer_discovery_queue(sender);

    let peer_details = PeerDetails {
        info_hash: vec![1, 2, 3],
        peer_id: Some("test-peer".to_string()),
        ip: "2.3.4.5".to_string(),
        port: 6881,
    };

    manager.queue_peer_for_discovery(peer_details.clone());
    let received = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("No peer received");
    assert_eq!(received.ip, peer_details.ip);
    assert_eq!(received.port, peer_details.port);
}

#[test]
fn test_tracker_can_use_manager_queue() {
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("tracker_manager_queue_test");
    let _ = fs::remove_dir_all(&download_path);

    let mut meta =
        MetaInfoFile::new(sample_file("singlefile.torrent")).expect("Failed to load torrent");
    meta.parse().expect("Failed to parse torrent");
    let piece_length = meta.get_piece_length().expect("Failed to get piece length");
    let (_, files_to_download) = meta.local_files_to_download_list(&download_path).expect("Failed to get files list");
    let disk_io = Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().expect("Failed to create file structure");
    let selector = Selector::new();
    let mut context = TorrentContext::new(&meta, selector, disk_io.clone(), &download_path, false)
        .expect("Failed to create torrent context");
    disk_io.create_torrent_bitfield(&mut context).expect("Failed to create torrent bitfield");
    let context = Arc::new(Mutex::new(context));

    let manager = Arc::new(Manager::new());
    let (sender, receiver) = channel();
    manager.set_peer_discovery_queue(sender);

    let mut tracker = Tracker::new_with_announcer(context.clone(), Box::new(FakeAnnouncer {}))
        .expect("Failed to create tracker");
    tracker.set_peer_manager(manager.clone());

    context.lock().unwrap().status = TorrentStatus::Downloading;
    tracker
        .announce_started()
        .expect("Failed to announce started");

    let received = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("No peer received from manager queue");
    assert_eq!(received.ip, "1.2.3.4");

    let _ = fs::remove_dir_all(&download_path);
}
