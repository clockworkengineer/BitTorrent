//! Range-Based Metadata Fetching & Expedited Partial File Metadata Resolution (BEP 9 & BEP 53)
//!
//! Provides multi-peer metadata range pipelining (`MetadataRangeFetcher`) and expedited
//! early parsing of partial torrent file manifests (`PartialMetadataManifest`) directly
//! from initial metadata blocks without waiting for whole torrent hash trees.

use crate::error::BitTorrentError;
use crate::utils::bencode::{Bencode, BNode};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

/// Canonical metadata block size defined by BEP 9 (16 KiB).
pub const METADATA_PIECE_SIZE: usize = 16384;

/// Maximum allowable metadata size (50 MiB) to protect against memory exhaustion.
pub const MAX_METADATA_SIZE: usize = 50 * 1024 * 1024;

/// Information about a single file extracted from the torrent metadata dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialFileInfo {
    /// 0-based index of this file in the torrent file list.
    pub index: usize,
    /// Relative path / file name within the torrent directory structure.
    pub name: String,
    /// Length of the file in bytes.
    pub length: u64,
    /// Inclusive range of pieces overlapping this file: `(start_piece, end_piece)`.
    pub piece_range: (u32, u32),
}

/// Evaluation report comparing BEP 53 selected files against the complete torrent.
#[derive(Debug, Clone, PartialEq)]
pub struct BEP53SelectionEvaluation {
    /// 0-based file indices selected for download.
    pub selected_indices: Vec<usize>,
    /// Files matching the selection indices.
    pub selected_files: Vec<PartialFileInfo>,
    /// Files skipped by the selection indices.
    pub unselected_files: Vec<PartialFileInfo>,
    /// Aggregate byte size of selected files.
    pub selected_bytes: u64,
    /// Aggregate byte size of all files in the torrent.
    pub total_bytes: u64,
    /// Bandwidth/storage savings percentage (0.0 to 100.0).
    pub byte_savings_percent: f64,
    /// Number of distinct pieces required to download selected files.
    pub selected_piece_count: usize,
    /// Total piece count of the complete torrent.
    pub total_piece_count: usize,
    /// Piece download reduction percentage (0.0 to 100.0).
    pub piece_savings_percent: f64,
    /// Specific piece indices required to reconstruct the selected files.
    pub overlapping_pieces: Vec<u32>,
}

/// Expedited partial torrent metadata manifest parsed early from initial metadata pieces.
#[derive(Debug, Clone, PartialEq)]
pub struct PartialMetadataManifest {
    /// Torrent display name.
    pub name: String,
    /// Piece length in bytes.
    pub piece_length: u64,
    /// Total torrent payload size in bytes.
    pub total_bytes: u64,
    /// Total calculated piece count.
    pub total_pieces: u32,
    /// Complete list of files parsed from the metadata dictionary.
    pub files: Vec<PartialFileInfo>,
    /// True if the metadata dictionary contained all files and lengths.
    pub has_complete_file_list: bool,
}

impl PartialMetadataManifest {
    /// Attempts to parse file entries and torrent structure from raw metadata bytes.
    ///
    /// Even if trailing piece hashes (`pieces`) are truncated or absent in partial
    /// downloads, this extracts the file manifest as soon as the initial dictionary
    /// keys are present.
    pub fn try_parse_partial(data: &[u8]) -> Option<Self> {
        if data.is_empty() || data[0] != b'd' {
            return None;
        }

        // Attempt full bencode decode first
        if let Ok(node) = Bencode::decode(data) {
            if let Some(manifest) = Self::from_bnode(&node) {
                return Some(manifest);
            }
        }

        // Fallback: Scan top-level keys up to the truncated pieces string
        Self::scan_partial_dictionary(data)
    }

