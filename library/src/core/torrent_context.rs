//! Torrent session context
//!
//! Models the state and configuration of an active torrent session, including
//! files, bitfield vectors, missing piece indices, and the connected peer swarm.

use crate::assembler::Assembler;
use crate::constants::{BLOCK_SIZE, ENDGAME_THRESHOLD};
use crate::error::BitTorrentError;
use crate::manual_reset_event::ManualResetEvent;
use crate::metainfo::{FileDetails, FilePriority, MetaInfoFile};
use crate::network::tracker::PeerDetails;
use crate::peer::Peer;
use crate::piece_buffer::PieceBuffer;
use crate::selector::PieceSelector;
use crate::session::SessionConfig;
use crate::util::get_bitfield_index_and_mask;
use sha1::Digest;
use std::cmp::min;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub use crate::core::piece_store::{PieceInfo, PieceRarity, PieceStore};
pub use crate::core::swarm_topology::SwarmTopology;
pub use crate::core::transfer_metrics::TransferMetrics;
pub use crate::core::metadata_bootstrap::MetadataBootstrap;

/// Placeholder structure representing a tracker client.
#[derive(Debug, Clone)]
pub struct Tracker;

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
    /// Cumulative count of pieces offered/suggested in super-seeding mode.
    pub total_pieces_offered: usize,
    /// Cumulative count of pieces verified as propagated across the swarm.
    pub total_pieces_propagated: usize,
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
    pub pex_dropped_peers: Mutex<Vec<PeerDetails>>,
    pub pex_added_peers_count: AtomicUsize,
    pub pex_dropped_peers_count: AtomicUsize,
    #[cfg(feature = "v2")]
    pub file_merkle_trees: Mutex<HashMap<usize, crate::core::merkle::MerkleTree>>,
    #[cfg(feature = "v2")]
    pub file_merkle_proofs: Mutex<HashMap<(usize, usize), crate::core::merkle::MerkleProof>>,
    #[cfg(feature = "v2")]
    pub piece_layers: Mutex<HashMap<[u8; 32], Vec<[u8; 32]>>>,
    #[cfg(feature = "v2")]
    pub verified_v2_hashes: Mutex<HashMap<([u8; 32], u32, u32), Vec<[u8; 32]>>>,
    #[cfg(feature = "v2")]
    pub v2_hash_requests_sent: AtomicUsize,
    #[cfg(feature = "v2")]
    pub v2_hash_requests_received: AtomicUsize,
    #[cfg(feature = "v2")]
    pub v2_hashes_sent: AtomicUsize,
    #[cfg(feature = "v2")]
    pub v2_hashes_received: AtomicUsize,
    #[cfg(feature = "v2")]
    pub v2_hash_rejects_sent: AtomicUsize,
    #[cfg(feature = "v2")]
    pub v2_hash_rejects_received: AtomicUsize,
    pub holepunch_rendezvous_sent: AtomicUsize,
    pub holepunch_rendezvous_received: AtomicUsize,
    pub holepunch_connect_sent: AtomicUsize,
    pub holepunch_connect_received: AtomicUsize,
    pub holepunch_error_sent: AtomicUsize,
    pub holepunch_error_received: AtomicUsize,
    #[cfg(feature = "webrtc")]
    pub webrtc_bridge: Arc<crate::network::webrtc::WebTorrentBridge>,
    #[cfg(all(feature = "std", feature = "v2"))]
    pub piece_layers_disk_cache: Arc<crate::storage::piece_layers_cache::PieceLayersDiskCache>,
    #[cfg(feature = "std")]
    pub verification_pool: Arc<crate::storage::verification_pool::HashVerificationPool>,
    pub pieces_verifying: Mutex<std::collections::HashSet<u32>>,
    /// BEP 53: 0-based file indices selected for download (`so=`).
    pub selected_file_indices: Option<Vec<usize>>,
    /// Expedited partial metadata manifest parsed early from initial metadata pieces.
    pub expedited_manifest: Option<crate::network::metadata_fetcher::PartialMetadataManifest>,
    /// Adaptive erasure staging cache managing tail blocks and straggler preemption in endgame mode.
    pub endgame_staging: Mutex<crate::storage::erasure_staging::AdaptiveErasureStaging>,
    /// Swarm-wide latency profiler tracking smoothed RTT and ECT for connected peers.
    pub latency_profiler: Mutex<crate::network::latency_profiler::SwarmLatencyProfiler>,
    /// BEP 38 WebSeed mirror health tracker managing dynamic endpoint scoring and fallback routing.
    pub webseed_tracker: Mutex<crate::network::webseed_tracker::WebSeedMirrorTracker>,
    // Domain aggregates (Phase 1 SOLID decomposition)
    pub piece_store: PieceStore,
    pub swarm_topology: SwarmTopology,
    pub transfer_metrics: TransferMetrics,
    pub metadata_bootstrap: MetadataBootstrap,
}

struct TorrentContextParams {
    pub info_hash: Vec<u8>,
    pub tracker_url: String,
    pub tracker_urls: Vec<String>,
    pub web_seeds: Vec<String>,
    pub is_private: bool,
    pub number_of_pieces: usize,
    pub piece_length: u32,
    pub pieces_info_hash: Vec<u8>,
    pub bitfield: Vec<u8>,
    pub pieces_missing: Vec<u8>,
    pub piece_data: Vec<PieceInfo>,
    pub files_to_download: Vec<FileDetails>,
    pub total_bytes_to_download: u64,
    pub status: TorrentStatus,
    pub file_name: String,
    pub selector: Arc<dyn PieceSelector>,
    pub storage: Arc<dyn crate::io_traits::BlockStorage>,
    pub download_path: std::path::PathBuf,
    pub config: SessionConfig,
    #[cfg(feature = "v2")]
    pub piece_layers: HashMap<[u8; 32], Vec<[u8; 32]>>,
}

