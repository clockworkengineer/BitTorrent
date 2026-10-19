//! Fast Resume Data Serialization (BEP 47 & libtorrent fastresume format)
//!
//! Provides structures and functions for serializing, deserializing, validating,
//! and restoring torrent transfer sessions without requiring full re-hashing of
//! multi-gigabyte files from disk on startup.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::UNIX_EPOCH;

use crate::core::metainfo::FilePriority;
use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::error::BitTorrentError;
use crate::utils::bencode::{BNode, Bencode};

/// Metadata for an individual file stored within a fast-resume entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastResumeFileEntry {
    /// Relative or absolute path of the file.
    pub path: PathBuf,
    /// Expected file size in bytes.
    pub length: u64,
    /// Last modified timestamp (seconds since Unix epoch), or 0 if unknown.
    pub mtime: u64,
    /// Priority level for selective download (0=Skip, 1=Low, 4=Normal, 7=High).
    pub priority: u8,
}

/// Resume state adhering to BEP 47 / libtorrent fastresume conventions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastResumeData {
    /// 20-byte SHA-1 info-hash identifying the torrent.
    pub info_hash: [u8; 20],
    /// Directory path where downloaded files are saved.
    pub save_path: PathBuf,
    /// Total number of pieces in the torrent.
    pub num_pieces: usize,
    /// Size of each piece in bytes.
    pub piece_length: u32,
    /// Packed bitfield representing possessed pieces.
    pub bitfield: Vec<u8>,
    /// Total bytes uploaded so far.
    pub total_uploaded: u64,
    /// Total bytes downloaded so far.
    pub total_downloaded: u64,
    /// Total duration in seconds spent active.
    pub active_time: u64,
    /// Total duration in seconds spent seeding.
    pub seeding_time: u64,
    /// Whether the torrent is in seeding mode.
    pub is_seeding: bool,
    /// File metadata list for files included in this torrent.
    pub files: Vec<FastResumeFileEntry>,
    /// Known peer addresses ("ip:port") to quickly resume connections.
    pub peers: Vec<String>,
}

impl FastResumeData {
    /// Constructs a `FastResumeData` snapshot from an active `TorrentContext` and save directory.
    pub fn from_context(ctx: &TorrentContext, base_path: &Path) -> Result<Self, BitTorrentError> {
        let mut info_hash = [0u8; 20];
        if ctx.info_hash.len() >= 20 {
            info_hash.copy_from_slice(&ctx.info_hash[..20]);
        }

        let mut files = Vec::new();
        for file in &ctx.files_to_download {
            let file_path = PathBuf::from(&file.torrent_path);
            let full_path = if Path::new(&file.name).exists() {
                PathBuf::from(&file.name)
            } else {
                base_path.join(&file.torrent_path)
            };

            let mtime = if let Ok(meta) = std::fs::metadata(&full_path) {
                meta.modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            } else {
                0
            };

            files.push(FastResumeFileEntry {
                path: file_path,
                length: file.length,
                mtime,
                priority: file.priority as u8,
            });
        }

        let peers = if let Ok(swarm) = ctx.peer_swarm.read() {
            swarm.keys().cloned().collect()
        } else {
            Vec::new()
        };

        Ok(FastResumeData {
            info_hash,
            save_path: base_path.to_path_buf(),
            num_pieces: ctx.number_of_pieces,
            piece_length: ctx.piece_length,
            bitfield: ctx.bitfield.clone(),
            total_uploaded: ctx.total_bytes_uploaded.load(Ordering::Relaxed),
            total_downloaded: ctx.total_bytes_downloaded.load(Ordering::Relaxed),
            active_time: 0,
            seeding_time: 0,
            is_seeding: ctx.status == TorrentStatus::Seeding,
            files,
            peers,
        })
    }

    /// Returns whether the specified piece index is marked as possessed in the bitfield.
    pub fn has_piece(&self, index: usize) -> bool {
        if index >= self.num_pieces {
            return false;
        }
        let byte_idx = index / 8;
        let bit_mask = 0x80 >> (index % 8);
        if byte_idx < self.bitfield.len() {
            (self.bitfield[byte_idx] & bit_mask) != 0
        } else {
            false
        }
    }

