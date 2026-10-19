//! Persistent on-disk and in-memory cache for BitTorrent v2 Merkle tree piece layers
//! and intermediate authentication hashes (BEP 52).
//!
//! Provides high-throughput, low-latency serving of incoming peer wire `hash_request`
//! messages without keeping large tree structures permanently in RAM or re-hashing disk blocks.

use crate::error::BitTorrentError;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Magic header bytes identifying a binary BitTorrent v2 Layers cache file (`b"BT2L"`).
pub const BT2L_MAGIC: &[u8; 4] = b"BT2L";

/// Binary format version.
pub const BT2L_VERSION: u16 = 1;

/// Performance and operational telemetry metrics for `PieceLayersDiskCache`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PieceLayersCacheMetrics {
    /// Total number of cache hits served from memory.
    pub cache_hits: usize,
    /// Total number of memory cache misses requiring on-disk lookup.
    pub cache_misses: usize,
    /// Total number of `.layers` files loaded from disk.
    pub disk_reads: usize,
    /// Total number of `.layers` files saved to disk.
    pub disk_writes: usize,
    /// Total number of incoming BEP 52 `hash_request` messages successfully served.
    pub hash_requests_served: usize,
    /// Total bytes of Merkle hash payloads served to peers.
    pub bytes_served: u64,
    /// Number of distinct root hashes currently cached in memory.
    pub cached_roots_count: usize,
    /// Total count of hashes held in the in-memory tier.
    pub total_cached_hashes: usize,
    /// Total evictions from memory tier due to capacity limits.
    pub evictions: usize,
}

/// In-memory LRU cache state.
struct MemoryTier {
    layers_by_root: HashMap<[u8; 32], Vec<Vec<[u8; 32]>>>,
    lru_order: VecDeque<[u8; 32]>,
    total_hashes: usize,
    max_hashes: usize,
}

impl MemoryTier {
    fn new(max_hashes: usize) -> Self {
        Self {
            layers_by_root: HashMap::new(),
            lru_order: VecDeque::new(),
            total_hashes: 0,
            max_hashes,
        }
    }

    fn get(&mut self, root: &[u8; 32]) -> Option<&Vec<Vec<[u8; 32]>>> {
        if self.layers_by_root.contains_key(root) {
            // Move to front of LRU queue
            if let Some(pos) = self.lru_order.iter().position(|r| r == root) {
                self.lru_order.remove(pos);
            }
            self.lru_order.push_back(*root);
            self.layers_by_root.get(root)
        } else {
            None
        }
    }

    fn insert(&mut self, root: [u8; 32], layers: Vec<Vec<[u8; 32]>>) -> usize {
        let mut evicted = 0;
        let hashes_in_layers: usize = layers.iter().map(|l| l.len()).sum();

        // Evict oldest until within capacity
        while self.total_hashes + hashes_in_layers > self.max_hashes && !self.lru_order.is_empty() {
            if let Some(oldest_root) = self.lru_order.pop_front() {
                if let Some(old_layers) = self.layers_by_root.remove(&oldest_root) {
                    let old_count: usize = old_layers.iter().map(|l| l.len()).sum();
                    self.total_hashes = self.total_hashes.saturating_sub(old_count);
                    evicted += 1;
                }
            }
        }

        if let Some(old) = self.layers_by_root.insert(root, layers) {
            let old_count: usize = old.iter().map(|l| l.len()).sum();
            self.total_hashes = self.total_hashes.saturating_sub(old_count);
            if let Some(pos) = self.lru_order.iter().position(|r| r == &root) {
                self.lru_order.remove(pos);
            }
        }

        self.lru_order.push_back(root);
        self.total_hashes += hashes_in_layers;
        evicted
    }

    fn remove(&mut self, root: &[u8; 32]) -> bool {
        if let Some(pos) = self.lru_order.iter().position(|r| r == root) {
            self.lru_order.remove(pos);
        }
        if let Some(layers) = self.layers_by_root.remove(root) {
            let count: usize = layers.iter().map(|l| l.len()).sum();
            self.total_hashes = self.total_hashes.saturating_sub(count);
            true
        } else {
            false
        }
    }

