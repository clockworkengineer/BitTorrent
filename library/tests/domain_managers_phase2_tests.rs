//! Phase 2 Architecture & Domain Managers Integration Tests
//!
//! Validates the isolated operation, ISP views, and reliability of the 5 decomposed domain managers:
//! - `TelemetryManager`
//! - `PieceManager`
//! - `SwarmManager`
//! - `StorageManager`
//! - `MetadataManager`

use bittorrent_rs::core::views::{PieceAvailabilityView, RemotePeerPieceView, TransferStatsView};
use bittorrent_rs::io_traits::MemStorage;
use bittorrent_rs::metainfo::{FileDetails, FilePriority};
use bittorrent_rs::storage::verification_pool::HashVerificationPool;
use bittorrent_rs::{
    MetadataManager, Peer, PieceManager, StorageManager, SwarmManager, TelemetryManager,
};
use sha1::Digest;
use std::sync::Arc;

#[test]
fn test_telemetry_manager_rates_progress_and_eta() {
    let telemetry = TelemetryManager::new(100_000);
    assert_eq!(telemetry.total_downloaded(), 0);
    assert_eq!(telemetry.progress_percent(), 0.0);

    telemetry.record_download(25_000);
    assert_eq!(telemetry.total_downloaded(), 25_000);
    assert_eq!(telemetry.progress_ppm(), 2500);
    assert_eq!(telemetry.progress_percent(), 25.0);

    let left = telemetry.bytes_left_to_download().unwrap();
    assert_eq!(left, 75_000);

    // Simulate cached download rate of 25,000 Bps
    telemetry
        .cached_download_bps
        .store(25_000, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(telemetry.current_download_rate(), 25_000);
    assert_eq!(telemetry.eta_seconds(), Some(3)); // 75,000 / 25,000 = 3s

    // Protocol counters
    telemetry
        .pex_added_peers_count
        .fetch_add(5, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        telemetry
            .pex_added_peers_count
            .load(std::sync::atomic::Ordering::Relaxed),
        5
    );
}

#[test]
fn test_piece_manager_bitfields_and_rarest_first() {
    let num_pieces = 16;
    let piece_length = 32768; // 32 KiB

    let mut hashes = Vec::new();
    for p in 0..num_pieces {
        let fake_data = vec![p as u8; piece_length as usize];
        let digest = sha1::Sha1::digest(&fake_data);
        hashes.extend_from_slice(&digest);
    }

    let mut pm = PieceManager::new(num_pieces, piece_length, hashes);
    assert_eq!(pm.number_of_pieces(), 16);
    assert_eq!(pm.piece_length(0), 32768);
    assert!(!pm.is_piece_local(0));
    assert!(pm.is_piece_wanted(0));

    // Verify blocks in piece
    assert_eq!(pm.blocks_in_piece(0), 2); // 32KB / 16KB = 2

    // Update piece status
    pm.mark_piece_local(0, true);
    assert!(pm.is_piece_local(0));
    assert!(!pm.is_piece_wanted(0));

    // Peer availability view pop rarest
    struct MockPeerPossession(Vec<bool>);
    impl RemotePeerPieceView for MockPeerPossession {
        fn is_piece_on_remote_peer(&self, piece: u32) -> bool {
            self.0.get(piece as usize).copied().unwrap_or(false)
        }
    }

    let mut peer_has = vec![false; 16];
    peer_has[5] = true;
    peer_has[7] = true;
    let peer = MockPeerPossession(peer_has);

    let picked = pm.pop_rarest_piece(&peer);
    assert!(picked.is_some());
    let piece_idx = picked.unwrap().piece_index;
    assert!(piece_idx == 5 || piece_idx == 7);
}

#[test]
fn test_swarm_manager_admissions_and_reputation() {
    let sm = SwarmManager::new(2);
    assert_eq!(sm.connected_peers_count(), 0);

    let (sock1, _, _) = bittorrent_rs::io_traits::MockSocket::new();
    let (sock2, _, _) = bittorrent_rs::io_traits::MockSocket::new();
    let (sock3, _, _) = bittorrent_rs::io_traits::MockSocket::new();

    let p1 = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "192.168.1.10".into(),
        6881,
        Arc::new(bittorrent_rs::io_traits::Socket::Mock(sock1)),
    )));
    let p2 = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "192.168.1.11".into(),
        6881,
        Arc::new(bittorrent_rs::io_traits::Socket::Mock(sock2)),
    )));
    let p3 = Arc::new(std::sync::Mutex::new(Peer::new_with_socket(
        "192.168.1.12".into(),
        6881,
        Arc::new(bittorrent_rs::io_traits::Socket::Mock(sock3)),
    )));

    assert!(sm.insert_peer("192.168.1.10".into(), p1));
    assert!(sm.insert_peer("192.168.1.11".into(), p2));
    assert_eq!(sm.connected_peers_count(), 2);

    // Exceeding capacity should reject new peer
    assert!(!sm.insert_peer("192.168.1.12".into(), p3));
    assert_eq!(sm.connected_peers_count(), 2);

    // Bad peer reputation penalty
    assert_eq!(sm.record_bad_peer("192.168.1.10"), 1);
    assert_eq!(sm.record_bad_peer("192.168.1.10"), 2);
    assert!(sm.is_bad_peer("192.168.1.10", 2));
    assert!(!sm.is_bad_peer("192.168.1.10", 3));

    // Snapshot generation
    let snaps = sm.peer_snapshots();
    assert_eq!(snaps.len(), 2);

    // Removal
    assert!(sm.remove_peer("192.168.1.10").is_some());
    assert_eq!(sm.connected_peers_count(), 1);
}