impl TorrentContext {
    fn new_base(params: TorrentContextParams) -> Self {
        let piece_priority_queue = {
            let mut pq = std::collections::BinaryHeap::new();
            for piece_index in 0..params.number_of_pieces as u32 {
                pq.push(PieceRarity {
                    peer_count: 0,
                    piece_index,
                });
            }
            pq
        };

        let webseed_tracker = {
            let mut tracker = crate::network::webseed_tracker::WebSeedMirrorTracker::new(
                params.config.webseed_failure_threshold,
                params.config.webseed_base_cooldown_secs,
                params.config.webseed_max_cooldown_secs,
            );
            for ws in &params.web_seeds {
                tracker.register_mirror(ws);
            }
            Mutex::new(tracker)
        };

        #[cfg(all(feature = "std", feature = "v2"))]
        let piece_layers_disk_cache = {
            let cache_dir = params
                .config
                .v2_piece_layers_cache_dir
                .clone()
                .unwrap_or_else(|| params.download_path.join(".piece_layers"));
            let cache = Arc::new(
                crate::storage::piece_layers_cache::PieceLayersDiskCache::new(
                    &cache_dir,
                    params.config.v2_piece_layers_cache_max_mb,
                )
                .unwrap_or_else(|_| {
                    crate::storage::piece_layers_cache::PieceLayersDiskCache::new(
                        std::env::temp_dir().join("bt_piece_layers_fallback"),
                        params.config.v2_piece_layers_cache_max_mb,
                    )
                    .unwrap()
                }),
            );
            if params.config.v2_piece_layers_cache_enabled {
                for (root, layer) in &params.piece_layers {
                    let _ = cache.save_piece_layer(root, layer);
                }
            }
            cache
        };

        let total_bytes_downloaded = Arc::new(AtomicU64::new(0));
        let total_bytes_uploaded = Arc::new(AtomicU64::new(0));
        let mut transfer_metrics = TransferMetrics::new(params.total_bytes_to_download);
        transfer_metrics.total_bytes_downloaded = total_bytes_downloaded.clone();
        transfer_metrics.total_bytes_uploaded = total_bytes_uploaded.clone();

        let piece_store = PieceStore::new(
            params.number_of_pieces,
            params.piece_length,
            params.pieces_info_hash.clone(),
        );
        let swarm_topology = SwarmTopology::new(crate::constants::MAXIMUM_SWARM_SIZE);
        let metadata_bootstrap = MetadataBootstrap::new();

        TorrentContext {
            info_hash: params.info_hash,
            tracker_url: params.tracker_url,
            tracker_urls: params.tracker_urls,
            number_of_pieces: params.number_of_pieces,
            piece_length: params.piece_length,
            pieces_info_hash: params.pieces_info_hash,
            bitfield: params.bitfield,
            files_to_download: params.files_to_download,
            total_bytes_downloaded,
            initial_bytes_downloaded: 0,
            total_bytes_to_download: params.total_bytes_to_download,
            total_bytes_uploaded,
            status: params.status,
            file_name: params.file_name,
            main_tracker: None,
            callback_data: None,
            call_back: None,
            paused: ManualResetEvent::new(false),
            download_finished: Arc::new(ManualResetEvent::new(false)),
            selector: params.selector,
            peer_swarm: RwLock::new(HashMap::new()),
            missing_pieces_count: 0,
            maximum_swarm_size: crate::constants::MAXIMUM_SWARM_SIZE,
            assembler: Assembler::new(),
            pieces_missing: params.pieces_missing,
            piece_data: params.piece_data,
            bad_peer_scores: Mutex::new(HashMap::new()),
            storage: params.storage,
            super_seeding: SuperSeedingState {
                enabled: params.config.super_seeding,
                ..Default::default()
            },
            config: params.config.clone(),
            download_path: params.download_path,
            web_seeds: params.web_seeds,
            is_private: params.is_private,
            metadata_size: None,
            metadata_pieces: alloc::collections::BTreeMap::new(),
            requested_metadata_pieces: alloc::collections::BTreeMap::new(),
            info_dict_bytes: None,
            download_speed_tracker: Mutex::new((std::time::Instant::now(), 0, 0)),
            upload_speed_tracker: Mutex::new((std::time::Instant::now(), 0, 0)),
            piece_priority_queue: Mutex::new(piece_priority_queue),
            unchoked_peers_count: AtomicUsize::new(0),
            pex_dropped_peers: Mutex::new(Vec::new()),
            pex_added_peers_count: AtomicUsize::new(0),
            pex_dropped_peers_count: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            file_merkle_trees: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            file_merkle_proofs: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            piece_layers: Mutex::new(params.piece_layers),
            #[cfg(feature = "v2")]
            verified_v2_hashes: Mutex::new(HashMap::new()),
            #[cfg(feature = "v2")]
            v2_hash_requests_sent: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            v2_hash_requests_received: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            v2_hashes_sent: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            v2_hashes_received: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            v2_hash_rejects_sent: AtomicUsize::new(0),
            #[cfg(feature = "v2")]
            v2_hash_rejects_received: AtomicUsize::new(0),
            holepunch_rendezvous_sent: AtomicUsize::new(0),
            holepunch_rendezvous_received: AtomicUsize::new(0),
            holepunch_connect_sent: AtomicUsize::new(0),
            holepunch_connect_received: AtomicUsize::new(0),
            holepunch_error_sent: AtomicUsize::new(0),
            holepunch_error_received: AtomicUsize::new(0),
            #[cfg(feature = "webrtc")]
            webrtc_bridge: Arc::new(crate::network::webrtc::WebTorrentBridge::new()),
            #[cfg(all(feature = "std", feature = "v2"))]
            piece_layers_disk_cache,
            #[cfg(feature = "std")]
            verification_pool: Arc::new(
                crate::storage::verification_pool::HashVerificationPool::new(
                    params.config.verification_workers,
                    params.config.verification_queue_capacity,
                ),
            ),
            pieces_verifying: Mutex::new(std::collections::HashSet::new()),
            selected_file_indices: None,
            expedited_manifest: None,
            endgame_staging: Mutex::new(
                crate::storage::erasure_staging::AdaptiveErasureStaging::new(
                    params.config.endgame_max_clones_per_block,
                    params.config.endgame_straggler_ratio,
                ),
            ),
            latency_profiler: Mutex::new(
                crate::network::latency_profiler::SwarmLatencyProfiler::new(),
            ),
            webseed_tracker,
            piece_store,
            swarm_topology,
            transfer_metrics,
            metadata_bootstrap,
        }
    }

