//! BitTorrent v2 Hash-Tree Torrent Exporter & Infohash Upgrade Tool (BEP 52)
//!
//! Provides utilities to upgrade BitTorrent v1 metadata into BitTorrent v2 (pure v2)
//! or dual-stack Hybrid (v1 + v2) `.torrent` files by computing SHA-256 Merkle trees,
//! piece layers, and dual info-hashes from existing downloaded content data.

use crate::core::merkle::BLOCK_SIZE;
use crate::core::metainfo::MetaInfoFile;
use crate::core::torrent_builder::TorrentBuilder;
use crate::error::BitTorrentError;
use crate::util::info_hash_to_string;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use std::fs;
#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

/// Target format when converting or exporting torrents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TorrentExportFormat {
    /// Standard legacy BitTorrent v1 (SHA-1 piece hashes).
    V1,
    /// Pure BitTorrent v2 with Merkle trees and file tree (BEP 52).
    V2,
    /// Dual-stack Hybrid torrent with both v1 pieces and v2 Merkle hash trees (BEP 52).
    Hybrid,
}

impl TorrentExportFormat {
    /// Parses a string loosely into a `TorrentExportFormat`.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().trim() {
            "v1" | "1" => Some(Self::V1),
            "v2" | "2" | "pure-v2" => Some(Self::V2),
            "hybrid" | "v1+v2" | "both" => Some(Self::Hybrid),
            _ => None,
        }
    }

    /// Returns format name as a lowercase string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
            Self::Hybrid => "hybrid",
        }
    }
}

/// Detailed report generated following a torrent upgrade or export operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TorrentUpgradeReport {
    /// Name of the torrent.
    pub name: String,
    /// Output export format (V1, V2, or Hybrid).
    pub format: TorrentExportFormat,
    /// Total combined size of all files in bytes.
    pub total_size: u64,
    /// Piece length in bytes.
    pub piece_length: usize,
    /// 20-byte SHA-1 info-hash of the info dictionary (hex string), if available.
    pub v1_info_hash: Option<String>,
    /// 32-byte SHA-256 info-hash of the info dictionary (hex string), if available.
    pub v2_info_hash: Option<String>,
    /// Truncated 20-byte v2 info-hash for compact swarms (hex string), if available.
    pub v2_truncated_info_hash: Option<String>,
    /// List of file paths and their 32-byte SHA-256 Merkle root hashes (`pieces root` in hex).
    pub file_pieces_roots: Vec<(String, String)>,
    /// Number of entries stored in `piece layers`.
    pub piece_layers_count: usize,
    /// Complete, canonical Magnet URI with dual-hash parameters when applicable.
    pub magnet_uri: String,
}

/// Utility for upgrading existing v1 `.torrent` metadata to v2 or Hybrid format.
pub struct TorrentUpgrader;

