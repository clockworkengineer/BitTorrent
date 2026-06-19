//! Builder types for constructing a [`super::session::TorrentSession`].
//!
//! Use [`TorrentSessionBuilder`] for `.torrent` file sessions and
//! [`MagnetSessionBuilder`] for magnet-link sessions.  Both ultimately call
//! through to `TorrentSession::new_with_options` / `new_magnet_with_options`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::BitTorrentError;
use crate::manager::Manager;
use crate::selector::{PieceSelector, RarestFirstSelector};

use super::config::SessionConfig;
use super::session::TorrentSession;

// ─── .torrent builder ────────────────────────────────────────────────────────

/// Fluent builder for configuring and creating a `.torrent`-file `TorrentSession`.
///
/// # Example
/// ```no_run
/// use bittorrent_rs::TorrentSession;
///
/// let session = TorrentSession::builder("my.torrent", "/downloads")
///     .seeding(false)
///     .build()
///     .unwrap();
/// ```
pub struct TorrentSessionBuilder {
    pub(super) torrent_path: PathBuf,
    pub(super) download_path: PathBuf,
    pub(super) seeding: bool,
    pub(super) manager: Option<Arc<Manager>>,
    pub(super) config: SessionConfig,
    pub(super) selector: Arc<dyn PieceSelector>,
}

impl TorrentSessionBuilder {
    /// Creates a new builder with the given torrent file and download directory.
    pub fn new(torrent_path: impl AsRef<Path>, download_path: impl AsRef<Path>) -> Self {
        TorrentSessionBuilder {
            torrent_path: torrent_path.as_ref().to_path_buf(),
            download_path: download_path.as_ref().to_path_buf(),
            seeding: false,
            manager: None,
            config: SessionConfig::default(),
            selector: Arc::new(RarestFirstSelector),
        }
    }

    /// Sets whether the session begins in seeding mode (all pieces already local).
    pub fn seeding(mut self, seeding: bool) -> Self {
        self.seeding = seeding;
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

    /// Sets a custom piece-selection strategy (default: `RarestFirstSelector`).
    pub fn selector(mut self, selector: Arc<dyn PieceSelector>) -> Self {
        self.selector = selector;
        self
    }

    /// Consumes the builder and constructs the `TorrentSession`.
    ///
    /// # Errors
    /// Returns [`BitTorrentError`] if the torrent file cannot be read, parsed,
    /// or validated, or if the download directory cannot be created.
    pub fn build(self) -> Result<TorrentSession, BitTorrentError> {
        let mut session = TorrentSession::new_with_options(
            self.torrent_path,
            self.download_path,
            self.seeding,
            self.config,
            self.selector,
        )?;
        session.manager = self.manager;
        Ok(session)
    }
}

// ─── Magnet-link builder ─────────────────────────────────────────────────────

/// Fluent builder for constructing a magnet-link `TorrentSession`.
///
/// # Example
/// ```no_run
/// use bittorrent_rs::MagnetSessionBuilder;
///
/// let session = MagnetSessionBuilder::new(
///     "magnet:?xt=urn:btih:…",
///     "/downloads",
/// )
/// .build()
/// .unwrap();
/// ```
pub struct MagnetSessionBuilder {
    pub(super) magnet_link: String,
    pub(super) download_path: PathBuf,
    pub(super) manager: Option<Arc<Manager>>,
    pub(super) config: SessionConfig,
    pub(super) selector: Arc<dyn PieceSelector>,
}

impl MagnetSessionBuilder {
    /// Creates a new builder from a magnet URI and a target download directory.
    pub fn new(magnet_link: impl Into<String>, download_path: impl AsRef<Path>) -> Self {
        MagnetSessionBuilder {
            magnet_link: magnet_link.into(),
            download_path: download_path.as_ref().to_path_buf(),
            manager: None,
            config: SessionConfig::default(),
            selector: Arc::new(RarestFirstSelector),
        }
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

    /// Sets a custom piece-selection strategy (default: `RarestFirstSelector`).
    pub fn selector(mut self, selector: Arc<dyn PieceSelector>) -> Self {
        self.selector = selector;
        self
    }

    /// Consumes the builder and constructs the `TorrentSession`.
    ///
    /// # Errors
    /// Returns [`BitTorrentError`] if the magnet link cannot be parsed or the
    /// download directory cannot be created.
    pub fn build(self) -> Result<TorrentSession, BitTorrentError> {
        let mut session = TorrentSession::new_magnet_with_options(
            &self.magnet_link,
            self.download_path,
            self.config,
            self.selector,
        )?;
        session.manager = self.manager;
        Ok(session)
    }
}
