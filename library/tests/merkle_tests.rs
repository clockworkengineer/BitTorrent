use bittorrent_rs::core::merkle::{BLOCK_SIZE, MerkleTree};
use bittorrent_rs::metainfo::{FileDetails, MetaInfoFile};
use bittorrent_rs::{Bencode, BNode};
use sha2::{Digest, Sha256};

#[test]
fn test_merkle_tree_empty_and_single_block() {
    // 1. Empty file
    let empty_tree = MerkleTree::from_bytes(&[]);
    assert_eq!(empty_tree.file_length(), 0);
    assert_eq!(empty_tree.num_blocks(), 0);
    assert_eq!(empty_tree.root(), [0u8; 32]);
    assert_eq!(empty_tree.proof(0), None);

    // 2. Single block (< 16 KiB)
    let data = b"Hello, BitTorrent v2 Merkle Trees!";
    let single_tree = MerkleTree::from_bytes(data);
    let expected_leaf: [u8; 32] = Sha256::digest(data).into();

    assert_eq!(single_tree.num_blocks(), 1);
    assert_eq!(single_tree.root(), expected_leaf);
    assert_eq!(single_tree.block_hash(0), Some(expected_leaf));
    assert!(single_tree.verify_block(0, data));
    assert!(!single_tree.verify_block(0, b"Tampered data!"));

    // Proof for single block has empty sibling path
    let proof = single_tree.proof(0).expect("Proof should exist");
    assert!(proof.path.is_empty());
    assert!(proof.verify(data, &expected_leaf));
    assert!(!proof.verify(b"Wrong data", &expected_leaf));
}

#[test]
fn test_merkle_tree_power_of_two_four_blocks() {
    // 4 blocks = exactly 64 KiB
    let block_0 = vec![1u8; BLOCK_SIZE];
    let block_1 = vec![2u8; BLOCK_SIZE];
    let block_2 = vec![3u8; BLOCK_SIZE];
    let block_3 = vec![4u8; BLOCK_SIZE];

    let mut full_data = Vec::with_capacity(4 * BLOCK_SIZE);
    full_data.extend_from_slice(&block_0);
    full_data.extend_from_slice(&block_1);
    full_data.extend_from_slice(&block_2);
    full_data.extend_from_slice(&block_3);

    let tree = MerkleTree::from_bytes(&full_data);
    assert_eq!(tree.num_blocks(), 4);

    let h0: [u8; 32] = Sha256::digest(&block_0).into();
    let h1: [u8; 32] = Sha256::digest(&block_1).into();
    let h2: [u8; 32] = Sha256::digest(&block_2).into();
    let h3: [u8; 32] = Sha256::digest(&block_3).into();

    // Verify parent hashes: H(left || right)
    let p0: [u8; 32] = Sha256::digest([h0.as_slice(), h1.as_slice()].concat()).into();
    let p1: [u8; 32] = Sha256::digest([h2.as_slice(), h3.as_slice()].concat()).into();
    let root: [u8; 32] = Sha256::digest([p0.as_slice(), p1.as_slice()].concat()).into();

    assert_eq!(tree.root(), root);

    // Verify all blocks via tree
    assert!(tree.verify_block(0, &block_0));
    assert!(tree.verify_block(1, &block_1));
    assert!(tree.verify_block(2, &block_2));
    assert!(tree.verify_block(3, &block_3));

    // Test Merkle Proofs (audit paths) for each leaf
    for (i, block) in [&block_0, &block_1, &block_2, &block_3].into_iter().enumerate() {
        let proof = tree.proof(i).expect("Proof should be generated");
        assert_eq!(proof.path.len(), 2); // log2(4) = 2 sibling hashes
        assert!(proof.verify(block, &root));

        // Corrupted block must fail proof
        let mut corrupted = block.clone();
        corrupted[0] ^= 0xFF;
        assert!(!proof.verify(&corrupted, &root));
    }
}

#[test]
fn test_merkle_tree_odd_blocks_with_zero_padding() {
    // 3 blocks: requires padding to 4 with [0u8; 32]
    let block_0 = vec![10u8; BLOCK_SIZE];
    let block_1 = vec![20u8; BLOCK_SIZE];
    let block_2 = vec![30u8; 1000]; // last block < 16 KiB

    let mut full_data = Vec::new();
    full_data.extend_from_slice(&block_0);
    full_data.extend_from_slice(&block_1);
    full_data.extend_from_slice(&block_2);

    let tree = MerkleTree::from_bytes(&full_data);
    assert_eq!(tree.num_blocks(), 3);

    let h0: [u8; 32] = Sha256::digest(&block_0).into();
    let h1: [u8; 32] = Sha256::digest(&block_1).into();
    let h2: [u8; 32] = Sha256::digest(&block_2).into();
    let h3: [u8; 32] = [0u8; 32]; // Zero padding per BEP 52

    let p0: [u8; 32] = Sha256::digest([h0.as_slice(), h1.as_slice()].concat()).into();
    let p1: [u8; 32] = Sha256::digest([h2.as_slice(), h3.as_slice()].concat()).into();
    let root: [u8; 32] = Sha256::digest([p0.as_slice(), p1.as_slice()].concat()).into();

    assert_eq!(tree.root(), root);

    // Verify proof for block 2 (which pairs with the zero padding node)
    let proof_2 = tree.proof(2).expect("Proof for block 2");
    assert_eq!(proof_2.path[0], [0u8; 32]); // sibling is zero-padding
    assert_eq!(proof_2.path[1], p0);
    assert!(proof_2.verify(&block_2, &root));
}

