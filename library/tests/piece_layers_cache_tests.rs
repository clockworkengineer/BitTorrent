use bittorrent_rs::{
    FileDetails, FilePriority, MerkleTree, SequentialSelector, SessionConfig,
    TorrentSessionBuilder,
    internals::TorrentContext,
    storage::piece_layers_cache::{
        PieceLayersDiskCache, decode_layers, encode_layers, reconstruct_tree_from_piece_layer,
    },
};
use std::sync::Arc;

fn create_sample_merkle_tree(num_blocks: usize) -> MerkleTree {
    let mut leaves = Vec::with_capacity(num_blocks);
    for i in 0..num_blocks {
        let mut leaf = [0u8; 32];
        leaf[0] = (i & 0xFF) as u8;
        leaf[1] = ((i >> 8) & 0xFF) as u8;
        leaf[31] = 0xAA;
        leaves.push(leaf);
    }
    MerkleTree::from_leaf_hashes(leaves, (num_blocks * 16384) as u64)
}

#[test]
fn test_piece_layers_cache_encode_decode_roundtrip() {
    let tree = create_sample_merkle_tree(8);
    let root = tree.root();
    let original_layers = tree.layers().to_vec();

    let encoded = encode_layers(&root, &original_layers);
    assert!(encoded.starts_with(b"BT2L"));

    let (decoded_root, decoded_layers) = decode_layers(&encoded).expect("Decode valid layer data");
    assert_eq!(decoded_root, root);
    assert_eq!(decoded_layers.len(), original_layers.len());
    for (l1, l2) in original_layers.iter().zip(decoded_layers.iter()) {
        assert_eq!(l1, l2);
    }

    // Tampered checksum should trigger error
    let mut corrupted = encoded.clone();
    let last_idx = corrupted.len() - 1;
    corrupted[last_idx] ^= 0xFF;
    assert!(decode_layers(&corrupted).is_err());
}

