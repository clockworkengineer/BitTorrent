//! Synchronization directory and session coordinator (Single Responsibility Principle).
//!
//! Decomposes synchronization into `SyncSessionController` (session lifecycle & torrent generation)
//! and `SyncManager` (directory watcher orchestration).

use std::path::PathBuf;

use bittorrent_rs::session::{SessionConfig, TorrentSession};

use crate::torrent_gen::generate_torrent_bytes;
use crate::watcher::DirectoryWatcher;

/// Controls torrent metainfo generation and local seeding session lifecycle (SRP).
pub struct SyncSessionController {
    sync_dir: PathBuf,
    torrent_path: PathBuf,
    session: Option<TorrentSession>,
    listen_port: u16,
}

impl SyncSessionController {
    /// Creates a new `SyncSessionController`.
    pub fn new(sync_dir: PathBuf, torrent_path: PathBuf, listen_port: u16) -> Self {
        Self {
            sync_dir,
            torrent_path,
            session: None,
            listen_port,
        }
    }

    /// Accesses the active seeding session if one is currently running.
    pub fn session(&self) -> Option<&TorrentSession> {
        self.session.as_ref()
    }

    /// Stops and waits for the active torrent session to terminate.
    pub fn stop(&mut self) {
        if let Some(mut sess) = self.session.take() {
            let _ = sess.stop();
            sess.join_peer_workers();
        }
    }

    /// Regenerates the `.sync.torrent` file and launches a fresh seeding session.
    pub fn rebuild_and_start(&mut self) -> Result<String, Box<dyn std::error::Error>> {
        self.stop();

        // Remove old torrent file if it exists to avoid self-inclusion
        if self.torrent_path.exists() {
            let _ = std::fs::remove_file(&self.torrent_path);
        }

        // Generate new torrent bytes (using 256 KB pieces)
        let name = self
            .sync_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let bytes = generate_torrent_bytes(&self.sync_dir, &name, 262144)?;

        // Write torrent to file
        std::fs::write(&self.torrent_path, &bytes)?;

        // Configure session
        let config = SessionConfig {
            mse_enabled: true,
            allow_private_lsd: true,
            dht_enabled: false,
            dht_port: self.listen_port + 10,
            ..Default::default()
        };

        // Build and start the session
        let mut session = TorrentSession::builder(&self.torrent_path, &self.sync_dir)
            .config(config)
            .seeding(true) // We are seeding our local files
            .build()?;

        session.start_download()?;

        let info_hash = session.context().lock().unwrap().info_hash.clone();
        let info_hash_hex = bittorrent_rs::util::to_hex(&info_hash);

        self.session = Some(session);
        Ok(info_hash_hex)
    }
}

/// Orchestrates filesystem change watching and delegates session rebuilds (SRP).
pub struct SyncManager {
    controller: SyncSessionController,
    sync_dir: PathBuf,
}

impl SyncManager {
    /// Creates a new `SyncManager` for the specified synchronization directory.
    pub fn new(sync_dir: PathBuf, listen_port: u16) -> Self {
        let torrent_path = sync_dir.join(".sync.torrent");
        let controller = SyncSessionController::new(sync_dir.clone(), torrent_path, listen_port);
        SyncManager {
            controller,
            sync_dir,
        }
    }

    /// Starts directory synchronization, performing an initial rebuild and watching for changes.
    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let info_hash_hex = self.controller.rebuild_and_start()?;
        println!("Sync Session Started. Info Hash: {}", info_hash_hex);

        let _watcher = DirectoryWatcher::watch(&self.sync_dir)?;
        println!("Watching directory {:?} for sync changes...", self.sync_dir);

        Ok(())
    }

    /// Rebuilds the torrent and restarts the seeding session.
    pub fn rebuild_torrent(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let info_hash_hex = self.controller.rebuild_and_start()?;
        println!("Sync Session Started. Info Hash: {}", info_hash_hex);
        Ok(())
    }

    /// Reference to the underlying session controller.
    pub fn controller(&self) -> &SyncSessionController {
        &self.controller
    }

    /// Mutable reference to the underlying session controller.
    pub fn controller_mut(&mut self) -> &mut SyncSessionController {
        &mut self.controller
    }
}