    fn clear(&mut self) {
        self.layers_by_root.clear();
        self.lru_order.clear();
        self.total_hashes = 0;
    }
}

/// Persistent on-disk and in-memory cache for BitTorrent v2 Merkle tree piece layers.
pub struct PieceLayersDiskCache {
    cache_dir: PathBuf,
    memory_tier: Mutex<MemoryTier>,
    cache_hits: AtomicUsize,
    cache_misses: AtomicUsize,
    disk_reads: AtomicUsize,
    disk_writes: AtomicUsize,
    hash_requests_served: AtomicUsize,
    bytes_served: AtomicU64,
    evictions: AtomicUsize,
}

impl PieceLayersDiskCache {
    /// Creates a new `PieceLayersDiskCache` rooted at `cache_dir`.
    ///
    /// Automatically ensures the cache directory exists on disk.
    /// `max_memory_mb` configures the upper threshold of RAM hashes retained in memory.
    pub fn new(cache_dir: impl AsRef<Path>, max_memory_mb: usize) -> Result<Self, BitTorrentError> {
        let dir = cache_dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        // Each 32-byte hash takes ~32 bytes. 1 MB ~ 32,768 hashes.
        let max_hashes = (max_memory_mb.max(1) * 1024 * 1024) / 32;

        Ok(Self {
            cache_dir: dir,
            memory_tier: Mutex::new(MemoryTier::new(max_hashes)),
            cache_hits: AtomicUsize::new(0),
            cache_misses: AtomicUsize::new(0),
            disk_reads: AtomicUsize::new(0),
            disk_writes: AtomicUsize::new(0),
            hash_requests_served: AtomicUsize::new(0),
            bytes_served: AtomicU64::new(0),
            evictions: AtomicUsize::new(0),
        })
    }

    /// Returns the absolute path to the on-disk cache directory.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Computes the destination `.layers` file path for a given root hash.
    pub fn path_for_root(&self, pieces_root: &[u8; 32]) -> PathBuf {
        let mut hex = String::with_capacity(64);
        for b in pieces_root {
            hex.push_str(&format!("{:02x}", b));
        }
        self.cache_dir.join(format!("{}.layers", hex))
    }

    /// Returns `true` if layers for the specified `pieces_root` are cached in memory or on disk.
    pub fn has_layers(&self, pieces_root: &[u8; 32]) -> bool {
        {
            let mem = self.memory_tier.lock().unwrap();
            if mem.layers_by_root.contains_key(pieces_root) {
                return true;
            }
        }
        self.path_for_root(pieces_root).exists()
    }

    /// Removes cached layers for a given root from both the memory tier and disk.
    pub fn remove(&self, pieces_root: &[u8; 32]) -> bool {
        let mut removed = {
            let mut mem = self.memory_tier.lock().unwrap();
            mem.remove(pieces_root)
        };
        let path = self.path_for_root(pieces_root);
        if path.exists() {
            let _ = fs::remove_file(path);
            removed = true;
        }
        removed
    }

