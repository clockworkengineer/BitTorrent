use bittorrent_rs::{
    core::merkle::{BLOCK_SIZE, MerkleTree},
    core::torrent_builder::TorrentBuilder,
    metainfo::{FileDetails, MetaInfoFile},
    internals::TorrentContext,
    SessionConfig, SequentialSelector,
};
use std::io::Cursor;
use std::sync::Arc;

#[test]
fn test_torrent_builder_v2_generation_and_parsing() {
    let file1_data = vec![0x11u8; 32 * 1024]; // 32 KiB (2 blocks)
    let file2_data = vec![0x22u8; 128 * 1024]; // 128 KiB (8 blocks)
    let file3_data = b"small single block text file".to_vec();

    let torrent_bytes = TorrentBuilder::new("v2_test_torrent")
        .piece_length(64 * 1024) // 64 KiB piece size
        .tracker("http://tracker1.example.com:8080/announce")
        .tracker("http://tracker2.example.com:8080/announce")
        .comment("BitTorrent v2 BEP 52 test")
        .private(false)
        .add_file("video/intro.mp4", file1_data.clone())
        .add_file("video/feature.mp4", file2_data.clone())
        .add_file("readme.txt", file3_data.clone())
        .build_v2()
        .expect("Failed to build v2 torrent");

    let mut meta = MetaInfoFile::from_bytes(&torrent_bytes);
    meta.parse().expect("Failed to parse generated v2 torrent");

    assert!(meta.is_v2());
    assert!(!meta.is_hybrid());

    // Pure v2 info hash is 32-byte SHA-256
    let v2_hash = meta.get_v2_info_hash().expect("Must have v2 info hash");
    assert_eq!(v2_hash.len(), 32);
    assert_eq!(meta.get_info_hash().unwrap(), v2_hash.to_vec());

    // Tracker announcement list
    let trackers = meta.get_tracker_urls().expect("Trackers must exist");
    assert_eq!(trackers.len(), 2);
    assert!(trackers[0].contains("tracker1"));
    assert!(trackers[1].contains("tracker2"));

    // Files list and pieces roots
    let (total_bytes, files) = meta.local_files_to_download_list(std::path::Path::new("."))
        .expect("Failed to list files");
    assert_eq!(total_bytes, (32 * 1024 + 128 * 1024 + file3_data.len()) as u64);
    assert_eq!(files.len(), 3);

    // Each file must have its computed pieces root
    let tree1 = MerkleTree::from_bytes(&file1_data);
    let tree2 = MerkleTree::from_bytes(&file2_data);
    let tree3 = MerkleTree::from_bytes(&file3_data);

    let find_file = |name: &str| -> &FileDetails {
        files.iter().find(|f| f.torrent_path.ends_with(name)).unwrap()
    };

    assert_eq!(find_file("intro.mp4").pieces_root, Some(tree1.root()));
    assert_eq!(find_file("feature.mp4").pieces_root, Some(tree2.root()));
    assert_eq!(find_file("readme.txt").pieces_root, Some(tree3.root()));

    // Piece layers: file2 is 128 KiB > 64 KiB piece size, so it must have an entry in piece layers
    let piece_layers = meta.get_piece_layers();
    assert!(piece_layers.contains_key(&tree2.root()));
    let layer = &piece_layers[&tree2.root()];
    assert_eq!(layer.len(), 2); // 128 KiB / 64 KiB = 2 pieces
    assert_eq!(MerkleTree::compute_root_from_piece_hashes(layer), tree2.root());
}

#[test]
fn test_torrent_builder_hybrid_generation_and_dual_info_hashes() {
    let file1_data = vec![0xAAu8; 48 * 1024]; // 48 KiB
    let file2_data = vec![0xBBu8; 80 * 1024]; // 80 KiB

    let torrent_bytes = TorrentBuilder::new("hybrid_torrent")
        .piece_length(64 * 1024) // 64 KiB
        .tracker("http://tracker.example.com/announce")
        .add_file("data1.bin", file1_data.clone())
        .add_file("data2.bin", file2_data.clone())
        .build_hybrid()
        .expect("Failed to build hybrid torrent");

    let mut meta = MetaInfoFile::from_bytes(&torrent_bytes);
    meta.parse().expect("Failed to parse hybrid torrent");

    // Both v1 and v2 attributes are active
    assert!(meta.is_v2());
    assert!(meta.is_hybrid());

    // Dual info-hashes
    let v1_hash = meta.get_v1_info_hash().expect("Must have v1 info hash");
    let v2_hash = meta.get_v2_info_hash().expect("Must have v2 info hash");
    assert_eq!(v1_hash.len(), 20);
    assert_eq!(v2_hash.len(), 32);

    // Primary info hash for hybrid torrents is the 20-byte v1 hash for backward compatibility
    assert_eq!(meta.get_info_hash().unwrap(), v1_hash.to_vec());

    // Inspect files list for pad files
    let (_total_bytes, files) = meta.local_files_to_download_list(std::path::Path::new("."))
        .expect("Failed to list files");

    // File 1 has 48 KiB, needs 16 KiB pad to reach 64 KiB piece boundary
    let pad_files: Vec<&FileDetails> = files.iter().filter(|f| f.is_pad_file()).collect();
    assert!(!pad_files.is_empty(), "Hybrid torrent must insert pad files for piece alignment");
    for pad in &pad_files {
        assert!(pad.is_pad_file());
    }

    // Verify v2 pieces roots are properly populated
    let tree1 = MerkleTree::from_bytes(&file1_data);
    let tree2 = MerkleTree::from_bytes(&file2_data);
    let real_files: Vec<&FileDetails> = files.iter().filter(|f| !f.is_pad_file()).collect();
    assert_eq!(real_files[0].pieces_root, Some(tree1.root()));
    assert_eq!(real_files[1].pieces_root, Some(tree2.root()));
}

