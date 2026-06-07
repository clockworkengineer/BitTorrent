//! Torrent download session management
//!
//! Orchestrates the active downloading/seeding processes for a torrent. Spawn
//! workers to establish connections, request/download blocks, process messages, and handle disk writes.

use crate::disk_io::DiskIO;
use crate::error::BitTorrentError;
use crate::manager::Manager;
use crate::metainfo::MetaInfoFile;
use crate::peer::Peer;
use crate::peer_id;
use crate::selector::Selector;
use crate::torrent_context::{TorrentContext, TorrentStatus};
use crate::tracker::{PeerDetails, Tracker};
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::util::log_debug as log;

/// Represents an active Torrent transfer session.
pub struct TorrentSession {
    pub context: Arc<Mutex<TorrentContext>>,
    pub disk_io: Arc<DiskIO>,
    pub download_path: PathBuf,
    pub peer_workers: Vec<thread::JoinHandle<()>>,
}

impl TorrentSession {
    /// Creates and initializes a new `TorrentSession` using the specified torrent file and target download path.
    pub fn new(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
        seeding: bool,
    ) -> Result<Self, BitTorrentError> {
        let torrent_path = torrent_path.as_ref();
        let download_path = download_path.as_ref().to_path_buf();
        fs::create_dir_all(&download_path)?;

        let mut meta_info = MetaInfoFile::new(torrent_path)?;
        meta_info.parse()?;
        meta_info.validate()?;

        let disk_io = Arc::new(DiskIO::new());
        let selector = Selector::new();
        let context = Arc::new(Mutex::new(TorrentContext::new(
            &meta_info,
            selector,
            &disk_io,
            &download_path,
            seeding,
        )?));

        let session = TorrentSession {
            context,
            disk_io,
            download_path,
            peer_workers: Vec::new(),
        };

        session.context.lock().unwrap().validate()?;

        let context_stats = session.context.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(5));
            if let Ok(ctx) = context_stats.try_lock() {
                if ctx.status == TorrentStatus::Ended {
                    break;
                }
                let peers = ctx.peer_swarm.read().map(|s| s.len()).unwrap_or(0);
                let unchoked = ctx.number_of_unchoked_peers();
                let done = ctx.total_bytes_downloaded;
                let total = ctx.total_bytes_to_download;
                let bps = ctx.bytes_per_second();
                let reserved = ctx.requested_blocks.read().map(|r| r.len()).unwrap_or(0);
                log(&format!(
                    "[stats] peers={}/{} downloaded={}/{} ({:.1}%) speed={}/s reserved_blocks={}",
                    unchoked, peers,
                    done, total,
                    if total > 0 { done as f64 / total as f64 * 100.0 } else { 0.0 },
                    bps,
                    reserved,
                ));
            }
        });

        Ok(session)
    }

    /// Transitions the session status to `Downloading`, commencing active peer-based downloading.
    pub fn start_download(&mut self) -> Result<(), BitTorrentError> {
        let mut context = self.context.lock().unwrap();
        if context.status == TorrentStatus::Seeding {
            return Err(BitTorrentError::Parse(
                "Cannot start download while torrent is configured for seeding.".into(),
            ));
        }
        context.start_downloading()
    }

    /// Pauses the torrent download session.
    pub fn pause(&mut self) -> Result<(), BitTorrentError> {
        self.context.lock().unwrap().pause()
    }

    /// Resumes the torrent download session from a paused state.
    pub fn resume(&mut self) -> Result<(), BitTorrentError> {
        self.context.lock().unwrap().resume()
    }

    /// Stops the session, disconnecting all connected peers and releasing resources.
    pub fn stop(&mut self) -> Result<(), BitTorrentError> {
        let context = self.context.lock().unwrap();
        context.disconnect_all_peers();
        drop(context);
        self.context.lock().unwrap().stop()
    }

    /// Returns the current state (e.g. paused, downloading, seeding) of the torrent.
    pub fn status(&self) -> TorrentStatus {
        self.context.lock().unwrap().status
    }

    /// Returns the percentage of download completion (0.0 to 100.0).
    pub fn progress(&self) -> f32 {
        self.context.lock().unwrap().progress_percent()
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

    /// Returns the root download directory path.
    pub fn download_path(&self) -> &Path {
        &self.download_path
    }

    /// Identifies the next missing block to download and transmits a Request packet to the specified peer.
    pub fn request_next_block_from_peer(
        &mut self,
        peer: &mut Peer,
    ) -> Result<Option<(u32, u32, u32)>, BitTorrentError> {
        let mut context = self.context.lock().unwrap();
        if let Some((piece_number, begin, length)) = context.next_block_request_for_peer(peer) {
            peer.send_request(piece_number, begin, length)?;
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
        manager: Option<Arc<Manager>>,
    ) -> Result<(), BitTorrentError> {
        if let Some(manager) = &manager {
            if manager.is_peer_dead(&peer_details.ip) {
                return Ok(());
            }
        }

        let context = self.context.clone();
        let disk_io = self.disk_io.clone();
        let manager_clone = manager.clone();

        let handle = thread::spawn(move || {
            handle_peer_session(peer_details, context, disk_io, manager_clone);
        });

        self.peer_workers.push(handle);
        Ok(())
    }

    /// Spawns connections and download worker threads for all peers listed in the provided details array.
    pub fn download_from_peers(
        &mut self,
        peers: Vec<PeerDetails>,
        manager: Option<Arc<Manager>>,
    ) -> Result<(), BitTorrentError> {
        if peers.is_empty() {
            return Err(BitTorrentError::Parse(
                "No peers are available to download from.".into(),
            ));
        }
        for peer in peers {
            self.connect_and_download_peer(peer, manager.clone())?;
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
        while let Some(handle) = self.peer_workers.pop() {
            let _ = handle.join();
        }
    }

    /// Spawns a background thread that re-announces to the tracker at the interval returned by
    /// the tracker, connects newly discovered peers, and sends a `completed` event once the
    /// download finishes.  The caller should join the returned handle before exiting.
    pub fn start_reannounce_loop(
        &self,
        mut tracker: Tracker,
        manager: Option<Arc<Manager>>,
    ) -> thread::JoinHandle<()> {
        let context = self.context.clone();
        let disk_io = self.disk_io.clone();

        thread::spawn(move || {
            let mut announced_completed = false;
            loop {
                let interval = tracker.interval.max(60);
                thread::sleep(Duration::from_secs(interval as u64));

                let status = context.lock().unwrap().status;
                if status == TorrentStatus::Ended {
                    break;
                }

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
                            let disk2 = disk_io.clone();
                            let mgr2 = manager.clone();

                            thread::spawn(move || {
                                handle_peer_session(peer_details, ctx2, disk2, mgr2);
                            });
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

fn mark_peer_dead(manager: &Option<Arc<Manager>>, ip: &str) {
    if let Some(mgr) = manager {
        mgr.add_to_dead_peer_list(ip);
    }
}

fn handle_peer_session(
    peer_details: PeerDetails,
    context: Arc<Mutex<TorrentContext>>,
    disk_io: Arc<DiskIO>,
    manager: Option<Arc<Manager>>,
) {
    let info_hash = context.lock().unwrap().info_hash.clone();
    let local_peer_id = peer_id::get();

    if let Some(ref mgr) = manager {
        if mgr.is_peer_dead(&peer_details.ip) {
            return;
        }
    }

    let address = format!("{}:{}", peer_details.ip, peer_details.port);
    let stream = match address
        .parse::<std::net::SocketAddr>()
        .ok()
        .and_then(|addr| {
            TcpStream::connect_timeout(&addr, Duration::from_secs(10)).ok()
        }) {
        Some(s) => s,
        None => {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
    };

    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let peer = Arc::new(Mutex::new(Peer::new(
        peer_details.ip.clone(),
        peer_details.port,
        stream,
    )));

    {
        let mut pg = peer.lock().unwrap();
        pg.set_torrent_context(context.clone());
        if pg.handshake(&info_hash, local_peer_id.as_bytes()).is_err() {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
        println!(
            "Handshake completed with peer {}:{}",
            peer_details.ip, peer_details.port
        );
        let bitfield = context.lock().unwrap().bitfield.clone();
        if pg.send_bitfield(bitfield).is_err() {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
        println!(
            "Sent Bitfield to peer {}:{}",
            peer_details.ip, peer_details.port
        );
        if pg.send_unchoke().is_err() {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
        pg.am_choking = false;
        println!(
            "Sent Unchoke to peer {}:{}",
            peer_details.ip, peer_details.port
        );
        if pg.send_interested().is_err() {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
        println!(
            "Sent Interested to peer {}:{}",
            peer_details.ip, peer_details.port
        );
    }

    {
        let ctx = context.lock().unwrap();
        if !ctx.add_peer(peer.clone()) {
            return;
        }
    }

    let mut last_progress = Instant::now();
    loop {
        let status = {
            let ctx = context.lock().unwrap();
            if ctx.status == TorrentStatus::Ended {
                break;
            }
            ctx.paused.wait_one(0)
        };
        if status {
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let mut pg = peer.lock().unwrap();
        let message = match pg.read_message() {
            Ok(m) => m,
            Err(err) => {
                if let BitTorrentError::Io(ref io_err) = err {
                    if io_err.kind() == std::io::ErrorKind::WouldBlock
                        || io_err.kind() == std::io::ErrorKind::TimedOut
                    {
                        if last_progress.elapsed() > Duration::from_secs(30) {
                            log(&format!("[peer {}:{}] 30s idle timeout, dropping",
                                peer_details.ip, peer_details.port));
                            mark_peer_dead(&manager, &peer_details.ip);
                            break;
                        }
                        continue;
                    }
                }
                log(&format!("[peer {}:{}] read error: {}", peer_details.ip, peer_details.port, err));
                mark_peer_dead(&manager, &peer_details.ip);
                break;
            }
        };
        let mut ctx = context.lock().unwrap();
        if pg.handle_peer_message(message, &mut ctx, disk_io.as_ref()).is_err() {
            mark_peer_dead(&manager, &peer_details.ip);
            break;
        }
        if pg.peer_choking.wait_one(0) && ctx.status == TorrentStatus::Downloading {
            let to_send = 10usize.saturating_sub(pg.outstanding_requests_count);
            let mut send_error = false;
            let mut none_count = 0;
            for _ in 0..to_send {
                match ctx.next_block_request_for_peer(&pg) {
                    Some((pn, begin, length)) => {
                        if pg.send_request(pn, begin, length).is_err() {
                            mark_peer_dead(&manager, &peer_details.ip);
                            send_error = true;
                            break;
                        }
                        let bi = begin / crate::constants::BLOCK_SIZE as u32;
                        pg.reserved_blocks.push((pn, bi));
                        pg.outstanding_requests_count = pg.outstanding_requests_count.saturating_add(1);
                        last_progress = Instant::now();
                    }
                    None => { none_count += 1; break; }
                }
            }
            if none_count > 0 {
                log(&format!("[peer {}:{}] no blocks available (outstanding={} missing_pieces={})",
                    peer_details.ip, peer_details.port,
                    pg.outstanding_requests_count,
                    pg.number_of_missing_pieces));
            }
            if send_error {
                break;
            }
        }
        if ctx.is_download_complete() {
            break;
        }
    }

    log(&format!("[peer {}:{}] thread exiting", peer_details.ip, peer_details.port));
    {
        let ctx = context.lock().unwrap();
        if let Ok(pg) = peer.try_lock() {
            for (pn, bi) in &pg.reserved_blocks {
                ctx.release_block_request(*pn, *bi);
            }
        }
        ctx.remove_peer(&peer_details.ip);
    }
}
