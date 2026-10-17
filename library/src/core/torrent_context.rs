//! Torrent session context
//!
//! Models the state and configuration of an active torrent session, including
//! files, bitfield vectors, missing piece indices, and the connected peer swarm.

use crate::constants::{BLOCK_SIZE, ENDGAME_THRESHOLD};
use crate::error::BitTorrentError;
use crate::manual_reset_event::ManualResetEvent;
use crate::metainfo::{FileDetails, FilePriority, MetaInfoFile};
use crate::peer::Peer;
use crate::piece_buffer::PieceBuffer;
use crate::selector::PieceSelector;
use crate::assembler::Assembler;
use crate::session::SessionConfig;
use crate::util::get_bitfield_index_and_mask;
use sha1::Digest;
use std::cmp::min;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

/// Represents a piece rarity entry in the priority queue.
/// A binary heap containing this type will order entries such that pieces with the fewest peers are popped first.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PieceRarity {
    pub peer_count: usize,
    pub piece_index: u32,
}

impl Ord for PieceRarity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // We want a min-heap on peer_count, then piece_index
        other.peer_count.cmp(&self.peer_count)
            .then_with(|| other.piece_index.cmp(&self.piece_index))
    }
}

impl PartialOrd for PieceRarity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Placeholder structure representing a tracker client.
#[derive(Debug, Clone)]
pub struct Tracker;

/// Piece availability metadata tracked per piece.
#[derive(Debug, Clone)]
pub struct PieceInfo {
    pub peer_count: usize,
    pub piece_length: u32,
}

/// Tracks the super-seeding (initial seeding / BEP 16) distribution state.
#[derive(Debug, Clone, Default)]
pub struct SuperSeedingState {
    /// Whether super-seeding mode is actively enabled.
    pub enabled: bool,
    /// Maps piece index to the peer IP it was exclusively offered to.
    pub piece_offered_to: HashMap<u32, String>,
    /// Maps each peer IP to the piece index currently offered to them pending swarm validation.
    pub peer_pending_piece: HashMap<String, u32>,
    /// Counts how many times each piece has been witnessed in the swarm (from peers other than the one it was offered to).
    pub piece_swarm_count: HashMap<u32, usize>,
}

// Removed AssemblerData definition (now in assembler.rs)

/// Enumeration of states a torrent transfer session can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TorrentStatus {
    Initialised,
    Seeding,
    Downloading,
    Paused,
    Ended,
}

/// The core session context containing all transfer states, piece indices, and connection maps.
pub struct TorrentContext {
    pub info_hash: Vec<u8>,
    pub tracker_url: String,
    pub tracker_urls: Vec<String>,
    pub number_of_pieces: usize,
    pub piece_length: u32,
    pub pieces_info_hash: Vec<u8>,
    pub bitfield: Vec<u8>,
    pub files_to_download: Vec<FileDetails>,
    pub total_bytes_downloaded: Arc<AtomicU64>,
    pub initial_bytes_downloaded: u64,
    pub total_bytes_to_download: u64,
    pub total_bytes_uploaded: Arc<AtomicU64>,
    pub status: TorrentStatus,
    pub file_name: String,
    pub main_tracker: Option<Tracker>,
    #[allow(dead_code)]
    callback_data: Option<String>,
    #[allow(dead_code)]
    call_back: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    paused: ManualResetEvent,
    pub download_finished: Arc<ManualResetEvent>,
    pub selector: Arc<dyn PieceSelector>,
    pub peer_swarm: RwLock<HashMap<String, Arc<Mutex<Peer>>>>,
    pub missing_pieces_count: usize,
    pub maximum_swarm_size: usize,
    assembler: Assembler,
    bad_peer_scores: Mutex<HashMap<String, usize>>,
    pub pieces_missing: Vec<u8>,
    piece_data: Vec<PieceInfo>,
    pub storage: Arc<dyn crate::io_traits::BlockStorage>,
    pub config: SessionConfig,
    pub download_path: std::path::PathBuf,
    pub web_seeds: Vec<String>,
    pub is_private: bool,
    // Metadata bootstrap fields
    pub metadata_size: Option<usize>,
    pub metadata_pieces: alloc::collections::BTreeMap<u32, Vec<u8>>,
    pub requested_metadata_pieces: alloc::collections::BTreeMap<u32, std::time::Instant>,
    pub info_dict_bytes: Option<Vec<u8>>,
    pub download_speed_tracker: Mutex<(std::time::Instant, u64, i64)>,
    pub upload_speed_tracker: Mutex<(std::time::Instant, u64, i64)>,
    pub piece_priority_queue: Mutex<std::collections::BinaryHeap<PieceRarity>>,
    pub unchoked_peers_count: AtomicUsize,
    pub super_seeding: SuperSeedingState,
    #[cfg(feature = "v2")]
    pub file_merkle_trees: Mutex<HashMap<usize, crate::core::merkle::MerkleTree>>,
    #[cfg(feature = "v2")]
    pub file_merkle_proofs: Mutex<HashMap<(usize, usize), crate::core::merkle::MerkleProof>>,
    #[cfg(feature = "v2")]
    pub piece_layers: Mutex<HashMap<[u8; 32], Vec<[u8; 32]>>>,
}

impl TorrentContext {
    /// Creates and initializes a `TorrentContext` from parsed metainfo, creating target directories and scanning existing disk files.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent contains no tracker URLs,
    /// or if there are errors parsing pieces or files from the metadata.
    pub fn new(
        torrent_meta_info: &MetaInfoFile,
        selector: Arc<dyn PieceSelector>,
        storage: Arc<dyn crate::io_traits::BlockStorage>,
        download_path: &std::path::Path,
        seeding: bool,
        config: SessionConfig,
    ) -> Result<Self, crate::error::BitTorrentError> {
        let info_hash = torrent_meta_info.get_info_hash()?;
        let tracker_urls = torrent_meta_info.get_tracker_urls()?;
        let web_seeds = torrent_meta_info.get_web_seeds();
        let is_private = torrent_meta_info.is_private();
        let tracker_url = tracker_urls.get(0).cloned().ok_or_else(|| {
            crate::error::BitTorrentError::Parse("Torrent contains no tracker URLs.".into())
        })?;
        let (total_download_length, all_files_to_download) =
            torrent_meta_info.local_files_to_download_list(download_path)?;
        let piece_length = torrent_meta_info.get_piece_length()?;
        let pieces_info_hash = torrent_meta_info.get_pieces_info_hash()?;
        let number_of_pieces = pieces_info_hash.len() / crate::constants::HASH_LENGTH;
        let bitfield = vec![0u8; (number_of_pieces + 7) / 8];
        let pieces_missing = vec![0u8; bitfield.len()];
        let piece_data = vec![
            PieceInfo {
                peer_count: 0,
                piece_length
            };
            number_of_pieces
        ];

        let context = TorrentContext {
            info_hash,
            tracker_url,
            number_of_pieces,
            piece_length,
            pieces_info_hash,
            bitfield,
            files_to_download: all_files_to_download,
            total_bytes_downloaded: Arc::new(AtomicU64::new(0)),
            initial_bytes_downloaded: 0,
            total_bytes_to_download: total_download_length,
            total_bytes_uploaded: Arc::new(AtomicU64::new(0)),
            status: if seeding {
                TorrentStatus::Seeding
            } else {
                TorrentStatus::Initialised
            },
            file_name: torrent_meta_info
                .torrent_file_name
                .to_string_lossy()
                .to_string(),
            tracker_urls,
            main_tracker: None,
            callback_data: None,
            call_back: None,
            paused: ManualResetEvent::new(false),
            download_finished: Arc::new(ManualResetEvent::new(false)),
            selector,
            peer_swarm: RwLock::new(HashMap::new()),
            missing_pieces_count: 0,
            maximum_swarm_size: crate::constants::MAXIMUM_SWARM_SIZE,
            assembler: Assembler::new(),
            pieces_missing,
            piece_data,
            bad_peer_scores: Mutex::new(HashMap::new()),
            storage,
            super_seeding: SuperSeedingState {
                enabled: config.super_seeding,
                ..Default::default()
            },
            config,
            download_path: download_path.to_path_buf(),
            web_seeds,
            is_private,
            metadata_size: None,
            metadata_pieces: alloc::collections::BTreeMap::new(),
            requested_metadata_pieces: alloc::collections::BTreeMap::new(),
            info_dict_bytes: None,
            download_speed_tracker: Mutex::new((std::time::Instant::now(), 0, 0)),
            upload_speed_tracker: Mutex::new((std::time::Instant::now(), 0, 0)),
            piece_priority_queue: Mutex::new({
                let mut pq = std::collections::BinaryHeap::new();
                for piece_index in 0..number_of_pieces as u32 {
                    pq.push(PieceRarity { peer_count: 0, piece_index });
                }
                pq
            }),
            unchoked_peers_count: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            file_merkle_trees: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            file_merkle_proofs: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            piece_layers: Mutex::new(torrent_meta_info.get_piece_layers().clone()),
        };
        Ok(context)
    }

