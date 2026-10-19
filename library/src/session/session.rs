//! Torrent download session management
//!
//! Orchestrates the active downloading/seeding processes for a torrent. Spawn
//! workers to establish connections, request/download blocks, process messages,
//! and handle disk writes.
//!
//! Configuration lives in [`super::config`]; builder types live in [`super::builder`].

use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::disk_io::DiskIO;
use crate::error::BitTorrentError;
use crate::magnet::MagnetLink;
use crate::manager::Manager;
use crate::metainfo::{FilePriority, MetaInfoFile};
use crate::peer::Peer;
use crate::selector::PieceSelector;
use crate::storage::fast_resume::FastResumeData;
use crate::tracker::{PeerDetails, Tracker};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::log_debug;
#[cfg(feature = "nat-pmp")]
use crate::nat::PortMapper;

pub use crate::session::worker;
use crate::session::worker::delay;

use core::future::Future;
use core::pin::Pin;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

pub use super::builder::{TorrentSessionBuilder, TorrentSource};
pub use super::config::SessionConfig;

/// Statistics snapshot of an active torrent session.
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub progress: f32,
    pub status: TorrentStatus,
    pub bytes_downloaded: u64,
    pub bytes_uploaded: u64,
    pub bytes_left: u64,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub connected_peers: usize,
}

/// Represents an active torrent transfer session.
pub struct TorrentSession {
    pub context: Arc<Mutex<TorrentContext>>,
    pub download_path: PathBuf,
    pub peer_workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    pub task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    pub executor_thread: Option<thread::JoinHandle<()>>,
    pub manager: Option<Arc<Manager>>,
    #[cfg(feature = "dht")]
    pub dht: Option<Arc<crate::dht::Dht>>,
    #[cfg(feature = "nat-pmp")]
    pub nat_pmp: Option<Arc<dyn crate::nat::PortMapper>>,
}

impl TorrentSession {
    /// Creates a builder to configure and construct a new `TorrentSession` from a `.torrent` file.
    pub fn builder(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
    ) -> TorrentSessionBuilder {
        TorrentSessionBuilder::new(torrent_path, download_path)
    }

    /// Creates a builder to configure and construct a new `TorrentSession` from a magnet link.
    pub fn from_magnet(
        magnet_link: impl Into<String>,
        download_path: impl AsRef<Path>,
    ) -> TorrentSessionBuilder {
        TorrentSessionBuilder::from_magnet(magnet_link, download_path)
    }

    /// Creates and initializes a new `TorrentSession` using options.
    pub(crate) fn new_with_options_internal(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
        seeding: bool,
        config: SessionConfig,
        selector: Arc<dyn PieceSelector>,
        fast_resume: Option<FastResumeData>,
    ) -> Result<Self, BitTorrentError> {
        let torrent_path = torrent_path.as_ref();
        let download_path = download_path.as_ref().to_path_buf();
        fs::create_dir_all(&download_path)?;

        let mut meta_info = MetaInfoFile::new(torrent_path)?;
        meta_info.parse()?;
        meta_info.validate()?;

        let piece_length = meta_info.get_piece_length()?;
        let (_, files_to_download) = meta_info.local_files_to_download_list(&download_path)?;
        let disk_io = Arc::new(DiskIO::new(
            &download_path,
            files_to_download.clone(),
            piece_length,
        ));
        let storage: Arc<dyn crate::io_traits::BlockStorage> = if config.disk_cache_size_mb > 0 {
            Arc::new(crate::storage::disk_cache::CachedBlockStorage::new(
                disk_io.clone(),
                config.disk_cache_size_mb * 1024 * 1024,
            ))
        } else {
            disk_io.clone()
        };
        let context = Arc::new(Mutex::new(TorrentContext::new(
            &meta_info,
            selector,
            storage,
            &download_path,
            seeding,
            config.clone(),
        )?));

        disk_io.create_local_torrent_structure()?;

        let mut resumed = false;
        if let Some(mut resume) = fast_resume {
            if resume.files.len() == files_to_download.len() {
                for (entry, file) in resume.files.iter_mut().zip(&files_to_download) {
                    if entry.path.as_os_str().is_empty() {
                        entry.path = PathBuf::from(&file.name);
                    }
                }
            }
            if resume.verify_on_disk(&download_path).unwrap_or(false) {
                if resume
                    .apply_to_context(&mut context.lock().unwrap())
                    .is_ok()
                {
                    resumed = true;
                }
            }
        }

        if !resumed {
            if seeding {
                disk_io.fully_downloaded_torrent_bitfield(&mut context.lock().unwrap())?;
                let total = context.lock().unwrap().total_bytes_to_download;
                context
                    .lock()
                    .unwrap()
                    .total_bytes_downloaded
                    .store(total, std::sync::atomic::Ordering::Relaxed);
                context.lock().unwrap().initial_bytes_downloaded = total;
            } else {
                if !config.skip_hash_check {
                    disk_io.create_torrent_bitfield(&mut context.lock().unwrap())?;
                } else {
                    let mut ctx = context.lock().unwrap();
                    let number_of_pieces = ctx.number_of_pieces;
                    let piece_length = ctx.piece_length;
                    let mut total_bytes_left = ctx.total_bytes_to_download as i64;
                    for i in 0..number_of_pieces as u32 {
                        ctx.mark_piece_missing(i, true);
                        if total_bytes_left / piece_length as i64 != 0 {
                            ctx.set_piece_length(i, piece_length);
                        } else {
                            ctx.set_piece_length(i, total_bytes_left as u32);
                        }
                        total_bytes_left -= piece_length as i64;
                    }
                }
                let downloaded = context
                    .lock()
                    .unwrap()
                    .total_bytes_downloaded
                    .load(std::sync::atomic::Ordering::Relaxed);
                context.lock().unwrap().initial_bytes_downloaded = downloaded;
            }
        }

        let (task_tx, task_rx) =
            std::sync::mpsc::channel::<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>();
        let executor_thread = spawn_executor(context.clone(), task_rx);
        spawn_stats_loop_standard(context.clone(), task_tx.clone());

        let session = TorrentSession {
            context,
            download_path,
            peer_workers: Arc::new(Mutex::new(Vec::new())),
            task_tx,
            executor_thread: Some(executor_thread),
            manager: None,
            #[cfg(feature = "dht")]
            dht: None,
            #[cfg(feature = "nat-pmp")]
            nat_pmp: None,
        };

        session.context.lock().unwrap().validate()?;
        config.choking_strategy.spawn_choking_loop(
            session.context.clone(),
            session.task_tx.clone(),
            None,
        );

        Ok(session)
    }

