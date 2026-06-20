use crate::session::session::{TorrentSession, SessionConfig};
use crate::tracker::Tracker;
use crate::error::BitTorrentError;
use crate::selector::PieceSelector;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// High-level client facade that orchestrates both downloading and tracking.
pub struct TorrentClient {
    session: TorrentSession,
    tracker_thread: Option<thread::JoinHandle<()>>,
}

impl TorrentClient {
    /// Creates a new `TorrentClient` with default settings.
    pub fn new(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> Result<Self, BitTorrentError> {
        let session = TorrentSession::builder(torrent_path, download_path)
            .seeding(false)
            .build()?;
        Ok(Self {
            session,
            tracker_thread: None,
        })
    }

    /// Creates a new `TorrentClient` with options.
    pub fn new_with_options(
        torrent_path: impl AsRef<Path>,
        download_path: impl AsRef<Path>,
        seeding: bool,
        config: SessionConfig,
        selector: Arc<dyn PieceSelector>,
    ) -> Result<Self, BitTorrentError> {
        let session = TorrentSession::builder(torrent_path, download_path)
            .seeding(seeding)
            .config(config)
            .selector(selector)
            .build()?;
        Ok(Self {
            session,
            tracker_thread: None,
        })
    }

    /// Commences passive downloading, local service discovery, and registers with tracker.
    pub fn start(&mut self) -> Result<(), BitTorrentError> {
        self.session.start_download()?;

        let context_clone = self.session.context();
        let mut tracker = Tracker::new(context_clone.clone())?;

        let task_tx = self.session.task_tx.clone();
        let peer_workers = self.session.peer_workers.clone();
        let manager = self.session.manager.clone();

        let handle = thread::spawn(move || {
            if let Ok(response) = tracker.start_announcing() {
                for peer_details in response.peer_list {
                    if let Some(ref mgr) = manager {
                        if mgr.is_peer_dead(&peer_details.ip) {
                            continue;
                        }
                    }
                    let ctx2 = context_clone.clone();
                    let mgr2 = manager.clone();
                    let peer_workers2 = peer_workers.clone();
                    let handle = thread::spawn(move || {
                        futures::executor::block_on(crate::session::worker::handle_peer_session(peer_details, ctx2, mgr2));
                    });
                    peer_workers2.lock().unwrap().push(handle);
                }

                // Start the reannounce loop
                let context_reannounce = context_clone.clone();
                let peer_workers_reannounce = peer_workers.clone();
                let manager_reannounce = manager.clone();
                let _ = task_tx.send(Box::pin(async move {
                    let mut announced_completed = false;
                    loop {
                        let min_reannounce = {
                            let ctx = context_reannounce.lock().unwrap();
                            ctx.config.min_reannounce_interval
                        };
                        let interval = tracker.interval.max(min_reannounce as usize);
                        let start_time = std::time::Instant::now();
                        let duration = Duration::from_secs(interval as u64);
                        let mut ended = false;
                        while start_time.elapsed() < duration {
                            if context_reannounce.lock().unwrap().status == crate::TorrentStatus::Ended {
                                ended = true;
                                break;
                            }
                            crate::session::worker::delay(Duration::from_millis(100)).await;
                        }
                        if ended {
                            break;
                        }

                        let status = {
                            let ctx = context_reannounce.lock().unwrap();
                            if ctx.status == crate::TorrentStatus::Ended {
                                break;
                            }
                            ctx.status
                        };

                        if status == crate::TorrentStatus::Seeding && !announced_completed {
                            announced_completed = true;
                            let _ = tracker.announce_completed();
                            continue;
                        }

                        match tracker.announce_once() {
                            Ok(response) => {
                                for peer_details in response.peer_list {
                                    let ctx2 = context_reannounce.clone();
                                    let mgr2 = manager_reannounce.clone();
                                    let peer_workers2 = peer_workers_reannounce.clone();

                                    let handle = std::thread::spawn(move || {
                                        futures::executor::block_on(crate::session::worker::handle_peer_session(peer_details, ctx2, mgr2));
                                    });
                                    peer_workers2.lock().unwrap().push(handle);
                                }
                            }
                            Err(_) => {}
                        }
                    }
                    let _ = tracker.announce_stopped();
                }));
            }
        });

        self.tracker_thread = Some(handle);
        Ok(())
    }

    /// Stops the download session and waits for tracker announcements to finish.
    pub fn stop(&mut self) -> Result<(), BitTorrentError> {
        self.session.stop()?;
        if let Some(handle) = self.tracker_thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    /// Returns the session status.
    pub fn status(&self) -> crate::TorrentStatus {
        self.session.status()
    }

    /// Returns progress percentage (0.0 to 100.0).
    pub fn progress(&self) -> f32 {
        self.session.progress()
    }
}