impl TorrentUpgrader {
    /// Upgrades metadata from an existing parsed `MetaInfoFile` and in-memory file contents.
    pub fn upgrade_metainfo(
        metainfo: &MetaInfoFile,
        files_data: &[(String, Vec<u8>)],
        target_format: TorrentExportFormat,
    ) -> Result<(Vec<u8>, TorrentUpgradeReport), BitTorrentError> {
        if files_data.is_empty() {
            return Err(BitTorrentError::Parse(
                "Cannot upgrade torrent with no files".into(),
            ));
        }

        let name = metainfo
            .get_name()
            .unwrap_or_else(|_| "untitled".to_string());
        let mut piece_len = metainfo.get_piece_length().unwrap_or(64 * 1024) as usize;

        // Ensure piece length conforms to BEP 52 (power of two >= 16 KiB)
        if target_format != TorrentExportFormat::V1 {
            if piece_len < BLOCK_SIZE || (piece_len & (piece_len - 1)) != 0 {
                let mut valid_len = BLOCK_SIZE;
                while valid_len < piece_len {
                    valid_len <<= 1;
                }
                piece_len = valid_len;
            }
        }

        let mut builder = TorrentBuilder::new(&name)
            .piece_length(piece_len)
            .private(metainfo.is_private());

        if let Some(comment) = metainfo.get_comment() {
            builder = builder.comment(comment);
        }

        if let Ok(trackers) = metainfo.get_tracker_urls() {
            builder = builder.trackers(trackers);
        } else if let Ok(primary) = metainfo.get_tracker() {
            builder = builder.tracker(primary);
        }

        let web_seeds = metainfo.get_web_seeds();
        for ws in web_seeds {
            builder = builder.web_seed(ws);
        }

        let mut total_size = 0u64;
        for (path, data) in files_data {
            total_size += data.len() as u64;
            builder = builder.add_file(path, data.clone());
        }

        let torrent_bytes = match target_format {
            TorrentExportFormat::V1 => builder.build_v1()?,
            TorrentExportFormat::V2 => builder.build_v2()?,
            TorrentExportFormat::Hybrid => builder.build_hybrid()?,
        };

        // Parse generated torrent bytes to extract accurate upgrade report
        let mut generated_metainfo = MetaInfoFile::from_bytes(&torrent_bytes);
        generated_metainfo.parse()?;

        let v1_info_hash = if target_format == TorrentExportFormat::V2 {
            None
        } else {
            generated_metainfo
                .get_v1_info_hash()
                .ok()
                .map(|h| info_hash_to_string(&h))
        };
        let v2_info_hash = if target_format == TorrentExportFormat::V1 {
            None
        } else {
            generated_metainfo
                .get_v2_info_hash()
                .ok()
                .map(|h| info_hash_to_string(&h))
        };
        let v2_truncated_info_hash = v2_info_hash.as_ref().map(|s| s[..40.min(s.len())].to_string());

        let mut file_pieces_roots = Vec::new();
        #[cfg(feature = "std")]
        if let Ok((_size, files)) = generated_metainfo.local_files_to_download_list(Path::new(".")) {
            for f in files {
                if let Some(root) = f.pieces_root {
                    file_pieces_roots.push((f.torrent_path, info_hash_to_string(&root)));
                }
            }
        }

        let piece_layers_count = generated_metainfo.get_piece_layers().len();

        let trackers_list = generated_metainfo
            .get_tracker_urls()
            .unwrap_or_else(|_| Vec::new());
        let magnet_uri = Self::generate_magnet_uri(
            &name,
            v1_info_hash.as_deref(),
            v2_info_hash.as_deref(),
            &trackers_list,
        );

        let report = TorrentUpgradeReport {
            name,
            format: target_format,
            total_size,
            piece_length: piece_len,
            v1_info_hash,
            v2_info_hash,
            v2_truncated_info_hash,
            file_pieces_roots,
            piece_layers_count,
            magnet_uri,
        };

        Ok((torrent_bytes, report))
    }

    #[cfg(feature = "std")]
    /// Upgrades a torrent file on disk by reading file data from the specified download directory.
    pub fn upgrade_from_file(
        torrent_path: impl AsRef<Path>,
        data_dir: impl AsRef<Path>,
        target_format: TorrentExportFormat,
    ) -> Result<(Vec<u8>, TorrentUpgradeReport), BitTorrentError> {
        let mut metainfo = MetaInfoFile::new(torrent_path.as_ref())?;
        metainfo.parse()?;

        let (_, file_details_list) = metainfo.local_files_to_download_list(data_dir.as_ref())?;
        let mut files_data = Vec::with_capacity(file_details_list.len());

        let base_data_dir = data_dir.as_ref();
        for file in file_details_list {
            let direct_path = PathBuf::from(&file.name);
            let candidate_path = base_data_dir.join(&file.torrent_path);
            let full_path = if direct_path.exists() {
                direct_path
            } else if candidate_path.exists() {
                candidate_path
            } else {
                direct_path
            };

            if !full_path.exists() {
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Content file missing on disk: {}", full_path.display()),
                )));
            }

            let data = fs::read(&full_path)?;
            files_data.push((file.torrent_path.clone(), data));
        }

        Self::upgrade_metainfo(&metainfo, &files_data, target_format)
    }

    /// Generates standard BitTorrent Magnet URI based on available info-hashes.
    pub fn generate_magnet_uri(
        name: &str,
        v1_info_hash: Option<&str>,
        v2_info_hash: Option<&str>,
        trackers: &[String],
    ) -> String {
        let mut uri = String::from("magnet:?");
        let mut params = Vec::new();

        if let Some(v1) = v1_info_hash {
            params.push(format!("xt=urn:btih:{}", v1));
        }

        if let Some(v2) = v2_info_hash {
            params.push(format!("xt=urn:btmh:1220{}", v2));
        }

        let encoded_name = urlencoding::encode(name);
        params.push(format!("dn={}", encoded_name));

        for tr in trackers {
            params.push(format!("tr={}", urlencoding::encode(tr)));
        }

        uri.push_str(&params.join("&"));
        uri
    }
}