    /// Creates and initializes a new `TorrentSession` using a magnet link and options.
    pub(crate) fn new_magnet_with_options_internal(
        magnet_link: &str,
        download_path: impl AsRef<Path>,
        config: SessionConfig,
        selector: Arc<dyn PieceSelector>,
        custom_web_seeds: Vec<String>,
        auto_inject_webtorrent: bool,
        selected_files: Option<Vec<usize>>,
    ) -> Result<Self, BitTorrentError> {
        let mut magnet = MagnetLink::parse(magnet_link)?;
        let download_path = download_path.as_ref().to_path_buf();
        fs::create_dir_all(&download_path)?;

        for ws in custom_web_seeds {
            magnet.add_web_seed(ws);
        }

        let context = Arc::new(Mutex::new(TorrentContext::new_magnet_bootstrap(
            magnet.info_hash.clone(),
            magnet.trackers.clone(),
            selector,
            &download_path,
            config.clone(),
        )?));

        // Inject WebSeeds and WebTorrent WebSocket trackers
        crate::network::webtorrent_injector::WebTorrentMagnetInjector::inject_into_shared_context(
            &magnet,
            &context,
            auto_inject_webtorrent && (config.enable_webtorrent || config.webtorrent_bridge_mode),
        );

        if let Some(ref sel) = selected_files {
            context.lock().unwrap().set_selected_files(sel);
        } else if magnet.has_file_selection() {
            context
                .lock()
                .unwrap()
                .set_selected_files(magnet.selected_indices());
        }

        let (task_tx, task_rx) =
            std::sync::mpsc::channel::<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>();
        let executor_thread = spawn_executor(context.clone(), task_rx);
        spawn_stats_loop_magnet(context.clone(), task_tx.clone());

        let session = TorrentSession {
            context,
            download_path,
            peer_workers: Arc::new(Mutex::new(Vec::new())),
            task_tx,
            executor_thread: Some(executor_thread),
            manager: None,
            #[cfg(feature = "dht")]
            dht: None,
            #[cfg(feature = "nat-pmp")]
            nat_pmp: None,
        };

        config.choking_strategy.spawn_choking_loop(
            session.context.clone(),
            session.task_tx.clone(),
            None,
        );

        Ok(session)
    }

    /// Dynamically injects one or more BEP 19 WebSeed mirror URLs into the active session.
    ///
    /// If the WebSeed download loop is not yet running and the session is downloading,
    /// it will automatically start the background WebSeed download loop.
    /// Returns the number of newly added WebSeeds.
    pub fn inject_web_seeds(&mut self, web_seeds: Vec<String>) -> usize {
        let mut added = 0;
        let mut should_spawn_webseed = false;
        {
            let mut ctx = self.context.lock().unwrap();
            for ws in web_seeds {
                if ctx.add_web_seed(ws) {
                    added += 1;
                }
            }
            if added > 0 && ctx.status == TorrentStatus::Downloading && !ctx.web_seeds.is_empty() {
                should_spawn_webseed = true;
            }
        }
        #[cfg(feature = "webseed")]
        if should_spawn_webseed {
            let active_seeds = self.context.lock().unwrap().web_seeds.clone();
            let handle = crate::webseed::start_webseed_loop(self.context.clone(), active_seeds);
            self.peer_workers.lock().unwrap().push(handle);
        }
        added
    }

    /// Dynamically injects WebTorrent WebSocket tracker URLs into the active session.
    ///
    /// If WebTorrent bridge mode is active, triggers initial offer generation.
    /// Returns the number of newly added WebSocket trackers.
    pub fn inject_websocket_trackers(&mut self, trackers: Vec<String>) -> usize {
        let mut added = 0;
        {
            let mut ctx = self.context.lock().unwrap();
            for tr in trackers {
                if crate::network::webtorrent_injector::WebTorrentMagnetInjector::is_websocket_tracker(&tr) {
                    if ctx.add_tracker_url(tr) {
                        added += 1;
                    }
                }
            }
            #[cfg(feature = "webrtc")]
            if added > 0 && ctx.config.webtorrent_bridge_mode {
                let _ = ctx.webrtc_bridge.generate_offers(4);
            }
        }
        added
    }

    /// Returns a list of currently active WebSeed mirror URLs registered with this session.
    pub fn web_seeds(&self) -> Vec<String> {
        self.context.lock().unwrap().web_seeds.clone()
    }

    /// Returns a list of currently active WebTorrent WebSocket tracker URLs registered with this session.
    pub fn websocket_trackers(&self) -> Vec<String> {
        self.context
            .lock()
            .unwrap()
            .tracker_urls
            .iter()
            .filter(|u| {
                crate::network::webtorrent_injector::WebTorrentMagnetInjector::is_websocket_tracker(
                    u,
                )
            })
            .cloned()
            .collect()
    }

    /// Returns a reference-counted mutex guarding the active torrent's context.
    pub fn context(&self) -> Arc<Mutex<TorrentContext>> {
        self.context.clone()
    }

    /// Returns true if super-seeding mode (BEP 16) is active.
    pub fn is_super_seeding(&self) -> bool {
        self.context.lock().unwrap().is_super_seeding()
    }

    /// Enables or disables super-seeding mode (BEP 16).
    pub fn set_super_seeding(&mut self, enabled: bool) {
        self.context.lock().unwrap().set_super_seeding(enabled);
    }

