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
use crate::tracker::PeerDetails;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

static LOG: OnceLock<Mutex<fs::File>> = OnceLock::new();

/// Appends a debug message to `debug.log`.
fn log(msg: &str) {
    let file = LOG.get_or_init(|| {
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open("debug.log")
            .expect("cannot open debug.log");
        Mutex::new(f)
    });
    if let Ok(mut f) = file.lock() {
        let _ = writeln!(f, "{}", msg);
        let _ = f.flush();
    }
}

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
        peer: &mut crate::peer::Peer,
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

    /// Helper to process a decoded `PeerMessage::Piece` payload and store the block in the context.
    pub fn process_peer_message(
        &mut self,
        _peer: &mut crate::peer::Peer,
        message: crate::peer_message::PeerMessage,
    ) -> Result<(), BitTorrentError> {
        if let crate::peer_message::PeerMessage::Piece {
            index,
            begin,
            block,
        } = message
        {
            self.context.lock().unwrap().process_piece_block(
                &self.disk_io,
                index,
                begin,
                &block,
            )?;
        }
        Ok(())
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
        let info_hash = context.lock().unwrap().info_hash.clone();
        let local_peer_id = peer_id::get();

        let handle = thread::spawn(move || {
            // Periodic stats thread — prints every 5s so we can see what's happening
            {
                let ctx_stats = context.clone();
                thread::spawn(move || loop {
                    thread::sleep(Duration::from_secs(5));
                    if let Ok(ctx) = ctx_stats.try_lock() {
                        if ctx.status == TorrentStatus::Ended {
                            break;
                        }
                        let peers = ctx.peer_swarm.read().map(|s| s.len()).unwrap_or(0);
                        let unchoked = ctx.number_of_unchoked_peers();
                        let done = ctx.total_bytes_downloaded;
                        let total = ctx.total_bytes_to_download + done;
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
            }
            if let Some(manager) = &manager_clone {
                if manager.is_peer_dead(&peer_details.ip) {
                    return;
                }
            }

            let address = format!("{}:{}", peer_details.ip, peer_details.port);
            let stream = match address
                .parse::<std::net::SocketAddr>()
                .ok()
                .and_then(|addr| {
                    TcpStream::connect_timeout(&addr, Duration::from_secs(10)).ok()
                })
            {
                Some(stream) => stream,
                None => {
                    if let Some(manager) = &manager_clone {
                        manager.add_to_dead_peer_list(&peer_details.ip);
                    }
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
                let mut peer_guard = peer.lock().unwrap();
                peer_guard.set_torrent_context(context.clone());
                if let Err(_) = peer_guard.handshake(&info_hash, local_peer_id.as_bytes()) {
                    if let Some(manager) = &manager_clone {
                        manager.add_to_dead_peer_list(&peer_details.ip);
                    }
                    return;
                }
                println!(
                    "Handshake completed with peer {}:{}",
                    peer_details.ip, peer_details.port
                );
                let bitfield = context.lock().unwrap().bitfield.clone();
                if let Err(_) = peer_guard.send_bitfield(bitfield) {
                    if let Some(manager) = &manager_clone {
                        manager.add_to_dead_peer_list(&peer_details.ip);
                    }
                    return;
                }
                println!(
                    "Sent Bitfield to peer {}:{}",
                    peer_details.ip, peer_details.port
                );
                if let Err(_) = peer_guard.send_unchoke() {
                    if let Some(manager) = &manager_clone {
                        manager.add_to_dead_peer_list(&peer_details.ip);
                    }
                    return;
                }
                peer_guard.am_choking = false;
                println!(
                    "Sent Unchoke to peer {}:{}",
                    peer_details.ip, peer_details.port
                );
                if let Err(_) = peer_guard.send_interested() {
                    if let Some(manager) = &manager_clone {
                        manager.add_to_dead_peer_list(&peer_details.ip);
                    }
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

                let mut peer_guard = peer.lock().unwrap();
                let message = match peer_guard.read_message() {
                    Ok(message) => message,
                    Err(err) => {
                        if let BitTorrentError::Io(io_err) = &err {
                            if io_err.kind() == std::io::ErrorKind::WouldBlock
                                || io_err.kind() == std::io::ErrorKind::TimedOut
                            {
                                if last_progress.elapsed() > Duration::from_secs(30) {
                                    log(&format!("[peer {}:{}] 30s idle timeout, dropping",
                                        peer_details.ip, peer_details.port));
                                    if let Some(manager) = &manager_clone {
                                        manager.add_to_dead_peer_list(&peer_details.ip);
                                    }
                                    break;
                                }
                                continue;
                            }
                        }
                        log(&format!("[peer {}:{}] read error: {}", peer_details.ip, peer_details.port, err));
                        if let Some(manager) = &manager_clone {
                            manager.add_to_dead_peer_list(&peer_details.ip);
                        }
                        break;
                    }
                };

                let mut ctx = context.lock().unwrap();
                if peer_guard
                    .handle_peer_message(message, &mut ctx, disk_io.as_ref())
                    .is_err()
                {
                    if let Some(manager) = &manager_clone {
                        manager.add_to_dead_peer_list(&peer_details.ip);
                    }
                    break;
                }

                if peer_guard.peer_choking.wait_one(0) && ctx.status == TorrentStatus::Downloading {
                    let to_send = 10usize.saturating_sub(peer_guard.outstanding_requests_count);
                    let mut send_error = false;
                    let mut none_count = 0;
                    for _ in 0..to_send {
                        match ctx.next_block_request_for_peer(&peer_guard) {
                            Some((piece_number, begin, length)) => {
                                if peer_guard
                                    .send_request(piece_number, begin, length)
                                    .is_err()
                                  {
                                    if let Some(manager) = &manager_clone {
                                        manager.add_to_dead_peer_list(&peer_details.ip);
                                    }
                                    send_error = true;
                                    break;
                                }
                                let block_index = begin / crate::constants::BLOCK_SIZE as u32;
                                peer_guard.reserved_blocks.push((piece_number, block_index));
                                peer_guard.outstanding_requests_count =
                                    peer_guard.outstanding_requests_count.saturating_add(1);
                                last_progress = Instant::now();
                            }
                            None => { none_count += 1; break; }
                        }
                    }
                    if none_count > 0 {
                        log(&format!("[peer {}:{}] no blocks available (outstanding={} missing_pieces={})",
                            peer_details.ip, peer_details.port,
                            peer_guard.outstanding_requests_count,
                            peer_guard.number_of_missing_pieces));
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
                let mut ctx = context.lock().unwrap();
                if let Ok(peer_lock) = peer.try_lock() {
                    for (piece_number, block_index) in &peer_lock.reserved_blocks {
                        ctx.release_block_request(*piece_number, *block_index);
                    }
                }
                ctx.remove_peer(&peer_details.ip);
            }
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
        self.context
            .lock()
            .unwrap()
            .download_finished
            .wait_one(timeout_ms)
    }

    /// Joins and halts all peer worker threads spawned during the session.
    pub fn join_peer_workers(&mut self) {
        while let Some(handle) = self.peer_workers.pop() {
            let _ = handle.join();
        }
    }
}
