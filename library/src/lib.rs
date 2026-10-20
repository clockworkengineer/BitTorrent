//! BitTorrent Library
//!
//! A comprehensive BitTorrent protocol implementation in Rust, containing
//! metainfo parsing, tracker communication, peer wire protocol, piece selection,
//! disk I/O, and session management.
//!
//! # Examples
//!
//! ```no_run
//! use bittorrent_rs::{TorrentSession, Tracker};
//! use std::path::Path;
//! use std::sync::Arc;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let torrent_path = Path::new("torrents/wired-cd.torrent");
//!     let download_dir = Path::new("downloads");
//!
//!     // Initialize a new BitTorrent session using the builder pattern
//!     let mut session = TorrentSession::builder(torrent_path, download_dir)
//!         .seeding(false)
//!         .build()?;
//!
//!     // Start the download thread and initialize storage
//!     session.start_download()?;
//!
//!     // Initialize the tracker client for peer discovery
//!     let mut tracker = Tracker::new(session.context())?;
//!
//!     // Query trackers to retrieve the list of active peers
//!     let announce_response = tracker.start_announcing()?;
//!     println!("Discovered {} peers", announce_response.peer_list.len());
//!
//!     // Start downloading pieces from discovered peers
//!     session.download_from_peers(announce_response.peer_list)?;
//!
//!     // Monitor progress in a loop or await completion
//!     loop {
//!         if let Ok(ctx) = session.context().lock() {
//!             println!("Progress: {:.2}%", ctx.progress_percent());
//!             if ctx.progress_percent() >= 100.0 {
//!                 println!("Download completed!");
//!                 break;
//!             }
//!         }
//!         std::thread::sleep(std::time::Duration::from_secs(5));
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Portability and Testing
//!
//! To enable testing and running in environments without physical disk access or active network connections (such as simulated testing setups or clockless systems), `bittorrent-rs` abstracts transport sockets and block storage under the `AsyncSocket` and `BlockStorage` traits.
//!
//! In-memory storage is provided by `MemStorage`, and simulated peer connections can be modeled using `MockSocket`:
//!
//! ```no_run
//! use bittorrent_rs::{BlockStorage, BlockWriter, MemStorage, MockSocket, AsyncSocket};
//! use std::sync::Arc;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 1. Setup in-memory block storage
//!     let storage = MemStorage::new(1024 * 1024); // 1 MB buffer
//!     storage.write_block(0, b"mock piece block data")?;
//!
//!     // 2. Setup mock peer communications
//!     let (socket, in_tx, out_rx) = MockSocket::new();
//!     let socket = Arc::new(socket);
//!
//!     // Simulate incoming bytes from a remote peer
//!     in_tx.send(b"incoming wire protocol handshake data".to_vec())?;
//!
//!     // The client interacts with the storage and socket interfaces polymorphically
//!     Ok(())
//! }
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![allow(
    clippy::collapsible_if,
    clippy::manual_is_ascii_check,
    clippy::needless_borrow,
    clippy::unnecessary_lazy_evaluations
)]

extern crate alloc;

pub mod core;
pub mod network;
pub mod session;
pub mod storage;
pub mod utils;

// Re-export original modules under their original names for backward compatibility
pub use core::constants;
pub use core::magnet;
pub use core::metainfo;
#[cfg(feature = "std")]
pub use core::selector;
#[cfg(feature = "std")]
pub use core::torrent_context::{
    PieceWriteAccess, SessionStatusQuery, SwarmReadAccess, TorrentStatus,
};
#[cfg(feature = "std")]
pub use core::views;
#[cfg(feature = "std")]
pub use core::views::{
    MockPieceAvailabilityView, PeerSnapshot, PieceAvailabilityView, RemotePeerPieceView, SwarmView,
    TransferStatsView,
};