    /// Creates a placeholder `TorrentContext` for magnet links bootstrapping.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError`](crate::error::BitTorrentError) if the tracker URLs list is empty or initialization fails.
    pub fn new_magnet_bootstrap(
        info_hash: Vec<u8>,
        tracker_urls: Vec<String>,
        selector: Arc<dyn PieceSelector>,
        download_path: &std::path::Path,
        config: SessionConfig,
    ) -> Result<Self, crate::error::BitTorrentError> {
        let tracker_url = tracker_urls.get(0).cloned().unwrap_or_default();
        let storage = Arc::new(crate::io_traits::MemStorage::new(0));

        let context = TorrentContext {
            info_hash,
            tracker_url,
            web_seeds: Vec::new(),
            is_private: false,
            number_of_pieces: 0,
            piece_length: 0,
            pieces_info_hash: Vec::new(),
            bitfield: Vec::new(),
            files_to_download: Vec::new(),
            total_bytes_downloaded: Arc::new(AtomicU64::new(0)),
            initial_bytes_downloaded: 0,
            total_bytes_to_download: 0,
            total_bytes_uploaded: Arc::new(AtomicU64::new(0)),
            status: TorrentStatus::Initialised,
            file_name: String::new(),
            tracker_urls,
            main_tracker: None,
            callback_data: None,
            call_back: None,
            paused: ManualResetEvent::new(false),
            download_finished: Arc::new(ManualResetEvent::new(false)),
            selector,
            peer_swarm: RwLock::new(HashMap::new()),
            missing_pieces_count: 0,
            maximum_swarm_size: crate::constants::MAXIMUM_SWARM_SIZE,
            assembler: Assembler::new(),
            pieces_missing: Vec::new(),
            piece_data: Vec::new(),
            bad_peer_scores: Mutex::new(HashMap::new()),
            storage,
            super_seeding: SuperSeedingState {
                enabled: config.super_seeding,
                ..Default::default()
            },
            config,
            download_path: download_path.to_path_buf(),
            metadata_size: None,
            metadata_pieces: alloc::collections::BTreeMap::new(),
            requested_metadata_pieces: alloc::collections::BTreeMap::new(),
            info_dict_bytes: None,
            download_speed_tracker: Mutex::new((std::time::Instant::now(), 0, 0)),
            upload_speed_tracker: Mutex::new((std::time::Instant::now(), 0, 0)),
            piece_priority_queue: Mutex::new(std::collections::BinaryHeap::new()),
            unchoked_peers_count: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            file_merkle_trees: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            file_merkle_proofs: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            piece_layers: Mutex::new(HashMap::new()),
        };
        Ok(context)
    }

    /// Resizes and initializes piece tracking structures.
    pub fn init_pieces(&mut self, number_of_pieces: usize, piece_length: u32) {
        self.number_of_pieces = number_of_pieces;
        self.piece_length = piece_length;
        self.bitfield = vec![0u8; (number_of_pieces + 7) / 8];
        self.pieces_missing = vec![0u8; self.bitfield.len()];
        self.piece_data = vec![
            PieceInfo {
                peer_count: 0,
                piece_length,
            };
            number_of_pieces
        ];
        if let Ok(mut pq) = self.piece_priority_queue.lock() {
            pq.clear();
            for piece_index in 0..number_of_pieces as u32 {
                pq.push(PieceRarity {
                    peer_count: 0,
                    piece_index,
                });
            }
        }
    }

    /// Transitions a magnet bootstrap context to a standard torrent download session once metadata is fetched.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError`](crate::error::BitTorrentError) if decoding the info dictionary or parsing files fails,
    /// or if storage files cannot be created.
    pub fn transition_from_metadata(
        &mut self,
        info_dict_bytes: &[u8],
        download_path: &std::path::Path,
    ) -> Result<(), crate::error::BitTorrentError> {
        let info_node = crate::bencode::Bencode::decode(info_dict_bytes)?;
        
        let mut root_entries = Vec::new();
        root_entries.push((b"announce".as_slice(), crate::bencode::BNode::String(self.tracker_url.as_bytes())));
        root_entries.push((b"info".as_slice(), info_node));
        
        let root_node = crate::bencode::BNode::Dictionary(root_entries);
        let torrent_bytes = crate::bencode::Bencode::encode(&root_node);
        
        let mut meta_info = MetaInfoFile::from_bytes(&torrent_bytes);
        meta_info.parse()?;
        meta_info.validate()?;
        
        let piece_length = meta_info.get_piece_length()?;
        let pieces_info_hash = meta_info.get_pieces_info_hash()?;
        let number_of_pieces = pieces_info_hash.len() / crate::constants::HASH_LENGTH;
        let bitfield = vec![0u8; (number_of_pieces + 7) / 8];
        let pieces_missing = vec![0u8; bitfield.len()];
        let piece_data = vec![
            PieceInfo {
                peer_count: 0,
                piece_length
            };
            number_of_pieces
        ];
        
        let (total_download_length, all_files_to_download) =
            meta_info.local_files_to_download_list(download_path)?;
            
        let disk_io = Arc::new(crate::disk_io::DiskIO::new(
            download_path,
            all_files_to_download.clone(),
            piece_length,
        ));
        
        disk_io.create_local_torrent_structure()?;
        let storage: Arc<dyn crate::io_traits::BlockStorage> = if self.config.disk_cache_size_mb > 0 {
            Arc::new(crate::storage::disk_cache::CachedBlockStorage::new(
                disk_io.clone(),
                self.config.disk_cache_size_mb * 1024 * 1024,
            ))
        } else {
            disk_io.clone()
        };
        
        self.web_seeds = meta_info.get_web_seeds();
        self.is_private = meta_info.is_private();
        self.number_of_pieces = number_of_pieces;
        self.piece_length = piece_length;
        self.pieces_info_hash = pieces_info_hash;
        self.bitfield = bitfield;
        self.pieces_missing = pieces_missing;
        self.piece_data = piece_data;
        self.files_to_download = all_files_to_download;
        self.total_bytes_to_download = total_download_length;
        self.storage = storage;
        
        #[cfg(feature = "std")]
        {
            self.file_name = meta_info.torrent_file_name.to_string_lossy().to_string();
        }
        
        disk_io.create_torrent_bitfield(self)?;
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        self.initial_bytes_downloaded = downloaded;
        
        let torrent_filename = format!("{}.torrent", crate::util::info_hash_to_string(&self.info_hash));
        let torrent_path = download_path.join(torrent_filename);
        let _ = std::fs::write(torrent_path, &torrent_bytes);
        
        self.info_dict_bytes = Some(info_dict_bytes.to_vec());
        
        {
            let mut pq = self.piece_priority_queue.lock().unwrap();
            pq.clear();
            for piece_index in 0..number_of_pieces as u32 {
                pq.push(PieceRarity { peer_count: 0, piece_index });
            }
        }

        Ok(())
    }