#[test]
fn test_storage_manager_selective_download_and_block_io() {
    let files = vec![
        FileDetails {
            name: "file_a.bin".into(),
            torrent_path: "file_a.bin".into(),
            length: 1024,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
        FileDetails {
            name: "file_b.bin".into(),
            torrent_path: "file_b.bin".into(),
            length: 2048,
            md5sum: None,
            offset: 1024,
            pieces_root: None,
            priority: FilePriority::Normal,
        },
    ];

    let mem_storage = Arc::new(MemStorage::new(3072));
    let pool = Arc::new(HashVerificationPool::new(1, 16));
    let mut sm = StorageManager::new(mem_storage, "C:\\downloads", files, pool);

    assert_eq!(sm.file_count(), 2);
    assert_eq!(sm.total_files_size(), 3072);

    // Block write and read
    let sample = b"BitTorrent-rs storage engine block payload!";
    sm.write_block(0, sample).unwrap();

    let mut buf = vec![0u8; sample.len()];
    let read_len = sm.read_block(0, &mut buf).unwrap();
    assert_eq!(read_len, sample.len());
    assert_eq!(&buf[..], sample);

    // Selective download filtering
    sm.set_selected_files(&[1]);
    assert!(!sm.is_file_selected(0));
    assert!(sm.is_file_selected(1));
    assert_eq!(sm.file_priority(0), Some(FilePriority::Skip));
    assert_eq!(sm.file_priority(1), Some(FilePriority::Normal));
}

#[test]
fn test_metadata_manager_bep9_assembly() {
    let dummy_payload = b"d4:infod6:lengthi1024e4:name6:sampleee";
    let digest = sha1::Sha1::digest(dummy_payload);
    let mut info_hash = [0u8; 20];
    info_hash.copy_from_slice(&digest);

    #[cfg(all(feature = "std", feature = "v2"))]
    let disk_cache = Arc::new(
        bittorrent_rs::storage::piece_layers_cache::PieceLayersDiskCache::new_in_memory(16),
    );

    let mut mm = MetadataManager::empty(
        info_hash.to_vec(),
        #[cfg(all(feature = "std", feature = "v2"))]
        disk_cache,
    );
    assert!(!mm.has_metadata());

    mm.set_metadata_size(dummy_payload.len());
    assert!(mm.add_metadata_piece(0, dummy_payload.to_vec()));

    let assembled = mm.assemble_metadata();
    assert!(assembled.is_some());
    assert_eq!(assembled.unwrap(), dummy_payload);
    assert!(mm.has_metadata());

    // Tracker & web seed registration
    assert!(mm.add_tracker_url("http://tracker.test:80/announce".into()));
    assert!(!mm.add_tracker_url("http://tracker.test:80/announce".into())); // Duplicate ignored
    assert!(mm.add_web_seed("https://mirror.example.com/data".into()));
}