#[test]
fn test_merkle_tree_piece_layer_and_reconstruction() {
    // 8 blocks of 16 KiB = 128 KiB
    let full_data = vec![42u8; 8 * BLOCK_SIZE];
    let tree = MerkleTree::from_bytes(&full_data);

    // Piece length 32 KiB = 2 blocks per piece -> 4 pieces
    let pieces_32k = tree.piece_layer(32 * 1024).expect("Valid piece layer");
    assert_eq!(pieces_32k.len(), 4);

    let computed_root = MerkleTree::compute_root_from_piece_hashes(&pieces_32k);
    assert_eq!(computed_root, tree.root());

    // Piece length 64 KiB = 4 blocks per piece -> 2 pieces
    let pieces_64k = tree.piece_layer(64 * 1024).expect("Valid piece layer");
    assert_eq!(pieces_64k.len(), 2);
    let computed_root_64k = MerkleTree::compute_root_from_piece_hashes(&pieces_64k);
    assert_eq!(computed_root_64k, tree.root());
}

#[test]
fn test_metainfo_v2_file_tree_pieces_root_parsing() {
    let dummy_root = [0xABu8; 32];

    let file_props = BNode::Dictionary(vec![
        (b"length" as &[u8], BNode::Number(b"32768")),
        (b"pieces root" as &[u8], BNode::String(&dummy_root)),
    ]);
    let file_leaf = BNode::Dictionary(vec![
        (b"" as &[u8], file_props),
    ]);
    let file_tree = BNode::Dictionary(vec![
        (b"test_video.mp4" as &[u8], file_leaf),
    ]);

    let info_dict = BNode::Dictionary(vec![
        (b"name" as &[u8], BNode::String(b"v2_torrent")),
        (b"piece length" as &[u8], BNode::Number(b"65536")),
        (b"meta version" as &[u8], BNode::Number(b"2")),
        (b"file tree" as &[u8], file_tree),
    ]);

    let root_dict = BNode::Dictionary(vec![
        (b"announce" as &[u8], BNode::String(b"http://tracker.example.com/announce")),
        (b"info" as &[u8], info_dict),
    ]);

    let encoded = Bencode::encode(&root_dict);
    let mut meta = MetaInfoFile::from_bytes(&encoded);
    meta.parse().expect("Parse failed");

    assert!(meta.is_v2());

    let (total_bytes, files) = meta.local_files_to_download_list(std::path::Path::new("."))
        .expect("Failed to list files");
    assert_eq!(total_bytes, 32768);
    assert_eq!(files.len(), 1);

    let file = &files[0];
    assert_eq!(file.pieces_root, Some(dummy_root));
    assert_eq!(file.pieces_root(), Some(&dummy_root));
    assert_eq!(file.pieces_root_hex(), Some("ab".repeat(32)));
}

#[test]
fn test_torrent_context_v2_block_verification() {
    let block_data = vec![7u8; BLOCK_SIZE];
    let tree = MerkleTree::from_bytes(&block_data);

    let file = FileDetails {
        name: "test.dat".to_string(),
        torrent_path: "test.dat".to_string(),
        length: BLOCK_SIZE as u64,
        md5sum: None,
        offset: 0,
        pieces_root: Some(tree.root()),
        priority: bittorrent_rs::FilePriority::Normal,
    };

    let config = bittorrent_rs::SessionConfig::default();
    let selector = std::sync::Arc::new(bittorrent_rs::SequentialSelector);

    let mut ctx = bittorrent_rs::internals::TorrentContext::new_magnet_bootstrap(
        vec![0u8; 32],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        std::path::Path::new("."),
        config,
    ).expect("Bootstrap failed");

    ctx.piece_length = BLOCK_SIZE as u32;
    ctx.files_to_download = vec![file];

    // 1. Single block file validation using pieces_root
    assert!(ctx.verify_v2_block(0, 0, &block_data).is_ok());

    let mut corrupted = block_data.clone();
    corrupted[0] ^= 0x55;
    assert!(ctx.verify_v2_block(0, 0, &corrupted).is_err());

    // 2. Multi-block file with registered MerkleTree
    let multi_data = vec![99u8; 2 * BLOCK_SIZE];
    let multi_tree = MerkleTree::from_bytes(&multi_data);

    let multi_file = FileDetails {
        name: "multi.dat".to_string(),
        torrent_path: "multi.dat".to_string(),
        length: (2 * BLOCK_SIZE) as u64,
        md5sum: None,
        offset: 0,
        pieces_root: Some(multi_tree.root()),
        priority: bittorrent_rs::FilePriority::Normal,
    };
    ctx.files_to_download = vec![multi_file];
    ctx.register_file_merkle_tree(0, multi_tree);

    let block_0 = &multi_data[..BLOCK_SIZE];
    let block_1 = &multi_data[BLOCK_SIZE..];

    assert!(ctx.verify_v2_block(0, 0, block_0).is_ok());
    assert!(ctx.verify_v2_block(1, 0, block_1).is_ok());

    let mut corrupt_block = block_0.to_vec();
    corrupt_block[4] ^= 0xAA;
    assert!(ctx.verify_v2_block(0, 0, &corrupt_block).is_err());
}