    /// Validates the structure and length constraints of context data fields.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if any validated field constraints are violated.
    pub fn validate(&self) -> Result<(), crate::error::BitTorrentError> {
        if self.pieces_info_hash.is_empty() {
            // In magnet bootstrap mode
            return Ok(());
        }
        if self.number_of_pieces == 0 {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent must contain at least one piece.".to_string(),
            ));
        }
        if self.piece_length == 0 {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent piece length must be greater than zero.".to_string(),
            ));
        }
        if self.files_to_download.is_empty() {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent contains no files to download.".to_string(),
            ));
        }
        if self.pieces_info_hash.len() != self.number_of_pieces * crate::constants::HASH_LENGTH {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent pieces hash list length does not match number of pieces.".to_string(),
            ));
        }
        let expected_bitfield_length = (self.number_of_pieces + 7) / 8;
        if self.bitfield.len() != expected_bitfield_length {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent bitfield length is invalid.".to_string(),
            ));
        }
        Ok(())
    }

    /// Sets the status to `Downloading` if initialized.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent is already downloading or ended.
    pub fn start_downloading(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status == TorrentStatus::Downloading {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent is already downloading.".to_string(),
            ));
        }
        if self.status == TorrentStatus::Ended {
            return Err(crate::error::BitTorrentError::Parse(
                "Cannot restart a finished torrent.".to_string(),
            ));
        }
        if self.is_download_complete() {
            self.status = TorrentStatus::Seeding;
            self.download_finished.set();
        } else {
            self.status = TorrentStatus::Downloading;
        }
        Ok(())
    }

    /// Pauses the torrent download thread, updating state flags.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent is not downloading or seeding.
    pub fn pause(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status != TorrentStatus::Downloading && self.status != TorrentStatus::Seeding {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent can only be paused while downloading or seeding.".to_string(),
            ));
        }
        self.status = TorrentStatus::Paused;
        self.paused.set();
        Ok(())
    }

    /// Resumes the paused torrent download thread.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent is not currently paused.
    pub fn resume(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status != TorrentStatus::Paused {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent can only be resumed when paused.".to_string(),
            ));
        }
        if self.is_download_complete() {
            self.status = TorrentStatus::Seeding;
        } else {
            self.status = TorrentStatus::Downloading;
        }
        self.paused.reset();
        Ok(())
    }

    /// Transitions the state to `Ended` and signals download completion.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent is already stopped.
    pub fn stop(&mut self) -> Result<(), crate::error::BitTorrentError> {
        if self.status == TorrentStatus::Ended {
            return Err(crate::error::BitTorrentError::Parse(
                "Torrent has already been stopped.".to_string(),
            ));
        }
        self.status = TorrentStatus::Ended;
        self.download_finished.set();
        Ok(())
    }

    /// Returns the progress in parts per ten thousand (0 to 10000).
    pub fn progress_ppm(&self) -> u32 {
        if self.total_bytes_to_download == 0 {
            return 10000;
        }
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let ppm = (downloaded * 10000) / self.total_bytes_to_download;
        ppm.min(10000) as u32
    }

    /// Computes percentage completion from bytes downloaded versus total.
    #[cfg(feature = "std")]
    pub fn progress_percent(&self) -> f32 {
        self.progress_ppm() as f32 / 100.0
    }

    /// Sets or clears a specific piece completion bit in the local bitfield.
    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if local {
            self.bitfield[byte_index] |= bit_mask;
        } else {
            self.bitfield[byte_index] &= !bit_mask;
        }
    }

    /// Checks if a specific piece has been fully downloaded and verified locally.
    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        self.bitfield[byte_index] & bit_mask != 0
    }

    /// Sets or clears a specific piece index in the missing pieces tracking vector.
    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if missing {
            if self.pieces_missing[byte_index] & bit_mask == 0 {
                self.pieces_missing[byte_index] |= bit_mask;
                self.missing_pieces_count += 1;
            }
        } else if self.pieces_missing[byte_index] & bit_mask != 0 {
            self.pieces_missing[byte_index] &= !bit_mask;
            self.missing_pieces_count = self.missing_pieces_count.saturating_sub(1);
        }
    }

    /// Checks if a specific piece index is currently marked as missing.
    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        self.pieces_missing[byte_index] & bit_mask != 0
    }

    fn update_piece_peer_counts(&mut self, remote_peer: &Peer, increment: bool) {
        let mut piece_number = 0u32;
        let mut pq = self.piece_priority_queue.lock().unwrap();
        for byte in &remote_peer.remote_piece_bitfield {
            for bit in &[0x80u8, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01] {
                if piece_number as usize >= self.number_of_pieces {
                    return;
                }
                if byte & bit != 0 {
                    let idx = piece_number as usize;
                    if increment {
                        self.piece_data[idx].peer_count += 1;
                    } else {
                        self.piece_data[idx].peer_count = self.piece_data[idx].peer_count.saturating_sub(1);
                    }
                    pq.push(PieceRarity {
                        peer_count: self.piece_data[idx].peer_count,
                        piece_index: piece_number,
                    });
                }
                piece_number += 1;
            }
        }
    }

    /// Increments peer counts for all pieces present in a newly connected peer's bitfield.
    pub fn merge_piece_bitfield(&mut self, remote_peer: &Peer) {
        self.update_piece_peer_counts(remote_peer, true);
    }

    /// Computes the SHA-1 checksum of the piece buffer and compares it to the expected metainfo hash.
    pub fn check_piece_hash(
        &self,
        piece_number: u32,
        piece_buffer: &[u8],
        number_of_bytes: u32,
    ) -> bool {
        let hash = sha1::Sha1::digest(&piece_buffer[..number_of_bytes as usize]);
        let offset = piece_number as usize * crate::constants::HASH_LENGTH;
        hash.iter()
            .zip(&self.pieces_info_hash[offset..offset + crate::constants::HASH_LENGTH])
            .all(|(a, b)| a == b)
    }

    /// Computes the SHA-1 checksum of the piece by reading it block-by-block from storage.
    pub fn check_piece_hash_streaming(
        &self,
        storage: &dyn crate::io_traits::BlockStorage,
        piece_number: u32,
        number_of_bytes: u32,
    ) -> bool {
        use sha1::Digest;
        let mut hasher = sha1::Sha1::new();
        let mut read_offset = piece_number as u64 * self.piece_length as u64;
        let mut remaining = number_of_bytes as usize;
        
        let mut temp_buf = [0u8; BLOCK_SIZE];
        while remaining > 0 {
            let to_read = std::cmp::min(remaining, BLOCK_SIZE);
            match storage.read_block(read_offset, &mut temp_buf[..to_read]) {
                Ok(n) if n == to_read => {
                    hasher.update(&temp_buf[..to_read]);
                    read_offset += to_read as u64;
                    remaining -= to_read;
                }
                _ => return false,
            }
        }

        let hash = hasher.finalize();
        let offset = piece_number as usize * crate::constants::HASH_LENGTH;
        hash.iter()
            .zip(&self.pieces_info_hash[offset..offset + crate::constants::HASH_LENGTH])
            .all(|(a, b)| a == b)
    }

    /// Parameter-driven update of bitfield status without buffer reference.
    pub fn update_bitfield_status(
        &mut self,
        piece_number: u32,
        piece_there: bool,
        number_of_bytes: u32,
    ) {
        if piece_there && !self.is_piece_local(piece_number) {
            self.total_bytes_downloaded.fetch_add(number_of_bytes as u64, Ordering::Relaxed);
        }
        self.set_piece_length(piece_number, number_of_bytes);
        self.mark_piece_local(piece_number, piece_there);
        self.mark_piece_missing(piece_number, !piece_there);
    }

    /// Returns the number of bytes remaining to be downloaded.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the computed remaining bytes turn negative.
    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        if self.total_bytes_to_download < downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(self.total_bytes_to_download - downloaded)
    }

    /// Integrates a verified piece buffer, updating local bitfields, download speeds, and completion metrics.
    pub fn update_bitfield_from_buffer(
        &mut self,
        piece_number: u32,
        piece_buffer: &[u8],
        number_of_bytes: u32,
    ) {
        let piece_there = self.check_piece_hash(piece_number, piece_buffer, number_of_bytes);
        self.update_bitfield_status(piece_number, piece_there, number_of_bytes);
    }

    /// Returns the download priority of a piece based on all files that overlap it.
    ///
    /// If multiple files overlap this piece, the highest priority among them is selected.
    /// If all overlapping files are `FilePriority::Skip`, the piece itself is skipped.
    pub fn get_piece_priority(&self, piece: u32) -> FilePriority {
        if self.files_to_download.is_empty() || self.piece_length == 0 {
            return FilePriority::Normal;
        }

        let piece_start = piece as u64 * self.piece_length as u64;
        let piece_end = if self.total_bytes_to_download > 0 {
            min(piece_start + self.piece_length as u64, self.total_bytes_to_download)
        } else {
            piece_start + self.piece_length as u64
        };

        if piece_end <= piece_start {
            return FilePriority::Normal;
        }

        let mut max_priority = None;
        for file in &self.files_to_download {
            let file_start = file.offset;
            let file_end = file.offset + file.length;
            if file.length > 0 && file_start < piece_end && file_end > piece_start {
                max_priority = match max_priority {
                    None => Some(file.priority),
                    Some(p) => Some(std::cmp::max(p, file.priority)),
                };
            }
        }

        max_priority.unwrap_or(FilePriority::Normal)
    }

    /// Returns true if this piece is desired for download (i.e. not marked as `Skip`).
    pub fn is_piece_wanted(&self, piece: u32) -> bool {
        self.get_piece_priority(piece) != FilePriority::Skip
    }

    /// Sets the download priority for a file at the given index.
    ///
    /// If a file is skipped and this causes pieces to become un-wanted,
    /// any pending block reservations for those pieces are released.
    pub fn set_file_priority(
        &mut self,
        file_index: usize,
        priority: FilePriority,
    ) -> Result<(), BitTorrentError> {
        if file_index >= self.files_to_download.len() {
            return Err(BitTorrentError::Parse(format!(
                "File index {} out of range",
                file_index
            )));
        }

        self.files_to_download[file_index].priority = priority;

        // If priority is Skip, clear any pending reservations for pieces that are no longer wanted
        if priority == FilePriority::Skip {
            for piece in 0..self.number_of_pieces as u32 {
                if !self.is_piece_wanted(piece) {
                    self.clear_piece_requests(piece);
                }
            }
        } else {
            // Re-seed the priority queue with wanted missing pieces
            if let Ok(mut pq) = self.piece_priority_queue.lock() {
                pq.clear();
                for piece_index in 0..self.number_of_pieces as u32 {
                    if self.is_piece_wanted(piece_index) && !self.is_piece_local(piece_index) {
                        let count = self.piece_data.get(piece_index as usize).map(|p| p.peer_count).unwrap_or(0);
                        pq.push(PieceRarity { peer_count: count, piece_index });
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns true if super-seeding mode (BEP 16) is actively running.
    pub fn is_super_seeding(&self) -> bool {
        self.super_seeding.enabled && (self.status == TorrentStatus::Seeding || self.is_download_complete())
    }

    /// Enables or disables super-seeding mode.
    pub fn set_super_seeding(&mut self, enabled: bool) {
        self.super_seeding.enabled = enabled;
        if !enabled {
            self.super_seeding.piece_offered_to.clear();
            self.super_seeding.peer_pending_piece.clear();
        }
    }

    /// Selects an unseeded/rarest piece to offer to a connected peer in super-seeding mode.
    ///
    /// Adheres to BEP 16:
    /// - Only offers a piece if this peer does not already have a pending piece waiting for swarm validation.
    /// - Picks a piece that the peer does NOT have.
    /// - Prioritizes pieces that have not yet been offered to anyone in the swarm.
    /// - Breaks ties by lowest piece swarm count (least witnessed copies).
    pub fn offer_super_seed_piece(&mut self, peer_ip: &str, peer_bitfield: &[u8]) -> Option<u32> {
        if !self.is_super_seeding() {
            return None;
        }

        // Do not offer another piece if this peer is already holding a pending piece awaiting validation
        if self.super_seeding.peer_pending_piece.contains_key(peer_ip) {
            return None;
        }

        let total_pieces = self.number_of_pieces as u32;
        if total_pieces == 0 {
            return None;
        }

        let mut candidate_pieces: Vec<u32> = (0..total_pieces)
            .filter(|&piece| {
                if !self.is_piece_local(piece) {
                    return false;
                }
                let (b_idx, mask) = get_bitfield_index_and_mask(piece);
                if b_idx < peer_bitfield.len() && (peer_bitfield[b_idx] & mask) != 0 {
                    return false;
                }
                true
            })
            .collect();

        if candidate_pieces.is_empty() {
            return None;
        }

        candidate_pieces.sort_by_key(|&p| {
            let is_currently_offered = self.super_seeding.piece_offered_to.contains_key(&p);
            let swarm_count = self.super_seeding.piece_swarm_count.get(&p).copied().unwrap_or(0);
            (is_currently_offered, swarm_count, p)
        });

        let chosen_piece = candidate_pieces[0];
        self.super_seeding.piece_offered_to.insert(chosen_piece, peer_ip.to_string());
        self.super_seeding.peer_pending_piece.insert(peer_ip.to_string(), chosen_piece);

        Some(chosen_piece)
    }

    /// Evaluates incoming `Have` announcements from peers to validate piece propagation in super-seeding mode.
    ///
    /// If `reporting_peer_ip` received a piece from another peer to whom it was originally offered,
    /// the original peer is freed up and credited, and a new piece is selected for them.
    /// Returns `Some((offered_peer_ip, next_piece_for_offered_peer))` if a new piece should be sent to that peer.
    pub fn on_peer_have(&mut self, reporting_peer_ip: &str, piece_index: u32) -> Option<(String, u32)> {
        if !self.is_super_seeding() {
            return None;
        }

        // Record that this piece was seen in the swarm
        *self.super_seeding.piece_swarm_count.entry(piece_index).or_insert(0) += 1;

        // Check if this piece was offered to someone
        if let Some(offered_peer_ip) = self.super_seeding.piece_offered_to.get(&piece_index).cloned() {
            if offered_peer_ip != reporting_peer_ip {
                // The piece was successfully uploaded by offered_peer to another peer!
                if self.super_seeding.peer_pending_piece.get(&offered_peer_ip) == Some(&piece_index) {
                    self.super_seeding.peer_pending_piece.remove(&offered_peer_ip);

                    // Fetch offered_peer's bitfield to find their next piece
                    let remote_bitfield = {
                        if let Ok(swarm) = self.peer_swarm.read() {
                            swarm.get(&offered_peer_ip)
                                .and_then(|p| p.lock().ok())
                                .map(|p| p.remote_piece_bitfield.clone())
                        } else {
                            None
                        }
                    };

                    if let Some(bitfield) = remote_bitfield {
                        if let Some(next_piece) = self.offer_super_seed_piece(&offered_peer_ip, &bitfield) {
                            return Some((offered_peer_ip, next_piece));
                        }
                    }
                }
            }
        }

        None
    }

    /// Cleans up state when a peer disconnects in super-seeding mode.
    pub fn on_peer_disconnect(&mut self, peer_ip: &str) {
        if let Some(piece) = self.super_seeding.peer_pending_piece.remove(peer_ip) {
            if self.super_seeding.piece_offered_to.get(&piece).map(|ip| ip == peer_ip).unwrap_or(false) {
                self.super_seeding.piece_offered_to.remove(&piece);
            }
        }
    }

    /// Checks if a piece request from a peer is permissible in super-seeding mode.
    pub fn can_upload_piece_to_peer(&self, peer_ip: &str, piece_index: u32) -> bool {
        if !self.is_super_seeding() {
            return true;
        }
        self.super_seeding.peer_pending_piece.get(peer_ip) == Some(&piece_index)
            || self.super_seeding.piece_offered_to.get(&piece_index).map(|ip| ip == peer_ip).unwrap_or(false)
    }

    /// Checks if the session download is complete (all wanted pieces downloaded).
    pub fn is_download_complete(&self) -> bool {
        if self.number_of_pieces == 0 {
            return false;
        }
        if self.bytes_left_to_download().unwrap_or(1) == 0 {
            return true;
        }
        let mut any_wanted = false;
        for piece in 0..self.number_of_pieces as u32 {
            if self.is_piece_wanted(piece) {
                any_wanted = true;
                if !self.is_piece_local(piece) {
                    return false;
                }
            }
        }
        any_wanted
    }

    /// Returns true when the torrent has entered endgame mode.
    pub fn is_endgame(&self) -> bool {
        let missing_wanted = (0..self.number_of_pieces as u32)
            .filter(|&p| self.is_piece_wanted(p) && !self.is_piece_local(p))
            .take(ENDGAME_THRESHOLD + 1)
            .count();
        missing_wanted <= ENDGAME_THRESHOLD
    }

    /// Checks and updates status to `Seeding` if downloading is completed.
    pub fn try_complete_download(&mut self) {
        if self.status == TorrentStatus::Downloading && self.is_download_complete() {
            self.status = TorrentStatus::Seeding;
            self.download_finished.set();
        }
    }

    /// Returns a tuple containing:
    /// 1. A boolean vector indicating which pieces are verified and present locally.
    /// 2. A frequency vector counting how many connected peers currently hold each piece.
    pub fn get_piece_availability(&self) -> (alloc::vec::Vec<bool>, alloc::vec::Vec<u16>) {
        let total = self.number_of_pieces;
        let mut local_have = alloc::vec![false; total];
        for (i, have) in local_have.iter_mut().enumerate() {
            *have = self.is_piece_local(i as u32);
        }

        let mut swarm_rarity = alloc::vec![0u16; total];
        if let Ok(swarm) = self.peer_swarm.read() {
            for peer_arc in swarm.values() {
                if let Ok(peer) = peer_arc.try_lock() {
                    for piece_idx in 0..total {
                        let (byte_idx, mask) = crate::util::get_bitfield_index_and_mask(piece_idx as u32);
                        if (byte_idx as usize) < peer.remote_piece_bitfield.len()
                            && (peer.remote_piece_bitfield[byte_idx as usize] & mask) != 0
                        {
                            swarm_rarity[piece_idx] = swarm_rarity[piece_idx].saturating_add(1);
                        }
                    }
                }
            }
        }

        (local_have, swarm_rarity)
    }

    /// Appends incoming sub-block data, writing the fully assembled piece to disk upon completion and hash validation.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Protocol`](crate::error::BitTorrentError::Protocol) if the block parameters are invalid or if hash verification fails.
    /// Returns a [`BitTorrentError::Io`](crate::error::BitTorrentError::Io) if writing to block storage fails.
    pub fn process_piece_block(
        &mut self,
        storage: &dyn crate::io_traits::BlockStorage,
        piece_number: u32,
        begin: u32,
        block_data: &[u8],
        peer_ip: &str,
    ) -> Result<bool, crate::error::BitTorrentError> {
        if piece_number >= self.number_of_pieces as u32 {
            return Err(crate::error::BitTorrentError::Protocol("Invalid piece index".into()));
        }
        // Validate alignment before any early-return so misaligned blocks from any
        // peer are always rejected — even if we already have this piece locally.
        if begin % BLOCK_SIZE as u32 != 0 {
            return Err(crate::error::BitTorrentError::Protocol("Block offset not aligned".into()));
        }
        let expected_piece_length = self.get_piece_length(piece_number);
        if begin.checked_add(block_data.len() as u32).map_or(true, |end| end > expected_piece_length) {
            return Err(crate::error::BitTorrentError::Protocol("Block out of piece bounds".into()));
        }

        if self.is_piece_local(piece_number) {
            return Ok(false);
        }
        let piece_length = self.get_piece_length(piece_number);
        let block_index = begin / BLOCK_SIZE as u32;

        let mut piece_buffers = self.assembler.piece_buffers.lock().unwrap();
        let piece_buffer_arc = piece_buffers
            .entry(piece_number)
            .or_insert_with(|| Arc::new(Mutex::new(PieceBuffer::new(piece_number, piece_length))))
            .clone();

        let mut piece_buffer = piece_buffer_arc.lock().unwrap();
        let already_present = piece_buffer.is_block_present(block_index);
        if !already_present {
            #[cfg(feature = "v2")]
            {
                self.verify_v2_block(piece_number, block_index, block_data)?;
            }
            let block_offset = (block_index as u64) * BLOCK_SIZE as u64;
            let global_offset = (piece_number as u64) * self.piece_length as u64 + block_offset;
            storage.write_block(global_offset, block_data)?;
            piece_buffer.add_block(block_index, peer_ip);
        }

        let piece_complete = piece_buffer.all_blocks_there();
        let block_sources = if piece_complete {
            piece_buffer.block_sources.clone()
        } else {
            Vec::new()
        };
        if piece_complete {
            piece_buffers.remove(&piece_number);
        }
        drop(piece_buffer);
        drop(piece_buffers);

        if piece_complete {
            if self.check_piece_hash_streaming(storage, piece_number, piece_length) {
                crate::log_debug!(
                    "[piece] {} passed hash verification ({} bytes)",
                    piece_number,
                    piece_length
                );
                let _ = storage.flush();
                self.update_bitfield_status(
                    piece_number,
                    true,
                    piece_length,
                );
                // Broadcast Have to all connected peers so they know we have this piece.
                {
                    let swarm = self.peer_swarm.read().unwrap();
                    for peer_arc in swarm.values() {
                        if let Ok(peer_guard) = peer_arc.try_lock() {
                            let _ = peer_guard.send_have(piece_number);
                        }
                    }
                }
                self.try_complete_download();
                self.clear_piece_requests(piece_number);
                return Ok(true);
            } else {
                crate::log_debug!("[piece] {} failed hash verification", piece_number);
                self.clear_piece_requests(piece_number);

                // Discard failed piece dirty blocks from cache so corrupt data never touches disk
                let piece_offset = (piece_number as u64) * (self.piece_length as u64);
                storage.discard_range(piece_offset, piece_length as u64);

                // Report all peers that contributed blocks to this piece
                for source_ip_opt in block_sources {
                    if let Some(ip) = source_ip_opt {
                        self.report_bad_peer(&ip);
                    }
                }

                return Err(crate::error::BitTorrentError::Protocol(
                    "Piece failed hash verification".to_string(),
                ));
            }
        }

        Ok(false)
    }

    /// Appends block data and marks the piece as verified immediately when completed, bypassing hash checks.
    /// Exclusively intended for testing and mocking.
    pub fn process_verified_block(
        &mut self,
        storage: &dyn crate::io_traits::BlockStorage,
        piece_number: u32,
        begin: u32,
        block_data: &[u8],
        peer_ip: &str,
    ) -> Result<bool, crate::error::BitTorrentError> {
        if piece_number >= self.number_of_pieces as u32 {
            return Err(crate::error::BitTorrentError::Protocol("Invalid piece index".into()));
        }
        if begin % BLOCK_SIZE as u32 != 0 {
            return Err(crate::error::BitTorrentError::Protocol("Block offset not aligned".into()));
        }
        let expected_piece_length = self.get_piece_length(piece_number);
        if begin.checked_add(block_data.len() as u32).map_or(true, |end| end > expected_piece_length) {
            return Err(crate::error::BitTorrentError::Protocol("Block out of piece bounds".into()));
        }

        if self.is_piece_local(piece_number) {
            return Ok(false);
        }
        let piece_length = self.get_piece_length(piece_number);
        let block_index = begin / BLOCK_SIZE as u32;

        let mut piece_buffers = self.assembler.piece_buffers.lock().unwrap();
        let piece_buffer_arc = piece_buffers
            .entry(piece_number)
            .or_insert_with(|| Arc::new(Mutex::new(PieceBuffer::new(piece_number, piece_length))))
            .clone();

        let mut piece_buffer = piece_buffer_arc.lock().unwrap();
        let already_present = piece_buffer.is_block_present(block_index);
        if !already_present {
            let block_offset = (block_index as u64) * BLOCK_SIZE as u64;
            let global_offset = (piece_number as u64) * self.piece_length as u64 + block_offset;
            storage.write_block(global_offset, block_data)?;
            piece_buffer.add_block(block_index, peer_ip);
        }

        let piece_complete = piece_buffer.all_blocks_there();
        if piece_complete {
            piece_buffers.remove(&piece_number);
        }
        drop(piece_buffer);
        drop(piece_buffers);

        if piece_complete {
            let _ = storage.flush();
            self.update_bitfield_status(piece_number, true, piece_length);
            {
                let swarm = self.peer_swarm.read().unwrap();
                for peer_arc in swarm.values() {
                    if let Ok(peer_guard) = peer_arc.try_lock() {
                        let _ = peer_guard.send_have(piece_number);
                    }
                }
            }
            self.try_complete_download();
            self.clear_piece_requests(piece_number);
            return Ok(true);
        }
        Ok(false)
    }

    /// Registers a peer connection in the active swarm. Returns `true` if registered, `false` if swarm is full.
    pub fn add_peer(&self, peer: Arc<Mutex<Peer>>) -> bool {
        let ip = peer.lock().unwrap().ip.clone();
        if self.is_peer_blacklisted(&ip) {
            return false;
        }
        if self.is_space_in_swarm(&ip) {
            self.peer_swarm.write().unwrap().insert(ip, peer).is_none()
        } else {
            false
        }
    }

    /// Unregisters and drops a peer connection from the swarm by IP address.
    pub fn remove_peer(&mut self, ip: &str) {
        let peer_opt = self.peer_swarm.write().unwrap().remove(ip);
        if let Some(peer_arc) = peer_opt {
            if let Ok(mut peer_guard) = peer_arc.lock() {
                if peer_guard.peer_choking.wait_one(0) {
                    self.unchoked_peers_count.fetch_sub(1, Ordering::Relaxed);
                }
                self.unmerge_piece_bitfield(&peer_guard);
                peer_guard.close();
            }
        }
    }

    /// Checks if a peer IP address is blacklisted (i.e. has 3 or more bad blocks).
    pub fn is_peer_blacklisted(&self, ip: &str) -> bool {
        let scores = self.bad_peer_scores.lock().unwrap();
        if let Some(&score) = scores.get(ip) {
            score >= 3
        } else {
            false
        }
    }

    /// Reports a bad block from a peer IP address, potentially blacklisting and disconnecting them.
    ///
    /// # Deadlock Safety
    /// This method is careful to release the `bad_peer_scores` lock before acquiring the
    /// `peer_swarm` write lock to avoid a potential lock-ordering deadlock.
    pub fn report_bad_peer(&self, ip: &str) {
        let should_blacklist = {
            let mut scores = self.bad_peer_scores.lock().unwrap();
            let score = scores.entry(ip.to_string()).or_insert(0);
            *score += 1;
            *score >= 3
        }; // bad_peer_scores lock released here
        if should_blacklist {
            crate::log_debug!("[swarm] Blacklisting peer {} due to repeated bad blocks", ip);
            let peer_opt = self.peer_swarm.write().unwrap().remove(ip);
            if let Some(peer_arc) = peer_opt {
                if let Ok(mut peer_guard) = peer_arc.lock() {
                    if peer_guard.peer_choking.wait_one(0) {
                        self.unchoked_peers_count.fetch_sub(1, Ordering::Relaxed);
                    }
                    peer_guard.close();
                }
            }
        }
    }

    /// Safely terminates connection streams and unregisters all active peers in the swarm.
    pub fn disconnect_all_peers(&self) {
        let mut swarm = self.peer_swarm.write().unwrap();
        for peer in swarm.values() {
            peer.lock().unwrap().close();
        }
        swarm.clear();
    }

    /// Decrements piece peer counts when a peer disconnects.
    pub fn unmerge_piece_bitfield(&mut self, remote_peer: &Peer) {
        self.update_piece_peer_counts(remote_peer, false);
    }

    /// Returns the length of the specified piece.
    pub fn get_piece_length(&self, piece_number: u32) -> u32 {
        self.piece_data[piece_number as usize].piece_length
    }

    /// Returns the number of peers that have announced possession of the specified piece.
    pub fn get_piece_peer_count(&self, piece_number: u32) -> usize {
        self.piece_data[piece_number as usize].peer_count
    }

    /// Sets the byte length for a given piece index.
    ///
    /// If `piece_length` is larger than the torrent's standard piece length, the call is
    /// ignored and an error is logged — the piece length is left unchanged.
    ///
    /// # Panics
    /// Panics if `piece_number` is out of bounds (greater than or equal to the total number of pieces).
    pub fn set_piece_length(&mut self, piece_number: u32, piece_length: u32) {
        if piece_length <= self.piece_length {
            self.piece_data[piece_number as usize].piece_length = piece_length;
        } else {
            crate::log_debug!(
                "[context] set_piece_length: piece {} length {} exceeds torrent max {}; ignoring",
                piece_number, piece_length, self.piece_length
            );
        }
    }

    /// Helper asserting whether a peer IP can join the swarm.
    ///
    /// Acquires the `peer_swarm` read lock exactly once to avoid a TOCTOU race between
    /// the duplicate-check and the capacity-check.
    pub fn is_space_in_swarm(&self, ip: &str) -> bool {
        if ip.is_empty() {
            return false;
        }
        let swarm = self.peer_swarm.read().unwrap();
        !swarm.contains_key(ip) && swarm.len() < self.maximum_swarm_size
    }

    /// Finds the next unrequested block offset and length within a given piece.
    pub fn next_pending_block(&self, piece_number: u32) -> Option<(u32, u32)> {
        let piece_length = self.get_piece_length(piece_number);
        let block_size = self.config.block_size;
        let block_count = ((piece_length as usize + block_size - 1) / block_size) as u32;
        for block_index in 0..block_count {
            if !self.is_block_requested(piece_number, block_index) {
                let begin = block_index * block_size as u32;
                let length = min(block_size as u32, piece_length.saturating_sub(begin));
                return Some((begin, length));
            }
        }
        None
    }

    /// Checks if a block request has been registered in the context.
    pub fn is_block_requested(&self, piece_number: u32, block_index: u32) -> bool {
        self.assembler.is_block_requested(piece_number, block_index)
    }

    /// Marks a block index as requested/reserved in the session context.
    pub fn reserve_block_request(&self, piece_number: u32, block_index: u32) -> bool {
        self.assembler.reserve_block_request(piece_number, block_index)
    }

    /// Releases a block reservation, allowing other peers to request it.
    pub fn release_block_request(&self, piece_number: u32, block_index: u32) {
        self.assembler.release_block_request(piece_number, block_index);
    }

    /// Drops all block request reservations registered under a given piece index.
    pub fn clear_piece_requests(&self, piece_number: u32) {
        self.assembler.clear_piece_requests(piece_number);
    }

    /// Selects the rarest missing piece that the remote peer possesses.
    pub fn select_next_piece_for_peer(&self, remote_peer: &Peer) -> Option<u32> {
        self.selector.select_piece(self, remote_peer)
    }

    /// Identifies and reserves the next download block from a piece available on the specified peer.
    /// In endgame mode (few pieces remaining), allows duplicate requests to speed up completion.
    pub fn next_block_request_for_peer(&mut self, peer: &Peer) -> Option<(u32, u32, u32)> {
        if self.status != TorrentStatus::Downloading {
            return None;
        }
        let block_size = self.config.block_size;
        let endgame = self.is_endgame();
        if endgame {
            // Endgame mode: request any block from the rarest available piece.
            let piece_number = (0..self.number_of_pieces as u32)
                .filter(|&piece| self.is_piece_wanted(piece) && !self.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
                .min_by_key(|&piece| (std::cmp::Reverse(self.get_piece_priority(piece)), self.piece_data[piece as usize].peer_count, piece))?;

            let piece_length = self.get_piece_length(piece_number);
            let block_count = ((piece_length as usize + block_size - 1) / block_size) as u32;

            let piece_buffers = self.assembler.piece_buffers.lock().unwrap();
            let present_blocks = piece_buffers.get(&piece_number)
                .map(|pb| {
                    let guard = pb.lock().unwrap();
                    (0..block_count).map(|bi| guard.is_block_present(bi)).collect::<Vec<bool>>()
                })
                .unwrap_or_else(|| vec![false; block_count as usize]);
            drop(piece_buffers);

            for block_index in 0..block_count {
                if present_blocks[block_index as usize] {
                    continue;
                }
                if peer.reserved_blocks.iter().any(|&(p, b, _)| p == piece_number && b == block_index) {
                    continue;
                }
                let begin = block_index * block_size as u32;
                let length = min(block_size as u32, piece_length.saturating_sub(begin));
                self.reserve_block_request(piece_number, block_index);
                return Some((piece_number, begin, length));
            }
        } else {
            // Standard mode: find the rarest piece with an unrequested block, prioritizing High > Normal > Low pieces.
            let best = (0..self.number_of_pieces as u32)
                .filter(|&piece| self.is_piece_wanted(piece) && !self.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
                .filter_map(|piece| {
                    self.next_pending_block(piece).map(|block| (piece, block))
                })
                .min_by_key(|&(piece, _)| (std::cmp::Reverse(self.get_piece_priority(piece)), self.piece_data[piece as usize].peer_count, piece));

            if let Some((piece_number, (begin, length))) = best {
                let block_index = begin / block_size as u32;
                if self.reserve_block_request(piece_number, block_index) {
                    return Some((piece_number, begin, length));
                }
            }
        }
        None
    }

    /// Identifies and reserves the next download block from pieces in the peer's Allowed Fast set.
    /// Used when the peer is choking us, but supports BEP 6 Fast Extensions and has granted Allowed Fast pieces.
    pub fn next_allowed_fast_block_request_for_peer(&mut self, peer: &Peer) -> Option<(u32, u32, u32)> {
        if self.status != TorrentStatus::Downloading {
            return None;
        }
        let block_size = self.config.block_size;
        let best = peer.allowed_fast_pieces
            .iter()
            .copied()
            .filter(|&piece| (piece as usize) < self.number_of_pieces && self.is_piece_wanted(piece) && !self.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
            .filter_map(|piece| {
                self.next_pending_block(piece).map(|block| (piece, block))
            })
            .min_by_key(|&(piece, _)| (std::cmp::Reverse(self.get_piece_priority(piece)), self.piece_data[piece as usize].peer_count, piece));

        if let Some((piece_number, (begin, length))) = best {
            let block_index = begin / block_size as u32;
            if self.reserve_block_request(piece_number, block_index) {
                return Some((piece_number, begin, length));
            }
        }
        None
    }


    /// Increments the local peer availability count for a given piece.
    pub fn increment_peer_count(&mut self, piece_number: u32) {
        self.piece_data[piece_number as usize].peer_count += 1;
        let new_count = self.piece_data[piece_number as usize].peer_count;
        self.piece_priority_queue.lock().unwrap().push(PieceRarity {
            peer_count: new_count,
            piece_index: piece_number,
        });
    }

    /// Walks piece availability vectors to identify the next missing piece starting search from `start_piece`.
    pub fn find_next_missing_piece(&self, start_piece: u32) -> (bool, u32) {
        let mut current_piece = start_piece;
        loop {
            if self.is_piece_missing(current_piece)
                && self.is_piece_wanted(current_piece)
                && self.piece_data[current_piece as usize].peer_count > 0
            {
                return (true, current_piece);
            }
            current_piece = (current_piece + 1) % self.number_of_pieces as u32;
            if current_piece == start_piece {
                break;
            }
        }
        (false, current_piece)
    }

    /// Estimates the current download transfer rate in bytes per second.
    pub fn bytes_per_second(&self) -> i64 {
        let (elapsed, delta_bytes, needs_update, now, current_bytes, old_bps) = {
            if let Ok(guard) = self.download_speed_tracker.lock() {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(guard.0);
                let current_bytes = self.total_bytes_downloaded.load(Ordering::Relaxed);
                if elapsed.as_millis() >= 500 {
                    (elapsed, current_bytes.saturating_sub(guard.1), true, now, current_bytes, guard.2)
                } else {
                    (elapsed, 0, false, now, current_bytes, guard.2)
                }
            } else {
                return 0;
            }
        };

        if needs_update {
            let bps = (delta_bytes as f64 / elapsed.as_secs_f64()) as i64;
            if let Ok(mut guard) = self.download_speed_tracker.lock() {
                *guard = (now, current_bytes, bps);
            }
            bps
        } else {
            old_bps
        }
    }

    /// Estimates the current upload transfer rate in bytes per second.
    pub fn upload_bytes_per_second(&self) -> i64 {
        let (elapsed, delta_bytes, needs_update, now, current_bytes, old_bps) = {
            if let Ok(guard) = self.upload_speed_tracker.lock() {
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(guard.0);
                let current_bytes = self.total_bytes_uploaded.load(Ordering::Relaxed);
                if elapsed.as_millis() >= 500 {
                    (elapsed, current_bytes.saturating_sub(guard.1), true, now, current_bytes, guard.2)
                } else {
                    (elapsed, 0, false, now, current_bytes, guard.2)
                }
            } else {
                return 0;
            }
        };

        if needs_update {
            let bps = (delta_bytes as f64 / elapsed.as_secs_f64()) as i64;
            if let Ok(mut guard) = self.upload_speed_tracker.lock() {
                *guard = (now, current_bytes, bps);
            }
            bps
        } else {
            old_bps
        }
    }

    /// Checks if a peer is in the active connection swarm map.
    pub fn is_peer_in_swarm(&self, ip: &str) -> bool {
        !ip.is_empty() && self.peer_swarm.read().unwrap().contains_key(ip)
    }

    /// Counts how many peers in the swarm have unchoked us.
    pub fn number_of_unchoked_peers(&self) -> usize {
        self.unchoked_peers_count.load(Ordering::Relaxed)
    }

    /// Checks if the session is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused.wait_one(0)
    }

    /// Returns the number of blocks currently reserved/requested.
    pub fn number_of_reserved_blocks(&self) -> usize {
        self.assembler.requested_blocks.read().map(|r| r.len()).unwrap_or(0)
    }

    #[cfg(feature = "v2")]
    /// Validates an incoming 16 KiB block for BitTorrent v2 torrents against its Merkle tree or pieces root.
    pub fn verify_v2_block(&self, piece_number: u32, block_index: u32, block_data: &[u8]) -> Result<(), BitTorrentError> {
        let block_offset = (block_index as u64) * BLOCK_SIZE as u64;
        let global_offset = (piece_number as u64) * self.piece_length as u64 + block_offset;

        // Find the file that spans global_offset
        let file = self.files_to_download.iter().enumerate().find(|(_, f)| {
            global_offset >= f.offset && global_offset < f.offset + f.length
        });

        if let Some((file_idx, file_details)) = file {
            let file_rel_offset = global_offset - file_details.offset;
            let file_block_idx = (file_rel_offset / BLOCK_SIZE as u64) as usize;

            // 1. Check if we have a full MerkleTree registered for this file
            let trees = self.file_merkle_trees.lock().unwrap();
            if let Some(tree) = trees.get(&file_idx) {
                if !tree.verify_block(file_block_idx, block_data) {
                    return Err(BitTorrentError::Parse(format!(
                        "BEP 52 Merkle block hash mismatch for file {} at block {}",
                        file_idx, file_block_idx
                    )));
                }
                return Ok(());
            }

            // 2. Check if we have an audit path MerkleProof registered for this block
            {
                let proofs = self.file_merkle_proofs.lock().unwrap();
                if let Some(proof) = proofs.get(&(file_idx, file_block_idx)) {
                    if let Some(expected_root) = file_details.pieces_root {
                        if !proof.verify(block_data, &expected_root) {
                            return Err(BitTorrentError::Parse(format!(
                                "BEP 52 Merkle audit path proof mismatch for file {} at block {}",
                                file_idx, file_block_idx
                            )));
                        }
                        return Ok(());
                    }
                }
            }

            // 3. If single-block file (length <= 16 KiB) with pieces_root, root IS the leaf hash!
            if file_details.length <= BLOCK_SIZE as u64 {
                if let Some(expected_root) = file_details.pieces_root {
                    use sha2::Digest;
                    let actual_hash: [u8; 32] = sha2::Sha256::digest(block_data).into();
                    if actual_hash != expected_root {
                        return Err(BitTorrentError::Parse(format!(
                            "BEP 52 block hash mismatch for file {}: expected {:?}, got {:?}",
                            file_details.name, expected_root, actual_hash
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "v2")]
    /// Registers a MerkleTree for a file to enable strict block-by-block validation.
    pub fn register_file_merkle_tree(&self, file_index: usize, tree: crate::core::merkle::MerkleTree) {
        let mut trees = self.file_merkle_trees.lock().unwrap();
        trees.insert(file_index, tree);
    }

    #[cfg(feature = "v2")]
    /// Registers an individual block MerkleProof (audit path) for a file block.
    pub fn register_merkle_proof(&self, file_index: usize, proof: crate::core::merkle::MerkleProof) {
        let mut proofs = self.file_merkle_proofs.lock().unwrap();
        proofs.insert((file_index, proof.block_index), proof);
    }

    #[cfg(feature = "v2")]
    /// Registers a piece layer (list of 32-byte piece hashes) for a file identified by its `pieces_root`.
    pub fn register_piece_layer(&self, pieces_root: [u8; 32], layer: Vec<[u8; 32]>) {
        let mut layers = self.piece_layers.lock().unwrap();
        layers.insert(pieces_root, layer);
    }

    #[cfg(feature = "v2")]
    /// Retrieves the `pieces_root` hash for the specified file index, if present.
    pub fn get_file_pieces_root(&self, file_index: usize) -> Option<[u8; 32]> {
        self.files_to_download.get(file_index).and_then(|f| f.pieces_root)
    }
}

/// Interface Segregation Principle: Swarm read/query interface.
pub trait SwarmReadAccess: Send + Sync {
    fn connected_peer_count(&self) -> usize;
    fn contains_peer(&self, ip: &str) -> bool;
    fn is_peer_blacklisted(&self, ip: &str) -> bool;
}

/// Interface Segregation Principle: Piece progress write/mutation interface.
pub trait PieceWriteAccess: Send + Sync {
    fn is_piece_needed(&self, index: u32) -> bool;
    fn mark_piece_completed(&mut self, index: u32, piece_length: u32);
}

/// Interface Segregation Principle: Status and stats query view.
pub trait SessionStatusQuery: Send + Sync {
    fn status(&self) -> TorrentStatus;
    fn progress(&self) -> f32;
    fn bytes_downloaded(&self) -> u64;
    fn bytes_uploaded(&self) -> u64;
}

impl SwarmReadAccess for TorrentContext {
    fn connected_peer_count(&self) -> usize {
        self.peer_swarm.read().unwrap().len()
    }
    fn contains_peer(&self, ip: &str) -> bool {
        self.peer_swarm.read().unwrap().contains_key(ip)
    }
    fn is_peer_blacklisted(&self, ip: &str) -> bool {
        self.is_peer_blacklisted(ip)
    }
}

impl PieceWriteAccess for TorrentContext {
    fn is_piece_needed(&self, index: u32) -> bool {
        self.is_piece_missing(index)
    }
    fn mark_piece_completed(&mut self, index: u32, piece_length: u32) {
        self.update_bitfield_status(index, true, piece_length);
    }
}

impl SessionStatusQuery for TorrentContext {
    fn status(&self) -> TorrentStatus {
        self.status
    }
    fn progress(&self) -> f32 {
        if self.total_bytes_to_download == 0 {
            0.0
        } else {
            (self.total_bytes_downloaded.load(Ordering::Relaxed) as f32) / (self.total_bytes_to_download as f32)
        }
    }
    fn bytes_downloaded(&self) -> u64 {
        self.total_bytes_downloaded.load(Ordering::Relaxed)
    }
    fn bytes_uploaded(&self) -> u64 {
        self.total_bytes_uploaded.load(Ordering::Relaxed)
    }
}
