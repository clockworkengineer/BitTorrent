//! Builder types for constructing a [`super::session::TorrentSession`].
//!
//! Use [`TorrentSessionBuilder`] to configure and construct a session from a `.torrent` file
//! or a magnet link.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::BitTorrentError;
use crate::manager::Manager;
use crate::selector::{PieceSelector, RarestFirstSelector};

use crate::storage::fast_resume::FastResumeData;

use super::config::SessionConfig;
use super::session::TorrentSession;

/// The source of a torrent session (either a `.torrent` file or a magnet link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TorrentSource {
    File(PathBuf),
    Magnet(String),
}

/// Fluent builder for configuring and creating a `TorrentSession`.
///
/// # Examples
/// ```no_run
/// use bittorrent_rs::TorrentSession;
///
/// // Create from a torrent file
/// let session = TorrentSession::builder("my.torrent", "/downloads")
///     .seeding(false)
///     .build()
///     .unwrap();
///
/// // Create from a magnet link
/// let magnet_session = TorrentSession::from_magnet("magnet:?xt=urn:btih:...", "/downloads")
///     .build()
///     .unwrap();
/// ```
pub struct TorrentSessionBuilder {
    pub(super) source: TorrentSource,
    pub(super) download_path: PathBuf,
    pub(super) seeding: bool,
    pub(super) manager: Option<Arc<Manager>>,
    pub(super) config: SessionConfig,
    pub(super) selector: Arc<dyn PieceSelector>,
    pub(super) fast_resume: Option<FastResumeData>,
    pub(super) fast_resume_file: Option<PathBuf>,
    pub(super) web_seeds: Vec<String>,
    pub(super) auto_inject_webtorrent: bool,
}