    /// Creates a seekable streaming reader ([`TorrentStream`](crate::storage::stream::TorrentStream)) over the entire torrent.
    ///
    /// Automatically installs a [`StreamingPieceSelector`](crate::core::selector::StreamingPieceSelector) into the session context
    /// so that active peer downloads dynamically prioritize the streaming cursor piece and lookahead window.
    pub fn create_stream(&self) -> Result<crate::storage::stream::TorrentStream, BitTorrentError> {
        let (storage, total_len, cursor) = {
            let mut ctx = self.context.lock().unwrap();
            let storage = ctx.storage.clone();
            let total_len = ctx.total_bytes_to_download;
            let cursor = Arc::new(core::sync::atomic::AtomicU32::new(0));
            ctx.selector = Arc::new(crate::core::selector::StreamingPieceSelector::new(
                cursor.clone(),
                8,
            ));
            (storage, total_len, cursor)
        };

        Ok(crate::storage::stream::TorrentStream::new(
            self.context.clone(),
            Arc::new(crate::storage::stream::StorageBlockReader(storage)),
            0,
            total_len,
            Some(cursor),
        ))
    }

    /// Creates a seekable streaming reader ([`TorrentStream`](crate::storage::stream::TorrentStream)) for a specific file within the torrent.
    ///
    /// Automatically installs a [`StreamingPieceSelector`](crate::core::selector::StreamingPieceSelector) into the session context
    /// so that active peer downloads dynamically prioritize the streaming cursor piece and lookahead window.
    pub fn create_file_stream(
        &self,
        file_index: usize,
    ) -> Result<crate::storage::stream::TorrentStream, BitTorrentError> {
        let cursor = Arc::new(core::sync::atomic::AtomicU32::new(0));
        {
            let mut ctx = self.context.lock().unwrap();
            ctx.selector = Arc::new(crate::core::selector::StreamingPieceSelector::new(
                cursor.clone(),
                8,
            ));
        }

        crate::storage::stream::TorrentStream::for_file(
            self.context.clone(),
            file_index,
            Some(cursor),
        )
    }

    /// Generates BEP 47 fast-resume data capturing current completion state and on-disk file metadata.
    pub fn generate_fast_resume(&self) -> Result<FastResumeData, BitTorrentError> {
        let ctx = self.context.lock().map_err(|_| {
            BitTorrentError::Parse("Failed to acquire torrent context lock for fast resume".into())
        })?;
        FastResumeData::from_context(&ctx, &self.download_path)
    }

    /// Saves BEP 47 fast-resume data directly to the specified file path on disk.
    pub fn save_fast_resume(&self, path: impl AsRef<Path>) -> Result<(), BitTorrentError> {
        let resume = self.generate_fast_resume()?;
        resume.save_to_file(path)
    }

    /// Transitions the session status to `Downloading`, commencing active peer-based downloading.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent status is already `Seeding`.
    ///
    /// # Examples
    /// ```no_run
    /// # use bittorrent_rs::TorrentSession;
    /// # fn run(mut session: TorrentSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session.start_download()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn start_download(&mut self) -> Result<(), BitTorrentError> {
        {
            let mut context = self.context.lock().unwrap();
            if context.status == TorrentStatus::Seeding {
                return Err(BitTorrentError::Parse(
                    "Cannot start download while torrent is configured for seeding.".into(),
                ));
            }
            context.start_downloading()?;
        }

        let context = self.context.lock().unwrap();
        let config = context.config.clone();
        let info_hash = context.info_hash.clone();
        drop(context);

        let (peer_tx, peer_rx): (
            std::sync::mpsc::Sender<crate::tracker::PeerDetails>,
            std::sync::mpsc::Receiver<crate::tracker::PeerDetails>,
        ) = std::sync::mpsc::channel();
        let context_clone = self.context.clone();
        let manager_clone = self.manager.clone();
        let peer_workers = self.peer_workers.clone();

        let max_connections = config.max_connections;
        thread::spawn(move || {
            while let Ok(peer_details) = peer_rx.recv() {
                let pg = context_clone.lock().unwrap();
                if pg.status != TorrentStatus::Ended && pg.status != TorrentStatus::Paused {
                    if !pg.peer_swarm.read().unwrap().contains_key(&peer_details.ip) {
                        // Prune finished worker handles and enforce connection limit
                        {
                            let mut workers_guard = peer_workers.lock().unwrap();
                            workers_guard.retain(|h| !h.is_finished());
                            if workers_guard.len() >= max_connections {
                                continue; // At capacity; drop this candidate
                            }
                        }
                        let ctx2 = context_clone.clone();
                        let mgr2 = manager_clone.clone();
                        let workers = peer_workers.clone();
                        let handle = thread::spawn(move || {
                            futures::executor::block_on(self::worker::handle_peer_session(
                                peer_details,
                                ctx2,
                                mgr2,
                            ));
                        });
                        workers.lock().unwrap().push(handle);
                    }
                }
            }
        });

        // Start LSD (Local Service Discovery) listener and announcer
        #[cfg(feature = "lsd")]
        {
            let is_private = self.context.lock().unwrap().is_private;
            if !is_private || config.allow_private_lsd {
                let lsd_listener = crate::lsd::LsdListener::new(info_hash.clone(), peer_tx.clone());
                let _lsd_listener_handle = lsd_listener.start();

                let lsd_announcer =
                    crate::lsd::LsdAnnouncer::new(info_hash.clone(), config.listen_port);
                let _lsd_announcer_handle = lsd_announcer.start(self.context.clone());
            }
        }

        #[cfg(feature = "dht")]
        {
            let is_private = self.context.lock().unwrap().is_private;
            if config.dht_enabled && self.dht.is_none() && !is_private {
                if let Ok(d) = crate::dht::Dht::new(config.dht_port) {
                    let _ = d.start();
                    d.bootstrap();

                    let d_arc = Arc::new(d);
                    self.dht = Some(d_arc.clone());

                    let mut ih = [0u8; 20];
                    if info_hash.len() == 20 {
                        ih.copy_from_slice(&info_hash);
                    }

                    d_arc.lookup_peers(ih, peer_tx.clone());
                }
            }
        }