    /// Extracts manifest details from a parsed BNode dictionary.
    fn from_bnode(node: &BNode<'_>) -> Option<Self> {
        let name = node.dict_get(b"name")
            .and_then(|n| n.as_string())
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_else(|| "unnamed".to_string());

        let piece_length = node.dict_get(b"piece length")
            .and_then(|n| n.as_number_bytes())
            .and_then(|b| core::str::from_utf8(b).ok())
            .and_then(|s| s.parse::<u64>().ok())?;

        if piece_length == 0 {
            return None;
        }

        let mut files = Vec::new();
        let mut total_bytes = 0u64;

        if let Some(files_node) = node.dict_get(b"files") {
            if let BNode::List(file_list) = files_node {
                for (idx, file_node) in file_list.iter().enumerate() {
                    let len = file_node.dict_get(b"length")
                        .and_then(|n| n.as_number_bytes())
                        .and_then(|b| core::str::from_utf8(b).ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);

                    let mut path_parts = Vec::new();
                    if let Some(path_node) = file_node.dict_get(b"path") {
                        if let BNode::List(parts) = path_node {
                            for part in parts {
                                if let Some(bytes) = part.as_string() {
                                    path_parts.push(String::from_utf8_lossy(bytes).to_string());
                                }
                            }
                        }
                    }
                    let file_name = if path_parts.is_empty() {
                        format!("file_{}", idx)
                    } else {
                        path_parts.join("/")
                    };

                    let start_piece = (total_bytes / piece_length) as u32;
                    let end_piece = if len == 0 {
                        start_piece
                    } else {
                        ((total_bytes + len - 1) / piece_length) as u32
                    };

                    files.push(PartialFileInfo {
                        index: idx,
                        name: file_name,
                        length: len,
                        piece_range: (start_piece, end_piece),
                    });

                    total_bytes += len;
                }
            }
        } else if let Some(length_node) = node.dict_get(b"length") {
            // Single-file torrent
            let len = length_node.as_number_bytes()
                .and_then(|b| core::str::from_utf8(b).ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            total_bytes = len;
            let end_piece = if len == 0 { 0 } else { ((len - 1) / piece_length) as u32 };
            files.push(PartialFileInfo {
                index: 0,
                name: name.clone(),
                length: len,
                piece_range: (0, end_piece),
            });
        }

        let total_pieces = if total_bytes == 0 {
            0
        } else {
            ((total_bytes + piece_length - 1) / piece_length) as u32
        };

        Some(PartialMetadataManifest {
            name,
            piece_length,
            total_bytes,
            total_pieces,
            files,
            has_complete_file_list: true,
        })
    }

    /// Scans top-level dictionary entries until a truncated or invalid element is hit.
    fn scan_partial_dictionary(data: &[u8]) -> Option<Self> {
        let mut pos = 1; // skip leading 'd'
        let mut entries = Vec::new();

        while pos < data.len() && data[pos] != b'e' {
            // Parse string key: `<len>:<key>`
            let colon_idx = data[pos..].iter().position(|&b| b == b':')?;
            let len_str = core::str::from_utf8(&data[pos..pos + colon_idx]).ok()?;
            let key_len = len_str.parse::<usize>().ok()?;
            let key_start = pos + colon_idx + 1;
            let key_end = key_start + key_len;
            if key_end > data.len() {
                break;
            }
            let key_slice = &data[key_start..key_end];
            pos = key_end;

            // Attempt to decode value
            match Bencode::decode_partial(&data[pos..]) {
                Ok((node, consumed)) => {
                    entries.push((key_slice, node));
                    pos += consumed;
                }
                Err(_) => {
                    // Truncated value (e.g. incomplete trailing piece hashes string)
                    break;
                }
            }
        }

        if entries.is_empty() {
            return None;
        }

        let dict_node = BNode::Dictionary(entries);
        Self::from_bnode(&dict_node)
    }

    /// Evaluates a BEP 53 `so=` file selection against this manifest.
    pub fn evaluate_bep53_selection(&self, select_only: &[usize]) -> BEP53SelectionEvaluation {
        let mut selected_files = Vec::new();
        let mut unselected_files = Vec::new();
        let mut selected_bytes = 0u64;
        let mut overlapping_pieces_set = BTreeSet::new();

        let use_all = select_only.is_empty();

        for file in &self.files {
            let is_selected = use_all || select_only.contains(&file.index);
            if is_selected {
                selected_files.push(file.clone());
                selected_bytes += file.length;
                for p in file.piece_range.0..=file.piece_range.1 {
                    overlapping_pieces_set.insert(p);
                }
            } else {
                unselected_files.push(file.clone());
            }
        }

        let overlapping_pieces: Vec<u32> = overlapping_pieces_set.into_iter().collect();
        let selected_piece_count = overlapping_pieces.len();
        let total_piece_count = self.total_pieces as usize;

        let byte_savings_percent = if self.total_bytes > 0 {
            (1.0 - (selected_bytes as f64 / self.total_bytes as f64)) * 100.0
        } else {
            0.0
        };

        let piece_savings_percent = if total_piece_count > 0 {
            (1.0 - (selected_piece_count as f64 / total_piece_count as f64)) * 100.0
        } else {
            0.0
        };

        let mut sorted_indices = if use_all {
            self.files.iter().map(|f| f.index).collect()
        } else {
            select_only.to_vec()
        };
        sorted_indices.sort_unstable();
        sorted_indices.dedup();

        BEP53SelectionEvaluation {
            selected_indices: sorted_indices,
            selected_files,
            unselected_files,
            selected_bytes,
            total_bytes: self.total_bytes,
            byte_savings_percent: byte_savings_percent.max(0.0),
            selected_piece_count,
            total_piece_count,
            piece_savings_percent: piece_savings_percent.max(0.0),
            overlapping_pieces,
        }
    }
}

/// Telemetry metrics for the metadata fetcher pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataFetchMetrics {
    /// Total metadata size in bytes (if discovered).
    pub total_size: usize,
    /// Total number of 16 KiB metadata pieces.
    pub total_pieces: u32,
    /// Number of metadata pieces successfully received.
    pub pieces_received: u32,
    /// Number of metadata pieces currently in-flight across peers.
    pub pieces_in_flight: u32,
    /// Number of distinct peers that contributed metadata pieces.
    pub active_peers: usize,
    /// True if all metadata pieces have been received and verified.
    pub is_complete: bool,
    /// Milliseconds elapsed since fetcher initialization.
    pub elapsed_millis: u64,
    /// True if an early partial manifest has been resolved.
    pub expedited_manifest_resolved: bool,
}

