use crate::torrent_gen::generate_torrent_bytes;
use crate::watcher::DirectoryWatcher;
use bittorrent_rs::session::{SessionConfig, TorrentSession};
use std::path::PathBuf;

pub struct SyncManager {
    sync_dir: PathBuf,
    torrent_path: PathBuf,
    session: Option<TorrentSession>,
    listen_port: u16,
}

impl SyncManager {
    pub fn new(sync_dir: PathBuf, listen_port: u16) -> Self {
        let torrent_path = sync_dir.join(".sync.torrent");
        SyncManager {
            sync_dir,
            torrent_path,
            session: None,
            listen_port,
        }
    }

    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Initial generation of the torrent
        self.rebuild_torrent()?;

        // 2. Start directory watcher
        let _watcher = DirectoryWatcher::watch(&self.sync_dir)?;

        println!("Watching directory {:?} for sync changes...", self.sync_dir);

        Ok(())
    }

    pub fn rebuild_torrent(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        // Stop current session if any
        if let Some(mut sess) = self.session.take() {
            let _ = sess.stop();
            sess.join_peer_workers();
        }

        // Remove old torrent file if it exists to avoid self-inclusion
        if self.torrent_path.exists() {
            let _ = std::fs::remove_file(&self.torrent_path);
        }

        // Generate new torrent bytes (using 256 KB pieces)
        let name = self.sync_dir.file_name().unwrap().to_string_lossy();
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
        println!("Sync Session Started. Info Hash: {}", info_hash_hex);

        self.session = Some(session);
        Ok(())
    }
}