    /// Creates and initializes a `TorrentContext` from parsed metainfo, creating target directories and scanning existing disk files.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if the torrent contains no tracker URLs,
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
                piece_length,
            };
            number_of_pieces
        ];

        let context = Self::new_base(TorrentContextParams {
            info_hash,
            tracker_url,
            tracker_urls,
            web_seeds: web_seeds.clone(),
            is_private,
            number_of_pieces,
            piece_length,
            pieces_info_hash,
            bitfield,
            pieces_missing,
            piece_data,
            files_to_download: all_files_to_download,
            total_bytes_to_download: total_download_length,
            status: if seeding {
                TorrentStatus::Seeding
            } else {
                TorrentStatus::Initialised
            },
            file_name: torrent_meta_info
                .torrent_file_name
                .to_string_lossy()
                .to_string(),
            selector,
            storage,
            download_path: download_path.to_path_buf(),
            config,
            #[cfg(feature = "v2")]
            piece_layers: torrent_meta_info.get_piece_layers().clone(),
        });
        Ok(context)
    }

    /// Creates a placeholder `TorrentContext` for magnet links bootstrapping.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError`] if the tracker URLs list is empty or initialization fails.
    pub fn new_magnet_bootstrap(
        info_hash: Vec<u8>,
        tracker_urls: Vec<String>,
        selector: Arc<dyn PieceSelector>,
        download_path: &std::path::Path,
        config: SessionConfig,
    ) -> Result<Self, crate::error::BitTorrentError> {
        let tracker_url = tracker_urls.get(0).cloned().unwrap_or_default();
        let storage = Arc::new(crate::io_traits::NullBlockStorage);

        let context = Self::new_base(TorrentContextParams {
            info_hash,
            tracker_url,
            tracker_urls,
            web_seeds: Vec::new(),
            is_private: false,
            number_of_pieces: 0,
            piece_length: 0,
            pieces_info_hash: Vec::new(),
            bitfield: Vec::new(),
            pieces_missing: Vec::new(),
            piece_data: Vec::new(),
            files_to_download: Vec::new(),
            total_bytes_to_download: 0,
            status: TorrentStatus::Initialised,
            file_name: String::new(),
            selector,
            storage,
            download_path: download_path.to_path_buf(),
            config,
            #[cfg(feature = "v2")]
            piece_layers: HashMap::new(),
        });
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
        self.piece_store.init_pieces(number_of_pieces, piece_length);
    }

    /// Adds a BEP 19 WebSeed mirror URL to the context if not already present.
    /// Returns `true` if the URL was newly added.
    pub fn add_web_seed(&mut self, url: impl Into<String>) -> bool {
        let u = url.into();
        if !self.web_seeds.contains(&u) {
            self.web_seeds.push(u);
            true
        } else {
            false
        }
    }

    /// Adds an announce tracker URL to the context if not already present.
    /// Returns `true` if the URL was newly added.
    pub fn add_tracker_url(&mut self, url: impl Into<String>) -> bool {
        let u = url.into();
        if !self.tracker_urls.contains(&u) {
            if self.tracker_url.is_empty() {
                self.tracker_url = u.clone();
            }
            self.tracker_urls.push(u);
            true
        } else {
            false
        }
    }

    /// Transitions a magnet bootstrap context to a standard torrent download session once metadata is fetched.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError`] if decoding the info dictionary or parsing files fails,
    /// or if storage files cannot be created.
    pub fn transition_from_metadata(
        &mut self,
        info_dict_bytes: &[u8],
        download_path: &std::path::Path,
    ) -> Result<(), crate::error::BitTorrentError> {
        let info_node = crate::bencode::Bencode::decode(info_dict_bytes)?;

        let mut root_entries = Vec::new();
        root_entries.push((
            b"announce".as_slice(),
            crate::bencode::BNode::String(self.tracker_url.as_bytes()),
        ));
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
        let storage: Arc<dyn crate::io_traits::BlockStorage> = if self.config.disk_cache_size_mb > 0
        {
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

        let torrent_filename = format!(
            "{}.torrent",
            crate::util::info_hash_to_string(&self.info_hash)
        );
        let torrent_path = download_path.join(torrent_filename);
        let _ = std::fs::write(torrent_path, &torrent_bytes);

        self.info_dict_bytes = Some(info_dict_bytes.to_vec());

        if self.expedited_manifest.is_none() {
            self.expedited_manifest =
                crate::network::metadata_fetcher::PartialMetadataManifest::try_parse_partial(
                    info_dict_bytes,
                );
        }

        if let Some(ref selected) = self.selected_file_indices {
            for (idx, file) in self.files_to_download.iter_mut().enumerate() {
                if selected.contains(&idx) {
                    file.priority = FilePriority::Normal;
                } else {
                    file.priority = FilePriority::Skip;
                }
            }
        }

        {
            let mut pq = self.piece_priority_queue.lock().unwrap();
            pq.clear();
            for piece_index in 0..number_of_pieces as u32 {
                if self.is_piece_wanted(piece_index) && !self.is_piece_local(piece_index) {
                    pq.push(PieceRarity {
                        peer_count: 0,
                        piece_index,
                    });
                }
            }
        }

        Ok(())
    }

    /// Validates the structure and length constraints of context data fields.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if any validated field constraints are violated.
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
    /// Returns a [`BitTorrentError::Parse`] if the torrent is already downloading or ended.
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
    /// Returns a [`BitTorrentError::Parse`] if the torrent is not downloading or seeding.
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
    /// Returns a [`BitTorrentError::Parse`] if the torrent is not currently paused.
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
    /// Returns a [`BitTorrentError::Parse`] if the torrent has already been stopped.
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
        let total = if self.total_bytes_to_download > 0 {
            self.total_bytes_to_download
        } else {
            self.transfer_metrics.total_bytes_to_download
        };
        if total == 0 {
            return 10000;
        }
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let ppm = (downloaded * 10000) / total;
        ppm.min(10000) as u32
    }

    /// Computes percentage completion from bytes downloaded versus total.
    #[cfg(feature = "std")]
    pub fn progress_percent(&self) -> f32 {
        self.transfer_metrics.progress_percent()
    }

    /// Sets or clears a specific piece completion bit in the local bitfield.
    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.bitfield.len() {
            if local {
                self.bitfield[byte_index] |= bit_mask;
            } else {
                self.bitfield[byte_index] &= !bit_mask;
            }
        }
        self.piece_store.mark_piece_local(piece_number, local);
    }

    /// Checks if a specific piece has been fully downloaded and verified locally.
    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.bitfield.len() {
            return self.bitfield[byte_index] & bit_mask != 0;
        }
        self.piece_store.is_piece_local(piece_number)
    }

    /// Sets or clears a specific piece index in the missing pieces tracking vector.
    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.pieces_missing.len() {
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
        self.piece_store.mark_piece_missing(piece_number, missing);
    }

    /// Checks if a specific piece index is currently marked as missing.
    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.pieces_missing.len() {
            return self.pieces_missing[byte_index] & bit_mask != 0;
        }
        self.piece_store.is_piece_missing(piece_number)
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
                        self.piece_data[idx].peer_count =
                            self.piece_data[idx].peer_count.saturating_sub(1);
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
            self.total_bytes_downloaded
                .fetch_add(number_of_bytes as u64, Ordering::Relaxed);
        }
        self.set_piece_length(piece_number, number_of_bytes);
        self.mark_piece_local(piece_number, piece_there);
        self.mark_piece_missing(piece_number, !piece_there);
    }

    /// Returns the number of bytes remaining to be downloaded.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if the computed remaining bytes turn negative.
    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let total = if self.total_bytes_to_download > 0 {
            self.total_bytes_to_download
        } else {
            self.transfer_metrics.total_bytes_to_download
        };
        if total < downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(total - downloaded)
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
            min(
                piece_start + self.piece_length as u64,
                self.total_bytes_to_download,
            )
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
                        let count = self
                            .piece_data
                            .get(piece_index as usize)
                            .map(|p| p.peer_count)
                            .unwrap_or(0);
                        pq.push(PieceRarity {
                            peer_count: count,
                            piece_index,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Sets the list of selected 0-based file indices (BEP 53 `so=`).
    ///
    /// If files are already populated in this context, applies the selection immediately,
    /// setting unselected files to [`FilePriority::Skip`] and selected files to [`FilePriority::Normal`].
    pub fn set_selected_files(&mut self, indices: &[usize]) {
        let mut sorted = indices.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        self.selected_file_indices = Some(sorted.clone());
        if !self.files_to_download.is_empty() {
            self.apply_bep53_file_selection(&sorted);
        }
    }

    /// Applies BEP 53 file selection to existing files in this context.
    ///
    /// Elevates selected files to [`FilePriority::Normal`] if currently skipped,
    /// sets unselected files to [`FilePriority::Skip`], clears pending reservations
    /// for pieces that are no longer wanted, and re-seeds the piece priority queue.
    pub fn apply_bep53_file_selection(&mut self, selected_indices: &[usize]) {
        for (idx, file) in self.files_to_download.iter_mut().enumerate() {
            if selected_indices.contains(&idx) {
                if file.priority == FilePriority::Skip {
                    file.priority = FilePriority::Normal;
                }
            } else {
                file.priority = FilePriority::Skip;
            }
        }

        // Clear requests for pieces that became unwanted
        for piece in 0..self.number_of_pieces as u32 {
            if !self.is_piece_wanted(piece) {
                self.clear_piece_requests(piece);
            }
        }

        // Re-seed piece priority queue with wanted missing pieces
        if let Ok(mut pq) = self.piece_priority_queue.lock() {
            pq.clear();
            for piece_index in 0..self.number_of_pieces as u32 {
                if self.is_piece_wanted(piece_index) && !self.is_piece_local(piece_index) {
                    let count = self
                        .piece_data
                        .get(piece_index as usize)
                        .map(|p| p.peer_count)
                        .unwrap_or(0);
                    pq.push(PieceRarity {
                        peer_count: count,
                        piece_index,
                    });
                }
            }
        }
    }

    /// Returns the currently active BEP 53 selected file indices, if configured.
    pub fn selected_files(&self) -> Option<&[usize]> {
        self.selected_file_indices.as_deref()
    }

    /// Returns a BEP 53 selection evaluation comparing selected files to total torrent payload.
    pub fn bep53_evaluation(
        &self,
    ) -> Option<crate::network::metadata_fetcher::BEP53SelectionEvaluation> {
        if let Some(ref manifest) = self.expedited_manifest {
            let selected = self.selected_file_indices.as_deref().unwrap_or(&[]);
            return Some(manifest.evaluate_bep53_selection(selected));
        }

        if !self.files_to_download.is_empty() {
            let files = self
                .files_to_download
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    let start_piece = if self.piece_length == 0 {
                        0
                    } else {
                        (f.offset / self.piece_length as u64) as u32
                    };
                    let end_piece = if f.length == 0 || self.piece_length == 0 {
                        start_piece
                    } else {
                        ((f.offset + f.length - 1) / self.piece_length as u64) as u32
                    };
                    crate::network::metadata_fetcher::PartialFileInfo {
                        index: idx,
                        name: f.name.clone(),
                        length: f.length,
                        piece_range: (start_piece, end_piece),
                    }
                })
                .collect();

            let manifest = crate::network::metadata_fetcher::PartialMetadataManifest {
                name: self.file_name.clone(),
                piece_length: self.piece_length as u64,
                total_bytes: self.total_bytes_to_download,
                total_pieces: self.number_of_pieces as u32,
                files,
                has_complete_file_list: true,
            };
            let selected = self.selected_file_indices.as_deref().unwrap_or(&[]);
            return Some(manifest.evaluate_bep53_selection(selected));
        }

        None
    }

    /// Returns true if super-seeding mode (BEP 16) is actively running.
    pub fn is_super_seeding(&self) -> bool {
        self.super_seeding.enabled
            && (self.status == TorrentStatus::Seeding || self.is_download_complete())
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
            let swarm_count = self
                .super_seeding
                .piece_swarm_count
                .get(&p)
                .copied()
                .unwrap_or(0);
            (is_currently_offered, swarm_count, p)
        });

        let chosen_piece = candidate_pieces[0];
        self.super_seeding
            .piece_offered_to
            .insert(chosen_piece, peer_ip.to_string());
        self.super_seeding
            .peer_pending_piece
            .insert(peer_ip.to_string(), chosen_piece);
        self.super_seeding.total_pieces_offered += 1;

        Some(chosen_piece)
    }

    /// Evaluates incoming `Have` announcements from peers to validate piece propagation in super-seeding mode.
    ///
    /// If `reporting_peer_ip` received a piece from another peer to whom it was originally offered,
    /// the original peer is freed up and credited, and a new piece is selected for them.
    /// Returns `Some((offered_peer_ip, next_piece_for_offered_peer))` if a new piece should be sent to that peer.
    pub fn on_peer_have(
        &mut self,
        reporting_peer_ip: &str,
        piece_index: u32,
    ) -> Option<(String, u32)> {
        if !self.is_super_seeding() {
            return None;
        }

        // Record that this piece was seen in the swarm
        *self
            .super_seeding
            .piece_swarm_count
            .entry(piece_index)
            .or_insert(0) += 1;

        // Check if this piece was offered to someone
        if let Some(offered_peer_ip) = self
            .super_seeding
            .piece_offered_to
            .get(&piece_index)
            .cloned()
        {
            if offered_peer_ip != reporting_peer_ip {
                // The piece was successfully uploaded by offered_peer to another peer!
                if self.super_seeding.peer_pending_piece.get(&offered_peer_ip) == Some(&piece_index)
                {
                    self.super_seeding
                        .peer_pending_piece
                        .remove(&offered_peer_ip);
                    self.super_seeding.total_pieces_propagated += 1;

                    // Fetch offered_peer's bitfield to find their next piece
                    let remote_bitfield = {
                        if let Ok(swarm) = self.peer_swarm.read() {
                            swarm
                                .get(&offered_peer_ip)
                                .and_then(|p| p.lock().ok())
                                .map(|p| p.remote_piece_bitfield.clone())
                        } else {
                            None
                        }
                    };

                    if let Some(bitfield) = remote_bitfield {
                        if let Some(next_piece) =
                            self.offer_super_seed_piece(&offered_peer_ip, &bitfield)
                        {
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
            if self
                .super_seeding
                .piece_offered_to
                .get(&piece)
                .map(|ip| ip == peer_ip)
                .unwrap_or(false)
            {
                self.super_seeding.piece_offered_to.remove(&piece);
            }
        }
    }

    /// Returns the super-seeding distribution metrics: (total_pieces_offered, total_pieces_propagated).
    pub fn super_seed_stats(&self) -> (usize, usize) {
        (
            self.super_seeding.total_pieces_offered,
            self.super_seeding.total_pieces_propagated,
        )
    }

    /// Selects the next suggested piece for a peer in sequential or advisory seeding mode.
    pub fn suggest_next_piece_for_peer(&self, peer: &Peer) -> Option<u32> {
        let total_pieces = self.number_of_pieces as u32;
        if total_pieces == 0 {
            return None;
        }
        for piece in 0..total_pieces {
            if self.is_piece_local(piece) && !peer.is_piece_on_remote_peer(piece) {
                return Some(piece);
            }
        }
        None
    }

    /// Checks if a piece request from a peer is permissible in super-seeding mode.
    pub fn can_upload_piece_to_peer(&self, peer_ip: &str, piece_index: u32) -> bool {
        if !self.is_super_seeding() {
            return true;
        }
        self.super_seeding.peer_pending_piece.get(peer_ip) == Some(&piece_index)
            || self
                .super_seeding
                .piece_offered_to
                .get(&piece_index)
                .map(|ip| ip == peer_ip)
                .unwrap_or(false)
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
                        let (byte_idx, mask) =
                            crate::util::get_bitfield_index_and_mask(piece_idx as u32);
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
    /// Returns a [`BitTorrentError::Protocol`] if the block parameters are invalid or if hash verification fails.
    /// Returns a [`BitTorrentError::Io`] if writing to block storage fails.
    pub fn process_piece_block(
        &mut self,
        storage: &dyn crate::io_traits::BlockStorage,
        piece_number: u32,
        begin: u32,
        block_data: &[u8],
        peer_ip: &str,
    ) -> Result<bool, crate::error::BitTorrentError> {
        if piece_number >= self.number_of_pieces as u32 {
            return Err(crate::error::BitTorrentError::Protocol(
                "Invalid piece index".into(),
            ));
        }
        // Validate alignment before any early-return so misaligned blocks from any
        // peer are always rejected — even if we already have this piece locally.
        if begin % BLOCK_SIZE as u32 != 0 {
            return Err(crate::error::BitTorrentError::Protocol(
                "Block offset not aligned".into(),
            ));
        }
        let expected_piece_length = self.get_piece_length(piece_number);
        if begin
            .checked_add(block_data.len() as u32)
            .map_or(true, |end| end > expected_piece_length)
        {
            return Err(crate::error::BitTorrentError::Protocol(
                "Block out of piece bounds".into(),
            ));
        }

        if self.is_piece_local(piece_number) {
            return Ok(false);
        }
        let piece_length = self.get_piece_length(piece_number);
        let block_index = begin / BLOCK_SIZE as u32;

        if self.config.enable_adaptive_endgame {
            let mut staging = self.endgame_staging.lock().unwrap();
            match staging.on_block_arrived(piece_number, block_index, peer_ip) {
                crate::storage::erasure_staging::BlockArrivalOutcome::AbsorbDuplicate {
                    ..
                } => {
                    return Ok(false);
                }
                crate::storage::erasure_staging::BlockArrivalOutcome::CommitAndCancelOthers {
                    ..
                } => {}
            }
        }

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
            if self.config.enable_adaptive_endgame {
                self.endgame_staging
                    .lock()
                    .unwrap()
                    .clear_piece(piece_number);
            }
            #[cfg(feature = "std")]
            if self.config.enable_async_verification {
                let piece_offset = (piece_number as u64) * (self.piece_length as u64);
                let expected_v1_hash =
                    if (piece_number as usize * 20 + 20) <= self.pieces_info_hash.len() {
                        let start = piece_number as usize * 20;
                        let mut h = [0u8; 20];
                        h.copy_from_slice(&self.pieces_info_hash[start..start + 20]);
                        Some(h)
                    } else {
                        None
                    };

                self.mark_piece_verifying(piece_number, true);

                let job = crate::storage::verification_pool::PieceVerificationJob {
                    piece_number,
                    piece_length,
                    piece_offset,
                    expected_v1_hash,
                    expected_v2_root: None,
                    storage: Some(self.storage.clone()),
                    direct_buffer: None,
                    block_sources: block_sources.clone(),
                };

                if let Err(e) = self.verification_pool.submit(job) {
                    crate::log_debug!(
                        "[pool] failed to submit verification job: {}, falling back to sync",
                        e
                    );
                    self.mark_piece_verifying(piece_number, false);
                } else {
                    return Ok(true);
                }
            }

            if self.check_piece_hash_streaming(storage, piece_number, piece_length) {
                crate::log_debug!(
                    "[piece] {} passed hash verification ({} bytes)",
                    piece_number,
                    piece_length
                );
                let _ = storage.flush();
                self.update_bitfield_status(piece_number, true, piece_length);
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
            return Err(crate::error::BitTorrentError::Protocol(
                "Invalid piece index".into(),
            ));
        }
        if begin % BLOCK_SIZE as u32 != 0 {
            return Err(crate::error::BitTorrentError::Protocol(
                "Block offset not aligned".into(),
            ));
        }
        let expected_piece_length = self.get_piece_length(piece_number);
        if begin
            .checked_add(block_data.len() as u32)
            .map_or(true, |end| end > expected_piece_length)
        {
            return Err(crate::error::BitTorrentError::Protocol(
                "Block out of piece bounds".into(),
            ));
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
            self.peer_swarm.write().unwrap().insert(ip, peer);
            true
        } else {
            false
        }
    }

    /// Unregisters and drops a peer connection from the swarm by IP address.
    pub fn remove_peer(&mut self, ip: &str) {
        let peer_opt = if let Ok(mut swarm) = self.peer_swarm.try_write() {
            swarm.remove(ip)
        } else {
            None
        };
        if let Some(peer_arc) = peer_opt {
            if let Ok(mut peer_guard) = peer_arc.try_lock() {
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
            crate::log_debug!(
                "[swarm] Blacklisting peer {} due to repeated bad blocks",
                ip
            );
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
        let peers: Vec<_> = {
            if let Ok(mut swarm) = self.peer_swarm.write() {
                let peers = swarm.values().cloned().collect();
                swarm.clear();
                peers
            } else {
                Vec::new()
            }
        };
        for peer in peers {
            if let Ok(mut p) = peer.try_lock() {
                p.close();
            }
        }
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
                piece_number,
                piece_length,
                self.piece_length
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
        self.assembler
            .reserve_block_request(piece_number, block_index)
    }

    /// Releases a block reservation, allowing other peers to request it.
    pub fn release_block_request(&self, piece_number: u32, block_index: u32) {
        self.assembler
            .release_block_request(piece_number, block_index);
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
            // Priority 0: Straggler Preemption in Adaptive Endgame
            if self.config.enable_adaptive_endgame {
                let stalled_blocks = self.endgame_staging.lock().unwrap().detect_stalled_blocks();
                for (stalled_piece, stalled_bi, slow_peer) in stalled_blocks {
                    if slow_peer != peer.ip
                        && (stalled_piece as usize) < self.number_of_pieces
                        && self.is_piece_wanted(stalled_piece)
                        && !self.is_piece_local(stalled_piece)
                        && peer.is_piece_on_remote_peer(stalled_piece)
                        && !peer
                            .reserved_blocks
                            .iter()
                            .any(|&(p, b, _)| p == stalled_piece && b == stalled_bi)
                    {
                        let piece_length = self.get_piece_length(stalled_piece);
                        let begin = stalled_bi * block_size as u32;
                        let length = min(block_size as u32, piece_length.saturating_sub(begin));
                        let ect = self
                            .latency_profiler
                            .lock()
                            .unwrap()
                            .get_or_create(&peer.ip)
                            .estimated_completion_time(block_size);
                        self.endgame_staging.lock().unwrap().stage_request(
                            stalled_piece,
                            stalled_bi,
                            &peer.ip,
                            ect,
                        );
                        self.reserve_block_request(stalled_piece, stalled_bi);
                        return Some((stalled_piece, begin, length));
                    }
                }
            }

            // Endgame mode: request blocks across rarest wanted pieces.
            let mut pieces: Vec<u32> = (0..self.number_of_pieces as u32)
                .filter(|&piece| {
                    self.is_piece_wanted(piece)
                        && !self.is_piece_local(piece)
                        && peer.is_piece_on_remote_peer(piece)
                })
                .collect();
            pieces.sort_by_key(|&piece| {
                (
                    std::cmp::Reverse(self.get_piece_priority(piece)),
                    self.piece_data[piece as usize].peer_count,
                    piece,
                )
            });

            for piece_number in pieces {
                let piece_length = self.get_piece_length(piece_number);
                let block_count = ((piece_length as usize + block_size - 1) / block_size) as u32;

                let piece_buffers = self.assembler.piece_buffers.lock().unwrap();
                let present_blocks = piece_buffers
                    .get(&piece_number)
                    .map(|pb| {
                        let guard = pb.lock().unwrap();
                        (0..block_count)
                            .map(|bi| guard.is_block_present(bi))
                            .collect::<Vec<bool>>()
                    })
                    .unwrap_or_else(|| vec![false; block_count as usize]);
                drop(piece_buffers);

                for block_index in 0..block_count {
                    if present_blocks[block_index as usize] {
                        continue;
                    }
                    if peer
                        .reserved_blocks
                        .iter()
                        .any(|&(p, b, _)| p == piece_number && b == block_index)
                    {
                        continue;
                    }
                    if self.config.enable_adaptive_endgame {
                        if !self
                            .endgame_staging
                            .lock()
                            .unwrap()
                            .can_clone_block(piece_number, block_index)
                        {
                            continue;
                        }
                    }
                    let begin = block_index * block_size as u32;
                    let length = min(block_size as u32, piece_length.saturating_sub(begin));
                    if self.config.enable_adaptive_endgame {
                        let ect = self
                            .latency_profiler
                            .lock()
                            .unwrap()
                            .get_or_create(&peer.ip)
                            .estimated_completion_time(block_size);
                        self.endgame_staging.lock().unwrap().stage_request(
                            piece_number,
                            block_index,
                            &peer.ip,
                            ect,
                        );
                    }
                    self.reserve_block_request(piece_number, block_index);
                    return Some((piece_number, begin, length));
                }
            }
        } else {
            // Priority 1: Check suggested pieces from this peer (BEP 6 & BEP 16 super-seeding optimization)
            for &suggested in &peer.suggested_pieces {
                if (suggested as usize) < self.number_of_pieces
                    && self.is_piece_wanted(suggested)
                    && !self.is_piece_local(suggested)
                    && peer.is_piece_on_remote_peer(suggested)
                {
                    if let Some((begin, length)) = self.next_pending_block(suggested) {
                        let block_index = begin / block_size as u32;
                        if self.reserve_block_request(suggested, block_index) {
                            return Some((suggested, begin, length));
                        }
                    }
                }
            }

            // Priority 2: Standard mode - rarest-first with unrequested blocks.
            let best = (0..self.number_of_pieces as u32)
                .filter(|&piece| {
                    self.is_piece_wanted(piece)
                        && !self.is_piece_local(piece)
                        && peer.is_piece_on_remote_peer(piece)
                })
                .filter_map(|piece| self.next_pending_block(piece).map(|block| (piece, block)))
                .min_by_key(|&(piece, _)| {
                    (
                        std::cmp::Reverse(self.get_piece_priority(piece)),
                        self.piece_data[piece as usize].peer_count,
                        piece,
                    )
                });

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
    pub fn next_allowed_fast_block_request_for_peer(
        &mut self,
        peer: &Peer,
    ) -> Option<(u32, u32, u32)> {
        if self.status != TorrentStatus::Downloading {
            return None;
        }
        let block_size = self.config.block_size;

        // Priority 1: Pieces that are both suggested and allowed-fast
        for &piece in &peer.suggested_pieces {
            if peer.allowed_fast_pieces.contains(&piece)
                && (piece as usize) < self.number_of_pieces
                && self.is_piece_wanted(piece)
                && !self.is_piece_local(piece)
                && peer.is_piece_on_remote_peer(piece)
            {
                if let Some((begin, length)) = self.next_pending_block(piece) {
                    let block_index = begin / block_size as u32;
                    if self.reserve_block_request(piece, block_index) {
                        return Some((piece, begin, length));
                    }
                }
            }
        }

        // Priority 2: Standard allowed-fast pieces
        let best = peer
            .allowed_fast_pieces
            .iter()
            .copied()
            .filter(|&piece| {
                (piece as usize) < self.number_of_pieces
                    && self.is_piece_wanted(piece)
                    && !self.is_piece_local(piece)
                    && peer.is_piece_on_remote_peer(piece)
            })
            .filter_map(|piece| self.next_pending_block(piece).map(|block| (piece, block)))
            .min_by_key(|&(piece, _)| {
                (
                    std::cmp::Reverse(self.get_piece_priority(piece)),
                    self.piece_data[piece as usize].peer_count,
                    piece,
                )
            });

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

    /// Estimates the current download transfer rate in bytes per second with EMA smoothing.
    pub fn bytes_per_second(&self) -> i64 {
        self.transfer_metrics.bytes_per_second()
    }

    /// Estimates the current upload transfer rate in bytes per second with EMA smoothing.
    pub fn upload_bytes_per_second(&self) -> i64 {
        self.transfer_metrics.upload_bytes_per_second()
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
        self.assembler
            .requested_blocks
            .read()
            .map(|r| r.len())
            .unwrap_or(0)
    }

    #[cfg(feature = "v2")]
    /// Validates an incoming 16 KiB block for BitTorrent v2 torrents against its Merkle tree or pieces root.
    pub fn verify_v2_block(
        &self,
        piece_number: u32,
        block_index: u32,
        block_data: &[u8],
    ) -> Result<(), BitTorrentError> {
        let block_offset = (block_index as u64) * BLOCK_SIZE as u64;
        let global_offset = (piece_number as u64) * self.piece_length as u64 + block_offset;

        // Find the file that spans global_offset
        let file = self
            .files_to_download
            .iter()
            .enumerate()
            .find(|(_, f)| global_offset >= f.offset && global_offset < f.offset + f.length);

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
                    return Ok(());
                }
            }

            // 4. Check if we have verified leaf hashes cached for this block
            if let Some(expected_root) = file_details.pieces_root {
                let verified_cache = self.verified_v2_hashes.lock().unwrap();
                for (&(root, base_layer, chunk_index), hashes) in verified_cache.iter() {
                    if root == expected_root && base_layer == 0 {
                        let c_idx = chunk_index as usize;
                        if file_block_idx >= c_idx && file_block_idx < c_idx + hashes.len() {
                            let expected_leaf = hashes[file_block_idx - c_idx];
                            use sha2::Digest;
                            let actual_hash: [u8; 32] = sha2::Sha256::digest(block_data).into();
                            if actual_hash != expected_leaf {
                                return Err(BitTorrentError::Parse(format!(
                                    "BEP 52 Merkle verified leaf mismatch for file {} at block {}",
                                    file_idx, file_block_idx
                                )));
                            }
                            return Ok(());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "v2")]
    /// Registers a MerkleTree for a file to enable strict block-by-block validation.
    pub fn register_file_merkle_tree(
        &self,
        file_index: usize,
        tree: crate::core::merkle::MerkleTree,
    ) {
        #[cfg(all(feature = "std", feature = "v2"))]
        if self.config.v2_piece_layers_cache_enabled {
            let _ = self
                .piece_layers_disk_cache
                .save_layers(&tree.root(), tree.layers());
        }
        let mut trees = self.file_merkle_trees.lock().unwrap();
        trees.insert(file_index, tree);
    }

    #[cfg(feature = "v2")]
    /// Registers an individual block MerkleProof (audit path) for a file block.
    pub fn register_merkle_proof(
        &self,
        file_index: usize,
        proof: crate::core::merkle::MerkleProof,
    ) {
        let mut proofs = self.file_merkle_proofs.lock().unwrap();
        proofs.insert((file_index, proof.block_index), proof);
    }

    #[cfg(feature = "v2")]
    /// Registers a piece layer (list of 32-byte piece hashes) for a file identified by its `pieces_root`.
    pub fn register_piece_layer(&self, pieces_root: [u8; 32], layer: Vec<[u8; 32]>) {
        #[cfg(all(feature = "std", feature = "v2"))]
        if self.config.v2_piece_layers_cache_enabled {
            let _ = self
                .piece_layers_disk_cache
                .save_piece_layer(&pieces_root, &layer);
        }
        let mut layers = self.piece_layers.lock().unwrap();
        layers.insert(pieces_root, layer);
    }

    #[cfg(feature = "v2")]
    /// Retrieves the `pieces_root` hash for the specified file index, if present.
    pub fn get_file_pieces_root(&self, file_index: usize) -> Option<[u8; 32]> {
        self.files_to_download
            .get(file_index)
            .and_then(|f| f.pieces_root)
    }

    #[cfg(feature = "v2")]
    /// Returns counts of (requests_sent, requests_received, hashes_sent, hashes_received, rejects_sent, rejects_received).
    pub fn v2_hash_stats(&self) -> (usize, usize, usize, usize, usize, usize) {
        (
            self.v2_hash_requests_sent.load(Ordering::Relaxed),
            self.v2_hash_requests_received.load(Ordering::Relaxed),
            self.v2_hashes_sent.load(Ordering::Relaxed),
            self.v2_hashes_received.load(Ordering::Relaxed),
            self.v2_hash_rejects_sent.load(Ordering::Relaxed),
            self.v2_hash_rejects_received.load(Ordering::Relaxed),
        )
    }

    #[cfg(all(feature = "std", feature = "v2"))]
    /// Returns a snapshot of current telemetry metrics from the persistent piece layers cache.
    pub fn piece_layers_cache_metrics(
        &self,
    ) -> crate::storage::piece_layers_cache::PieceLayersCacheMetrics {
        self.piece_layers_disk_cache.metrics()
    }

    /// Checks whether a piece is currently in flight undergoing background hash verification.
    pub fn is_piece_verifying(&self, piece_number: u32) -> bool {
        self.piece_store.is_piece_verifying(piece_number)
    }

    /// Marks or unmarks a piece as currently undergoing background verification.
    pub fn mark_piece_verifying(&self, piece_number: u32, verifying: bool) {
        if let Ok(mut guard) = self.pieces_verifying.lock() {
            if verifying {
                guard.insert(piece_number);
            } else {
                guard.remove(&piece_number);
            }
        }
        self.piece_store.set_piece_verifying(piece_number, verifying);
    }

    /// Applies a completed background piece verification result.
    ///
    /// Updates the local bitfield, flushes storage, and broadcasts Have messages if valid.
    /// Discards cached storage range and reports bad peers if corrupted.
    pub fn apply_piece_verification_result(
        &mut self,
        result: &crate::storage::verification_pool::PieceVerificationResult,
        storage: &dyn crate::io_traits::BlockStorage,
    ) -> bool {
        let piece_number = result.piece_number;
        let piece_length = result.piece_length;
        self.mark_piece_verifying(piece_number, false);

        if result.is_valid {
            crate::log_debug!(
                "[piece] {} passed async hash verification ({} bytes, {} us)",
                piece_number,
                piece_length,
                result.elapsed_us
            );
            let _ = storage.flush();
            self.update_bitfield_status(piece_number, true, piece_length);
            // Broadcast Have to all connected peers
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
            true
        } else {
            crate::log_debug!(
                "[piece] {} failed async hash verification ({:?})",
                piece_number,
                result.error
            );
            self.clear_piece_requests(piece_number);

            // Discard failed piece dirty blocks from cache so corrupt data never touches disk
            let piece_offset = (piece_number as u64) * (self.piece_length as u64);
            storage.discard_range(piece_offset, piece_length as u64);

            // Report all peers that contributed blocks to this piece
            for source_ip_opt in &result.block_sources {
                if let Some(ip) = source_ip_opt {
                    self.report_bad_peer(ip);
                }
            }

            self.mark_piece_missing(piece_number, true);
            false
        }
    }

    /// Drains all completed verification results currently in the worker pool results queue,
    /// applying their status to bitfields, disk cache, and swarm Have broadcasts.
    ///
    /// Returns the number of verification results processed.
    pub fn drain_verification_results(
        &mut self,
        storage: &dyn crate::io_traits::BlockStorage,
    ) -> usize {
        #[cfg(feature = "std")]
        {
            let results = self.verification_pool.poll_results();
            let count = results.len();
            for res in results {
                self.apply_piece_verification_result(&res, storage);
            }
            count
        }
        #[cfg(not(feature = "std"))]
        {
            0
        }
    }

    /// Captures a snapshot of current verification pool telemetry metrics.
    #[cfg(feature = "std")]
    pub fn verification_metrics(
        &self,
    ) -> crate::storage::verification_pool::VerificationPoolMetrics {
        self.verification_pool.metrics()
    }

    /// Triggers parallel multi-threaded rechecking of all pieces in this torrent.
    #[cfg(feature = "std")]
    pub fn recheck_all_pieces(
        &mut self,
    ) -> Result<Arc<crate::storage::verification_pool::RecheckProgress>, BitTorrentError> {
        let storage = self.storage.clone();
        let num_pieces = self.number_of_pieces as u32;
        let piece_length = self.piece_length;
        let total_bytes = self.total_bytes_to_download;
        let pieces_info_hash = self.pieces_info_hash.clone();

        self.verification_pool.recheck_pieces_parallel(
            num_pieces,
            piece_length,
            total_bytes,
            &pieces_info_hash,
            storage,
        )
    }

    #[cfg(feature = "v2")]
    /// Handles an incoming BEP 52 `hash_request`, attempting to generate the requested hashes response.
    ///
    /// First checks in-memory registered trees. On miss, queries the persistent on-disk
    /// piece layers cache (`PieceLayersDiskCache`) or reconstructs from known piece layers.
    /// Returns `Ok(hashes_bytes)` on success, or `Err(())` if the request cannot be fulfilled and must be rejected.
    pub fn handle_hash_request(
        &self,
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    ) -> Result<Vec<u8>, ()> {
        // 1. Check in-memory registered trees
        {
            let trees = self.file_merkle_trees.lock().unwrap();
            for tree in trees.values() {
                if tree.root() == pieces_root {
                    if let Some(resp) =
                        tree.generate_hashes_response(base_layer, index, length, proof_layers)
                    {
                        return Ok(resp);
                    }
                }
            }

            for (file_idx, file_details) in self.files_to_download.iter().enumerate() {
                if file_details.pieces_root == Some(pieces_root) {
                    if let Some(tree) = trees.get(&file_idx) {
                        if let Some(resp) =
                            tree.generate_hashes_response(base_layer, index, length, proof_layers)
                        {
                            return Ok(resp);
                        }
                    }
                }
            }
        }

        // 2. Query persistent PieceLayersDiskCache
        #[cfg(all(feature = "std", feature = "v2"))]
        if self.config.v2_piece_layers_cache_enabled {
            if let Some(resp) = self.piece_layers_disk_cache.serve_hash_request(
                &pieces_root,
                base_layer,
                index,
                length,
                proof_layers,
            ) {
                return Ok(resp);
            }
        }

        // 3. Fallback: check piece_layers map and attempt reconstruction
        let piece_layers_guard = self.piece_layers.lock().unwrap();
        if let Some(piece_layer) = piece_layers_guard.get(&pieces_root) {
            #[cfg(all(feature = "std", feature = "v2"))]
            if self.config.v2_piece_layers_cache_enabled {
                let _ = self
                    .piece_layers_disk_cache
                    .save_piece_layer(&pieces_root, piece_layer);
                if let Some(resp) = self.piece_layers_disk_cache.serve_hash_request(
                    &pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                ) {
                    return Ok(resp);
                }
            }

            #[cfg(all(feature = "std", feature = "v2"))]
            {
                let (reconstructed, calc_root) =
                    crate::storage::piece_layers_cache::reconstruct_tree_from_piece_layer(
                        piece_layer,
                    );
                if calc_root == pieces_root {
                    if let Some(resp) =
                        crate::storage::piece_layers_cache::generate_hashes_response_from_layers(
                            &reconstructed,
                            base_layer,
                            index,
                            length,
                            proof_layers,
                        )
                    {
                        return Ok(resp);
                    }
                }
            }
        }

        Err(())
    }

    #[cfg(feature = "v2")]
    /// Handles an incoming BEP 52 `hashes` message, validating the cryptographic Merkle proof against `pieces_root`.
    ///
    /// On success, caches verified base hashes in `verified_v2_hashes`.
    pub fn handle_hashes_response(
        &self,
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
        hashes_data: &[u8],
    ) -> Result<Vec<[u8; 32]>, BitTorrentError> {
        let verified = crate::core::merkle::MerkleTree::verify_hashes_response(
            &pieces_root,
            base_layer,
            index,
            length,
            proof_layers,
            hashes_data,
        )?;

        // Cache the verified hashes
        {
            let mut cache = self.verified_v2_hashes.lock().unwrap();
            cache.insert((pieces_root, base_layer, index), verified.clone());
        }

        Ok(verified)
    }

    /// Returns true if PEX (Peer Exchange / BEP 11) is enabled for this torrent.
    pub fn is_pex_enabled(&self) -> bool {
        self.config.enable_pex && !self.is_private
    }

    /// Returns true if the Holepunch extension (BEP 55 / ut_holepunch) is enabled.
    pub fn is_holepunch_enabled(&self) -> bool {
        self.config.enable_holepunch
    }

    /// Returns counts of (rendezvous_sent, rendezvous_rcv, connect_sent, connect_rcv, error_sent, error_rcv).
    pub fn holepunch_stats(&self) -> (usize, usize, usize, usize, usize, usize) {
        (
            self.holepunch_rendezvous_sent.load(Ordering::Relaxed),
            self.holepunch_rendezvous_received.load(Ordering::Relaxed),
            self.holepunch_connect_sent.load(Ordering::Relaxed),
            self.holepunch_connect_received.load(Ordering::Relaxed),
            self.holepunch_error_sent.load(Ordering::Relaxed),
            self.holepunch_error_received.load(Ordering::Relaxed),
        )
    }

    /// Handles an incoming BEP 55 Rendezvous message from an initiator peer requesting a connection bridge to `target`.
    ///
    /// Validates that:
    /// 1. Target is not the relay itself (returns `NoSelf`).
    /// 2. Target peer is currently connected in `peer_swarm` (returns `NotConnected`).
    /// 3. Target peer supports the `ut_holepunch` extension (returns `NoSupport`).
    ///
    /// If valid, generates a pair of `Connect` messages for the initiator and target peers.
    pub fn handle_holepunch_rendezvous(
        &self,
        initiator_ip: &str,
        initiator_port: u16,
        target: core::net::SocketAddr,
    ) -> Result<
        (
            crate::network::holepunch::HolepunchMessage,
            crate::network::holepunch::HolepunchMessage,
        ),
        crate::network::holepunch::HolepunchErrorCode,
    > {
        use crate::network::holepunch::{HolepunchErrorCode, HolepunchMessage};

        let target_ip_str = target.ip().to_string();

        // 1. Check NoSelf: target cannot be the relaying peer itself
        if target.port() == self.config.listen_port
            && (target_ip_str == "127.0.0.1"
                || target_ip_str == "0.0.0.0"
                || target_ip_str == "::1")
        {
            return Err(HolepunchErrorCode::NoSelf);
        }

        // 2. Check if target peer is connected in the swarm
        let swarm = self.peer_swarm.read().unwrap();
        let target_peer_arc = swarm
            .get(&target_ip_str)
            .ok_or(HolepunchErrorCode::NotConnected)?;
        let target_peer = target_peer_arc
            .lock()
            .map_err(|_| HolepunchErrorCode::NotConnected)?;

        // 3. Check if target peer supports ut_holepunch
        if !target_peer.supports_holepunch
            && !target_peer.extension_ids.contains_key("ut_holepunch")
        {
            return Err(HolepunchErrorCode::NoSupport);
        }

        // 4. Construct dual Connect messages
        let initiator_addr: core::net::SocketAddr = match initiator_ip.parse() {
            Ok(ip) => core::net::SocketAddr::new(ip, initiator_port),
            Err(_) => return Err(HolepunchErrorCode::NoSuchPeer),
        };

        let connect_to_initiator = HolepunchMessage::connect(target);
        let connect_to_target = HolepunchMessage::connect(initiator_addr);

        Ok((connect_to_initiator, connect_to_target))
    }

    /// Returns a snapshot of WebTorrent WebRTC bridge statistics and telemetry.
    #[cfg(feature = "webrtc")]
    pub fn webtorrent_bridge_stats(&self) -> crate::network::webrtc::WebTorrentBridgeStats {
        self.webrtc_bridge.stats()
    }

    /// Records a cross-swarm data transfer between BitTorrent (TCP/uTP) and WebTorrent (WebRTC).
    #[cfg(feature = "webrtc")]
    pub fn record_webrtc_bridge_transfer(&self, from_webrtc: bool, bytes: u64) {
        self.webrtc_bridge
            .record_bridge_transfer(from_webrtc, bytes);
    }

    /// Returns `true` if WebTorrent support is enabled in configuration.
    pub fn is_webtorrent_enabled(&self) -> bool {
        self.config.enable_webtorrent
    }

    /// Returns `true` if WebTorrent cross-swarm bridge mode is enabled.
    pub fn is_webtorrent_bridge_enabled(&self) -> bool {
        self.config.enable_webtorrent && self.config.webtorrent_bridge_mode
    }

    /// Records dropped peers received from a PEX message.
    pub fn record_pex_dropped(&self, peers: &[PeerDetails]) {
        if peers.is_empty() {
            return;
        }
        if let Ok(mut dropped) = self.pex_dropped_peers.lock() {
            dropped.extend_from_slice(peers);
            if dropped.len() > 100 {
                let overflow = dropped.len() - 100;
                dropped.drain(0..overflow);
            }
        }
        self.pex_dropped_peers_count
            .fetch_add(peers.len(), Ordering::Relaxed);
    }

    /// Records added peers received from a PEX message.
    pub fn record_pex_added(&self, count: usize) {
        self.pex_added_peers_count
            .fetch_add(count, Ordering::Relaxed);
    }

    /// Generates a BEP 11 PEX message comparing the active swarm against `sent_peers`.
    pub fn generate_pex_message(
        &self,
        sent_peers: &HashMap<String, u16>,
        exclude_ip: Option<&str>,
    ) -> (crate::network::pex::PexMessage, HashMap<String, u16>) {
        use crate::network::pex::{PexFlags, PexMessage};

        let mut pex_msg = PexMessage::new();
        let mut newly_sent = HashMap::new();

        if !self.is_pex_enabled() {
            return (pex_msg, newly_sent);
        }

        let swarm = self.peer_swarm.read().unwrap();
        let mut added_candidates = Vec::new();

        for (ip, p_arc) in swarm.iter() {
            if let Some(exc) = exclude_ip {
                if ip == exc {
                    continue;
                }
            }

            if let Ok(p_guard) = p_arc.try_lock() {
                let port = p_guard.port;
                newly_sent.insert(ip.clone(), port);

                if !sent_peers.contains_key(ip) {
                    let mut flags = 0u8;
                    if p_guard.is_seed(self.number_of_pieces) {
                        flags |= PexFlags::SEED;
                    }
                    flags |= PexFlags::OUTGOING;

                    let details = PeerDetails {
                        info_hash: self.info_hash.clone(),
                        peer_id: None,
                        ip: ip.clone(),
                        port,
                    };
                    added_candidates.push((details, PexFlags::new(flags)));
                }
            }
        }

        // Dropped peers: peers previously in sent_peers that are no longer in swarm
        let mut dropped_candidates = Vec::new();
        for (ip, &port) in sent_peers {
            if !newly_sent.contains_key(ip) {
                let details = PeerDetails {
                    info_hash: self.info_hash.clone(),
                    peer_id: None,
                    ip: ip.clone(),
                    port,
                };
                dropped_candidates.push(details);
            }
        }

        // Partition into IPv4 and IPv6, applying BEP 11 maximum 50 added / 50 dropped limit
        let mut added_count = 0;
        for (details, flags) in added_candidates {
            if added_count >= 50 {
                break;
            }
            if details.is_ipv6() {
                if self.config.enable_ipv6 {
                    pex_msg.add_v6(details, flags);
                    added_count += 1;
                }
            } else {
                pex_msg.add_v4(details, flags);
                added_count += 1;
            }
        }

        let mut dropped_count = 0;
        for details in dropped_candidates {
            if dropped_count >= 50 {
                break;
            }
            if details.is_ipv6() {
                if self.config.enable_ipv6 {
                    pex_msg.drop_v6(details);
                    dropped_count += 1;
                }
            } else {
                pex_msg.drop_v4(details);
                dropped_count += 1;
            }
        }

        (pex_msg, newly_sent)
    }

    /// Records an RTT block delivery sample for a peer in the latency profiler.
    pub fn record_peer_block_sample(&self, peer_ip: &str, rtt: std::time::Duration, bytes: usize) {
        self.latency_profiler
            .lock()
            .unwrap()
            .record_sample(peer_ip, rtt, bytes);
    }

    /// Records a block timeout for a peer in the latency profiler.
    pub fn record_peer_block_timeout(&self, peer_ip: &str) {
        self.latency_profiler
            .lock()
            .unwrap()
            .record_timeout(peer_ip);
    }

    /// Records a block request dispatch for a peer in the latency profiler.
    pub fn record_peer_request_sent(&self, peer_ip: &str) {
        self.latency_profiler
            .lock()
            .unwrap()
            .record_request_sent(peer_ip);
    }

    /// Returns telemetry metrics for the adaptive endgame erasure staging cache.
    pub fn endgame_staging_metrics(
        &self,
    ) -> crate::storage::erasure_staging::EndgameStagingMetrics {
        self.endgame_staging.lock().unwrap().metrics()
    }

    /// Returns a swarm latency profile summary for all connected peers.
    pub fn swarm_latency_summary(
        &self,
    ) -> Vec<crate::network::latency_profiler::PeerLatencySummary> {
        self.latency_profiler
            .lock()
            .unwrap()
            .summary(self.config.endgame_straggler_ratio)
    }

    /// Returns a list of health summaries for all registered WebSeed mirrors.
    pub fn webseed_mirror_summary(
        &self,
    ) -> Vec<crate::network::webseed_tracker::MirrorHealthSummary> {
        self.webseed_tracker.lock().unwrap().summaries()
    }

    /// Registers a new WebSeed mirror URL at runtime.
    pub fn add_webseed(&mut self, url: impl Into<String>) {
        let u = url.into();
        if !self.web_seeds.contains(&u) {
            self.web_seeds.push(u.clone());
        }
        self.webseed_tracker.lock().unwrap().register_mirror(u);
    }

    /// Removes a WebSeed mirror URL from active tracking and routing.
    pub fn remove_webseed(&mut self, url: &str) -> bool {
        self.web_seeds.retain(|ws| ws != url);
        self.webseed_tracker.lock().unwrap().remove_mirror(url)
    }

    /// Returns a reference to the `PieceStore` aggregate.
    pub fn pieces(&self) -> &PieceStore {
        &self.piece_store
    }

    /// Returns a mutable reference to the `PieceStore` aggregate.
    pub fn pieces_mut(&mut self) -> &mut PieceStore {
        &mut self.piece_store
    }

    /// Returns a reference to the `SwarmTopology` aggregate.
    pub fn swarm(&self) -> &SwarmTopology {
        &self.swarm_topology
    }

    /// Returns a mutable reference to the `SwarmTopology` aggregate.
    pub fn swarm_mut(&mut self) -> &mut SwarmTopology {
        &mut self.swarm_topology
    }

    /// Returns a reference to the `TransferMetrics` aggregate.
    pub fn metrics(&self) -> &TransferMetrics {
        &self.transfer_metrics
    }

    /// Returns a mutable reference to the `TransferMetrics` aggregate.
    pub fn metrics_mut(&mut self) -> &mut TransferMetrics {
        &mut self.transfer_metrics
    }

    /// Returns a reference to the `MetadataBootstrap` aggregate.
    pub fn metadata(&self) -> &MetadataBootstrap {
        &self.metadata_bootstrap
    }

    /// Returns a mutable reference to the `MetadataBootstrap` aggregate.
    pub fn metadata_mut(&mut self) -> &mut MetadataBootstrap {
        &mut self.metadata_bootstrap
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
            (self.total_bytes_downloaded.load(Ordering::Relaxed) as f32)
                / (self.total_bytes_to_download as f32)
        }
    }
    fn bytes_downloaded(&self) -> u64 {
        self.total_bytes_downloaded.load(Ordering::Relaxed)
    }
    fn bytes_uploaded(&self) -> u64 {
        self.total_bytes_uploaded.load(Ordering::Relaxed)
    }
}

impl crate::core::views::PieceAvailabilityView for TorrentContext {
    fn number_of_pieces(&self) -> usize {
        self.number_of_pieces
    }

    fn piece_length(&self, piece: u32) -> u32 {
        self.get_piece_length(piece)
    }

    fn is_piece_local(&self, piece: u32) -> bool {
        self.is_piece_local(piece)
    }

    fn is_piece_wanted(&self, piece: u32) -> bool {
        self.is_piece_wanted(piece)
    }

    fn is_piece_verifying(&self, piece: u32) -> bool {
        self.is_piece_verifying(piece)
    }

    fn peer_count_for_piece(&self, piece: u32) -> usize {
        self.get_piece_peer_count(piece)
    }

    fn pop_rarest_piece(&self, peer: &dyn crate::core::views::RemotePeerPieceView) -> Option<u32> {
        let mut pq = self.piece_priority_queue.lock().unwrap();
        let mut temporary_list = Vec::new();
        let mut selected = None;

        while let Some(rarity) = pq.peek() {
            let piece = rarity.piece_index;
            if !self.is_piece_wanted(piece) {
                pq.pop();
                continue;
            }
            if self.is_piece_local(piece) || self.is_piece_verifying(piece) {
                pq.pop();
                continue;
            }
            if self.get_piece_peer_count(piece) != rarity.peer_count {
                pq.pop();
                continue;
            }
            if peer.is_piece_on_remote_peer(piece) {
                selected = Some(piece);
                break;
            } else {
                temporary_list.push(pq.pop().unwrap());
            }
        }

        for item in temporary_list {
            pq.push(item);
        }

        selected
    }
}

impl crate::core::views::SwarmView for TorrentContext {
    fn connected_peers_count(&self) -> usize {
        self.peer_swarm.read().unwrap().len()
    }

    fn unchoked_peers_count(&self) -> usize {
        self.unchoked_peers_count.load(Ordering::Relaxed)
    }

    fn for_each_peer(&self, f: &mut dyn FnMut(&str, &crate::core::views::PeerSnapshot)) {
        if let Ok(swarm) = self.peer_swarm.read() {
            for (ip, peer_arc) in swarm.iter() {
                if let Ok(p) = peer_arc.lock() {
                    let snapshot = crate::core::views::PeerSnapshot {
                        ip: p.ip.clone(),
                        port: p.port,
                        am_choking: p.am_choking,
                        am_interested: p.am_interested,
                        peer_choking: p.peer_choking.wait_one(0),
                        peer_interested: p.peer_interested,
                        download_rate: p.rolling_download_rate,
                        upload_rate: p.rolling_upload_rate,
                    };
                    f(ip, &snapshot);
                }
            }
        }
    }
}

impl crate::core::views::TransferStatsView for TorrentContext {
    fn total_downloaded(&self) -> u64 {
        self.total_bytes_downloaded.load(Ordering::Relaxed)
    }

    fn total_uploaded(&self) -> u64 {
        self.total_bytes_uploaded.load(Ordering::Relaxed)
    }

    fn current_download_rate(&self) -> u64 {
        self.bytes_per_second().max(0) as u64
    }

    fn current_upload_rate(&self) -> u64 {
        self.upload_bytes_per_second().max(0) as u64
    }

    fn progress_percent(&self) -> f32 {
        self.progress_percent()
    }
}