/// Multi-peer metadata range pipelining and piece tracking engine (BEP 9 / BEP 53).
pub struct MetadataRangeFetcher {
    metadata_size: Option<usize>,
    total_pieces: Option<u32>,
    pieces: BTreeMap<u32, Vec<u8>>,
    in_flight: HashMap<u32, (Instant, String)>,
    peer_in_flight_count: HashMap<String, usize>,
    contributing_peers: BTreeSet<String>,
    failed_attempts: HashMap<u32, usize>,
    request_timeout: Duration,
    max_pipeline_depth: usize,
    start_time: Instant,
    expedited_manifest: Option<PartialMetadataManifest>,
    priority_pieces: Vec<u32>,
}

impl Default for MetadataRangeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl MetadataRangeFetcher {
    /// Creates a new `MetadataRangeFetcher` with default timeouts and depth.
    pub fn new() -> Self {
        Self {
            metadata_size: None,
            total_pieces: None,
            pieces: BTreeMap::new(),
            in_flight: HashMap::new(),
            peer_in_flight_count: HashMap::new(),
            contributing_peers: BTreeSet::new(),
            failed_attempts: HashMap::new(),
            request_timeout: Duration::from_secs(5),
            max_pipeline_depth: 4,
            start_time: Instant::now(),
            expedited_manifest: None,
            priority_pieces: vec![0], // Prioritize piece 0 for early manifest resolution
        }
    }

    /// Sets the request timeout duration before an in-flight piece can be re-dispatched.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Sets the maximum concurrent pipelined requests dispatched to a single peer.
    pub fn with_pipeline_depth(mut self, depth: usize) -> Self {
        self.max_pipeline_depth = depth.max(1);
        self
    }

    /// Explicitly sets or updates the expected metadata size in bytes.
    pub fn set_metadata_size(&mut self, size: usize) {
        if size > MAX_METADATA_SIZE {
            return;
        }
        self.metadata_size = Some(size);
        let num_pieces = (size + METADATA_PIECE_SIZE - 1) / METADATA_PIECE_SIZE;
        self.total_pieces = Some(num_pieces as u32);
    }

