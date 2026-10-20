mod common;

use bittorrent_rs::metainfo::FileDetails;
use bittorrent_rs::storage::{
    BitfieldScanner, CachedBlockStorage, DiskBlockStorage, FileAllocationManager,
};
use bittorrent_rs::{BlockReader, BlockWriter, MemStorage, NullBlockStorage};
use common::storage_contract::verify_block_storage_contract;
use sha1::Digest;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_test_dir(name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let dir = std::env::temp_dir().join(format!("solid_phase5_{}_{}", name, millis));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_test_file(path: &std::path::Path, length: u64, offset: u64) -> FileDetails {
    FileDetails {
        name: path.to_string_lossy().to_string(),
        torrent_path: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        length,
        offset,
        md5sum: None,
        pieces_root: None,
        priority: bittorrent_rs::metainfo::FilePriority::Normal,
    }
}

#[test]
fn test_mem_storage_lsp_contract() {
    let storage = MemStorage::new(128);
    verify_block_storage_contract(&storage, 128);
}

#[test]
fn test_disk_block_storage_lsp_contract() {
    let test_dir = unique_test_dir("disk_storage_contract");
    let file1_path = test_dir.join("file1.bin");
    let file2_path = test_dir.join("subdir").join("file2.bin");

    let files = vec![
        make_test_file(&file1_path, 64, 0),
        make_test_file(&file2_path, 64, 64),
    ];

    // Allocate structure via FileAllocationManager (SRP)
    FileAllocationManager::allocate_torrent_structure(&test_dir, &files)
        .expect("file allocation succeeds");
    assert!(FileAllocationManager::verify_files_exist(&files));

    let disk_storage = DiskBlockStorage::new(files);
    assert_eq!(disk_storage.total_capacity(), 128);

    verify_block_storage_contract(&disk_storage, 128);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cached_block_storage_lsp_contract() {
    let inner = std::sync::Arc::new(MemStorage::new(256));
    let cached = CachedBlockStorage::new(inner, 256);
    verify_block_storage_contract(&cached, 256);
}

#[test]
fn test_null_block_storage_lsp_contract() {
    let null_storage = NullBlockStorage;

    // 1. Zero-byte reads/writes are safe no-ops
    let mut empty_buf = [0u8; 0];
    assert_eq!(null_storage.read_block(0, &mut empty_buf).unwrap(), 0);
    assert!(null_storage.write_block(0, &[]).is_ok());

    // 2. Non-zero reads/writes fail with clear bootstrap uninitialized error
    let mut buf = [0u8; 4];
    let read_err = null_storage.read_block(0, &mut buf);
    assert!(read_err.is_err());
    assert!(format!("{}", read_err.unwrap_err()).contains("uninitialized"));

    let write_err = null_storage.write_block(0, &[1, 2, 3]);
    assert!(write_err.is_err());
    assert!(format!("{}", write_err.unwrap_err()).contains("uninitialized"));

    // 3. Flush & discard are idempotent no-ops
    assert!(null_storage.flush().is_ok());
    assert!(null_storage.flush().is_ok());
    null_storage.discard_range(0, 100);
}

#[test]
fn test_file_allocation_manager_isolated() {
    let test_dir = unique_test_dir("file_alloc_isolated");
    let file1_path = test_dir.join("sub1").join("sub2").join("alpha.dat");
    let file2_path = test_dir.join("beta.dat");

    let files = vec![
        make_test_file(&file1_path, 1024, 0),
        make_test_file(&file2_path, 2048, 1024),
    ];

    assert!(!FileAllocationManager::verify_files_exist(&files));

    FileAllocationManager::allocate_torrent_structure(&test_dir, &files)
        .expect("allocation succeeds");

    assert!(FileAllocationManager::verify_files_exist(&files));
    assert_eq!(fs::metadata(&file1_path).unwrap().len(), 1024);
    assert_eq!(fs::metadata(&file2_path).unwrap().len(), 2048);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_bitfield_scanner_isolated() {
    let piece_length = 32u32;
    let piece0 = vec![0x11u8; piece_length as usize];
    let piece1 = vec![0x22u8; piece_length as usize];

    let hash0 = sha1::Sha1::digest(&piece0);
    let hash1 = sha1::Sha1::digest(&piece1);

    let mut expected_hashes = Vec::new();
    expected_hashes.extend_from_slice(&hash0);
    expected_hashes.extend_from_slice(&hash1);

    // 1. Test scanning abstract BlockStorage (MemStorage)
    let storage = MemStorage::new((piece_length * 2) as usize);
    storage.write_block(0, &piece0).unwrap();
    // Leave piece1 zeroed (corrupted / incomplete)

    let scan_result = BitfieldScanner::scan_storage(
        &storage,
        2,
        piece_length,
        (piece_length * 2) as u64,
        &expected_hashes,
    )
    .expect("storage scan succeeds");

    assert_eq!(scan_result.piece_statuses.len(), 2);
    assert!(scan_result.piece_statuses[0].is_valid);
    assert!(!scan_result.piece_statuses[1].is_valid);
    assert_eq!(scan_result.downloaded_bytes, piece_length as u64);
    assert_eq!(scan_result.bitfield[0], 0x80); // First bit set

    // 2. Test scanning disk files
    let test_dir = unique_test_dir("bitfield_scanner_isolated");
    let file_path = test_dir.join("single_torrent.bin");

    let files = vec![make_test_file(&file_path, (piece_length * 2) as u64, 0)];

    FileAllocationManager::allocate_torrent_structure(&test_dir, &files).unwrap();

    // Write piece 0 and piece 1 to disk
    let disk_storage = DiskBlockStorage::new(files.clone());
    disk_storage.write_block(0, &piece0).unwrap();
    disk_storage.write_block(piece_length as u64, &piece1).unwrap();

    let disk_scan = BitfieldScanner::scan_local_files(&files, piece_length, &expected_hashes)
        .expect("disk scan succeeds");

    assert_eq!(disk_scan.piece_statuses.len(), 2);
    assert!(disk_scan.piece_statuses[0].is_valid);
    assert!(disk_scan.piece_statuses[1].is_valid);
    assert_eq!(disk_scan.downloaded_bytes, (piece_length * 2) as u64);
    assert_eq!(disk_scan.bitfield[0], 0xC0); // Both pieces verified

    let _ = fs::remove_dir_all(&test_dir);
}
