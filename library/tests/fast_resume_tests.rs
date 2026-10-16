use bittorrent_rs::{
    FastResumeData, FastResumeFileEntry, FileDetails, FilePriority, SequentialSelector,
    SessionConfig, TorrentSession, TorrentStatus,
};
use bittorrent_rs::internals::TorrentContext;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    dir.push(format!("bt_test_fastresume_{}_{}", name, epoch));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_fast_resume_bencode_roundtrip() {
    let data = FastResumeData {
        info_hash: [0x42; 20],
        save_path: PathBuf::from("/downloads/linux_iso"),
        num_pieces: 10,
        piece_length: 262144,
        bitfield: vec![0b10110000, 0b11000000],
        total_uploaded: 1048576,
        total_downloaded: 1310720,
        active_time: 3600,
        seeding_time: 1800,
        is_seeding: false,
        files: vec![
            FastResumeFileEntry {
                path: PathBuf::from("video.mkv"),
                length: 2000000,
                mtime: 1729000000,
                priority: 4,
            },
            FastResumeFileEntry {
                path: PathBuf::from("subtitles.srt"),
                length: 62144,
                mtime: 1729000010,
                priority: 7,
            },
        ],
        peers: vec![
            "192.168.1.50:6881".to_string(),
            "10.0.0.1:51413".to_string(),
        ],
    };

    let encoded = data.to_bencode();
    assert!(!encoded.is_empty());

    let decoded = FastResumeData::from_bencode(&encoded).expect("Failed to decode valid bencode");

    assert_eq!(decoded.info_hash, data.info_hash);
    assert_eq!(decoded.save_path, data.save_path);
    assert_eq!(decoded.num_pieces, data.num_pieces);
    assert_eq!(decoded.piece_length, data.piece_length);
    assert_eq!(decoded.bitfield, data.bitfield);
    assert_eq!(decoded.total_uploaded, data.total_uploaded);
    assert_eq!(decoded.total_downloaded, data.total_downloaded);
    assert_eq!(decoded.active_time, data.active_time);
    assert_eq!(decoded.seeding_time, data.seeding_time);
    assert_eq!(decoded.is_seeding, data.is_seeding);
    assert_eq!(decoded.files.len(), 2);
    assert_eq!(decoded.files[0].length, 2000000);
    assert_eq!(decoded.files[0].mtime, 1729000000);
    assert_eq!(decoded.files[0].priority, 4);
    assert_eq!(decoded.files[1].length, 62144);
    assert_eq!(decoded.files[1].mtime, 1729000010);
    assert_eq!(decoded.files[1].priority, 7);
    assert_eq!(decoded.peers.len(), 2);
    assert_eq!(decoded.peers[0], "192.168.1.50:6881");
}

#[test]
fn test_piece_helpers_and_progress() {
    let mut data = FastResumeData {
        info_hash: [0x11; 20],
        save_path: PathBuf::from("."),
        num_pieces: 10,
        piece_length: 16384,
        bitfield: vec![0u8; 2],
        total_uploaded: 0,
        total_downloaded: 0,
        active_time: 0,
        seeding_time: 0,
        is_seeding: false,
        files: Vec::new(),
        peers: Vec::new(),
    };

    assert_eq!(data.pieces_have_count(), 0);
    assert_eq!(data.progress(), 0.0);
    assert!(!data.has_piece(0));
    assert!(!data.has_piece(9));

    data.set_piece(0, true);
    data.set_piece(2, true);
    data.set_piece(9, true);

    assert!(data.has_piece(0));
    assert!(!data.has_piece(1));
    assert!(data.has_piece(2));
    assert!(data.has_piece(9));
    assert_eq!(data.pieces_have_count(), 3);
    assert!((data.progress() - 0.3).abs() < 1e-4);

    data.set_piece(0, false);
    assert!(!data.has_piece(0));
    assert_eq!(data.pieces_have_count(), 2);
}