    /// Persists complete Merkle tree layers to the cache (both memory tier and disk file).
    pub fn save_layers(
        &self,
        pieces_root: &[u8; 32],
        layers: &[Vec<[u8; 32]>],
    ) -> Result<(), BitTorrentError> {
        if layers.is_empty() {
            return Ok(());
        }

        // 1. Update in-memory tier
        {
            let mut mem = self.memory_tier.lock().unwrap();
            let ev = mem.insert(*pieces_root, layers.to_vec());
            if ev > 0 {
                self.evictions.fetch_add(ev, Ordering::Relaxed);
            }
        }

        // 2. Encode to binary format
        let encoded = encode_layers(pieces_root, layers);

        // 3. Atomically write to disk via temporary file
        let target_path = self.path_for_root(pieces_root);
        let temp_path = target_path.with_extension("tmp");

        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temp_path)?;
            f.write_all(&encoded)?;
            f.flush()?;
        }

        fs::rename(&temp_path, &target_path)?;
        self.disk_writes.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Persists a single piece layer (as found in BEP 52 metainfo), reconstructing the upper tree layers
    /// and writing to both memory tier and disk.
    pub fn save_piece_layer(
        &self,
        pieces_root: &[u8; 32],
        piece_layer: &[[u8; 32]],
    ) -> Result<(), BitTorrentError> {
        let (reconstructed, calc_root) = reconstruct_tree_from_piece_layer(piece_layer);
        if &calc_root != pieces_root && !piece_layer.is_empty() {
            crate::log_debug!(
                "[piece_layers_cache] Notice: reconstructed root mismatch for piece layer: calc {:?}, expected {:?}",
                calc_root,
                pieces_root
            );
        }
        self.save_layers(pieces_root, &reconstructed)
    }

    /// Retrieves complete Merkle tree layers for a given root hash.
    ///
    /// Checks the memory tier first. On miss, reads and verifies the on-disk file,
    /// populating the memory tier for subsequent requests.
    pub fn load_layers(
        &self,
        pieces_root: &[u8; 32],
    ) -> Result<Option<Vec<Vec<[u8; 32]>>>, BitTorrentError> {
        // 1. Check memory tier
        {
            let mut mem = self.memory_tier.lock().unwrap();
            if let Some(layers) = mem.get(pieces_root) {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(layers.clone()));
            }
        }

        self.cache_misses.fetch_add(1, Ordering::Relaxed);

        // 2. Check disk file
        let path = self.path_for_root(pieces_root);
        if !path.exists() {
            return Ok(None);
        }

        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        let (root, layers) = decode_layers(&data)?;
        if &root != pieces_root {
            return Err(BitTorrentError::Parse(
                "Cached layer file root does not match requested root".into(),
            ));
        }

        self.disk_reads.fetch_add(1, Ordering::Relaxed);

        // Populate memory tier
        {
            let mut mem = self.memory_tier.lock().unwrap();
            let ev = mem.insert(*pieces_root, layers.clone());
            if ev > 0 {
                self.evictions.fetch_add(ev, Ordering::Relaxed);
            }
        }

        Ok(Some(layers))
    }

    /// Retrieves only the piece layer (base leaf hashes) for a given root hash.
    pub fn load_piece_layer(
        &self,
        pieces_root: &[u8; 32],
    ) -> Result<Option<Vec<[u8; 32]>>, BitTorrentError> {
        let layers_opt = self.load_layers(pieces_root)?;
        Ok(layers_opt.and_then(|mut l| if !l.is_empty() { Some(l.remove(0)) } else { None }))
    }

    /// Answers a BEP 52 `hash_request` from cached layers.
    ///
    /// Generates `length` base layer hashes along with necessary ancestor proof hashes,
    /// omitting internal subtree proof layers per BEP 52 specifications.
    pub fn serve_hash_request(
        &self,
        pieces_root: &[u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    ) -> Option<Vec<u8>> {
        let layers = match self.load_layers(pieces_root) {
            Ok(Some(l)) => l,
            _ => return None,
        };

        if let Some(resp) = generate_hashes_response_from_layers(
            &layers,
            base_layer,
            index,
            length,
            proof_layers,
        ) {
            self.hash_requests_served.fetch_add(1, Ordering::Relaxed);
            self.bytes_served.fetch_add(resp.len() as u64, Ordering::Relaxed);
            Some(resp)
        } else {
            None
        }
    }

    /// Prunes on-disk cache files if exceeding `max_files` count or `max_bytes` total size.
    /// Returns the number of pruned files.
    pub fn prune(&self, max_files: usize, max_bytes: u64) -> Result<usize, BitTorrentError> {
        if !self.cache_dir.exists() {
            return Ok(0);
        }

        let mut entries = Vec::new();
        let mut total_size = 0u64;

        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "layers") {
                let metadata = entry.metadata()?;
                let size = metadata.len();
                let modified = metadata.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                total_size += size;
                entries.push((path, size, modified));
            }
        }

        // Sort oldest first
        entries.sort_by_key(|e| e.2);

        let mut pruned = 0;
        let total_count = entries.len();
        for (path, size, _) in &entries {
            if total_count - pruned > max_files || total_size > max_bytes {
                let _ = fs::remove_file(path);
                total_size = total_size.saturating_sub(*size);
                pruned += 1;
            } else {
                break;
            }
        }

        Ok(pruned)
    }

    /// Clears both memory tier and all on-disk cache files.
    pub fn clear(&self) -> Result<(), BitTorrentError> {
        {
            let mut mem = self.memory_tier.lock().unwrap();
            mem.clear();
        }

        if self.cache_dir.exists() {
            for entry in fs::read_dir(&self.cache_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().map_or(false, |ext| ext == "layers") {
                    let _ = fs::remove_file(path);
                }
            }
        }

        Ok(())
    }

    /// Takes a snapshot of current telemetry metrics.
    pub fn metrics(&self) -> PieceLayersCacheMetrics {
        let (cached_roots_count, total_cached_hashes) = {
            let mem = self.memory_tier.lock().unwrap();
            (mem.layers_by_root.len(), mem.total_hashes)
        };

        PieceLayersCacheMetrics {
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            disk_reads: self.disk_reads.load(Ordering::Relaxed),
            disk_writes: self.disk_writes.load(Ordering::Relaxed),
            hash_requests_served: self.hash_requests_served.load(Ordering::Relaxed),
            bytes_served: self.bytes_served.load(Ordering::Relaxed),
            cached_roots_count,
            total_cached_hashes,
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }
}