    /// Returns the total metadata size if known.
    pub fn metadata_size(&self) -> Option<usize> {
        self.metadata_size
    }

    /// Returns the total number of 16 KiB pieces if known.
    pub fn total_pieces(&self) -> Option<u32> {
        self.total_pieces
    }

    /// Returns the number of downloaded metadata pieces.
    pub fn pieces_count(&self) -> usize {
        self.pieces.len()
    }

    /// Returns true if all metadata pieces have been collected.
    pub fn is_complete(&self) -> bool {
        match self.total_pieces {
            Some(total) => self.pieces.len() == total as usize && total > 0,
            None => false,
        }
    }

    /// Returns the expedited partial manifest if one has been resolved.
    pub fn expedited_manifest(&self) -> Option<&PartialMetadataManifest> {
        self.expedited_manifest.as_ref()
    }

    /// Selects up to `limit` un-fetched metadata piece indices to request from the given peer.
    ///
    /// Respects `max_pipeline_depth` per peer, prioritizes piece 0 for expedited manifest
    /// discovery, and recycles pieces whose in-flight timer exceeded `request_timeout`.
    pub fn next_requests_for_peer(&mut self, peer_id: &str, limit: usize) -> Vec<u32> {
        let total = match self.total_pieces {
            Some(t) => t,
            None => return Vec::new(),
        };

        let current_peer_depth = self.peer_in_flight_count.get(peer_id).copied().unwrap_or(0);
        let allowed = self.max_pipeline_depth.saturating_sub(current_peer_depth).min(limit);
        if allowed == 0 {
            return Vec::new();
        }

        let now = Instant::now();
        let timeout = self.request_timeout;
        let mut candidates = Vec::with_capacity(allowed);

        // Check priority pieces first (piece 0)
        for &p in &self.priority_pieces {
            if p < total && !self.pieces.contains_key(&p) {
                let can_request = match self.in_flight.get(&p) {
                    None => true,
                    Some((requested_at, _)) => requested_at.elapsed() >= timeout,
                };
                if can_request && !candidates.contains(&p) {
                    candidates.push(p);
                    if candidates.len() == allowed {
                        break;
                    }
                }
            }
        }

        // Fill remaining pipeline slots sequentially
        if candidates.len() < allowed {
            for p in 0..total {
                if !self.pieces.contains_key(&p) && !candidates.contains(&p) {
                    let can_request = match self.in_flight.get(&p) {
                        None => true,
                        Some((requested_at, _)) => requested_at.elapsed() >= timeout,
                    };
                    if can_request {
                        candidates.push(p);
                        if candidates.len() == allowed {
                            break;
                        }
                    }
                }
            }
        }

        // Record in-flight state
        for &p in &candidates {
            if let Some((_, old_peer)) = self.in_flight.insert(p, (now, peer_id.to_string())) {
                if old_peer != peer_id {
                    if let Some(cnt) = self.peer_in_flight_count.get_mut(&old_peer) {
                        *cnt = cnt.saturating_sub(1);
                    }
                }
            }
            *self.peer_in_flight_count.entry(peer_id.to_string()).or_insert(0) += 1;
        }

        candidates
    }