        // Start WebSeeding loop if available
        #[cfg(feature = "webseed")]
        {
            let web_seeds = self.context.lock().unwrap().web_seeds.clone();
            if !web_seeds.is_empty() {
                let webseed_handle =
                    crate::webseed::start_webseed_loop(self.context.clone(), web_seeds);
                self.peer_workers.lock().unwrap().push(webseed_handle);
            }
        }

        #[cfg(feature = "nat-pmp")]
        {
            let gateway = crate::nat::get_default_gateway();
            let nat_client = Arc::new(crate::nat::FallbackPortMapper::new(gateway));
            self.nat_pmp = Some(nat_client.clone());
            let context_clone = self.context.clone();
            let port = config.listen_port;
            thread::spawn(move || {
                loop {
                    let status = {
                        if let Ok(ctx) = context_clone.try_lock() {
                            ctx.status
                        } else {
                            TorrentStatus::Downloading
                        }
                    };
                    if status == TorrentStatus::Ended {
                        break;
                    }
                    let _ = nat_client.request_mapping(true, port, port, 3600);
                    let _ = nat_client.request_mapping(false, port, port, 3600);
                    std::thread::sleep(Duration::from_secs(1800));
                }
            });
        }

        // Wire tracker automatically
        let mut tracker = Tracker::new(self.context.clone())?;
        tracker.set_peer_swarm_queue(peer_tx);
        if let Some(ref mgr) = self.manager {
            tracker.set_peer_manager(mgr.clone());
        }
        let _ = self.start_reannounce_loop(tracker);

