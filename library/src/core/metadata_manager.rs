//! Metadata manager, BEP 9 extension fetcher, and BEP 52 v2 Merkle proof cache (SRP & ISP).
//!
//! Encapsulates torrent identity, tracker endpoints, WebSeed mirrors, magnet link
//! metadata piece assembly, and Merkle tree piece layers.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use sha1::Digest;

#[cfg(feature = "v2")]
use crate::core::merkle::{MerkleProof, MerkleTree};
use crate::network::metadata_fetcher::PartialMetadataManifest;
#[cfg(all(feature = "std", feature = "v2"))]
use crate::storage::piece_layers_cache::PieceLayersDiskCache;

/// Manages torrent metadata, magnet bootstrapping, tracker URLs, and Merkle layers.
pub struct MetadataManager {
    /// 20-byte torrent info hash (or truncated SHA256 root in pure v2).
    pub info_hash: Vec<u8>,
    /// Display file name or torrent root name.
    pub file_name: String,
    /// Primary announce tracker URL.
    pub tracker_url: String,
    /// Complete list of tracker tier URLs for announce redundancy.
    pub tracker_urls: Vec<String>,
    /// BEP 19 WebSeed HTTP/HTTPS source URLs.
    pub web_seeds: Vec<String>,
    /// True if the torrent info-dict contains `private = 1` (disabling DHT/PEX).
    pub is_private: bool,
    /// Total byte size of the raw metadata (info-dict) advertised via BEP 9 extension.
    pub metadata_size: Option<usize>,
    /// Assembled BEP 9 metadata pieces indexed by 0-based piece index.
    pub metadata_pieces: BTreeMap<u32, Vec<u8>>,
    /// Metadata pieces currently requested and the dispatch timestamp.
    pub requested_metadata_pieces: BTreeMap<u32, Instant>,
    /// Raw unparsed bencode info dictionary bytes once fully acquired.
    pub info_dict_bytes: Option<Vec<u8>>,
    /// Expedited partial metadata manifest parsed early from initial metadata pieces.
    pub expedited_manifest: Option<PartialMetadataManifest>,
    /// BitTorrent v2 Merkle trees per file index.
    #[cfg(feature = "v2")]
    pub file_merkle_trees: Mutex<HashMap<usize, MerkleTree>>,
    /// BitTorrent v2 audit paths / Merkle proofs per (file_index, piece_index).
    #[cfg(feature = "v2")]
    pub file_merkle_proofs: Mutex<HashMap<(usize, usize), MerkleProof>>,
    /// BitTorrent v2 piece layers indexed by root hash.
    #[cfg(feature = "v2")]
    pub piece_layers: Mutex<HashMap<[u8; 32], Vec<[u8; 32]>>>,
    /// Verified BitTorrent v2 hashes received from peers.
    #[cfg(feature = "v2")]
    pub verified_v2_hashes: Mutex<HashMap<([u8; 32], u32, u32), Vec<[u8; 32]>>>,
    /// On-disk and in-memory cache for BitTorrent v2 Merkle tree piece layers.
    #[cfg(all(feature = "std", feature = "v2"))]
    pub piece_layers_disk_cache: Arc<PieceLayersDiskCache>,
}

impl MetadataManager {
    /// Creates a new `MetadataManager` for a standard torrent file.
    pub fn new(
        info_hash: Vec<u8>,
        file_name: String,
        tracker_url: String,
        tracker_urls: Vec<String>,
        web_seeds: Vec<String>,
        is_private: bool,
        #[cfg(all(feature = "std", feature = "v2"))] piece_layers_disk_cache: Arc<
            PieceLayersDiskCache,
        >,
    ) -> Self {
        Self {
            info_hash,
            file_name,
            tracker_url,
            tracker_urls,
            web_seeds,
            is_private,
            metadata_size: None,
            metadata_pieces: BTreeMap::new(),
            requested_metadata_pieces: BTreeMap::new(),
            info_dict_bytes: None,
            expedited_manifest: None,
            #[cfg(feature = "v2")]
            file_merkle_trees: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            file_merkle_proofs: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            piece_layers: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            verified_v2_hashes: Mutex::new(HashMap::new()),
            #[cfg(all(feature = "std", feature = "v2"))]
            piece_layers_disk_cache,
        }
    }

    /// Creates an empty `MetadataManager` for initializing magnet downloads.
    pub fn empty(
        info_hash: Vec<u8>,
        #[cfg(all(feature = "std", feature = "v2"))] piece_layers_disk_cache: Arc<
            PieceLayersDiskCache,
        >,
    ) -> Self {
        Self {
            info_hash,
            file_name: String::new(),
            tracker_url: String::new(),
            tracker_urls: Vec::new(),
            web_seeds: Vec::new(),
            is_private: false,
            metadata_size: None,
            metadata_pieces: BTreeMap::new(),
            requested_metadata_pieces: BTreeMap::new(),
            info_dict_bytes: None,
            expedited_manifest: None,
            #[cfg(feature = "v2")]
            file_merkle_trees: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            file_merkle_proofs: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            piece_layers: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            verified_v2_hashes: Mutex::new(HashMap::new()),
            #[cfg(all(feature = "std", feature = "v2"))]
            piece_layers_disk_cache,
        }
    }

    /// Returns `true` if full torrent metadata has been acquired.
    pub fn has_metadata(&self) -> bool {
        self.info_dict_bytes.is_some()
    }

    /// Sets the advertised total metadata byte length from a BEP 9 extension handshake.
    pub fn set_metadata_size(&mut self, size: usize) {
        self.metadata_size = Some(size);
    }

    /// Adds a newly downloaded BEP 9 metadata piece.
    /// Returns `true` if this was a new piece.
    pub fn add_metadata_piece(&mut self, piece: u32, data: Vec<u8>) -> bool {
        self.requested_metadata_pieces.remove(&piece);
        self.metadata_pieces.insert(piece, data).is_none()
    }

    /// Checks if all pieces of metadata have been received and matches the info hash.
    pub fn assemble_metadata(&mut self) -> Option<Vec<u8>> {
        let total_size = self.metadata_size?;
        let expected_pieces = (total_size + 16383) / 16384;
        if self.metadata_pieces.len() < expected_pieces {
            return None;
        }

        let mut full_bytes = Vec::with_capacity(total_size);
        for piece_idx in 0..expected_pieces as u32 {
            if let Some(piece) = self.metadata_pieces.get(&piece_idx) {
                full_bytes.extend_from_slice(piece);
            } else {
                return None;
            }
        }
        full_bytes.truncate(total_size);

        // Verify SHA1 digest matches the info_hash
        let mut hasher = sha1::Sha1::new();
        hasher.update(&full_bytes);
        let actual_hash = hasher.finalize();
        if actual_hash.as_slice() == self.info_hash.as_slice() {
            self.info_dict_bytes = Some(full_bytes.clone());
            Some(full_bytes)
        } else {
            None
        }
    }

    /// Appends a new tracker URL if not already present. Returns `true` if newly added.
    pub fn add_tracker_url(&mut self, url: String) -> bool {
        if self.tracker_url.is_empty() {
            self.tracker_url = url.clone();
        }
        if !self.tracker_urls.contains(&url) {
            self.tracker_urls.push(url);
            true
        } else {
            false
        }
    }

    /// Appends a new WebSeed source mirror URL if not already present. Returns `true` if newly added.
    pub fn add_web_seed(&mut self, seed: String) -> bool {
        if !self.web_seeds.contains(&seed) {
            self.web_seeds.push(seed);
            true
        } else {
            false
        }
    }
}