impl TorrentSessionBuilder {
    /// Creates a new builder with the given torrent file and download directory.
    pub fn new(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> Self {
        TorrentSessionBuilder {
            source: TorrentSource::File(torrent_path.as_ref().to_path_buf()),
            download_path: download_path.as_ref().to_path_buf(),
            seeding: false,
            manager: None,
            config: SessionConfig::default(),
            selector: Arc::new(RarestFirstSelector),
            fast_resume: None,
            fast_resume_file: None,
            web_seeds: Vec::new(),
            auto_inject_webtorrent: true,
        }
    }

    /// Creates a new builder from a magnet URI and a target download directory.
    pub fn from_magnet(magnet_link: impl Into<String>, download_path: impl AsRef<Path>) -> Self {
        TorrentSessionBuilder {
            source: TorrentSource::Magnet(magnet_link.into()),
            download_path: download_path.as_ref().to_path_buf(),
            seeding: false,
            manager: None,
            config: SessionConfig::default(),
            selector: Arc::new(RarestFirstSelector),
            fast_resume: None,
            fast_resume_file: None,
            web_seeds: Vec::new(),
            auto_inject_webtorrent: true,
        }
    }

    /// Sets whether the session begins in seeding mode (all pieces already local).
    pub fn seeding(mut self, seeding: bool) -> Self {
        self.seeding = seeding;
        self
    }

    /// Sets whether super-seeding mode (BEP 16) is enabled when seeding.
    pub fn super_seeding(mut self, super_seeding: bool) -> Self {
        self.config.super_seeding = super_seeding;
        self
    }

    /// Sets an explicit IPv6 address to announce to dual-stack trackers (BEP 7).
    pub fn ipv6(mut self, ipv6: impl Into<String>) -> Self {
        self.config.ipv6 = Some(ipv6.into());
        self
    }

    /// Enables or disables dual-stack IPv6 peer connections and announcements.
    pub fn enable_ipv6(mut self, enable: bool) -> Self {
        self.config.enable_ipv6 = enable;
        self
    }

    /// Attaches a peer manager registry for dead-peer tracking across sessions.
    pub fn manager(mut self, manager: Arc<Manager>) -> Self {
        self.manager = Some(manager);
        self
    }

    /// Overrides all session timeouts and feature flags via a custom `SessionConfig`.
    pub fn config(mut self, config: SessionConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the in-memory block write cache size in megabytes (0 disables caching).
    pub fn disk_cache_size_mb(mut self, size_mb: usize) -> Self {
        self.config.disk_cache_size_mb = size_mb;
        self
    }

    /// Enables or disables Peer Exchange (BEP 11 / ut_pex).
    pub fn enable_pex(mut self, enable: bool) -> Self {
        self.config.enable_pex = enable;
        self
    }

    /// Enables or disables the Holepunch extension (BEP 55 / ut_holepunch).
    pub fn enable_holepunch(mut self, enable: bool) -> Self {
        self.config.enable_holepunch = enable;
        self
    }

    /// Enables or disables WebTorrent WebSocket tracker signaling and WebRTC Data Channels.
    pub fn enable_webtorrent(mut self, enable: bool) -> Self {
        self.config.enable_webtorrent = enable;
        self
    }

    /// Enables or disables WebTorrent cross-swarm bridge mode.
    pub fn webtorrent_bridge_mode(mut self, enable: bool) -> Self {
        self.config.webtorrent_bridge_mode = enable;
        self
    }

    /// Sets the maximum concurrent WebRTC browser peers for WebTorrent bridging.
    pub fn webtorrent_max_web_peers(mut self, max_peers: usize) -> Self {
        self.config.webtorrent_max_web_peers = max_peers;
        self
    }

    /// Returns a reference to the configured SessionConfig.
    pub fn get_config(&self) -> &SessionConfig {
        &self.config
    }

    /// Sets a custom piece-selection strategy (default: `RarestFirstSelector`).
    pub fn selector(mut self, selector: Arc<dyn PieceSelector>) -> Self {
        self.selector = selector;
        self
    }

    /// Attaches pre-parsed BEP 47 fast-resume data for instant verification and resume.
    pub fn fast_resume(mut self, data: FastResumeData) -> Self {
        self.fast_resume = Some(data);
        self
    }

    /// Attaches pre-parsed BEP 47 fast-resume data for instant verification and resume.
    pub fn fast_resume_data(mut self, data: FastResumeData) -> Self {
        self.fast_resume = Some(data);
        self
    }

    /// Attaches a path to a `.resume` or `.fastresume` file for instant verification and resume.
    pub fn fast_resume_file(mut self, path: impl AsRef<Path>) -> Self {
        self.fast_resume_file = Some(path.as_ref().to_path_buf());
        self
    }


    /// Adds an HTTP/HTTPS WebSeed mirror URL (BEP 19) to be used alongside peer downloads.
    pub fn with_web_seed(mut self, url: impl Into<String>) -> Self {
        let u = url.into();
        if !self.web_seeds.contains(&u) {
            self.web_seeds.push(u);
        }
        self
    }

    /// Adds multiple HTTP/HTTPS WebSeed mirror URLs (BEP 19) to be used alongside peer downloads.
    pub fn with_web_seeds(mut self, urls: impl IntoIterator<Item = String>) -> Self {
        for u in urls {
            if !self.web_seeds.contains(&u) {
                self.web_seeds.push(u);
            }
        }
        self
    }

    /// Configures whether canonical public WebTorrent WebSocket trackers are automatically
    /// injected when bootstrapping a magnet session with WebTorrent enabled (default: `true`).
    pub fn auto_inject_webtorrent_trackers(mut self, auto_inject: bool) -> Self {
        self.auto_inject_webtorrent = auto_inject;
        self
    }

    /// Consumes the builder and constructs the `TorrentSession`.
    ///
    /// # Errors
    /// Returns [`BitTorrentError`] if the torrent file or magnet link cannot be parsed,
    /// or if the download directory cannot be created.
    pub fn build(self) -> Result<TorrentSession, BitTorrentError> {
        let fast_resume = if let Some(data) = self.fast_resume {
            Some(data)
        } else if let Some(ref file_path) = self.fast_resume_file {
            FastResumeData::load_from_file(file_path).ok()
        } else {
            None
        };

        let mut session = match &self.source {
            TorrentSource::File(path) => {
                let mut sess = TorrentSession::new_with_options_internal(
                    path,
                    &self.download_path,
                    self.seeding,
                    self.config,
                    self.selector,
                    fast_resume,
                )?;
                if !self.web_seeds.is_empty() {
                    sess.inject_web_seeds(self.web_seeds);
                }
                sess
            }
            TorrentSource::Magnet(link) => {
                TorrentSession::new_magnet_with_options_internal(
                    link,
                    &self.download_path,
                    self.config,
                    self.selector,
                    self.web_seeds,
                    self.auto_inject_webtorrent,
                )?
            }
        };
        session.manager = self.manager;
        Ok(session)
    }
}
