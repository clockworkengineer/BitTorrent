//! Session-wide configuration for `TorrentSession`.
//!
//! `SessionConfig` carries all tunable parameters: timeouts, feature flags,
//! pluggable socket/HTTP factories, and piece-selection hints.  Pass a custom
//! `SessionConfig` through `TorrentSessionBuilder::config()` to override the
//! defaults.

use std::sync::Arc;
use std::time::Duration;

/// Runtime configuration for a [`super::session::TorrentSession`].
///
/// All fields have sensible defaults via [`Default`]; override only what you need.
#[derive(Clone)]
pub struct SessionConfig {
    /// Timeout for establishing outgoing TCP connections (default: 10 s).
    pub connect_timeout: Duration,
    /// Timeout for socket read operations (default: 5 s).
    pub read_timeout: Duration,
    /// Timeout for socket write operations (default: 5 s).
    pub write_timeout: Duration,
    /// Minimum number of seconds between tracker re-announces (default: 60).
    pub min_reannounce_interval: u32,
    /// Factory used to create outbound peer sockets.
    pub socket_factory: Arc<dyn crate::io_traits::SocketFactory>,
    /// HTTP client used for HTTP tracker announces.
    #[cfg(feature = "http-tracker")]
    pub http_client: Arc<dyn crate::io_traits::HttpClient>,
    /// Enable the Kademlia DHT peer discovery subsystem.
    pub dht_enabled: bool,
    /// UDP port used for the DHT node (default: 6881).
    pub dht_port: u16,
    /// Listen port for incoming peer connections, LSD, and NAT-PMP (default: 6881).
    pub listen_port: u16,
    /// Enable Message Stream Encryption (MSE/PE) handshake obfuscation.
    pub mse_enabled: bool,
    /// Maximum concurrent peer connections per torrent (default: 50).
    pub max_connections: usize,
    /// Maximum candidate peers queued for discovery (default: 1000).
    pub max_peer_candidates: usize,
    /// Skip full piece hash verification of existing files on startup.
    pub skip_hash_check: bool,
    /// Allow Local Service Discovery even on private torrents.
    pub allow_private_lsd: bool,
    /// Timeout waiting for a peer handshake to complete (default: 5 s).
    pub handshake_timeout: Duration,
    /// Back-off delay before retrying a failed peer connection (default: 30 s).
    pub connection_backoff: Duration,
    /// Block size in bytes used for piece requests (default: `BLOCK_SIZE` constant).
    pub block_size: usize,
    /// Choking algorithm strategy.
    pub choking_strategy: Arc<dyn ChokingStrategy>,
}

pub trait ChokingStrategy: Send + Sync {
    fn spawn_choking_loop(
        &self,
        context: Arc<std::sync::Mutex<crate::core::torrent_context::TorrentContext>>,
        task_tx: std::sync::mpsc::Sender<core::pin::Pin<Box<dyn core::future::Future<Output = ()> + Send + 'static>>>,
        manager: Option<Arc<crate::manager::Manager>>,
    );
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StandardChoking;

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut ds = f.debug_struct("SessionConfig");
        ds.field("connect_timeout", &self.connect_timeout)
          .field("read_timeout", &self.read_timeout)
          .field("write_timeout", &self.write_timeout)
          .field("min_reannounce_interval", &self.min_reannounce_interval)
          .field("dht_enabled", &self.dht_enabled)
          .field("dht_port", &self.dht_port)
          .field("listen_port", &self.listen_port)
          .field("skip_hash_check", &self.skip_hash_check)
          .field("allow_private_lsd", &self.allow_private_lsd)
          .field("handshake_timeout", &self.handshake_timeout)
          .field("connection_backoff", &self.connection_backoff)
          .field("block_size", &self.block_size)
          .field("choking_strategy", &"Arc<dyn ChokingStrategy>");
        #[cfg(feature = "http-tracker")]
        ds.field("http_client", &self.http_client);
        ds.finish()
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        let connect_timeout = Duration::from_secs(10);
        let read_timeout    = Duration::from_secs(5);
        let write_timeout   = Duration::from_secs(5);
        SessionConfig {
            connect_timeout,
            read_timeout,
            write_timeout,
            min_reannounce_interval: 60,
            socket_factory: Arc::new(crate::peer_network::TcpSocketFactory {
                connect_timeout,
                read_timeout,
                write_timeout,
            }),
            #[cfg(feature = "http-tracker")]
            http_client: Arc::new(crate::io_traits::UreqHttpClient),
            dht_enabled: true,
            dht_port: 6881,
            listen_port: 6881,
            mse_enabled: false,
            max_connections: 50,
            max_peer_candidates: 1000,
            skip_hash_check: false,
            allow_private_lsd: false,
            handshake_timeout: Duration::from_secs(5),
            connection_backoff: Duration::from_secs(30),
            block_size: crate::constants::BLOCK_SIZE,
            choking_strategy: Arc::new(StandardChoking),
        }
    }
}

impl SessionConfig {
    /// Override the listen port for incoming connections, LSD, and NAT-PMP.
    pub fn with_listen_port(mut self, port: u16) -> Self {
        self.listen_port = port;
        self.dht_port = port;
        self
    }
}
