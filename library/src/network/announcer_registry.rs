//! Extensible protocol registry for tracker announcers (Open/Closed Principle).
//!
//! Replaces hardcoded string-matching factories with an open scheme registry,
//! enabling custom announcer protocols, mock announcers, and transport adapters
//! without modifying existing library code.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use std::sync::{OnceLock, RwLock};

use crate::error::BitTorrentError;
use crate::network::announcer::{Announcer, AnnouncerEnum, UdpAnnouncer};

#[cfg(feature = "http-tracker")]
use crate::network::announcer::HttpAnnouncer;

#[cfg(feature = "webrtc")]
use crate::network::webrtc::WebTorrentAnnouncer;

/// Factory builder trait for constructing protocol-specific `Announcer` instances (OCP).
pub trait AnnouncerBuilder: Send + Sync {
    /// Builds a trait-object `Announcer` for the given tracker URL.
    fn build_announcer(&self, url: &str) -> Result<Box<dyn Announcer>, BitTorrentError>;

    /// Builds an `AnnouncerEnum` variant for the given tracker URL.
    ///
    /// The default implementation wraps the trait-object from `build_announcer` into `AnnouncerEnum::Custom`.
    fn build_enum(&self, url: &str) -> Result<AnnouncerEnum, BitTorrentError> {
        let announcer = self.build_announcer(url)?;
        Ok(AnnouncerEnum::Custom(announcer))
    }
}

/// Builder for HTTP and HTTPS tracker announcers.
#[derive(Debug, Clone, Copy, Default)]
pub struct HttpAnnouncerBuilder;

impl AnnouncerBuilder for HttpAnnouncerBuilder {
    fn build_announcer(&self, _url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        #[cfg(feature = "http-tracker")]
        {
            Ok(Box::new(HttpAnnouncer::new()))
        }
        #[cfg(not(feature = "http-tracker"))]
        {
            Err(BitTorrentError::Parse(
                "HTTP tracker support is disabled".into(),
            ))
        }
    }

    fn build_enum(&self, _url: &str) -> Result<AnnouncerEnum, BitTorrentError> {
        #[cfg(feature = "http-tracker")]
        {
            Ok(AnnouncerEnum::Http(HttpAnnouncer::new()))
        }
        #[cfg(not(feature = "http-tracker"))]
        {
            Err(BitTorrentError::Parse(
                "HTTP tracker support is disabled".into(),
            ))
        }
    }
}

/// Builder for UDP tracker announcers.
#[derive(Debug, Clone, Copy, Default)]
pub struct UdpAnnouncerBuilder;

impl AnnouncerBuilder for UdpAnnouncerBuilder {
    fn build_announcer(&self, url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        Ok(Box::new(UdpAnnouncer::new(url)?))
    }

    fn build_enum(&self, url: &str) -> Result<AnnouncerEnum, BitTorrentError> {
        Ok(AnnouncerEnum::Udp(UdpAnnouncer::new(url)?))
    }
}

/// Builder for WebTorrent WebSocket tracker announcers (`ws://`, `wss://`).
#[derive(Debug, Clone, Copy, Default)]
pub struct WebTorrentAnnouncerBuilder;

impl AnnouncerBuilder for WebTorrentAnnouncerBuilder {
    fn build_announcer(&self, url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        #[cfg(feature = "webrtc")]
        {
            Ok(Box::new(WebTorrentAnnouncer::new(url)))
        }
        #[cfg(not(feature = "webrtc"))]
        {
            let _ = url;
            Err(BitTorrentError::Parse(
                "WebTorrent WebSocket tracker support is disabled".into(),
            ))
        }
    }

    fn build_enum(&self, url: &str) -> Result<AnnouncerEnum, BitTorrentError> {
        #[cfg(feature = "webrtc")]
        {
            Ok(AnnouncerEnum::WebTorrent(WebTorrentAnnouncer::new(url)))
        }
        #[cfg(not(feature = "webrtc"))]
        {
            let _ = url;
            Err(BitTorrentError::Parse(
                "WebTorrent WebSocket tracker support is disabled".into(),
            ))
        }
    }
}

/// Extensible registry mapping URI scheme prefixes to `AnnouncerBuilder` implementations.
pub struct AnnouncerRegistry {
    builders: Vec<(String, Box<dyn AnnouncerBuilder>)>,
}

impl Default for AnnouncerRegistry {
    fn default() -> Self {
        Self::new_with_defaults()
    }
}

impl AnnouncerRegistry {
    /// Creates an empty registry without pre-registered builders.
    pub fn new() -> Self {
        Self {
            builders: Vec::new(),
        }
    }

    /// Creates a registry pre-populated with standard BitTorrent protocol builders.
    pub fn new_with_defaults() -> Self {
        let mut registry = Self::new();

        #[cfg(feature = "http-tracker")]
        {
            registry.register("http://", HttpAnnouncerBuilder);
            registry.register("https://", HttpAnnouncerBuilder);
        }

        registry.register("udp://", UdpAnnouncerBuilder);

        #[cfg(feature = "webrtc")]
        {
            registry.register("ws://", WebTorrentAnnouncerBuilder);
            registry.register("wss://", WebTorrentAnnouncerBuilder);
        }

        registry
    }

    /// Registers a protocol prefix and associated builder (OCP).
    ///
    /// New registrations take precedence over older registrations with matching prefixes,
    /// enabling users to override default protocol handlers.
    pub fn register(&mut self, prefix: impl Into<String>, builder: impl AnnouncerBuilder + 'static) {
        self.builders.insert(0, (prefix.into(), Box::new(builder)));
    }

    /// Unregisters all builders matching the specified prefix.
    pub fn unregister(&mut self, prefix: &str) -> bool {
        let initial_len = self.builders.len();
        self.builders.retain(|(p, _)| p != prefix);
        self.builders.len() < initial_len
    }

    /// Returns the count of registered protocol prefixes.
    pub fn len(&self) -> usize {
        self.builders.len()
    }

    /// Returns true if the registry contains no protocol builders.
    pub fn is_empty(&self) -> bool {
        self.builders.is_empty()
    }

    /// Creates an `Announcer` trait object for a given URL by matching registered prefixes.
    pub fn create_announcer(&self, url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        for (prefix, builder) in &self.builders {
            if url.starts_with(prefix) {
                return builder.build_announcer(url);
            }
        }
        Err(BitTorrentError::Parse(alloc::format!(
            "Unsupported tracker scheme: {}",
            url
        )))
    }

    /// Creates an `AnnouncerEnum` variant for a given URL by matching registered prefixes.
    pub fn create(&self, url: &str) -> Result<AnnouncerEnum, BitTorrentError> {
        for (prefix, builder) in &self.builders {
            if url.starts_with(prefix) {
                return builder.build_enum(url);
            }
        }
        Err(BitTorrentError::Parse(alloc::format!(
            "Unsupported tracker scheme: {}",
            url
        )))
    }
}

static GLOBAL_REGISTRY: OnceLock<RwLock<AnnouncerRegistry>> = OnceLock::new();

/// Accesses the global shared `AnnouncerRegistry`.
pub fn global_registry() -> &'static RwLock<AnnouncerRegistry> {
    GLOBAL_REGISTRY.get_or_init(|| RwLock::new(AnnouncerRegistry::new_with_defaults()))
}

/// Registers a custom protocol prefix and builder into the global registry.
pub fn register_global(prefix: impl Into<String>, builder: impl AnnouncerBuilder + 'static) {
    let mut reg = global_registry().write().unwrap();
    reg.register(prefix, builder);
}