/// Encodes Merkle tree layers into standard binary `.layers` format with integrity checksum.
pub fn encode_layers(pieces_root: &[u8; 32], layers: &[Vec<[u8; 32]>]) -> Vec<u8> {
    let mut total_hashes = 0u64;
    for l in layers {
        total_hashes += l.len() as u64;
    }

    // Estimate capacity: Header (52) + Layer headers (8 * len) + Hashes (32 * count) + Checksum (32)
    let est_cap = 52 + (layers.len() * 8) + (total_hashes as usize * 32) + 32;
    let mut buf = Vec::with_capacity(est_cap);

    // 1. Header (52 bytes)
    buf.extend_from_slice(BT2L_MAGIC);
    buf.extend_from_slice(&BT2L_VERSION.to_be_bytes());
    buf.extend_from_slice(&[0u8; 2]); // reserved flags
    buf.extend_from_slice(pieces_root);
    buf.extend_from_slice(&(layers.len() as u32).to_be_bytes());
    buf.extend_from_slice(&total_hashes.to_be_bytes());

    // 2. Layer bodies
    for (layer_idx, layer) in layers.iter().enumerate() {
        buf.extend_from_slice(&(layer_idx as u32).to_be_bytes());
        buf.extend_from_slice(&(layer.len() as u32).to_be_bytes());
        for h in layer {
            buf.extend_from_slice(h);
        }
    }

    // 3. Trailing SHA-256 Checksum (32 bytes)
    let checksum = Sha256::digest(&buf);
    buf.extend_from_slice(&checksum);

    buf
}