    /// Sets the possession bit for a given piece index.
    pub fn set_piece(&mut self, index: usize, have: bool) {
        if index >= self.num_pieces {
            return;
        }
        let byte_idx = index / 8;
        let bit_mask = 0x80 >> (index % 8);
        if byte_idx < self.bitfield.len() {
            if have {
                self.bitfield[byte_idx] |= bit_mask;
            } else {
                self.bitfield[byte_idx] &= !bit_mask;
            }
        }
    }

    /// Returns the number of complete pieces possessed according to the bitfield.
    pub fn pieces_have_count(&self) -> usize {
        let mut count = 0;
        for i in 0..self.num_pieces {
            if self.has_piece(i) {
                count += 1;
            }
        }
        count
    }

    /// Returns the overall progress ratio as a float between 0.0 and 1.0.
    pub fn progress(&self) -> f32 {
        if self.num_pieces == 0 {
            0.0
        } else {
            self.pieces_have_count() as f32 / self.num_pieces as f32
        }
    }

    /// Fast-validates the on-disk files without computing cryptographic hashes.
    ///
    /// Checks that all non-empty files exist and match their expected byte lengths.
    pub fn verify_on_disk(&self, base_path: &Path) -> Result<bool, BitTorrentError> {
        self.verify_on_disk_opts(base_path, false)
    }

    /// Validates both existence, lengths, and modification timestamps (mtime) of on-disk files.
    pub fn verify_on_disk_strict(&self, base_path: &Path) -> Result<bool, BitTorrentError> {
        self.verify_on_disk_opts(base_path, true)
    }

    /// Validates on-disk files with configurable mtime verification.
    pub fn verify_on_disk_opts(
        &self,
        base_path: &Path,
        check_mtimes: bool,
    ) -> Result<bool, BitTorrentError> {
        if self.files.is_empty() && self.num_pieces > 0 {
            return Ok(false);
        }

        for entry in &self.files {
            let path_to_check = if entry.path.is_absolute() && entry.path.exists() {
                entry.path.clone()
            } else {
                base_path.join(&entry.path)
            };

            let meta = match std::fs::metadata(&path_to_check) {
                Ok(m) => m,
                Err(_) => {
                    if entry.length > 0 {
                        return Ok(false);
                    }
                    continue;
                }
            };

            if meta.len() != entry.length {
                return Ok(false);
            }

            if check_mtimes && entry.mtime > 0 {
                if let Ok(modified) = meta.modified() {
                    if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                        let cur_mtime = duration.as_secs();
                        // Allow small timestamp variance for FAT32 / filesystem precision differences
                        let diff = if cur_mtime > entry.mtime {
                            cur_mtime - entry.mtime
                        } else {
                            entry.mtime - cur_mtime
                        };
                        if diff > 2 {
                            return Ok(false);
                        }
                    }
                }
            }
        }