#[cfg(feature = "std")]
pub use network::announcer;
#[cfg(feature = "std")]
pub use network::announcer_registry;
#[cfg(feature = "std")]
pub use network::announcer_registry::{
    AnnouncerBuilder, AnnouncerRegistry, HttpAnnouncerBuilder, UdpAnnouncerBuilder,
    WebTorrentAnnouncerBuilder,
};
#[cfg(all(feature = "std", feature = "dht"))]
pub use network::dht;
#[cfg(feature = "std")]
pub use network::host;
#[cfg(all(feature = "std", feature = "lsd"))]
pub use network::lsd;
#[cfg(all(feature = "std", feature = "mse"))]
pub use network::mse;
#[cfg(all(feature = "std", feature = "nat-pmp"))]
pub use network::nat;
#[cfg(feature = "std")]
pub use network::peer;
#[cfg(feature = "std")]
pub use network::peer_id;
#[cfg(feature = "std")]
pub use network::peer_id::parse_peer_client;
pub use network::peer_message;
#[cfg(feature = "std")]
pub use network::peer_network;
#[cfg(feature = "std")]
pub use network::tracker;
#[cfg(all(feature = "std", feature = "utp"))]
pub use network::utp;
#[cfg(all(feature = "std", feature = "webrtc"))]
pub use network::webrtc;
#[cfg(feature = "std")]
pub use network::webseed_tracker::{
    MirrorHealthProfile, MirrorHealthSummary, WebSeedMirrorTracker,
};

#[cfg(feature = "std")]
pub use storage::assembler;
#[cfg(feature = "std")]
pub use storage::disk_io;
#[cfg(feature = "std")]
pub use storage::fast_resume::{FastResumeData, FastResumeFileEntry};
#[cfg(feature = "std")]
pub use storage::piece_buffer;
#[cfg(feature = "std")]
pub use storage::piece_request;
#[cfg(feature = "std")]
pub use storage::stream;
#[cfg(feature = "std")]
pub use storage::stream::{StorageBlockReader, TorrentStream};

#[cfg(feature = "std")]
pub use session::manager;
#[cfg(all(feature = "std", feature = "webseed"))]
pub use session::webseed;

pub use utils::average;
pub use utils::bencode;
pub use utils::error;
pub use utils::io_traits;
#[cfg(feature = "std")]
pub use utils::manual_reset_event;
pub use utils::util;
pub use utils::util::{from_hex, info_hash_to_string, to_hex, to_hex_upper};

pub use core::magnet::{
    MagnetLink, MagnetUriBuilder, format_bep53_select_only, parse_bep53_select_only,
    percent_decode, percent_encode,
};
pub use core::metainfo::{FileDetails, FilePriority, MetaInfoFile};
#[cfg(feature = "std")]
pub use core::selector::{
    PieceSelector, RarestFirstSelector, SequentialSelector, StreamingPieceSelector,
};
pub use network::holepunch::{HolepunchErrorCode, HolepunchMessage, HolepunchMessageType};
#[cfg(feature = "std")]
pub use network::metadata_fetcher::{
    BEP53SelectionEvaluation, MAX_METADATA_SIZE, METADATA_PIECE_SIZE, MetadataFetchMetrics,
    MetadataRangeFetcher, PartialFileInfo, PartialMetadataManifest,
};
#[cfg(feature = "std")]
pub use network::peer::{Peer, PeerAction};
pub use network::peer_details::PeerDetails;
#[cfg(feature = "std")]
pub use network::peer_id::get as get_peer_id;
pub use network::peer_message::PeerMessage;
pub use network::pex::{PexFlags, PexMessage, PexPeer};
#[cfg(feature = "std")]
pub use network::webtorrent_injector::{
    DEFAULT_WEBTORRENT_TRACKERS, InjectionSummary, TrackerExtractionReport,
    WebTorrentMagnetInjector,
};
#[cfg(feature = "std")]
pub use session::manager::Manager;
#[cfg(feature = "std")]
pub use storage::assembler::Assembler;
#[cfg(feature = "std")]
pub use storage::disk_cache::{CacheBlock, CacheMetrics, CachedBlockStorage};
pub use utils::average::Average;
pub use utils::bencode::{BNode, Bencode};
pub use utils::bencode_tokenizer::{BencodeToken, BencodeTokenizer};
pub use utils::error::{BencodeError, BitTorrentError};
#[cfg(feature = "std")]
pub use utils::io_traits::SocketFactory;
pub use utils::io_traits::{
    AsyncSocket, BlockReader, BlockStorage, BlockWriter, MemStorage, MockReceiver, MockSender,
    MockSocket, Socket,
};
#[cfg(all(feature = "std", feature = "http-tracker"))]
pub use utils::io_traits::{HttpClient, UreqHttpClient};

