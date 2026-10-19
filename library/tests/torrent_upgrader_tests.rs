use bittorrent_rs::{
    TorrentSession,
    core::merkle::{BLOCK_SIZE, MerkleTree},
    core::metainfo::MetaInfoFile,
    core::torrent_builder::TorrentBuilder,
    core::torrent_upgrader::{TorrentExportFormat, TorrentUpgrader},
};
use std::fs;
use std::path::PathBuf;

fn create_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.push(format!("bt_test_{}_{}_{}", name, std::process::id(), nanos));
    fs::create_dir_all(&dir).expect("Failed to create temporary directory");
    dir
}

#[test]
fn test_upgrade_v1_to_pure_v2_single_file() {
    let temp_dir = create_temp_dir("v1_to_v2_single");
    let file_data = vec![0x33u8; 32 * 1024]; // 32 KiB (2 blocks)
    let file_path = temp_dir.join("sample.bin");
    fs::write(&file_path, &file_data).expect("Failed to write test file");

    // Create legacy v1 torrent
    let v1_bytes = TorrentBuilder::new("sample.bin")
        .piece_length(16 * 1024)
        .tracker("http://tracker.example.com/announce")
        .comment("Original v1 metainfo")
        .add_file("sample.bin", file_data.clone())
        .build_v1()
        .expect("Failed to build v1 torrent");

    let v1_torrent_path = temp_dir.join("sample.torrent");
    fs::write(&v1_torrent_path, &v1_bytes).expect("Failed to write v1 torrent file");

    // Upgrade to Pure v2
    let (v2_bytes, report) =
        TorrentUpgrader::upgrade_from_file(&v1_torrent_path, &temp_dir, TorrentExportFormat::V2)
            .expect("Failed to upgrade to pure v2");

    assert_eq!(report.name, "sample.bin");
    assert_eq!(report.format, TorrentExportFormat::V2);
    assert_eq!(report.total_size, 32 * 1024);
    assert_eq!(report.piece_length, 16 * 1024);
    assert!(report.v1_info_hash.is_none());
    assert!(report.v2_info_hash.is_some());
    assert!(report.v2_truncated_info_hash.is_some());
    assert_eq!(report.v2_info_hash.as_ref().unwrap().len(), 64);
    assert_eq!(report.v2_truncated_info_hash.as_ref().unwrap().len(), 40);
    assert_eq!(report.file_pieces_roots.len(), 1);

    let expected_root = MerkleTree::from_bytes(&file_data).root();
    let expected_root_hex: String = expected_root.iter().map(|b| format!("{:02x}", b)).collect();
    assert_eq!(report.file_pieces_roots[0].1, expected_root_hex);

    // Verify Magnet URI contains v2 btmh prefix and not v1 btih
    assert!(report.magnet_uri.starts_with("magnet:?"));
    assert!(report.magnet_uri.contains("xt=urn:btmh:1220"));
    assert!(!report.magnet_uri.contains("xt=urn:btih:"));

    // Verify parsed upgraded metainfo
    let mut parsed_v2 = MetaInfoFile::from_bytes(&v2_bytes);
    parsed_v2
        .parse()
        .expect("Failed to parse upgraded v2 torrent");
    assert!(parsed_v2.is_v2());
    assert!(!parsed_v2.is_hybrid());
    assert_eq!(parsed_v2.get_name().unwrap(), "sample.bin");
    assert_eq!(parsed_v2.get_total_size().unwrap(), 32 * 1024);
    let parsed_v2_hash = parsed_v2.get_v2_info_hash().unwrap();
    let parsed_v2_hash_hex: String = parsed_v2_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    assert_eq!(
        report.v2_info_hash.as_deref(),
        Some(parsed_v2_hash_hex.as_str())
    );
    let (_, files) = parsed_v2.local_files_to_download_list(".").unwrap();
    assert_eq!(files[0].pieces_root, Some(expected_root));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_upgrade_v1_to_hybrid_multi_file() {
    let temp_dir = create_temp_dir("v1_to_hybrid_multi");
    let file1_data = vec![0xAAu8; 48 * 1024]; // 48 KiB (3 blocks)
    let file2_data = vec![0xBBu8; 16 * 1024]; // 16 KiB (1 block)

    let rel_dir = temp_dir.join("media");
    fs::create_dir_all(&rel_dir).expect("Failed to create media dir");
    fs::write(rel_dir.join("track1.mp3"), &file1_data).expect("Failed to write track1");
    fs::write(rel_dir.join("track2.mp3"), &file2_data).expect("Failed to write track2");

    // Build legacy v1 torrent
    let v1_bytes = TorrentBuilder::new("media")
        .piece_length(16 * 1024)
        .tracker("http://tracker.example.com/announce")
        .tracker("udp://tracker.openbittorrent.com:80/announce")
        .add_file("track1.mp3", file1_data.clone())
        .add_file("track2.mp3", file2_data.clone())
        .build_v1()
        .expect("Failed to build v1 multi-file torrent");

    let v1_torrent_path = temp_dir.join("media.torrent");
    fs::write(&v1_torrent_path, &v1_bytes).expect("Failed to write v1 torrent");

    // Upgrade to Hybrid
    let (hybrid_bytes, report) = TorrentUpgrader::upgrade_from_file(
        &v1_torrent_path,
        &temp_dir,
        TorrentExportFormat::Hybrid,
    )
    .expect("Failed to upgrade to hybrid");

    assert_eq!(report.name, "media");
    assert_eq!(report.format, TorrentExportFormat::Hybrid);
    assert_eq!(report.total_size, 64 * 1024);
    assert!(report.v1_info_hash.is_some());
    assert!(report.v2_info_hash.is_some());
    assert_eq!(report.v1_info_hash.as_ref().unwrap().len(), 40);
    assert_eq!(report.v2_info_hash.as_ref().unwrap().len(), 64);
    assert_eq!(report.file_pieces_roots.len(), 2);

    // Verify dual magnet parameters
    assert!(report.magnet_uri.contains("xt=urn:btih:"));
    assert!(report.magnet_uri.contains("xt=urn:btmh:1220"));

    // Verify parsed hybrid metainfo
    let mut parsed_hybrid = MetaInfoFile::from_bytes(&hybrid_bytes);
    parsed_hybrid
        .parse()
        .expect("Failed to parse upgraded hybrid torrent");
    assert!(parsed_hybrid.is_v2());
    assert!(parsed_hybrid.is_hybrid());
    assert_eq!(parsed_hybrid.get_name().unwrap(), "media");
    assert_eq!(parsed_hybrid.get_total_size().unwrap(), 64 * 1024);

    let v1_hash = parsed_hybrid
        .get_v1_info_hash()
        .expect("Must have v1 info hash");
    let v2_hash = parsed_hybrid
        .get_v2_info_hash()
        .expect("Must have v2 info hash");
    assert_eq!(v1_hash.len(), 20);
    assert_eq!(v2_hash.len(), 32);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_upgrade_piece_length_adjustment_to_bep52() {
    let temp_dir = create_temp_dir("bep52_clamp");
    let file_data = vec![0x44u8; 64 * 1024];
    fs::write(temp_dir.join("data.raw"), &file_data).expect("Failed to write test file");

    let mut metainfo = MetaInfoFile::from_bytes(
        &TorrentBuilder::new("data.raw")
            .piece_length(16 * 1024)
            .add_file("data.raw", file_data.clone())
            .build_v1()
            .expect("Failed to build v1"),
    );
    metainfo.parse().expect("Failed to parse");

    // Upgrade metainfo with in-memory files data
    let files_data = vec![("data.raw".to_string(), file_data)];
    let (_bytes, report) =
        TorrentUpgrader::upgrade_metainfo(&metainfo, &files_data, TorrentExportFormat::V2)
            .expect("Failed to upgrade");

    assert!(report.piece_length >= BLOCK_SIZE);
    assert_eq!(report.piece_length & (report.piece_length - 1), 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_upgrade_missing_file_returns_error() {
    let temp_dir = create_temp_dir("missing_file");
    let v1_bytes = TorrentBuilder::new("ghost.dat")
        .piece_length(16 * 1024)
        .add_file("ghost.dat", vec![0u8; 1024])
        .build_v1()
        .expect("Failed to build");

    let v1_path = temp_dir.join("ghost.torrent");
    fs::write(&v1_path, &v1_bytes).expect("Failed to write torrent");

    // Do NOT write "ghost.dat" to temp_dir
    let result = TorrentUpgrader::upgrade_from_file(&v1_path, &temp_dir, TorrentExportFormat::V2);
    assert!(result.is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_session_export_torrent_direct() {
    let temp_dir = create_temp_dir("session_export");
    let file_data = vec![0x77u8; 32 * 1024];
    let file_path = temp_dir.join("doc.bin");
    fs::write(&file_path, &file_data).expect("Failed to write file");

    let v1_bytes = TorrentBuilder::new("doc.bin")
        .piece_length(16 * 1024)
        .tracker("http://tracker.example.com/announce")
        .add_file("doc.bin", file_data.clone())
        .build_v1()
        .expect("Failed to build v1");

    let torrent_path = temp_dir.join("doc.torrent");
    fs::write(&torrent_path, &v1_bytes).expect("Failed to write torrent");

    let session = TorrentSession::builder(&torrent_path, &temp_dir)
        .seeding(true)
        .build()
        .expect("Failed to create session");

    // Export as Hybrid
    let (hybrid_bytes, report) = session
        .export_torrent(TorrentExportFormat::Hybrid)
        .expect("Failed to export as hybrid");

    assert_eq!(report.format, TorrentExportFormat::Hybrid);
    assert_eq!(report.total_size, 32 * 1024);
    assert!(!hybrid_bytes.is_empty());
    assert!(report.v1_info_hash.is_some());
    assert!(report.v2_info_hash.is_some());

    // Export as Pure v2
    let (v2_bytes, report_v2) = session
        .export_torrent(TorrentExportFormat::V2)
        .expect("Failed to export as v2");

    assert_eq!(report_v2.format, TorrentExportFormat::V2);
    assert!(!v2_bytes.is_empty());
    assert!(report_v2.v1_info_hash.is_none());
    assert!(report_v2.v2_info_hash.is_some());

    let _ = fs::remove_dir_all(&temp_dir);
}
