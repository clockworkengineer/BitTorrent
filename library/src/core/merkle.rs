//! BitTorrent v2 Merkle Tree & Block Validation (BEP 52)
//!
//! BitTorrent v2 replaces the flat SHA-1 piece hashes from v1 with SHA-256 Merkle trees
//! constructed for each file. This module provides binary Merkle tree generation, root
//! calculation, audit path proof extraction, and granular 16 KiB block validation.
//!
//! ## Structure (BEP 52)
//!
//! - **Block Size**: Files are divided into 16 KiB (16,384 bytes) leaf blocks.
//! - **Leaf Nodes**: `SHA256(block_data)`.
//! - **Padding**: If the number of leaves is not a power of 2, the layer is padded with
//!   32-byte zero hashes (`[0u8; 32]`).
//! - **Parent Nodes**: `SHA256(left_child || right_child)`.
//! - **Pieces Root**: The Merkle root hash of the file, recorded in the torrent metadata `file tree`.
//! - **Piece Layers**: The hashes at the level corresponding to the torrent's piece length.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sha2::{Digest, Sha256};
use crate::error::BitTorrentError;

/// Standard BitTorrent v2 block size in bytes (16 KiB).
pub const BLOCK_SIZE: usize = 16_384;

/// SHA-256 hash length in bytes.
pub const HASH_SIZE: usize = 32;

/// A cryptographic proof verifying that a 16 KiB block belongs to a file's Merkle tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleProof {
    /// 0-indexed leaf block index within the file.
    pub block_index: usize,
    /// Sibling hashes from the leaf level up to the root.
    pub path: Vec<[u8; 32]>,
}

impl MerkleProof {
    /// Creates a new `MerkleProof` for `block_index` with the given sibling `path`.
    pub fn new(block_index: usize, path: Vec<[u8; 32]>) -> Self {
        Self { block_index, path }
    }

    /// Verifies that `block_data` belongs to the Merkle tree with root `expected_root`.
    pub fn verify(&self, block_data: &[u8], expected_root: &[u8; 32]) -> bool {
        let block_hash = Sha256::digest(block_data);
        let mut cur: [u8; 32] = block_hash.into();
        let mut idx = self.block_index;

        if self.path.is_empty() {
            return cur == *expected_root;
        }

        for sibling in &self.path {
            let mut hasher = Sha256::new();
            if idx % 2 == 0 {
                hasher.update(&cur);
                hasher.update(sibling);
            } else {
                hasher.update(sibling);
                hasher.update(&cur);
            }
            cur = hasher.finalize().into();
            idx /= 2;
        }

        cur == *expected_root
    }
}

/// A complete binary SHA-256 Merkle tree for a file in a BitTorrent v2 torrent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleTree {
    /// Total file length in bytes.
    file_length: u64,
    /// Number of actual data blocks in the file.
    num_blocks: usize,
    /// Layers of the tree from bottom (0 = leaves) to top (last = root).
    layers: Vec<Vec<[u8; 32]>>,
    /// Cached root hash (`pieces root`).
    root: [u8; 32],
}

impl MerkleTree {
    /// Constructs a `MerkleTree` from raw file bytes.
    ///
    /// Chunks `data` into 16 KiB blocks, computes SHA-256 leaf hashes, pads to the
    /// next power of two with 32-byte zeroes, and computes all upper layers up to the root.
    pub fn from_bytes(data: &[u8]) -> Self {
        let file_length = data.len() as u64;
        if data.is_empty() {
            return Self {
                file_length: 0,
                num_blocks: 0,
                layers: Vec::new(),
                root: [0u8; 32],
            };
        }

        let mut leaves = Vec::new();
        for chunk in data.chunks(BLOCK_SIZE) {
            let hash = Sha256::digest(chunk);
            leaves.push(hash.into());
        }

        Self::from_leaf_hashes(leaves, file_length)
    }