        Ok(())
    }

    /// Pauses the torrent download session.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent is not downloading or seeding.
    ///
    /// # Examples
    /// ```no_run
    /// # use bittorrent_rs::TorrentSession;
    /// # fn run(mut session: TorrentSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session.pause()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn pause(&mut self) -> Result<(), BitTorrentError> {
        self.context.lock().unwrap().pause()
    }

    /// Resumes the torrent download session from a paused state.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the torrent is not currently paused.
    ///
    /// # Examples
    /// ```no_run
    /// # use bittorrent_rs::TorrentSession;
    /// # fn run(mut session: TorrentSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session.resume()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn resume(&mut self) -> Result<(), BitTorrentError> {
        self.context.lock().unwrap().resume()
    }

    /// Stops the session, disconnecting all connected peers and releasing resources.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`](crate::error::BitTorrentError::Parse) if the session has already ended.
    ///
    /// # Examples
    /// ```no_run
    /// # use bittorrent_rs::TorrentSession;
    /// # fn run(mut session: TorrentSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session.stop()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn stop(&mut self) -> Result<(), BitTorrentError> {
        #[cfg(feature = "dht")]
        if let Some(ref d) = self.dht {
            d.stop();
        }
        #[cfg(feature = "nat-pmp")]
        if let Some(ref nat) = self.nat_pmp {
            let nat_clone = nat.clone();
            let port = self.context.lock().unwrap().config.listen_port;
            thread::spawn(move || {
                let _ = nat_clone.release_mapping(true, port);
                let _ = nat_clone.release_mapping(false, port);
            });
        }
        let _ = self.flush_disk_cache();
        let context = self.context.lock().unwrap();
        context.disconnect_all_peers();
        drop(context);
        self.context.lock().unwrap().stop()
    }

    /// Flushes any pending dirty blocks in the in-memory write cache out to disk.
    pub fn flush_disk_cache(&self) -> Result<(), BitTorrentError> {
        let storage = self.context.lock().unwrap().storage.clone();
        storage.flush()
    }

    /// Returns the current state (e.g. paused, downloading, seeding) of the torrent.
    pub fn status(&self) -> TorrentStatus {
        self.context.lock().unwrap().status
    }

    /// Creates and returns a new `Tracker` linked to this session.
    pub fn tracker(&self) -> Result<crate::tracker::Tracker, BitTorrentError> {
        crate::tracker::Tracker::new(self.context.clone())
    }

    /// Returns the percentage of download completion (0.0 to 100.0).
    pub fn progress(&self) -> f32 {
        self.context.lock().unwrap().progress_percent()
    }

    /// Returns a snapshot of statistics for the current session.
    pub fn stats(&self) -> SessionStats {
        let ctx = self.context.lock().unwrap();
        let progress = ctx.progress_percent();
        let status = ctx.status;
        let bytes_downloaded = ctx
            .total_bytes_downloaded
            .load(std::sync::atomic::Ordering::Relaxed);
        let bytes_uploaded = ctx
            .total_bytes_uploaded
            .load(std::sync::atomic::Ordering::Relaxed);
        let bytes_left = ctx.bytes_left_to_download().unwrap_or(0);
        let download_speed = ctx.bytes_per_second() as f64;
        let upload_speed = ctx.upload_bytes_per_second() as f64;
        let connected_peers = ctx.peer_swarm.read().unwrap().len();

        SessionStats {
            progress,
            status,
            bytes_downloaded,
            bytes_uploaded,
            bytes_left,
            download_speed,
            upload_speed,
            connected_peers,
        }
    }

    /// Returns the local piece completion status and swarm-wide piece availability counts.
    pub fn piece_availability(&self) -> (Vec<bool>, Vec<u16>) {
        self.context.lock().unwrap().get_piece_availability()
    }

    /// Exports the session's torrent into BitTorrent v2 or Hybrid `.torrent` format.
    #[cfg(feature = "v2")]
    pub fn export_torrent(
        &self,
        format: crate::core::torrent_upgrader::TorrentExportFormat,
    ) -> Result<(Vec<u8>, crate::core::torrent_upgrader::TorrentUpgradeReport), BitTorrentError>
    {
        let (files, download_path, name, mut piece_length, trackers, web_seeds, is_private) = {
            let ctx = self.context.lock().unwrap();
            (
                ctx.files_to_download.clone(),
                ctx.download_path.clone(),
                ctx.file_name.clone(),
                ctx.piece_length as usize,
                ctx.tracker_urls.clone(),
                ctx.web_seeds.clone(),
                ctx.is_private,
            )
        };

        // Ensure piece length is power of two >= 16 KiB for v2/hybrid
        if format != crate::core::torrent_upgrader::TorrentExportFormat::V1 {
            if piece_length < crate::core::merkle::BLOCK_SIZE
                || (piece_length & (piece_length - 1)) != 0
            {
                let mut valid_len = crate::core::merkle::BLOCK_SIZE;
                while valid_len < piece_length {
                    valid_len <<= 1;
                }
                piece_length = valid_len;
            }
        }

        let mut files_data = Vec::with_capacity(files.len());
        for file in &files {
            let full_path = download_path.join(&file.torrent_path);
            let alt_path = download_path.join(&file.name);
            let path_to_read = if full_path.exists() {
                full_path
            } else if alt_path.exists() {
                alt_path
            } else {
                full_path
            };

            let data = if path_to_read.exists() {
                std::fs::read(&path_to_read)?
            } else {
                let mut buf = vec![0u8; file.length as usize];
                let ctx = self.context.lock().unwrap();
                ctx.storage.read_block(file.offset, &mut buf)?;
                buf
            };
            files_data.push((file.torrent_path.clone(), data));
        }

        let mut builder = crate::core::torrent_builder::TorrentBuilder::new(&name)
            .piece_length(piece_length)
            .private(is_private)
            .trackers(trackers);

        for ws in web_seeds {
            builder = builder.web_seed(ws);
        }

        let mut total_size = 0u64;
        for (p, d) in &files_data {
            total_size += d.len() as u64;
            builder = builder.add_file(p, d.clone());
        }

        let torrent_bytes = match format {
            crate::core::torrent_upgrader::TorrentExportFormat::V1 => builder.build_v1()?,
            crate::core::torrent_upgrader::TorrentExportFormat::V2 => builder.build_v2()?,
            crate::core::torrent_upgrader::TorrentExportFormat::Hybrid => builder.build_hybrid()?,
        };

        let mut metainfo = crate::core::metainfo::MetaInfoFile::from_bytes(&torrent_bytes);
        metainfo.parse()?;

        let v1_info_hash = if format == crate::core::torrent_upgrader::TorrentExportFormat::V2 {
            None
        } else {
            metainfo
                .get_v1_info_hash()
                .ok()
                .map(|h| crate::util::info_hash_to_string(&h))
        };
        let v2_info_hash = if format == crate::core::torrent_upgrader::TorrentExportFormat::V1 {
            None
        } else {
            metainfo
                .get_v2_info_hash()
                .ok()
                .map(|h| crate::util::info_hash_to_string(&h))
        };
        let v2_truncated_info_hash = v2_info_hash
            .as_ref()
            .map(|s| s[..40.min(s.len())].to_string());

        let mut file_pieces_roots = Vec::new();
        if let Ok((_size, flist)) = metainfo.local_files_to_download_list(std::path::Path::new("."))
        {
            for f in flist {
                if let Some(root) = f.pieces_root {
                    file_pieces_roots
                        .push((f.torrent_path, crate::util::info_hash_to_string(&root)));
                }
            }
        }

        let piece_layers_count = metainfo.get_piece_layers().len();

        let magnet_uri = crate::core::torrent_upgrader::TorrentUpgrader::generate_magnet_uri(
            &name,
            v1_info_hash.as_deref(),
            v2_info_hash.as_deref(),
            &metainfo.get_tracker_urls().unwrap_or_default(),
        );

        let report = crate::core::torrent_upgrader::TorrentUpgradeReport {
            name,
            format,
            total_size,
            piece_length,
            v1_info_hash,
            v2_info_hash,
            v2_truncated_info_hash,
            file_pieces_roots,
            piece_layers_count,
            magnet_uri,
        };

        Ok((torrent_bytes, report))
    }

    /// Validates the presence and integrity (exact sizes) of downloaded files on disk.
    pub fn validate(&self) -> Result<(), BitTorrentError> {
        let context = self.context.lock().unwrap();
        context.validate()?;
        for file in &context.files_to_download {
            let path = Path::new(&file.name);
            if !path.exists() {
                return Err(BitTorrentError::Parse(format!(
                    "Expected torrent file path is missing: {}",
                    file.name
                )));
            }
            let metadata = fs::metadata(path)?;
            if metadata.len() != file.length {
                return Err(BitTorrentError::Parse(format!(
                    "File length mismatch for {}: expected {} bytes, found {} bytes",
                    file.name,
                    file.length,
                    metadata.len()
                )));
            }
        }
        Ok(())
    }

    /// Drains all completed background piece hash verification results and applies them
    /// to the session context, updating bitfields, flushing storage, and broadcasting Have messages.
    ///
    /// Returns the number of results applied.
    pub fn drain_verification_results(&self) -> usize {
        let mut ctx = self.context.lock().unwrap();
        let storage = ctx.storage.clone();
        ctx.drain_verification_results(&*storage)
    }

    /// Returns a snapshot of current telemetry metrics from the piece hash verification worker pool.
    pub fn verification_metrics(
        &self,
    ) -> Option<crate::storage::verification_pool::VerificationPoolMetrics> {
        let ctx = self.context.lock().unwrap();
        Some(ctx.verification_metrics())
    }

    /// Triggers parallel multi-threaded rechecking of all torrent pieces on disk using the worker pool.
    pub fn recheck_pieces(
        &mut self,
    ) -> Result<Arc<crate::storage::verification_pool::RecheckProgress>, BitTorrentError> {
        let mut ctx = self.context.lock().unwrap();
        ctx.recheck_all_pieces()
    }

    /// Returns the root download directory path.
    pub fn download_path(&self) -> &Path {
        &self.download_path
    }

    /// Returns the download priority of a file at the given index, if found.
    pub fn file_priority(&self, index: usize) -> Option<FilePriority> {
        let ctx = self.context.lock().unwrap();
        ctx.files_to_download.get(index).map(|f| f.priority)
    }

    /// Sets the download priority of a file at the given index.
    pub fn set_file_priority(
        &mut self,
        index: usize,
        priority: FilePriority,
    ) -> Result<(), BitTorrentError> {
        let mut ctx = self.context.lock().unwrap();
        ctx.set_file_priority(index, priority)
    }

    /// Returns the currently active BEP 53 selected file indices, if configured.
    pub fn selected_files(&self) -> Option<Vec<usize>> {
        let ctx = self.context.lock().unwrap();
        ctx.selected_files().map(|s| s.to_vec())
    }

    /// Sets or updates the BEP 53 selective file download list (`so=`).
    ///
    /// Selected files are elevated to [`FilePriority::Normal`], and unselected files
    /// are set to [`FilePriority::Skip`].
    pub fn select_files(&mut self, indices: &[usize]) -> Result<(), BitTorrentError> {
        let mut ctx = self.context.lock().unwrap();
        ctx.set_selected_files(indices);
        Ok(())
    }

    /// Returns a BEP 53 selection evaluation comparing selected files to total torrent payload.
    pub fn bep53_evaluation(
        &self,
    ) -> Option<crate::network::metadata_fetcher::BEP53SelectionEvaluation> {
        let ctx = self.context.lock().unwrap();
        ctx.bep53_evaluation()
    }

    /// Returns telemetry metrics for the adaptive endgame erasure staging cache.
    pub fn endgame_metrics(&self) -> crate::storage::erasure_staging::EndgameStagingMetrics {
        let ctx = self.context.lock().unwrap();
        ctx.endgame_staging_metrics()
    }

    /// Returns a swarm latency profile summary for all connected peers.
    pub fn swarm_latency_summary(
        &self,
    ) -> Vec<crate::network::latency_profiler::PeerLatencySummary> {
        let ctx = self.context.lock().unwrap();
        ctx.swarm_latency_summary()
    }

    /// Returns a list of health summaries for all registered WebSeed mirror endpoints.
    pub fn webseed_mirror_summary(
        &self,
    ) -> Vec<crate::network::webseed_tracker::MirrorHealthSummary> {
        let ctx = self.context.lock().unwrap();
        ctx.webseed_mirror_summary()
    }

    /// Registers a new WebSeed mirror URL dynamically at runtime.
    pub fn add_webseed(&mut self, url: impl Into<String>) {
        let mut ctx = self.context.lock().unwrap();
        ctx.add_webseed(url);
    }

    /// Removes a WebSeed mirror URL from active tracking and routing.
    pub fn remove_webseed(&mut self, url: &str) -> bool {
        let mut ctx = self.context.lock().unwrap();
        ctx.remove_webseed(url)
    }

    /// Identifies the next missing block to download and transmits a Request packet to the specified peer.
    pub async fn request_next_block_from_peer(
        &mut self,
        peer: &mut Peer,
    ) -> Result<Option<(u32, u32, u32)>, BitTorrentError> {
        let mut context = self.context.lock().unwrap();
        if let Some((piece_number, begin, length)) = context.next_block_request_for_peer(peer) {
            peer.send_request(piece_number, begin, length).await?;
            peer.outstanding_requests_count = peer.outstanding_requests_count.saturating_add(1);
            Ok(Some((piece_number, begin, length)))
        } else {
            Ok(None)
        }
    }

    /// Establishes connection, handles handshakes, and starts a worker thread loop to read and write messages for a single peer.
    pub fn connect_and_download_peer(
        &mut self,
        peer_details: PeerDetails,
    ) -> Result<(), BitTorrentError> {
        if let Some(manager) = &self.manager {
            if manager.is_peer_dead(&peer_details.ip) {
                return Ok(());
            }
        }

        let context = self.context.clone();
        let manager_clone = self.manager.clone();
        let peer_workers = self.peer_workers.clone();

        let handle = thread::spawn(move || {
            futures::executor::block_on(self::worker::handle_peer_session(
                peer_details,
                context,
                manager_clone,
            ));
        });
        peer_workers.lock().unwrap().push(handle);

        Ok(())
    }

    /// Spawns connections and download worker threads for all peers listed in the provided details array.
    pub fn download_from_peers(&mut self, peers: Vec<PeerDetails>) -> Result<(), BitTorrentError> {
        if peers.is_empty() {
            return Err(BitTorrentError::Parse(
                "No peers are available to download from.".into(),
            ));
        }
        for peer in peers {
            self.connect_and_download_peer(peer)?;
        }
        Ok(())
    }

    /// Blocks the current thread waiting for the download finish event to be signaled.
    pub fn wait_for_download_finished(&self, timeout_ms: u64) -> bool {
        let event = self.context.lock().unwrap().download_finished.clone();
        event.wait_one(timeout_ms)
    }

    /// Joins and halts all peer worker threads spawned during the session.
    pub fn join_peer_workers(&mut self) {
        if self.status() != TorrentStatus::Ended {
            let _ = self.stop();
        }
        let mut workers = {
            let mut guard = self.peer_workers.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        while let Some(handle) = workers.pop() {
            let _ = handle.join();
        }
        if let Some(handle) = self.executor_thread.take() {
            let _ = handle.join();
        }
    }

    /// Creates a `TorrentSession` using an already constructed `TorrentContext`.
    /// Bypasses all metainfo file parsing and disk checks.
    pub fn new_from_context(context: Arc<Mutex<TorrentContext>>) -> Self {
        let (task_tx, task_rx) = std::sync::mpsc::channel();
        let executor_thread = spawn_executor(context.clone(), task_rx);
        let download_path = context.lock().unwrap().download_path.clone();
        let config = context.lock().unwrap().config.clone();

        spawn_stats_loop_standard(context.clone(), task_tx.clone());
        config
            .choking_strategy
            .spawn_choking_loop(context.clone(), task_tx.clone(), None);

        TorrentSession {
            context,
            download_path,
            peer_workers: Arc::new(Mutex::new(Vec::new())),
            task_tx,
            executor_thread: Some(executor_thread),
            manager: None,
            #[cfg(feature = "dht")]
            dht: None,
            #[cfg(feature = "nat-pmp")]
            nat_pmp: None,
        }
    }

    /// Spawns a background thread that re-announces to the tracker at the interval returned by
    /// the tracker, connects newly discovered peers, and sends a `completed` event once the
    /// download finishes.  The caller should join the returned handle before exiting.
    pub fn start_reannounce_loop(&self, mut tracker: Tracker) -> thread::JoinHandle<()> {
        let context = self.context.clone();
        let peer_workers = self.peer_workers.clone();
        let manager = self.manager.clone();

        let _ = self.task_tx.send(Box::pin(async move {
            let _ = tracker.start_announcing();
            let mut announced_completed = false;
            loop {
                let min_reannounce = {
                    let ctx = context.lock().unwrap();
                    ctx.config.min_reannounce_interval
                };
                let interval = tracker.interval.max(min_reannounce as usize);
                let start_time = std::time::Instant::now();
                let duration = Duration::from_secs(interval as u64);
                let mut ended = false;
                while start_time.elapsed() < duration {
                    if context.lock().unwrap().status == TorrentStatus::Ended {
                        ended = true;
                        break;
                    }
                    self::worker::delay(Duration::from_millis(100)).await;
                }
                if ended {
                    break;
                }

                let status = {
                    let ctx = context.lock().unwrap();
                    if ctx.status == TorrentStatus::Ended {
                        break;
                    }
                    ctx.status
                };

                if status == TorrentStatus::Seeding && !announced_completed {
                    announced_completed = true;
                    let _ = tracker.announce_completed();
                    // Stay in the loop so we keep announcing to remain visible as a seeder.
                    continue;
                }

                match tracker.announce_once() {
                    Ok(response) => {
                        for peer_details in response.peer_list {
                            let ctx2 = context.clone();
                            let mgr2 = manager.clone();
                            let peer_workers2 = peer_workers.clone();

                            let handle = thread::spawn(move || {
                                futures::executor::block_on(self::worker::handle_peer_session(
                                    peer_details,
                                    ctx2,
                                    mgr2,
                                ));
                            });
                            peer_workers2.lock().unwrap().push(handle);
                        }
                    }
                    Err(_) => {}
                }
            }

            // Send stopped when the session ends.
            let _ = tracker.announce_stopped();
        }));

        thread::spawn(|| {})
    }
}