        Ok(true)
    }

    /// Applies the fast-resume state directly to an initialized `TorrentContext`.
    ///
    /// Restores the bitfield, computes missing piece masks, updates total byte counts,
    /// and flags the context as seeding if all pieces are present.
    pub fn apply_to_context(&self, ctx: &mut TorrentContext) -> Result<(), BitTorrentError> {
        if ctx.info_hash != self.info_hash {
            return Err(BitTorrentError::Parse(
                "Fast resume info-hash does not match torrent info-hash".to_string(),
            ));
        }
        if ctx.number_of_pieces != self.num_pieces {
            return Err(BitTorrentError::Parse(
                "Fast resume piece count does not match torrent piece count".to_string(),
            ));
        }

        // Apply bitfield
        ctx.bitfield = self.bitfield.clone();

        // Reset pieces_missing and recalculate
        ctx.pieces_missing = vec![0u8; ctx.bitfield.len()];
        ctx.missing_pieces_count = 0;

        let mut total_downloaded: u64 = 0;
        let mut missing_count = 0;
        let mut total_bytes_left = ctx.total_bytes_to_download as i64;
        let piece_length = ctx.piece_length;

        for piece_idx in 0..ctx.number_of_pieces as u32 {
            let (byte_index, bit_mask) = crate::util::get_bitfield_index_and_mask(piece_idx);
            let has_piece = (ctx.bitfield[byte_index] & bit_mask) != 0;

            let cur_piece_len = if total_bytes_left / (piece_length as i64) != 0 {
                piece_length
            } else {
                total_bytes_left as u32
            };
            ctx.set_piece_length(piece_idx, cur_piece_len);
            total_bytes_left -= piece_length as i64;

            if has_piece {
                ctx.mark_piece_missing(piece_idx, false);
                total_downloaded += cur_piece_len as u64;
            } else {
                ctx.mark_piece_missing(piece_idx, true);
                missing_count += 1;
            }
        }

        ctx.missing_pieces_count = missing_count;
        ctx.total_bytes_downloaded
            .store(total_downloaded, Ordering::Relaxed);
        ctx.initial_bytes_downloaded = total_downloaded;
        ctx.total_bytes_uploaded
            .store(self.total_uploaded, Ordering::Relaxed);

        // Apply file priorities
        for (i, file_entry) in self.files.iter().enumerate() {
            if i < ctx.files_to_download.len() {
                ctx.files_to_download[i].priority = match file_entry.priority {
                    0 => FilePriority::Skip,
                    1 => FilePriority::Low,
                    7 => FilePriority::High,
                    _ => FilePriority::Normal,
                };
            }
        }

        if missing_count == 0 {
            ctx.status = TorrentStatus::Seeding;
            ctx.download_finished.set();
        }

        Ok(())
    }

    /// Serializes this fast-resume state into a standard BEP 47 / libtorrent Bencode buffer.
    pub fn to_bencode(&self) -> Vec<u8> {
        let mut entries: Vec<(&[u8], Vec<u8>)> = Vec::new();

        entries.push((b"file-format", encode_bencode_str("libtorrent resume file")));
        entries.push((b"file-version", encode_bencode_int(1)));
        entries.push((b"info-hash", encode_bencode_bytes(&self.info_hash)));
        entries.push((
            b"save_path",
            encode_bencode_str(&self.save_path.to_string_lossy()),
        ));
        entries.push((b"num_pieces", encode_bencode_int(self.num_pieces as i64)));
        entries.push((
            b"piece_length",
            encode_bencode_int(self.piece_length as i64),
        ));
        entries.push((b"bitfield", encode_bencode_bytes(&self.bitfield)));

        // Libtorrent pieces byte-string: 1 byte per piece (0x01 if possessed, 0x00 otherwise)
        let mut pieces_bytes = Vec::with_capacity(self.num_pieces);
        for i in 0..self.num_pieces {
            pieces_bytes.push(if self.has_piece(i) { 1u8 } else { 0u8 });
        }
        entries.push((b"pieces", encode_bencode_bytes(&pieces_bytes)));

        entries.push((
            b"total_uploaded",
            encode_bencode_int(self.total_uploaded as i64),
        ));
        entries.push((
            b"total_downloaded",
            encode_bencode_int(self.total_downloaded as i64),
        ));
        entries.push((b"active_time", encode_bencode_int(self.active_time as i64)));
        entries.push((
            b"seeding_time",
            encode_bencode_int(self.seeding_time as i64),
        ));
        entries.push((
            b"is_seeding",
            encode_bencode_int(if self.is_seeding { 1 } else { 0 }),
        ));

        // "file sizes": list of [size, mtime] pairs
        let mut file_sizes_items = Vec::new();
        let mut file_priorities_items = Vec::new();
        let mut file_paths_items = Vec::new();
        for file in &self.files {
            let pair = encode_bencode_list(&[
                encode_bencode_int(file.length as i64),
                encode_bencode_int(file.mtime as i64),
            ]);
            file_sizes_items.push(pair);
            file_priorities_items.push(encode_bencode_int(file.priority as i64));
            file_paths_items.push(encode_bencode_str(&file.path.to_string_lossy()));
        }
        entries.push((b"file sizes", encode_bencode_list(&file_sizes_items)));
        entries.push((b"file_paths", encode_bencode_list(&file_paths_items)));
        entries.push((
            b"file_priorities",
            encode_bencode_list(&file_priorities_items),
        ));

        // Peers
        let peer_items: Vec<Vec<u8>> = self.peers.iter().map(|p| encode_bencode_str(p)).collect();
        entries.push((b"peers", encode_bencode_list(&peer_items)));

        encode_bencode_dict(&mut entries)
    }

    /// Deserializes a fast-resume state from a Bencode byte slice.
    pub fn from_bencode(bytes: &[u8]) -> Result<Self, BitTorrentError> {
        let root = Bencode::decode(bytes)?;

        let info_hash_raw = get_bytes(&root, b"info-hash").ok_or_else(|| {
            BitTorrentError::Parse("Missing 'info-hash' in fast-resume data".into())
        })?;
        if info_hash_raw.len() != 20 {
            return Err(BitTorrentError::Parse(
                "Invalid info-hash length in fast-resume data (expected 20 bytes)".into(),
            ));
        }
        let mut info_hash = [0u8; 20];
        info_hash.copy_from_slice(info_hash_raw);

        let save_path_str = get_str(&root, b"save_path").unwrap_or_default();
        let save_path = PathBuf::from(save_path_str);

        let num_pieces = get_int(&root, b"num_pieces").unwrap_or(0) as usize;
        let piece_length = get_int(&root, b"piece_length").unwrap_or(0) as u32;

        // Extract bitfield: prefer "bitfield", fallback to "pieces"
        let bitfield = if let Some(bf) = get_bytes(&root, b"bitfield") {
            bf.to_vec()
        } else if let Some(pieces) = get_bytes(&root, b"pieces") {
            let actual_len = if num_pieces > 0 {
                num_pieces
            } else {
                pieces.len()
            };
            let mut bf = vec![0u8; (actual_len + 7) / 8];
            for (i, &p) in pieces.iter().enumerate().take(actual_len) {
                if p != 0 {
                    let byte_idx = i / 8;
                    let bit_mask = 0x80 >> (i % 8);
                    bf[byte_idx] |= bit_mask;
                }
            }
            bf
        } else {
            vec![0u8; (num_pieces + 7) / 8]
        };

        let total_uploaded = get_int(&root, b"total_uploaded").unwrap_or(0).max(0) as u64;
        let total_downloaded = get_int(&root, b"total_downloaded").unwrap_or(0).max(0) as u64;
        let active_time = get_int(&root, b"active_time").unwrap_or(0).max(0) as u64;
        let seeding_time = get_int(&root, b"seeding_time").unwrap_or(0).max(0) as u64;
        let is_seeding = get_int(&root, b"is_seeding").unwrap_or(0) != 0;

        // Parse file priorities
        let mut priorities: Vec<u8> = Vec::new();
        if let Some(BNode::List(pri_list)) = root
            .dict_get(b"file_priorities")
            .or_else(|| root.dict_get(b"file priorities"))
        {
            for item in pri_list {
                if let Some(bytes) = item.as_number_bytes() {
                    if let Ok(s) = core::str::from_utf8(bytes) {
                        if let Ok(p) = s.parse::<u8>() {
                            priorities.push(p);
                        }
                    }
                }
            }
        }

        // Parse file paths if present
        let mut paths: Vec<PathBuf> = Vec::new();
        if let Some(BNode::List(path_list)) = root
            .dict_get(b"file_paths")
            .or_else(|| root.dict_get(b"file paths"))
        {
            for item in path_list {
                if let Some(bytes) = item.as_string() {
                    paths.push(PathBuf::from(String::from_utf8_lossy(bytes).into_owned()));
                }
            }
        }

        // Parse file sizes / mtimes
        let mut files = Vec::new();
        if let Some(BNode::List(sizes_list)) = root
            .dict_get(b"file sizes")
            .or_else(|| root.dict_get(b"file_sizes"))
        {
            for (idx, item) in sizes_list.iter().enumerate() {
                let priority = priorities.get(idx).copied().unwrap_or(4);
                let path = paths.get(idx).cloned().unwrap_or_default();
                match item {
                    BNode::List(pair) => {
                        let len = pair.get(0).and_then(parse_bnode_int).unwrap_or(0).max(0) as u64;
                        let mtime =
                            pair.get(1).and_then(parse_bnode_int).unwrap_or(0).max(0) as u64;
                        files.push(FastResumeFileEntry {
                            path,
                            length: len,
                            mtime,
                            priority,
                        });
                    }
                    BNode::Number(_) => {
                        let len = parse_bnode_int(item).unwrap_or(0).max(0) as u64;
                        files.push(FastResumeFileEntry {
                            path,
                            length: len,
                            mtime: 0,
                            priority,
                        });
                    }
                    _ => {}
                }
            }
        }

        // Parse peers
        let mut peers = Vec::new();
        if let Some(BNode::List(peer_list)) = root.dict_get(b"peers") {
            for item in peer_list {
                if let Some(bytes) = item.as_string() {
                    if let Ok(s) = core::str::from_utf8(bytes) {
                        peers.push(s.to_string());
                    } else if bytes.len() == 6 {
                        // Compact IPv4 format
                        let ip_str = format!(
                            "{}.{}.{}.{}:{}",
                            bytes[0],
                            bytes[1],
                            bytes[2],
                            bytes[3],
                            u16::from_be_bytes([bytes[4], bytes[5]])
                        );
                        peers.push(ip_str);
                    }
                }
            }
        }

        Ok(FastResumeData {
            info_hash,
            save_path,
            num_pieces,
            piece_length,
            bitfield,
            total_uploaded,
            total_downloaded,
            active_time,
            seeding_time,
            is_seeding,
            files,
            peers,
        })
    }

    /// Saves the fast-resume file to the designated path on disk.
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), BitTorrentError> {
        let bytes = self.to_bencode();
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path.as_ref(), bytes)?;
        Ok(())
    }

    /// Loads and parses a fast-resume file from disk.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, BitTorrentError> {
        let bytes = std::fs::read(path.as_ref())?;
        Self::from_bencode(&bytes)
    }
}