#[test]
fn test_on_disk_verification() {
    let tmp = temp_dir("disk_verif");
    let file1_path = tmp.join("test_file1.bin");
    let file2_path = tmp.join("subfolder").join("test_file2.bin");

    fs::create_dir_all(file2_path.parent().unwrap()).unwrap();

    let mut f1 = File::create(&file1_path).unwrap();
    f1.write_all(&vec![0xAA; 1000]).unwrap();
    drop(f1);

    let mut f2 = File::create(&file2_path).unwrap();
    f2.write_all(&vec![0xBB; 500]).unwrap();
    drop(f2);

    let mtime1 = fs::metadata(&file1_path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mtime2 = fs::metadata(&file2_path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let data = FastResumeData {
        info_hash: [0x55; 20],
        save_path: tmp.clone(),
        num_pieces: 2,
        piece_length: 1000,
        bitfield: vec![0b11000000],
        total_uploaded: 0,
        total_downloaded: 1500,
        active_time: 0,
        seeding_time: 0,
        is_seeding: true,
        files: vec![
            FastResumeFileEntry {
                path: PathBuf::from("test_file1.bin"),
                length: 1000,
                mtime: mtime1,
                priority: 4,
            },
            FastResumeFileEntry {
                path: PathBuf::from("subfolder/test_file2.bin"),
                length: 500,
                mtime: mtime2,
                priority: 4,
            },
        ],
        peers: Vec::new(),
    };

    // Standard verify (size & existence)
    assert!(data.verify_on_disk(&tmp).unwrap());

    // Strict verify (including timestamps)
    assert!(data.verify_on_disk_strict(&tmp).unwrap());

    // Truncated file should fail verification
    {
        let mut f1_trunc = File::create(&file1_path).unwrap();
        f1_trunc.write_all(&vec![0xAA; 900]).unwrap(); // wrong size (900 instead of 1000)
    }
    assert!(!data.verify_on_disk(&tmp).unwrap());

    // Missing file should fail verification
    fs::remove_file(&file1_path).unwrap();
    assert!(!data.verify_on_disk(&tmp).unwrap());

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_to_context() {
    let selector = Arc::new(SequentialSelector);
    let config = SessionConfig::default();
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0x33; 20],
        vec!["http://tracker.test/announce".into()],
        selector,
        Path::new("."),
        config,
    ).unwrap();

    ctx.init_pieces(4, 1000);
    ctx.total_bytes_to_download = 3500;
    ctx.files_to_download = vec![
        FileDetails {
            name: "file.bin".into(),
            torrent_path: "file.bin".into(),
            length: 3500,
            md5sum: None,
            offset: 0,
            pieces_root: None,
            priority: FilePriority::Normal,
        }
    ];

    let resume = FastResumeData {
        info_hash: [0x33; 20],
        save_path: PathBuf::from("."),
        num_pieces: 4,
        piece_length: 1000,
        bitfield: vec![0b10100000], // piece 0 and 2 present
        total_uploaded: 50000,
        total_downloaded: 2000,
        active_time: 120,
        seeding_time: 0,
        is_seeding: false,
        files: vec![
            FastResumeFileEntry {
                path: PathBuf::from("file.bin"),
                length: 3500,
                mtime: 0,
                priority: 7, // High priority
            }
        ],
        peers: vec!["1.2.3.4:6881".into()],
    };

    resume.apply_to_context(&mut ctx).expect("Failed to apply fast resume to context");

    assert!(ctx.is_piece_local(0));
    assert!(!ctx.is_piece_local(1));
    assert!(ctx.is_piece_local(2));
    assert!(!ctx.is_piece_local(3));

    assert!(!ctx.is_piece_missing(0));
    assert!(ctx.is_piece_missing(1));
    assert!(!ctx.is_piece_missing(2));
    assert!(ctx.is_piece_missing(3));

    assert_eq!(ctx.missing_pieces_count, 2);
    assert_eq!(ctx.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed), 2000);
    assert_eq!(ctx.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed), 50000);
    assert_eq!(ctx.files_to_download[0].priority, FilePriority::High);
    assert_eq!(ctx.status, TorrentStatus::Initialised);

    // Fully seeding test
    let seeding_resume = FastResumeData {
        info_hash: [0x33; 20],
        save_path: PathBuf::from("."),
        num_pieces: 4,
        piece_length: 1000,
        bitfield: vec![0b11110000], // all 4 pieces present
        total_uploaded: 10000,
        total_downloaded: 3500,
        active_time: 500,
        seeding_time: 300,
        is_seeding: true,
        files: vec![
            FastResumeFileEntry {
                path: PathBuf::from("file.bin"),
                length: 3500,
                mtime: 0,
                priority: 4,
            }
        ],
        peers: Vec::new(),
    };

    seeding_resume.apply_to_context(&mut ctx).expect("Failed to apply seeding resume");
    assert_eq!(ctx.missing_pieces_count, 0);
    assert_eq!(ctx.status, TorrentStatus::Seeding);
    assert!(ctx.download_finished.is_set());
}

#[test]
fn test_save_load_file() {
    let tmp = temp_dir("save_load");
    let resume_file = tmp.join("torrent.fastresume");

    let data = FastResumeData {
        info_hash: [0x88; 20],
        save_path: tmp.clone(),
        num_pieces: 5,
        piece_length: 32768,
        bitfield: vec![0b11100000],
        total_uploaded: 65536,
        total_downloaded: 98304,
        active_time: 60,
        seeding_time: 0,
        is_seeding: false,
        files: vec![
            FastResumeFileEntry {
                path: PathBuf::from("data.bin"),
                length: 160000,
                mtime: 1729000000,
                priority: 4,
            }
        ],
        peers: vec!["127.0.0.1:8999".into()],
    };

    data.save_to_file(&resume_file).expect("Failed to save fast resume to disk");
    assert!(resume_file.exists());

    let loaded = FastResumeData::load_from_file(&resume_file).expect("Failed to load fast resume from disk");
    assert_eq!(loaded.info_hash, data.info_hash);
    assert_eq!(loaded.num_pieces, data.num_pieces);
    assert_eq!(loaded.bitfield, data.bitfield);
    assert_eq!(loaded.peers, data.peers);

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn test_session_builder_fast_resume_roundtrip() {
    let torrent_file = Path::new("tests/files/singlefile.torrent");
    if !torrent_file.exists() {
        return;
    }

    let tmp = temp_dir("session_fr");
    let resume_path = tmp.join("state.fastresume");

    // 1. Build initial seeding session to populate files on disk
    let session = TorrentSession::builder(torrent_file, &tmp)
        .seeding(true)
        .build()
        .expect("Failed to build initial session");

    // Save fast-resume state
    session.save_fast_resume(&resume_path).expect("Failed to save fast resume");
    assert!(resume_path.exists());

    let fr = FastResumeData::load_from_file(&resume_path).expect("Failed to load");
    assert!(fr.verify_on_disk(&tmp).unwrap());
    drop(session);

    // 2. Start a second session using the saved fast-resume file
    let resumed_session = TorrentSession::builder(torrent_file, &tmp)
        .fast_resume_file(&resume_path)
        .build()
        .expect("Failed to build session using fast resume");

    assert_eq!(resumed_session.progress(), 100.0);
    assert_eq!(resumed_session.status(), TorrentStatus::Seeding);

    drop(resumed_session);
    let _ = fs::remove_dir_all(&tmp);
}