/// Spawns the single-threaded async executor that drives all `task_tx`-submitted futures.
fn spawn_executor(
    context: Arc<Mutex<TorrentContext>>,
    task_rx: std::sync::mpsc::Receiver<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut pool = LocalPool::new();
        let spawner = pool.spawner();
        let spawner_clone = spawner.clone();
        spawner
            .spawn_local(async move {
                loop {
                    if context.lock().unwrap().status == TorrentStatus::Ended {
                        break;
                    }
                    match task_rx.try_recv() {
                        Ok(future) => {
                            let _ = spawner_clone.spawn_local(future);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                            crate::util::yield_now().await;
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            break;
                        }
                    }
                }
            })
            .unwrap();
        pool.run();
    })
}

/// Spawns the periodic stats-logging future for a standard torrent session.
fn spawn_stats_loop_standard(
    context: Arc<Mutex<TorrentContext>>,
    task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
) {
    let _ = task_tx.send(Box::pin(async move {
        loop {
            let start_time = std::time::Instant::now();
            let duration = Duration::from_secs(5);
            while start_time.elapsed() < duration {
                if context.lock().unwrap().status == TorrentStatus::Ended {
                    return;
                }
                delay(Duration::from_millis(100)).await;
            }
            if context.lock().unwrap().status == TorrentStatus::Ended {
                break;
            }
            if let Ok(ctx) = context.try_lock() {
                let peers = ctx.peer_swarm.read().map(|s| s.len()).unwrap_or(0);
                let unchoked = ctx.number_of_unchoked_peers();
                let done = ctx.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
                let total = ctx.total_bytes_to_download;
                let bps = ctx.bytes_per_second();
                let reserved = ctx.number_of_reserved_blocks();
                let progress = if total > 0 { (done * 10000) / total } else { 0 };
                log_debug!(
                    "[stats] peers={}/{} downloaded={}/{} ({}.{:02}%) speed={}/s reserved_blocks={}",
                    unchoked, peers,
                    done, total,
                    progress / 100, progress % 100,
                    bps,
                    reserved,
                );
            }
        }
    }));
}

