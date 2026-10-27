//! Torrent download session management
//!
//! Orchestrates the active downloading/seeding processes for a torrent. Spawn
//! workers to establish connections, request/download blocks, process messages,
//! and handle disk writes.
//!
//! Configuration lives in [`super::config`]; builder types live in [`super::builder`].

use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::core::transfer_metrics::TransferMetrics;
use crate::disk_io::DiskIO;
use crate::error::BitTorrentError;
use crate::magnet::MagnetLink;
use crate::manager::Manager;
use crate::metainfo::{FilePriority, MetaInfoFile};
use crate::peer::Peer;
use crate::selector::PieceSelector;
use crate::storage::fast_resume::FastResumeData;
use crate::tracker::{PeerDetails, Tracker};
use crate::utils::util::{LockExt, RwLockExt};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
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

pub use super::builder::{Initial, Ready, TorrentSessionBuilder, TorrentSource};
pub use super::config::SessionConfig;
pub use super::events::{EventBroadcaster, SessionEvent};

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
    pub atomic_status: Arc<AtomicU8>,
    pub transfer_metrics: TransferMetrics,
    pub peer_swarm: Arc<RwLock<HashMap<String, Arc<Mutex<Peer>>>>>,
    pub event_broadcaster: EventBroadcaster,
}

impl TorrentSession {
    /// Creates a builder to configure and construct a new `TorrentSession` from a `.torrent` file.
    pub fn builder(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
    ) -> TorrentSessionBuilder<Ready> {
        TorrentSessionBuilder::new(torrent_path, download_path)
    }

    /// Creates a builder to configure and construct a new `TorrentSession` from a magnet link.
    pub fn from_magnet(
        magnet_link: impl Into<String>,
        download_path: impl AsRef<Path>,
    ) -> TorrentSessionBuilder<Ready> {
        TorrentSessionBuilder::from_magnet(magnet_link, download_path)
    }

    fn finalize_assembly(
        context: Arc<Mutex<TorrentContext>>,
        download_path: PathBuf,
        config: &SessionConfig,
        is_magnet: bool,
        validate: bool,
    ) -> Result<Self, BitTorrentError> {
        let (task_tx, task_rx) =
            std::sync::mpsc::channel::<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>();
        let executor_thread = spawn_executor(context.clone(), task_rx);

        let (atomic_status, transfer_metrics, peer_swarm) = {
            let ctx = context.lock_safe();
            (
                ctx.atomic_status.clone(),
                ctx.transfer_metrics.clone(),
                ctx.peer_swarm.clone(),
            )
        };

        if is_magnet {
            spawn_stats_loop_magnet(context.clone(), atomic_status.clone(), task_tx.clone());
        } else {
            spawn_stats_loop_standard(context.clone(), atomic_status.clone(), task_tx.clone());
        }

        let event_broadcaster = EventBroadcaster::new();
        context.lock_safe().event_broadcaster = Some(event_broadcaster.clone());

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
            atomic_status,
            transfer_metrics,
            peer_swarm,
            event_broadcaster,
        };

        if validate {
            session.context.lock_safe().validate()?;
        }

        config.choking_strategy.spawn_choking_loop(
            session.context.clone(),
            session.task_tx.clone(),
            None,
        );

