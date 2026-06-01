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
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct TorrentSession {
    pub context: Arc<Mutex<TorrentContext>>,
    pub disk_io: Arc<DiskIO>,
    pub download_path: PathBuf,
    pub peer_workers: Vec<thread::JoinHandle<()>>,
}

impl TorrentSession {
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

    pub fn start_download(&mut self) -> Result<(), BitTorrentError> {
        let mut context = self.context.lock().unwrap();
        if context.status == TorrentStatus::Seeding {
            return Err(BitTorrentError::Parse(
                "Cannot start download while torrent is configured for seeding.".into(),
            ));
        }
        context.start_downloading()
    }

    pub fn pause(&mut self) -> Result<(), BitTorrentError> {
        self.context.lock().unwrap().pause()
    }

    pub fn resume(&mut self) -> Result<(), BitTorrentError> {
        self.context.lock().unwrap().resume()
    }

    pub fn stop(&mut self) -> Result<(), BitTorrentError> {
        let context = self.context.lock().unwrap();
        context.disconnect_all_peers();
        drop(context);
        self.context.lock().unwrap().stop()
    }

    pub fn status(&self) -> TorrentStatus {
        self.context.lock().unwrap().status
    }

    pub fn progress(&self) -> f32 {
        self.context.lock().unwrap().progress_percent()
    }

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

    pub fn download_path(&self) -> &Path {
        &self.download_path
    }

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
            if let Some(manager) = &manager_clone {
                if manager.is_peer_dead(&peer_details.ip) {
                    return;
                }
            }

            let address = format!("{}:{}", peer_details.ip, peer_details.port);
            let stream = match TcpStream::connect(&address) {
                Ok(stream) => stream,
                Err(_) => {
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
                                    if let Some(manager) = &manager_clone {
                                        manager.add_to_dead_peer_list(&peer_details.ip);
                                    }
                                    break;
                                }
                                continue;
                            }
                        }
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
                    if let Some((piece_number, begin, length)) =
                        ctx.next_block_request_for_peer(&peer_guard)
                    {
                        if peer_guard
                            .send_request(piece_number, begin, length)
                            .is_err()
                        {
                            if let Some(manager) = &manager_clone {
                                manager.add_to_dead_peer_list(&peer_details.ip);
                            }
                            break;
                        }
                        peer_guard.outstanding_requests_count =
                            peer_guard.outstanding_requests_count.saturating_add(1);
                        last_progress = Instant::now();
                    }
                }

                if ctx.is_download_complete() {
                    break;
                }
            }

            context.lock().unwrap().remove_peer(&peer_details.ip);
        });

        self.peer_workers.push(handle);
        Ok(())
    }

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

    pub fn wait_for_download_finished(&self, timeout_ms: u64) -> bool {
        self.context
            .lock()
            .unwrap()
            .download_finished
            .wait_one(timeout_ms)
    }

    pub fn join_peer_workers(&mut self) {
        while let Some(handle) = self.peer_workers.pop() {
            let _ = handle.join();
        }
    }
}