// Bencode encoding helper functions
fn encode_bencode_int(val: i64) -> Vec<u8> {
    format!("i{}e", val).into_bytes()
}

fn encode_bencode_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("{}:", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out
}

fn encode_bencode_str(s: &str) -> Vec<u8> {
    encode_bencode_bytes(s.as_bytes())
}

fn encode_bencode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(b'l');
    for item in items {
        out.extend_from_slice(item);
    }
    out.push(b'e');
    out
}

fn encode_bencode_dict(entries: &mut [(&[u8], Vec<u8>)]) -> Vec<u8> {
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = Vec::new();
    out.push(b'd');
    for (k, v) in entries.iter() {
        let key_encoded = encode_bencode_bytes(k);
        out.extend_from_slice(&key_encoded);
        out.extend_from_slice(v);
    }
    out.push(b'e');
    out
}

// Bencode decoding helper functions
fn get_int(node: &BNode, key: &[u8]) -> Option<i64> {
    node.dict_get(key).and_then(parse_bnode_int)
}

fn parse_bnode_int(node: &BNode) -> Option<i64> {
    node.as_number_bytes()
        .and_then(|bytes| core::str::from_utf8(bytes).ok())
        .and_then(|s| s.parse::<i64>().ok())
}

fn get_str(node: &BNode, key: &[u8]) -> Option<String> {
    node.dict_get(key).and_then(|n| {
        n.as_string()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    })
}

fn get_bytes<'a>(node: &'a BNode, key: &[u8]) -> Option<&'a [u8]> {
    node.dict_get(key).and_then(|n| n.as_string())
}