    /// Constructs a `MerkleTree` from precomputed 16 KiB leaf hashes.
    pub fn from_leaf_hashes(leaves: Vec<[u8; 32]>, file_length: u64) -> Self {
        let num_blocks = leaves.len();
        if num_blocks == 0 {
            return Self {
                file_length,
                num_blocks: 0,
                layers: Vec::new(),
                root: [0u8; 32],
            };
        }

        if num_blocks == 1 {
            let root = leaves[0];
            return Self {
                file_length,
                num_blocks: 1,
                layers: vec![leaves],
                root,
            };
        }

        let power_of_two = num_blocks.next_power_of_two();
        let mut leaf_layer = leaves;
        while leaf_layer.len() < power_of_two {
            leaf_layer.push([0u8; 32]);
        }

        let mut layers = Vec::new();
        layers.push(leaf_layer);

        while layers.last().unwrap().len() > 1 {
            let prev_layer = layers.last().unwrap();
            let mut next_layer = Vec::with_capacity(prev_layer.len() / 2);
            for pair in prev_layer.chunks_exact(2) {
                let mut hasher = Sha256::new();
                hasher.update(&pair[0]);
                hasher.update(&pair[1]);
                next_layer.push(hasher.finalize().into());
            }
            layers.push(next_layer);
        }

        let root = layers.last().unwrap()[0];

        Self {
            file_length,
            num_blocks,
            layers,
            root,
        }
    }

    /// Returns the root hash of the tree (`pieces root`).
    pub fn root(&self) -> [u8; 32] {
        self.root
    }

    /// Returns the root hash formatted as a 64-character lowercase hexadecimal string.
    pub fn root_hex(&self) -> String {
        self.root.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Returns the total file length in bytes.
    pub fn file_length(&self) -> u64 {
        self.file_length
    }

    /// Returns the number of 16 KiB blocks in the file.
    pub fn num_blocks(&self) -> usize {
        self.num_blocks
    }

    /// Returns the leaf hash for a specific 16 KiB block, if within file bounds.
    pub fn block_hash(&self, block_index: usize) -> Option<[u8; 32]> {
        if block_index < self.num_blocks && !self.layers.is_empty() {
            Some(self.layers[0][block_index])
        } else {
            None
        }
    }

    /// Generates a `MerkleProof` (audit path) for `block_index`.
    pub fn proof(&self, block_index: usize) -> Option<MerkleProof> {
        if block_index >= self.num_blocks || self.layers.is_empty() {
            return None;
        }

        if self.layers.len() == 1 {
            return Some(MerkleProof::new(block_index, Vec::new()));
        }

        let mut path = Vec::with_capacity(self.layers.len() - 1);
        let mut idx = block_index;

        for layer in &self.layers[..self.layers.len() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            path.push(layer[sibling_idx]);
            idx /= 2;
        }

        Some(MerkleProof::new(block_index, path))
    }

    /// Verifies whether `block_data` matches the expected hash at `block_index`.
    pub fn verify_block(&self, block_index: usize, block_data: &[u8]) -> bool {
        if let Some(expected_hash) = self.block_hash(block_index) {
            let actual_hash = Sha256::digest(block_data);
            actual_hash.as_slice() == expected_hash
        } else {
            false
        }
    }

    /// Returns the piece layer hashes corresponding to a given `piece_length`.
    ///
    /// For a BitTorrent v2 torrent, the piece layer contains the Merkle hashes at the
    /// height where each node represents `piece_length` bytes of file data.
    pub fn piece_layer(&self, piece_length: usize) -> Result<Vec<[u8; 32]>, BitTorrentError> {
        if piece_length < BLOCK_SIZE || (piece_length & (piece_length - 1)) != 0 {
            return Err(BitTorrentError::Parse(
                "Piece length must be a power of two and at least 16 KiB".into(),
            ));
        }

        let blocks_per_piece = piece_length / BLOCK_SIZE;
        let layer_height = blocks_per_piece.trailing_zeros() as usize;

        if layer_height >= self.layers.len() {
            // File is smaller than or equal to one piece
            return Ok(vec![self.root]);
        }

        let layer = &self.layers[layer_height];
        // Calculate the number of pieces required for the file
        let num_pieces = (self.file_length as usize).div_ceil(piece_length);
        Ok(layer[..num_pieces.min(layer.len())].to_vec())
    }

    /// Computes the root hash of a file given its complete list of piece hashes.
    pub fn compute_root_from_piece_hashes(piece_hashes: &[[u8; 32]]) -> [u8; 32] {
        if piece_hashes.is_empty() {
            return [0u8; 32];
        }
        if piece_hashes.len() == 1 {
            return piece_hashes[0];
        }

        let power_of_two = piece_hashes.len().next_power_of_two();
        let mut cur = piece_hashes.to_vec();
        while cur.len() < power_of_two {
            cur.push([0u8; 32]);
        }

        while cur.len() > 1 {
            let mut next = Vec::with_capacity(cur.len() / 2);
            for pair in cur.chunks_exact(2) {
                let mut hasher = Sha256::new();
                hasher.update(&pair[0]);
                hasher.update(&pair[1]);
                next.push(hasher.finalize().into());
            }
            cur = next;
        }

        cur[0]
    }
}
