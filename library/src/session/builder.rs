//! Builder types for constructing a [`super::session::TorrentSession`].
//!
//! Use [`TorrentSessionBuilder`] to configure and construct a session from a `.torrent` file
//! or a magnet link.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::BitTorrentError;
use crate::manager::Manager;
use crate::selector::{PieceSelector, RarestFirstSelector};

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
    /// Returns [`BitTorrentError`] if the torrent file or magnet link cannot be parsed,
    /// or if the download directory cannot be created.
    pub fn build(self) -> Result<TorrentSession, BitTorrentError> {
        let mut session = match &self.source {
            TorrentSource::File(path) => {
                TorrentSession::new_with_options_internal(
                    path,
                    &self.download_path,
                    self.seeding,
                    self.config,
                    self.selector,
                )?
            }
            TorrentSource::Magnet(link) => {
                TorrentSession::new_magnet_with_options_internal(
                    link,
                    &self.download_path,
                    self.config,
                    self.selector,
                )?
            }
        };
        session.manager = self.manager;
        Ok(session)
    }
}