/// Spawns the periodic stats-logging future for a magnet-bootstrap session.
fn spawn_stats_loop_magnet(
    context: Arc<Mutex<TorrentContext>>,
    task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
) {
    let _ = task_tx.send(Box::pin(async move {
        loop {
            let start_time = std::time::Instant::now();
            let duration = Duration::from_secs(5);
            while start_time.elapsed() < duration {
                if context.lock().unwrap().status == TorrentStatus::Ended {
                    return;
                }
                delay(Duration::from_millis(100)).await;
            }
            if context.lock().unwrap().status == TorrentStatus::Ended {
                break;
            }
            if let Ok(ctx) = context.try_lock() {
                let peers = ctx.peer_swarm.read().map(|s| s.len()).unwrap_or(0);
                let unchoked = ctx.number_of_unchoked_peers();
                let done = ctx.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
                let total = ctx.total_bytes_to_download;
                let bps = ctx.bytes_per_second();
                let reserved = ctx.number_of_reserved_blocks();
                let progress = if total > 0 { (done * 10000) / total } else { 0 };
                if ctx.pieces_info_hash.is_empty() {
                    let got = ctx.metadata_pieces.len();
                    let size = ctx.metadata_size.unwrap_or(0);
                    let total_pieces = if size > 0 { (size + 16383) / 16384 } else { 0 };
                    log_debug!(
                        "[stats] magnet bootstrap peers={} metadata_pieces={}/{} size={}",
                        peers, got, total_pieces, size
                    );
                } else {
                    log_debug!(
                        "[stats] peers={}/{} downloaded={}/{} ({}.{:02}%) speed={}/s reserved_blocks={}",
                        unchoked, peers,
                        done, total,
                        progress / 100, progress % 100,
                        bps,
                        reserved,
                    );
                }
            }
        }
    }));
}