        Ok(session)
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
                if resume.apply_to_context(&mut context.lock_safe()).is_ok() {
                    resumed = true;
                }
            }
        }

        if !resumed {
            if seeding {
                disk_io.fully_downloaded_torrent_bitfield(&mut context.lock_safe())?;
                let total = context.lock_safe().total_bytes_to_download;
                context
                    .lock_safe()
                    .total_bytes_downloaded
                    .store(total, std::sync::atomic::Ordering::Relaxed);
                context.lock_safe().initial_bytes_downloaded = total;
            } else {
                if !config.skip_hash_check {
                    disk_io.create_torrent_bitfield(&mut context.lock_safe())?;
                } else {
                    let mut ctx = context.lock_safe();
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
                    .lock_safe()
                    .total_bytes_downloaded
                    .load(std::sync::atomic::Ordering::Relaxed);
                context.lock_safe().initial_bytes_downloaded = downloaded;
            }
        }

        Self::finalize_assembly(context, download_path, &config, false, true)
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

        let mut trackers = magnet.trackers.clone();
        if trackers.is_empty() {
            trackers.push("udp://tracker.opentrackr.org:1337/announce".to_string());
            trackers.push("udp://open.stealth.si:80/announce".to_string());
            trackers.push("udp://tracker.torrent.eu.org:451/announce".to_string());
            trackers.push("http://tracker.openbittorrent.com:80/announce".to_string());
            trackers.push("udp://explodie.org:6969/announce".to_string());
        }

        let context = Arc::new(Mutex::new(TorrentContext::new_magnet_bootstrap(
            magnet.info_hash.clone(),
            trackers,
            selector,
            &download_path,
            config.clone(),
        )?));

        {
            let mut ctx = context.lock_safe();
            if let Some(ref name) = magnet.display_name {
                ctx.file_name = name.clone();
            } else {
                let hash_hex = crate::util::info_hash_to_string(&magnet.info_hash);
                let short_hash = if hash_hex.len() >= 8 {
                    &hash_hex[..8]
                } else {
                    &hash_hex
                };
                ctx.file_name = format!("Magnet ({})", short_hash);
            }
        }

        // Inject WebSeeds and WebTorrent WebSocket trackers
        crate::network::webtorrent_injector::WebTorrentMagnetInjector::inject_into_shared_context(
            &magnet,
            &context,
            auto_inject_webtorrent && (config.enable_webtorrent || config.webtorrent_bridge_mode),
        );

        if let Some(ref sel) = selected_files {
            context.lock_safe().set_selected_files(sel);
        } else if magnet.has_file_selection() {
            context
                .lock_safe()
                .set_selected_files(magnet.selected_indices());
        }

        Self::finalize_assembly(context, download_path, &config, true, false)
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
            let mut ctx = self.context.lock_safe();
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
            let active_seeds = self.context.lock_safe().web_seeds.clone();
            let handle = crate::webseed::start_webseed_loop(self.context.clone(), active_seeds);
            self.peer_workers.lock_safe().push(handle);
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
            let mut ctx = self.context.lock_safe();
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
        self.context.lock_safe().web_seeds.clone()
    }

    /// Returns a list of currently active WebTorrent WebSocket tracker URLs registered with this session.
    pub fn websocket_trackers(&self) -> Vec<String> {
        self.context
            .lock_safe()
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
        self.context.lock_safe().is_super_seeding()
    }

    /// Enables or disables super-seeding mode (BEP 16).
    pub fn set_super_seeding(&mut self, enabled: bool) {
        self.context.lock_safe().set_super_seeding(enabled);
    }

    /// Creates a seekable streaming reader ([`TorrentStream`](crate::storage::stream::TorrentStream)) over the entire torrent.
    ///
    /// Automatically installs a [`StreamingPieceSelector`](crate::core::selector::StreamingPieceSelector) into the session context
    /// so that active peer downloads dynamically prioritize the streaming cursor piece and lookahead window.
    pub fn create_stream(&self) -> Result<crate::storage::stream::TorrentStream, BitTorrentError> {
        let (storage, total_len, cursor) = {
            let mut ctx = self.context.lock_safe();
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
            let mut ctx = self.context.lock_safe();
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
    /// If the session is already in `Seeding` mode, commences serving pieces without altering status.
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
            let mut context = self.context.lock_safe();
            if context.status != TorrentStatus::Seeding {
                context.start_downloading()?;
            }
        }

        let context = self.context.lock_safe();
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
                let pg = context_clone.lock_safe();
                if pg.status != TorrentStatus::Ended && pg.status != TorrentStatus::Paused {
                    if !pg.peer_swarm.read_safe().contains_key(&peer_details.ip) {
                        // Prune finished worker handles and enforce connection limit
                        {
                            let mut workers_guard = peer_workers.lock_safe();
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
                        workers.lock_safe().push(handle);
                    }
                }
            }
        });

        // Start LSD (Local Service Discovery) listener and announcer
        #[cfg(feature = "lsd")]
        {
            let is_private = self.context.lock_safe().is_private;
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
            let is_private = self.context.lock_safe().is_private;
            if config.dht_enabled && !is_private {
                if self.dht.is_none() {
                    if let Ok(d) = crate::dht::Dht::new(config.dht_port) {
                        let _ = d.start();
                        d.bootstrap();
                        self.dht = Some(Arc::new(d));
                    }
                }

                if let Some(d_arc) = self.dht.clone() {
                    let mut ih = [0u8; 20];
                    if info_hash.len() >= 20 {
                        ih.copy_from_slice(&info_hash[..20]);
                    }
                    let peer_tx_clone = peer_tx.clone();
                    let context_clone = self.context.clone();
                    thread::spawn(move || {
                        loop {
                            if d_arc
                                .routing_table
                                .lock_safe()
                                .closest_nodes(&ih, 1)
                                .is_empty()
                            {
                                d_arc.bootstrap();
                            }
                            d_arc.lookup_peers(ih, peer_tx_clone.clone());
                            for _ in 0..20 {
                                std::thread::sleep(Duration::from_secs(1));
                                let status = {
                                    if let Ok(ctx) = context_clone.try_lock() {
                                        ctx.status
                                    } else {
                                        TorrentStatus::Downloading
                                    }
                                };
                                if status == TorrentStatus::Ended
                                    || status == TorrentStatus::Seeding
                                {
                                    return;
                                }
                            }
                        }
                    });
                }
            }
        }

        // Start WebSeeding loop if available
        #[cfg(feature = "webseed")]
        {
            let web_seeds = self.context.lock_safe().web_seeds.clone();
            if !web_seeds.is_empty() {
                let webseed_handle =
                    crate::webseed::start_webseed_loop(self.context.clone(), web_seeds);
                self.peer_workers.lock_safe().push(webseed_handle);
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
        if let Ok(mut tracker) = Tracker::new(self.context.clone()) {
            tracker.set_peer_swarm_queue(peer_tx);
            if let Some(ref mgr) = self.manager {
                tracker.set_peer_manager(mgr.clone());
            }
            let _ = self.start_reannounce_loop(tracker);
        }

        Ok(())
    }

    /// Commences active torrent transfer (downloading or seeding) in a single intuitive call.
    ///
    /// Automatically coordinates peer discovery, establishes connections, and manages
    /// protocol transitions. Emits a [`SessionEvent::StateChanged`] event to all active event stream subscribers.
    ///
    /// # Errors
    /// Returns [`BitTorrentError`] if background thread spawning, port mapping, or tracker
    /// requests fail fatally.
    ///
    /// # Examples
    /// ```no_run
    /// # use bittorrent_rs::TorrentSession;
    /// # fn run(mut session: TorrentSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session.start()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn start(&mut self) -> Result<(), BitTorrentError> {
        let is_seeding = {
            let ctx = self.context.lock_safe();
            ctx.status == TorrentStatus::Seeding
        };
        let res = if is_seeding {
            self.start_seeding()
        } else {
            self.start_download()
        };
        if res.is_ok() {
            let status = self.status();
            self.emit_event(SessionEvent::StateChanged(status));
        }
        res
    }

    /// Subscribes to the asynchronous event stream for this session.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<SessionEvent> {
        self.event_broadcaster.subscribe()
    }

    /// Returns an asynchronous event stream receiver emitting [`SessionEvent`] notifications.
    pub fn events(&self) -> std::sync::mpsc::Receiver<SessionEvent> {
        self.event_broadcaster.subscribe()
    }

    /// Broadcasts a [`SessionEvent`] notification to all active subscribers.
    pub fn emit_event(&self, event: SessionEvent) {
        self.event_broadcaster.emit(event);
    }

    /// Starts the session in seeding mode, serving verified local pieces to peers.
    pub fn start_seeding(&mut self) -> Result<(), BitTorrentError> {
        {
            let mut context = self.context.lock_safe();
            context.set_status(TorrentStatus::Seeding);
            context.download_finished.set();
        }
        self.start_download()
    }

    /// Pauses the torrent download session.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if the torrent is not downloading or seeding.
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
        let res = self.context.lock_safe().pause();
        if res.is_ok() {
            self.emit_event(SessionEvent::StateChanged(TorrentStatus::Paused));
        }
        res
    }

    /// Resumes the torrent download session from a paused state.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if the torrent is not currently paused.
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
        let res = self.context.lock_safe().resume();
        if res.is_ok() {
            let st = self.status();
            self.emit_event(SessionEvent::StateChanged(st));
        }
        res
    }

    /// Stops the session, disconnecting all connected peers and releasing resources.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if the session has already ended.
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
            let port = self.context.lock_safe().config.listen_port;
            thread::spawn(move || {
                let _ = nat_clone.release_mapping(true, port);
                let _ = nat_clone.release_mapping(false, port);
            });
        }
        let _ = self.flush_disk_cache();
        let context = self.context.lock_safe();
        context.disconnect_all_peers();
        drop(context);
        let res = self.context.lock_safe().stop();
        if res.is_ok() {
            self.emit_event(SessionEvent::StateChanged(TorrentStatus::Ended));
        }
        res
    }

    /// Flushes any pending dirty blocks in the in-memory write cache out to disk.
    pub fn flush_disk_cache(&self) -> Result<(), BitTorrentError> {
        let storage = self.context.lock_safe().storage.clone();
        storage.flush()
    }

    /// Returns the current state (e.g. paused, downloading, seeding) of the torrent via lock-free atomic read.
    #[inline]
    pub fn status(&self) -> TorrentStatus {
        TorrentStatus::from(self.atomic_status.load(Ordering::Relaxed))
    }

    /// Creates and returns a new `Tracker` linked to this session.
    pub fn tracker(&self) -> Result<crate::tracker::Tracker, BitTorrentError> {
        crate::tracker::Tracker::new(self.context.clone())
    }

    /// Returns the percentage of download completion (0.0 to 100.0) without acquiring the context lock.
    #[inline]
    pub fn progress(&self) -> f32 {
        self.transfer_metrics.progress_percent()
    }

    /// Returns a snapshot of statistics for the current session without acquiring the context lock.
    pub fn stats(&self) -> SessionStats {
        let status = TorrentStatus::from(self.atomic_status.load(Ordering::Relaxed));
        let progress = self.transfer_metrics.progress_percent();
        let bytes_downloaded = self
            .transfer_metrics
            .total_bytes_downloaded
            .load(Ordering::Relaxed);
        let bytes_uploaded = self
            .transfer_metrics
            .total_bytes_uploaded
            .load(Ordering::Relaxed);
        let bytes_left = self.transfer_metrics.bytes_left_to_download().unwrap_or(0);
        let download_speed = self.transfer_metrics.bytes_per_second().max(0) as f64;
        let upload_speed = self.transfer_metrics.upload_bytes_per_second().max(0) as f64;
        let connected_peers = self.peer_swarm.read().map(|s| s.len()).unwrap_or(0);

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

    /// Returns a snapshot of statistics if the peer swarm lock is immediately available without blocking.
    pub fn try_stats(&self) -> Option<SessionStats> {
        let connected_peers = self.peer_swarm.try_read().ok()?.len();
        let status = TorrentStatus::from(self.atomic_status.load(Ordering::Relaxed));
        let progress = self.transfer_metrics.progress_percent();
        let bytes_downloaded = self
            .transfer_metrics
            .total_bytes_downloaded
            .load(Ordering::Relaxed);
        let bytes_uploaded = self
            .transfer_metrics
            .total_bytes_uploaded
            .load(Ordering::Relaxed);
        let bytes_left = self.transfer_metrics.bytes_left_to_download().unwrap_or(0);
        let download_speed = self.transfer_metrics.current_download_bps().max(0) as f64;
        let upload_speed = self.transfer_metrics.current_upload_bps().max(0) as f64;

        Some(SessionStats {
            progress,
            status,
            bytes_downloaded,
            bytes_uploaded,
            bytes_left,
            download_speed,
            upload_speed,
            connected_peers,
        })
    }

    /// Returns the local piece completion status and swarm-wide piece availability counts.
    pub fn piece_availability(&self) -> (Vec<bool>, Vec<u16>) {
        self.context.lock_safe().get_piece_availability()
    }

    /// Exports the session's torrent into BitTorrent v2 or Hybrid `.torrent` format.
    #[cfg(feature = "v2")]
    pub fn export_torrent(
        &self,
        format: crate::core::torrent_upgrader::TorrentExportFormat,
    ) -> Result<(Vec<u8>, crate::core::torrent_upgrader::TorrentUpgradeReport), BitTorrentError>
    {
        let (files, download_path, name, mut piece_length, trackers, web_seeds, is_private) = {
            let ctx = self.context.lock_safe();
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
                let ctx = self.context.lock_safe();
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
        let context = self.context.lock_safe();
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
        let mut ctx = self.context.lock_safe();
        let storage = ctx.storage.clone();
        ctx.drain_verification_results(&*storage)
    }

    /// Returns a snapshot of current telemetry metrics from the piece hash verification worker pool.
    pub fn verification_metrics(
        &self,
    ) -> Option<crate::storage::verification_pool::VerificationPoolMetrics> {
        let ctx = self.context.lock_safe();
        Some(ctx.verification_metrics())
    }

    /// Triggers parallel multi-threaded rechecking of all torrent pieces on disk using the worker pool.
    pub fn recheck_pieces(
        &mut self,
    ) -> Result<Arc<crate::storage::verification_pool::RecheckProgress>, BitTorrentError> {
        let mut ctx = self.context.lock_safe();
        ctx.recheck_all_pieces()
    }

    /// Returns the root download directory path.
    pub fn download_path(&self) -> &Path {
        &self.download_path
    }

    /// Returns the download priority of a file at the given index, if found.
    pub fn file_priority(&self, index: usize) -> Option<FilePriority> {
        let ctx = self.context.lock_safe();
        ctx.files_to_download.get(index).map(|f| f.priority)
    }

    /// Sets the download priority of a file at the given index.
    pub fn set_file_priority(
        &mut self,
        index: usize,
        priority: FilePriority,
    ) -> Result<(), BitTorrentError> {
        let mut ctx = self.context.lock_safe();
        ctx.set_file_priority(index, priority)
    }

    /// Returns the currently active BEP 53 selected file indices, if configured.
    pub fn selected_files(&self) -> Option<Vec<usize>> {
        let ctx = self.context.lock_safe();
        ctx.selected_files().map(|s| s.to_vec())
    }

    /// Sets or updates the BEP 53 selective file download list (`so=`).
    ///
    /// Selected files are elevated to [`FilePriority::Normal`], and unselected files
    /// are set to [`FilePriority::Skip`].
    pub fn select_files(&mut self, indices: &[usize]) -> Result<(), BitTorrentError> {
        let mut ctx = self.context.lock_safe();
        ctx.set_selected_files(indices);
        Ok(())
    }

    /// Returns a BEP 53 selection evaluation comparing selected files to total torrent payload.
    pub fn bep53_evaluation(
        &self,
    ) -> Option<crate::network::metadata_fetcher::BEP53SelectionEvaluation> {
        let ctx = self.context.lock_safe();
        ctx.bep53_evaluation()
    }

    /// Returns telemetry metrics for the adaptive endgame erasure staging cache.
    pub fn endgame_metrics(&self) -> crate::storage::erasure_staging::EndgameStagingMetrics {
        let ctx = self.context.lock_safe();
        ctx.endgame_staging_metrics()
    }

    /// Returns a swarm latency profile summary for all connected peers.
    pub fn swarm_latency_summary(
        &self,
    ) -> Vec<crate::network::latency_profiler::PeerLatencySummary> {
        let ctx = self.context.lock_safe();
        ctx.swarm_latency_summary()
    }

    /// Returns a list of health summaries for all registered WebSeed mirror endpoints.
    pub fn webseed_mirror_summary(
        &self,
    ) -> Vec<crate::network::webseed_tracker::MirrorHealthSummary> {
        let ctx = self.context.lock_safe();
        ctx.webseed_mirror_summary()
    }

    /// Registers a new WebSeed mirror URL dynamically at runtime.
    pub fn add_webseed(&mut self, url: impl Into<String>) {
        let mut ctx = self.context.lock_safe();
        ctx.add_webseed(url);
    }

    /// Removes a WebSeed mirror URL from active tracking and routing.
    pub fn remove_webseed(&mut self, url: &str) -> bool {
        let mut ctx = self.context.lock_safe();
        ctx.remove_webseed(url)
    }

    /// Identifies the next missing block to download and transmits a Request packet to the specified peer.
    pub async fn request_next_block_from_peer(
        &mut self,
        peer: &mut Peer,
    ) -> Result<Option<(u32, u32, u32)>, BitTorrentError> {
        let mut context = self.context.lock_safe();
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
        peer_workers.lock_safe().push(handle);

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
        let event = self.context.lock_safe().download_finished.clone();
        event.wait_one(timeout_ms)
    }

    /// Joins and halts all peer worker threads spawned during the session.
    pub fn join_peer_workers(&mut self) {
        if self.status() != TorrentStatus::Ended {
            let _ = self.stop();
        }
        let mut workers = {
            let mut guard = self.peer_workers.lock_safe();
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
    pub fn new_from_context(context: Arc<Mutex<TorrentContext>>) -> Result<Self, BitTorrentError> {
        let download_path = context.lock_safe().download_path.clone();
        let config = context.lock_safe().config.clone();
        Self::finalize_assembly(context, download_path, &config, false, false)
    }

    /// Spawns a background thread that re-announces to the tracker at the interval returned by
    /// the tracker, connects newly discovered peers, and sends a `completed` event once the
    /// download finishes.  The caller should join the returned handle before exiting.
    pub fn start_reannounce_loop(&self, mut tracker: Tracker) -> thread::JoinHandle<()> {
        let context = self.context.clone();
        let peer_workers = self.peer_workers.clone();
        let manager = self.manager.clone();

        thread::spawn(move || {
            let _ = tracker.start_announcing();
            let mut announced_completed = false;
            loop {
                let min_reannounce = {
                    let ctx = context.lock_safe();
                    ctx.config.min_reannounce_interval
                };
                let has_peers = context
                    .lock_safe()
                    .peer_swarm
                    .read()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let interval = if !has_peers {
                    10
                } else {
                    tracker.interval.max(min_reannounce as usize)
                };
                let start_time = std::time::Instant::now();
                let duration = Duration::from_secs(interval as u64);
                let mut ended = false;
                while start_time.elapsed() < duration {
                    if context.lock_safe().status == TorrentStatus::Ended {
                        ended = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                if ended {
                    break;
                }

                let status = {
                    let ctx = context.lock_safe();
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
                            peer_workers2.lock_safe().push(handle);
                        }
                    }
                    Err(_) => {}
                }
            }

            // Send stopped when the session ends.
            let _ = tracker.announce_stopped();
        })
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
        let _ = spawner.spawn_local(async move {
            loop {
                if context.lock_safe().status == TorrentStatus::Ended {
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
        });
        pool.run();
    })
}

/// Spawns the periodic stats-logging future for a standard torrent session.
fn spawn_stats_loop_standard(
    context: Arc<Mutex<TorrentContext>>,
    atomic_status: Arc<AtomicU8>,
    task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
) {
    let _ = task_tx.send(Box::pin(async move {
        loop {
            let start_time = std::time::Instant::now();
            let duration = Duration::from_secs(5);
            while start_time.elapsed() < duration {
                if atomic_status.load(Ordering::Relaxed) == TorrentStatus::Ended as u8 {
                    return;
                }
                delay(Duration::from_millis(100)).await;
            }
            if atomic_status.load(Ordering::Relaxed) == TorrentStatus::Ended as u8 {
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
    atomic_status: Arc<AtomicU8>,
    task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
) {
    let _ = task_tx.send(Box::pin(async move {
        loop {
            let start_time = std::time::Instant::now();
            let duration = Duration::from_secs(5);
            while start_time.elapsed() < duration {
                if atomic_status.load(Ordering::Relaxed) == TorrentStatus::Ended as u8 {
                    return;
                }
                delay(Duration::from_millis(100)).await;
            }
            if atomic_status.load(Ordering::Relaxed) == TorrentStatus::Ended as u8 {
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
        let strategy = crate::session::choking_policy::PolicyChokingStrategy::new(Arc::new(
            crate::session::choking_policy::StandardTitForTatPolicy::default(),
        ));
        strategy.spawn_choking_loop(context, task_tx, manager);
    }
}
