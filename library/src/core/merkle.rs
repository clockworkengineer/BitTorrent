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

    /// Returns a slice of all layers in the Merkle tree from bottom (0 = leaf blocks) to top (last = root).
    pub fn layers(&self) -> &[Vec<[u8; 32]>] {
        &self.layers
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

    /// Generates a binary payload of hashes answering a BEP 52 `hash_request`.
    ///
    /// Contains `length` base layer hashes followed by necessary ancestor uncle hashes.
    /// In accordance with BEP 52, the first `log2(length) - 1` proof layers are omitted
    /// because they are internal to the requested subtree.
    pub fn generate_hashes_response(
        &self,
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    ) -> Option<Vec<u8>> {
        if length < 2 || (length & (length - 1)) != 0 {
            return None;
        }
        if index % length != 0 {
            return None;
        }
        let base = base_layer as usize;
        if base >= self.layers.len() {
            return None;
        }

        let layer = &self.layers[base];
        let idx = index as usize;
        let len = length as usize;
        if idx + len > layer.len() {
            return None;
        }

        let k = len.trailing_zeros() as usize; // log2(length)
        let omitted = k.saturating_sub(1);
        let max_uncles = (proof_layers as usize).saturating_sub(omitted);

        let mut out = Vec::with_capacity((len + max_uncles) * 32);

        // 1. Base layer hashes
        for h in &layer[idx..idx + len] {
            out.extend_from_slice(h);
        }

        // 2. Uncle hashes for proof layers
        let mut cur_layer = base + k;
        let mut j = idx / len;
        let mut uncles_included = 0;

        while cur_layer < self.layers.len() && self.layers.len() > 1 && cur_layer < self.layers.len() - 1 && uncles_included < max_uncles {
            let uncle_idx = j ^ 1;
            if uncle_idx < self.layers[cur_layer].len() {
                out.extend_from_slice(&self.layers[cur_layer][uncle_idx]);
            } else {
                out.extend_from_slice(&[0u8; 32]);
            }
            j /= 2;
            cur_layer += 1;
            uncles_included += 1;
        }

        Some(out)
    }

    /// Verifies that a BEP 52 `hashes` payload correctly authenticates against the file's `expected_root`.
    ///
    /// Hashes the `length` base layer hashes up `log2(length)` levels and combines with subsequent
    /// uncle hashes to verify against `expected_root`.
    /// Returns the verified base layer hashes on success.
    pub fn verify_hashes_response(
        expected_root: &[u8; 32],
        _base_layer: u32,
        index: u32,
        length: u32,
        _proof_layers: u32,
        hashes_data: &[u8],
    ) -> Result<Vec<[u8; 32]>, BitTorrentError> {
        if length < 2 || (length & (length - 1)) != 0 {
            return Err(BitTorrentError::Parse("Length must be a power of two and >= 2".into()));
        }
        if index % length != 0 {
            return Err(BitTorrentError::Parse("Index must be a multiple of length".into()));
        }
        let len = length as usize;
        if hashes_data.len() < len * 32 || hashes_data.len() % 32 != 0 {
            return Err(BitTorrentError::Parse("Invalid hashes data byte length".into()));
        }

        let num_hashes = hashes_data.len() / 32;
        let mut base_hashes = Vec::with_capacity(len);
        for chunk in hashes_data[..len * 32].chunks_exact(32) {
            let mut h = [0u8; 32];
            h.copy_from_slice(chunk);
            base_hashes.push(h);
        }

        let mut uncle_hashes = Vec::with_capacity(num_hashes - len);
        for chunk in hashes_data[len * 32..].chunks_exact(32) {
            let mut h = [0u8; 32];
            h.copy_from_slice(chunk);
            uncle_hashes.push(h);
        }

        // Hash up the base hashes
        let mut cur_level = base_hashes.clone();
        while cur_level.len() > 1 {
            let mut next = Vec::with_capacity(cur_level.len() / 2);
            for pair in cur_level.chunks_exact(2) {
                let mut hasher = Sha256::new();
                hasher.update(&pair[0]);
                hasher.update(&pair[1]);
                next.push(hasher.finalize().into());
            }
            cur_level = next;
        }

        let mut sub_root: [u8; 32] = cur_level[0];
        let mut j = (index / length) as usize;

        for uncle in &uncle_hashes {
            let mut hasher = Sha256::new();
            if j % 2 == 0 {
                hasher.update(&sub_root);
                hasher.update(uncle);
            } else {
                hasher.update(uncle);
                hasher.update(&sub_root);
            }
            sub_root = hasher.finalize().into();
            j /= 2;
        }

        if sub_root != *expected_root {
            return Err(BitTorrentError::Parse(format!(
                "BEP 52 hashes verification failed: root mismatch (expected {:02x?}, got {:02x?})",
                expected_root, sub_root
            )));
        }

        Ok(base_hashes)
    }

    #[cfg(feature = "std")]
    /// Constructs a `MerkleTree` from a readable stream, computing 16 KiB block hashes incrementally
    /// without loading the entire file into memory at once.
    pub fn from_reader<R: std::io::Read>(reader: &mut R, file_length: u64) -> Result<Self, std::io::Error> {
        if file_length == 0 {
            return Ok(Self {
                file_length: 0,
                num_blocks: 0,
                layers: Vec::new(),
                root: [0u8; 32],
            });
        }

        let mut leaves = Vec::new();
        let mut buffer = vec![0u8; BLOCK_SIZE];
        let mut total_read = 0u64;

        loop {
            let mut chunk_len = 0;
            while chunk_len < BLOCK_SIZE {
                let n = reader.read(&mut buffer[chunk_len..])?;
                if n == 0 {
                    break;
                }
                chunk_len += n;
            }

            if chunk_len == 0 {
                break;
            }

            total_read += chunk_len as u64;
            let hash = Sha256::digest(&buffer[..chunk_len]);
            leaves.push(hash.into());

            if total_read >= file_length {
                break;
            }
        }

        Ok(Self::from_leaf_hashes(leaves, total_read))
    }

    #[cfg(feature = "std")]
    /// Constructs a `MerkleTree` directly from a file on disk.
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> Result<Self, std::io::Error> {
        let mut file = std::fs::File::open(path.as_ref())?;
        let len = file.metadata()?.len();
        Self::from_reader(&mut file, len)
    }
}