#[test]
fn test_piece_layers_cache_disk_persistence_and_memory_tier() {
    let temp_dir = std::env::temp_dir().join("bt_test_piece_layers_persistence");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let cache = PieceLayersDiskCache::new(&temp_dir, 16).expect("Create cache");
    let tree = create_sample_merkle_tree(4);
    let root = tree.root();

    assert!(!cache.has_layers(&root));

    // Save to cache
    cache
        .save_layers(&root, tree.layers())
        .expect("Save layers");
    assert!(cache.has_layers(&root));
    assert!(cache.path_for_root(&root).exists());

    // Load from memory tier
    let loaded_mem = cache
        .load_layers(&root)
        .expect("Load from cache")
        .expect("Layers present");
    assert_eq!(loaded_mem.len(), tree.layers().len());
    assert_eq!(cache.metrics().cache_hits, 1);
    assert_eq!(cache.metrics().disk_reads, 0);

    // Instantiate fresh cold cache on the same directory
    let cold_cache = PieceLayersDiskCache::new(&temp_dir, 16).expect("Create cold cache");
    assert!(cold_cache.has_layers(&root));

    let loaded_disk = cold_cache
        .load_layers(&root)
        .expect("Load from disk")
        .expect("Layers present");
    assert_eq!(loaded_disk.len(), tree.layers().len());
    assert_eq!(cold_cache.metrics().cache_misses, 1);
    assert_eq!(cold_cache.metrics().disk_reads, 1);

    // Remove from cache
    assert!(cold_cache.remove(&root));
    assert!(!cold_cache.has_layers(&root));
    assert!(!cold_cache.path_for_root(&root).exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_serve_bep52_hash_request_from_cached_layers() {
    let temp_dir = std::env::temp_dir().join("bt_test_serve_hash_req");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let cache = PieceLayersDiskCache::new(&temp_dir, 16).expect("Create cache");
    let tree = create_sample_merkle_tree(8);
    let root = tree.root();
    cache
        .save_layers(&root, tree.layers())
        .expect("Save layers");

    // Serve hash request for block range [0..2] with 2 proof layers
    let resp = cache
        .serve_hash_request(&root, 0, 0, 2, 2)
        .expect("Serve hash request [0..2]");
    let verified = MerkleTree::verify_hashes_response(&root, 0, 0, 2, 2, &resp)
        .expect("Verify served hashes response");
    assert_eq!(verified.len(), 2);
    assert_eq!(verified[0], tree.layers()[0][0]);
    assert_eq!(verified[1], tree.layers()[0][1]);

    // Serve hash request for block range [4..6] with 2 proof layers
    let resp2 = cache
        .serve_hash_request(&root, 0, 4, 2, 2)
        .expect("Serve hash request [4..6]");
    let verified2 = MerkleTree::verify_hashes_response(&root, 0, 4, 2, 2, &resp2)
        .expect("Verify served hashes response 2");
    assert_eq!(verified2.len(), 2);
    assert_eq!(verified2[0], tree.layers()[0][4]);
    assert_eq!(verified2[1], tree.layers()[0][5]);

    // Verify invalid requests return None
    assert!(cache.serve_hash_request(&root, 0, 1, 2, 2).is_none()); // unaligned index
    assert!(cache.serve_hash_request(&root, 0, 0, 3, 2).is_none()); // non-power-of-two length
    assert!(cache.serve_hash_request(&root, 99, 0, 2, 2).is_none()); // out of bounds base layer

    let metrics = cache.metrics();
    assert_eq!(metrics.hash_requests_served, 2);
    assert!(metrics.bytes_served > 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_serve_bep52_hash_request_from_reconstructed_piece_layer() {
    let temp_dir = std::env::temp_dir().join("bt_test_reconstruct_piece_layer");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let cache = PieceLayersDiskCache::new(&temp_dir, 16).expect("Create cache");
    let tree = create_sample_merkle_tree(8);
    let root = tree.root();
    let piece_layer = &tree.layers()[0]; // 8 hashes

    let (recon_layers, recon_root) = reconstruct_tree_from_piece_layer(piece_layer);
    assert_eq!(recon_root, root);
    assert_eq!(recon_layers.len(), tree.layers().len());

    // Save only the piece layer
    cache
        .save_piece_layer(&root, piece_layer)
        .expect("Save piece layer");
    assert!(cache.has_layers(&root));

    // Serve hash request from reconstructed layers
    let resp = cache
        .serve_hash_request(&root, 0, 2, 2, 2)
        .expect("Serve from reconstructed layers");
    let verified = MerkleTree::verify_hashes_response(&root, 0, 2, 2, 2, &resp)
        .expect("Verify reconstructed response");
    assert_eq!(verified.len(), 2);
    assert_eq!(verified[0], piece_layer[2]);
    assert_eq!(verified[1], piece_layer[3]);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_piece_layers_cache_lru_memory_eviction_and_pruning() {
    let temp_dir = std::env::temp_dir().join("bt_test_piece_layers_lru");
    let _ = std::fs::remove_dir_all(&temp_dir);

    // 1 MB capacity ~ 32,768 hashes
    let cache = PieceLayersDiskCache::new(&temp_dir, 1).expect("Create cache");

    // Save 5 distinct trees
    let mut roots = Vec::new();
    for i in 0..5 {
        let tree = create_sample_merkle_tree(4 + i * 2);
        let root = tree.root();
        roots.push(root);
        cache.save_layers(&root, tree.layers()).expect("Save layer");
    }

    assert_eq!(cache.metrics().disk_writes, 5);

    // Prune cache to keep at most 2 files
    let pruned = cache.prune(2, 10_000_000).expect("Prune files");
    assert_eq!(pruned, 3);

    // Clear cache
    cache.clear().expect("Clear cache");
    assert_eq!(cache.metrics().cached_roots_count, 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_torrent_context_and_peer_wire_hash_request_serving() {
    let temp_dir = std::env::temp_dir().join("bt_test_context_hash_serving");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let mut config = SessionConfig::default();
    config.v2_piece_layers_cache_enabled = true;
    config.v2_piece_layers_cache_dir = Some(temp_dir.clone());

    let mut ctx = TorrentContext::new_magnet_bootstrap(
        vec![0xCC; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        Arc::new(SequentialSelector),
        &temp_dir,
        config,
    )
    .expect("Bootstrap context");

    let tree = create_sample_merkle_tree(8);
    let root = tree.root();

    ctx.files_to_download.push(FileDetails {
        name: "test_v2.bin".to_string(),
        torrent_path: "test_v2.bin".to_string(),
        length: 8 * 16384,
        md5sum: None,
        offset: 0,
        priority: FilePriority::Normal,
        pieces_root: Some(root),
    });

    // Register only the piece layer without in-memory full tree
    ctx.register_piece_layer(root, tree.layers()[0].clone());

    // Peer wire incoming hash_request: handle_hash_request should serve from disk cache!
    let resp = ctx
        .handle_hash_request(root, 0, 0, 2, 2)
        .expect("handle_hash_request served from cache");

    let verified = MerkleTree::verify_hashes_response(&root, 0, 0, 2, 2, &resp)
        .expect("Verify served hash response");
    assert_eq!(verified.len(), 2);
    assert_eq!(verified[0], tree.layers()[0][0]);
    assert_eq!(verified[1], tree.layers()[0][1]);

    let metrics = ctx.piece_layers_cache_metrics();
    assert!(metrics.hash_requests_served >= 1);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_session_builder_piece_layers_cache_configuration() {
    let temp_dir = std::env::temp_dir().join("bt_test_builder_layers_config");
    let custom_cache_dir = temp_dir.join("custom_layers");
    let _ = std::fs::remove_dir_all(&temp_dir);

    let session = TorrentSessionBuilder::from_magnet(
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        &temp_dir,
    )
    .enable_piece_layers_cache(true)
    .piece_layers_cache_dir(&custom_cache_dir)
    .piece_layers_cache_max_mb(128)
    .build()
    .expect("Build session with custom piece layers cache");

    let ctx_mutex = session.context();
    let ctx = ctx_mutex.lock().unwrap();
    assert!(ctx.config.v2_piece_layers_cache_enabled);
    assert_eq!(
        ctx.config.v2_piece_layers_cache_dir.as_deref(),
        Some(custom_cache_dir.as_path())
    );
    assert_eq!(ctx.config.v2_piece_layers_cache_max_mb, 128);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