    /// Handles an incoming metadata data piece (`msg_type = 1`).
    ///
    /// Stores the payload, updates peer counters, triggers early manifest resolution
    /// when piece 0 or consecutive initial pieces arrive, and returns `Ok(true)` if
    /// the full metadata buffer is complete.
    pub fn handle_data_piece(
        &mut self,
        piece: u32,
        total_size: Option<usize>,
        data: &[u8],
        peer_id: &str,
    ) -> Result<bool, BitTorrentError> {
        if let Some(size) = total_size {
            if self.metadata_size.is_none() {
                self.set_metadata_size(size);
            }
        }

        if let Some(total) = self.total_pieces {
            if piece >= total {
                return Err(BitTorrentError::Parse(format!(
                    "Metadata piece index {} out of range (total: {})",
                    piece, total
                )));
            }
        }

        if data.len() > METADATA_PIECE_SIZE {
            return Err(BitTorrentError::Parse(format!(
                "Metadata piece length {} exceeds maximum piece size {}",
                data.len(), METADATA_PIECE_SIZE
            )));
        }

        // Release in-flight tracking
        if let Some((_, assigned_peer)) = self.in_flight.remove(&piece) {
            if let Some(cnt) = self.peer_in_flight_count.get_mut(&assigned_peer) {
                *cnt = cnt.saturating_sub(1);
            }
        }

        self.pieces.insert(piece, data.to_vec());
        self.contributing_peers.insert(peer_id.to_string());

        // Attempt early manifest resolution if not yet resolved
        if self.expedited_manifest.is_none() {
            let mut prefix = Vec::new();
            let mut p = 0;
            while let Some(chunk) = self.pieces.get(&p) {
                prefix.extend_from_slice(chunk);
                p += 1;
            }
            if !prefix.is_empty() {
                if let Some(manifest) = PartialMetadataManifest::try_parse_partial(&prefix) {
                    self.expedited_manifest = Some(manifest);
                }
            }
        }

        Ok(self.is_complete())
    }

    /// Handles a metadata rejection (`msg_type = 2`) from a peer.
    pub fn handle_reject(&mut self, piece: u32, peer_id: &str) {
        if let Some((_, assigned_peer)) = self.in_flight.remove(&piece) {
            if let Some(cnt) = self.peer_in_flight_count.get_mut(&assigned_peer) {
                *cnt = cnt.saturating_sub(1);
            }
        }
        *self.failed_attempts.entry(piece).or_insert(0) += 1;
        let _ = peer_id;
    }

    /// Assembles all received metadata pieces into a single contiguous buffer.
    pub fn assemble(&self) -> Option<Vec<u8>> {
        let total = self.total_pieces?;
        let size = self.metadata_size?;
        if self.pieces.len() != total as usize {
            return None;
        }

        let mut assembled = Vec::with_capacity(size);
        for p in 0..total {
            let chunk = self.pieces.get(&p)?;
            assembled.extend_from_slice(chunk);
        }
        assembled.truncate(size);
        Some(assembled)
    }

    /// Assembles the complete metadata buffer and validates it against the expected info-hash.
    /// Supports 20-byte SHA-1 info-hashes (v1/hybrid) and 32-byte SHA-256 info-hashes (v2).
    pub fn verify_and_assemble(&self, expected_info_hash: &[u8]) -> Result<Vec<u8>, BitTorrentError> {
        let assembled = self.assemble().ok_or_else(|| {
            BitTorrentError::Parse("Metadata download incomplete, cannot assemble".into())
        })?;

        if expected_info_hash.len() == 20 {
            use sha1::Digest;
            let mut hasher = sha1::Sha1::new();
            hasher.update(&assembled);
            let calculated = hasher.finalize().to_vec();
            if calculated != expected_info_hash {
                return Err(BitTorrentError::Parse(format!(
                    "Metadata SHA-1 hash mismatch: expected {:?}, got {:?}",
                    expected_info_hash, calculated
                )));
            }
        } else if expected_info_hash.len() == 32 {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(&assembled);
            let calculated = hasher.finalize().to_vec();
            if calculated != expected_info_hash {
                return Err(BitTorrentError::Parse(format!(
                    "Metadata SHA-256 hash mismatch: expected {:?}, got {:?}",
                    expected_info_hash, calculated
                )));
            }
        } else {
            return Err(BitTorrentError::Parse("Unrecognized info hash length for metadata verification".into()));
        }

        Ok(assembled)
    }

    /// Returns current telemetry metrics for this fetcher.
    pub fn metrics(&self) -> MetadataFetchMetrics {
        MetadataFetchMetrics {
            total_size: self.metadata_size.unwrap_or(0),
            total_pieces: self.total_pieces.unwrap_or(0),
            pieces_received: self.pieces.len() as u32,
            pieces_in_flight: self.in_flight.len() as u32,
            active_peers: self.contributing_peers.len(),
            is_complete: self.is_complete(),
            elapsed_millis: self.start_time.elapsed().as_millis() as u64,
            expedited_manifest_resolved: self.expedited_manifest.is_some(),
        }
    }
}