impl super::config::ChokingStrategy for super::config::StandardChoking {
    fn spawn_choking_loop(
        &self,
        context: Arc<Mutex<TorrentContext>>,
        task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
        manager: Option<Arc<Manager>>,
    ) {
        let _ = task_tx.send(Box::pin(async move {
            let mut optimistic_timer = 0;
            let mut current_optimistic_ip: Option<String> = None;
            loop {
                let start_time = std::time::Instant::now();
                let duration = Duration::from_secs(10);
                let mut ended = false;
                while start_time.elapsed() < duration {
                    if context.lock().unwrap().status == TorrentStatus::Ended {
                        ended = true;
                        break;
                    }
                    delay(Duration::from_millis(100)).await;
                }
                if ended {
                    break;
                }

                let status = {
                    let ctx = context.lock().unwrap();
                    ctx.status
                };
                if status == TorrentStatus::Ended {
                    break;
                }

                optimistic_timer += 10;
                let trigger_optimistic = if optimistic_timer >= 30 {
                    optimistic_timer = 0;
                    true
                } else {
                    false
                };

                let mut peers_to_action = Vec::new();
                {
                    let ctx = context.lock().unwrap();
                    let swarm = ctx.peer_swarm.read().unwrap();

                    let mut interested_peers = Vec::new();
                    for peer_arc in swarm.values() {
                        let mut p = peer_arc.lock().unwrap();

                        let dl_rate = p.bytes_downloaded_in_interval as f64 / 10.0;
                        p.rolling_download_rate = p.rolling_download_rate * 0.8 + dl_rate * 0.2;
                        p.bytes_downloaded_in_interval = 0;

                        let ul_rate = p.bytes_uploaded_in_interval as f64 / 10.0;
                        p.rolling_upload_rate = p.rolling_upload_rate * 0.8 + ul_rate * 0.2;
                        p.bytes_uploaded_in_interval = 0;

                        if p.peer_interested {
                            interested_peers.push(peer_arc.clone());
                        }
                    }

                    if !interested_peers.is_empty() {
                        let is_seeding = ctx.status == TorrentStatus::Seeding;
                        interested_peers.sort_by(|a, b| {
                            let pa = a.lock().unwrap();
                            let pb = b.lock().unwrap();
                            if is_seeding {
                                pb.rolling_upload_rate
                                    .partial_cmp(&pa.rolling_upload_rate)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            } else {
                                pb.rolling_download_rate
                                    .partial_cmp(&pa.rolling_download_rate)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            }
                        });

                        let max_slots = 4;
                        let top_count = (max_slots - 1).min(interested_peers.len());
                        let mut chosen_peers = interested_peers[..top_count].to_vec();

                        let remaining_peers = &interested_peers[top_count..];
                        let mut optimistic_peer = None;
                        if !remaining_peers.is_empty() {
                            if trigger_optimistic || current_optimistic_ip.is_none() {
                                use rand::Rng;
                                let idx = rand::thread_rng().gen_range(0..remaining_peers.len());
                                let picked = &remaining_peers[idx];
                                let ip = picked.lock().unwrap().ip.clone();
                                current_optimistic_ip = Some(ip);
                                optimistic_peer = Some(picked.clone());
                            } else {
                                if let Some(ref ip) = current_optimistic_ip {
                                    if let Some(found) =
                                        remaining_peers.iter().find(|p| p.lock().unwrap().ip == *ip)
                                    {
                                        optimistic_peer = Some(found.clone());
                                    } else {
                                        use rand::Rng;
                                        let idx =
                                            rand::thread_rng().gen_range(0..remaining_peers.len());
                                        let picked = &remaining_peers[idx];
                                        let ip = picked.lock().unwrap().ip.clone();
                                        current_optimistic_ip = Some(ip);
                                        optimistic_peer = Some(picked.clone());
                                    }
                                }
                            }
                        } else {
                            current_optimistic_ip = None;
                        }

                        if let Some(opt_p) = optimistic_peer {
                            chosen_peers.push(opt_p);
                        }

                        for peer_arc in &interested_peers {
                            let ip = peer_arc.lock().unwrap().ip.clone();
                            let is_chosen = chosen_peers.iter().any(|p| p.lock().unwrap().ip == ip);
                            let mut p = peer_arc.lock().unwrap();
                            if is_chosen {
                                if p.am_choking {
                                    p.am_choking = false;
                                    peers_to_action.push((
                                        peer_arc.clone(),
                                        crate::peer::PeerAction::SendUnchoke,
                                    ));
                                }
                            } else {
                                if !p.am_choking {
                                    p.am_choking = true;
                                    peers_to_action.push((
                                        peer_arc.clone(),
                                        crate::peer::PeerAction::SendChoke,
                                    ));
                                }
                            }
                        }
                    }

                    for peer_arc in swarm.values() {
                        let mut p = peer_arc.lock().unwrap();
                        if !p.peer_interested && !p.am_choking {
                            p.am_choking = true;
                            peers_to_action
                                .push((peer_arc.clone(), crate::peer::PeerAction::SendChoke));
                        }
                    }
                }
                for (peer_arc, action) in peers_to_action {
                    let net_opt = peer_arc.lock().unwrap().network.clone();
                    if let Some(net) = net_opt {
                        let peer_ip = peer_arc.lock().unwrap().ip.clone();
                        let _ = Peer::execute_actions(
                            &peer_arc,
                            vec![action],
                            &net,
                            &context,
                            &peer_ip,
                            &manager,
                        )
                        .await;
                    }
                }
            }
        }));
    }
}