#[test]
fn test_merkle_tree_from_reader_incremental_streaming() {
    // 5 blocks = 4 * 16 KiB + 1000 bytes = 66,536 bytes
    let mut data = vec![0x77u8; 4 * BLOCK_SIZE];
    data.extend(vec![0x88u8; 1000]);

    let tree_from_mem = MerkleTree::from_bytes(&data);

    let mut cursor = Cursor::new(&data);
    let tree_from_stream = MerkleTree::from_reader(&mut cursor, data.len() as u64)
        .expect("Streaming from reader failed");

    assert_eq!(tree_from_stream.file_length(), tree_from_mem.file_length());
    assert_eq!(tree_from_stream.num_blocks(), tree_from_mem.num_blocks());
    assert_eq!(tree_from_stream.root(), tree_from_mem.root());
    assert_eq!(tree_from_stream.root_hex(), tree_from_mem.root_hex());

    for i in 0..tree_from_mem.num_blocks() {
        assert_eq!(tree_from_stream.block_hash(i), tree_from_mem.block_hash(i));
        assert_eq!(tree_from_stream.proof(i), tree_from_mem.proof(i));
    }
}

#[test]
fn test_merkle_tree_from_file_on_disk() {
    let test_dir = std::env::temp_dir().join("bep52_test");
    let _ = std::fs::create_dir_all(&test_dir);
    let file_path = test_dir.join("disk_test_file.bin");

    let file_data = vec![0x55u8; 3 * BLOCK_SIZE];
    std::fs::write(&file_path, &file_data).expect("Failed to write test file");

    let tree_disk = MerkleTree::from_file(&file_path).expect("from_file failed");
    let tree_mem = MerkleTree::from_bytes(&file_data);

    assert_eq!(tree_disk.root(), tree_mem.root());
    assert_eq!(tree_disk.num_blocks(), 3);

    let _ = std::fs::remove_file(&file_path);
}

#[test]
fn test_torrent_context_merkle_proof_audit_path_verification() {
    // 4 blocks = 64 KiB
    let b0 = vec![1u8; BLOCK_SIZE];
    let b1 = vec![2u8; BLOCK_SIZE];
    let b2 = vec![3u8; BLOCK_SIZE];
    let b3 = vec![4u8; BLOCK_SIZE];

    let mut full_file = Vec::new();
    full_file.extend_from_slice(&b0);
    full_file.extend_from_slice(&b1);
    full_file.extend_from_slice(&b2);
    full_file.extend_from_slice(&b3);

    let tree = MerkleTree::from_bytes(&full_file);
    let root = tree.root();

    let file = FileDetails {
        name: "audit.dat".to_string(),
        torrent_path: "audit.dat".to_string(),
        length: full_file.len() as u64,
        md5sum: None,
        offset: 0,
        pieces_root: Some(root),
        priority: bittorrent_rs::FilePriority::Normal,
    };

    let config = SessionConfig::default();
    let selector = Arc::new(SequentialSelector);
    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0u8; 32],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    ).expect("Bootstrap failed");

    ctx.piece_length = (2 * BLOCK_SIZE) as u32; // 32 KiB pieces (2 blocks per piece)
    ctx.files_to_download = vec![file];

    // Note: ctx does NOT have the full MerkleTree registered.
    // Instead, register only an individual MerkleProof for block 2 (in piece 1, block 0)
    let proof_2 = tree.proof(2).expect("Proof for block 2");
    ctx.register_merkle_proof(0, proof_2);

    // Verify block 2 using audit path proof
    // piece 1, block 0 corresponds to file block index (1 * 32768 + 0) / 16384 = 2
    assert!(ctx.verify_v2_block(1, 0, &b2).is_ok());

    // Corrupted block 2 must fail verification
    let mut corrupt_b2 = b2.clone();
    corrupt_b2[0] ^= 0xFF;
    assert!(ctx.verify_v2_block(1, 0, &corrupt_b2).is_err());

    // Block 3 without registered proof or tree defaults to Ok if not single block,
    // but once its proof is registered, it verifies strictly
    let proof_3 = tree.proof(3).expect("Proof for block 3");
    ctx.register_merkle_proof(0, proof_3);
    assert!(ctx.verify_v2_block(1, 1, &b3).is_ok());
}

#[test]
fn test_piece_layer_registration_and_hash_consistency() {
    let file_data = vec![0x99u8; 8 * BLOCK_SIZE]; // 128 KiB
    let tree = MerkleTree::from_bytes(&file_data);
    let piece_length = 32 * 1024; // 32 KiB

    let layer = tree.piece_layer(piece_length).expect("Piece layer");
    assert_eq!(layer.len(), 4);

    let config = SessionConfig::default();
    let selector = Arc::new(SequentialSelector);
    let ctx = TorrentContext::new_magnet_bootstrap(
        vec![0u8; 32],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    ).expect("Bootstrap failed");

    ctx.register_piece_layer(tree.root(), layer.clone());

    // Verify stored piece layer matches
    let layers_guard = ctx.piece_layers.lock().unwrap();
    assert_eq!(layers_guard.get(&tree.root()), Some(&layer));
    assert_eq!(MerkleTree::compute_root_from_piece_hashes(&layer), tree.root());
}