/// Decodes binary `.layers` file data, validating magic, version, and SHA-256 checksum.
pub fn decode_layers(data: &[u8]) -> Result<([u8; 32], Vec<Vec<[u8; 32]>>), BitTorrentError> {
    if data.len() < 52 + 32 {
        return Err(BitTorrentError::Parse(
            "Layer cache payload smaller than minimum header size".into(),
        ));
    }

    // Verify trailing checksum
    let payload_len = data.len() - 32;
    let expected_checksum = &data[payload_len..];
    let actual_checksum = Sha256::digest(&data[..payload_len]);
    if expected_checksum != actual_checksum.as_slice() {
        return Err(BitTorrentError::Parse(
            "Layer cache file integrity checksum mismatch".into(),
        ));
    }

    // Header validation
    if &data[0..4] != BT2L_MAGIC {
        return Err(BitTorrentError::Parse(
            "Invalid BT2L layer cache magic header".into(),
        ));
    }

    let version = u16::from_be_bytes([data[4], data[5]]);
    if version != BT2L_VERSION {
        return Err(BitTorrentError::Parse(format!(
            "Unsupported BT2L layer cache version: {}",
            version
        )));
    }

    let mut pieces_root = [0u8; 32];
    pieces_root.copy_from_slice(&data[8..40]);

    let num_layers = u32::from_be_bytes([data[40], data[41], data[42], data[43]]) as usize;
    let _total_hashes = u64::from_be_bytes([
        data[44], data[45], data[46], data[47], data[48], data[49], data[50], data[51],
    ]);

    let mut layers = Vec::with_capacity(num_layers);
    let mut offset = 52;

    for _ in 0..num_layers {
        if offset + 8 > payload_len {
            return Err(BitTorrentError::Parse("Unexpected EOF reading layer header".into()));
        }
        let _layer_idx = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
        offset += 4;
        let hash_count = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;

        let bytes_needed = hash_count * 32;
        if offset + bytes_needed > payload_len {
            return Err(BitTorrentError::Parse("Unexpected EOF reading layer hashes".into()));
        }

        let mut layer = Vec::with_capacity(hash_count);
        for _ in 0..hash_count {
            let mut h = [0u8; 32];
            h.copy_from_slice(&data[offset..offset + 32]);
            layer.push(h);
            offset += 32;
        }
        layers.push(layer);
    }

    Ok((pieces_root, layers))
}

/// Reconstructs all upper layers of a Merkle tree from a piece layer.
///
/// Pads the piece layer to a power of two with zero hashes, then pairwise computes parent
/// hashes `sha256(left || right)` until reaching a 1-hash root.
/// Returns `(layers, calculated_root)`.
pub fn reconstruct_tree_from_piece_layer(
    piece_layer: &[[u8; 32]],
) -> (Vec<Vec<[u8; 32]>>, [u8; 32]) {
    if piece_layer.is_empty() {
        return (Vec::new(), [0u8; 32]);
    }

    if piece_layer.len() == 1 {
        let root = piece_layer[0];
        return (vec![piece_layer.to_vec()], root);
    }

    let power_of_two = piece_layer.len().next_power_of_two();
    let mut base_layer = piece_layer.to_vec();
    while base_layer.len() < power_of_two {
        base_layer.push([0u8; 32]);
    }

    let mut layers = Vec::new();
    layers.push(base_layer);

    while layers.last().unwrap().len() > 1 {
        let prev = layers.last().unwrap();
        let mut next = Vec::with_capacity(prev.len() / 2);
        for pair in prev.chunks_exact(2) {
            let mut hasher = Sha256::new();
            hasher.update(&pair[0]);
            hasher.update(&pair[1]);
            next.push(hasher.finalize().into());
        }
        layers.push(next);
    }

    let root = layers.last().unwrap()[0];
    (layers, root)
}

/// Generates a BEP 52 hashes binary payload from pre-cached layers.
///
/// Complies with BEP 52:
/// - `length` must be a power of two >= 2.
/// - `index` must be an integer multiple of `length`.
/// - The first `log2(length) - 1` proof layers are omitted.
pub fn generate_hashes_response_from_layers(
    layers: &[Vec<[u8; 32]>],
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
    if base >= layers.len() {
        return None;
    }

    let layer = &layers[base];
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

    // 2. Ancestor proof uncles
    let mut cur_layer = base + k;
    let mut j = idx / len;
    let mut uncles_included = 0;

    while cur_layer < layers.len() && layers.len() > 1 && cur_layer < layers.len() - 1 && uncles_included < max_uncles {
        let uncle_idx = j ^ 1;
        if uncle_idx < layers[cur_layer].len() {
            out.extend_from_slice(&layers[cur_layer][uncle_idx]);
        } else {
            out.extend_from_slice(&[0u8; 32]);
        }
        j /= 2;
        cur_layer += 1;
        uncles_included += 1;
    }

    Some(out)
}