#[cfg(feature = "v2")]
pub use core::merkle::{BLOCK_SIZE as V2_BLOCK_SIZE, MerkleProof, MerkleTree};
#[cfg(all(feature = "std", feature = "v2"))]
pub use core::torrent_builder::TorrentBuilder;
#[cfg(feature = "std")]
pub use core::torrent_context::TorrentStatus as TorrentContextStatus;
#[cfg(all(feature = "std", feature = "v2"))]
pub use core::torrent_upgrader::{TorrentExportFormat, TorrentUpgradeReport, TorrentUpgrader};
#[cfg(all(feature = "std", feature = "dht"))]
pub use network::dht::Dht;
#[cfg(feature = "std")]
pub use network::latency_profiler::{PeerLatencyProfile, PeerLatencySummary, SwarmLatencyProfiler};
#[cfg(all(feature = "std", feature = "mse"))]
pub use network::peer_network::EncryptedSocket;
#[cfg(all(feature = "std", feature = "utp"))]
pub use network::utp::{
    DelayBucket, DelayTracker, LedbatCongestionController, LossRecoveryState, SentPacket,
    UTP_EXT_EXTENSION_BITS, UTP_EXT_SACK, UtpExtension, UtpHeader, UtpPacketType,
    UtpPipelineMetrics, UtpSocketAdapter, build_sack_bitmask, detect_receive_gaps, is_seq_sacked,
    parse_sacked_seqs,
};
#[cfg(all(feature = "std", feature = "webrtc"))]
pub use network::webrtc::{
    DataChannelState, MockWebSocketSignalingChannel, SdpDescription, WebRtcSocketAdapter,
    WebRtcStats, WebSocketSignalingClient, WebTorrentAnnounceRequest, WebTorrentAnnounceResponse,
    WebTorrentAnnouncer, WebTorrentAnswer, WebTorrentBridge, WebTorrentBridgeStats,
    WebTorrentIncomingMessage, WebTorrentOffer, WebTorrentScrapeFile, WebTorrentScrapeRequest,
    WebTorrentScrapeResponse, create_webtorrent_answer, create_webtorrent_offer,
};
#[cfg(feature = "std")]
pub use network::peer_event::{
    ContextEventListener, NullPeerEventListener, PeerEventListener, RecordingPeerEventListener,
};
#[cfg(feature = "std")]
pub use session::client::TorrentClient;
#[cfg(feature = "std")]
#[cfg(feature = "std")]
pub use session::config::SessionConfig;
#[cfg(feature = "std")]
pub use session::session::{TorrentSession, TorrentSessionBuilder};
#[cfg(feature = "std")]
pub use storage::erasure_staging::{
    AdaptiveErasureStaging, BlockArrivalOutcome, EndgameStagingMetrics, StagedBlock,
};
#[cfg(all(feature = "std", feature = "v2"))]
pub use storage::piece_layers_cache::{PieceLayersCacheMetrics, PieceLayersDiskCache};
#[cfg(feature = "std")]
pub use storage::verification_pool::{
    HashVerificationPool, PieceVerificationJob, PieceVerificationResult, RecheckProgress,
    VerificationPoolMetrics,
}; // Keep alias if needed, but TorrentStatus is already exported above

/// Internal and unstable APIs of the bittorrent-rs library.
///
/// WARNING: These APIs are exposed for advanced use and testing, but they are not
/// part of the stable API surface and may change without a major version bump.
pub mod internals {
    #[cfg(feature = "std")]
    pub use crate::core::torrent_context::TorrentContext;
}
#[cfg(feature = "std")]
pub use network::tracker::{
    AnnounceResponse, ScrapeResponse, Tracker, TrackerEvent, TrackerStatus,
};
