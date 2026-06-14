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
//! use bittorrent_rs::{BlockStorage, MemStorage, MockSocket, AsyncSocket};
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

extern crate alloc;

#[path = "core/mod.rs"]
mod core_mods;
#[path = "network/mod.rs"]
mod network_mods;
#[path = "session/mod.rs"]
mod session_mods;
#[path = "storage/mod.rs"]
mod storage_mods;
#[path = "utils/mod.rs"]
mod utils_mods;

// Re-export original modules under their original names for backward compatibility
pub use core_mods::constants;
pub use core_mods::magnet;
pub use core_mods::metainfo;
#[cfg(feature = "std")]
pub use core_mods::selector;
#[cfg(feature = "std")]
pub use core_mods::torrent_context;

#[cfg(feature = "std")]
pub use network_mods::announcer;
#[cfg(feature = "std")]
pub use network_mods::dht;
#[cfg(feature = "std")]
pub use network_mods::host;
#[cfg(feature = "std")]
pub use network_mods::lsd;
#[cfg(feature = "std")]
pub use network_mods::mse;
#[cfg(feature = "std")]
pub use network_mods::nat;
#[cfg(feature = "std")]
pub use network_mods::peer;
#[cfg(feature = "std")]
pub use network_mods::peer_id;
pub use network_mods::peer_message;
#[cfg(feature = "std")]
pub use network_mods::peer_network;
#[cfg(feature = "std")]
pub use network_mods::tracker;
#[cfg(feature = "std")]
pub use network_mods::utp;

#[cfg(feature = "std")]
pub use storage_mods::assembler;
#[cfg(feature = "std")]
pub use storage_mods::disk_io;
#[cfg(feature = "std")]
pub use storage_mods::piece_buffer;
#[cfg(feature = "std")]
pub use storage_mods::piece_request;

#[cfg(feature = "std")]
pub use session_mods::manager;
#[cfg(feature = "std")]
pub use session_mods::session;
#[cfg(feature = "std")]
pub use session_mods::webseed;

pub use utils_mods::average;
pub use utils_mods::bencode;
pub use utils_mods::error;
pub use utils_mods::io_traits;
#[cfg(feature = "std")]
pub use utils_mods::manual_reset_event;
pub use utils_mods::util;

pub use utils_mods::average::Average;
pub use utils_mods::bencode::{BNode, Bencode};
pub use utils_mods::error::BitTorrentError;
pub use utils_mods::io_traits::{AsyncSocket, BlockStorage, MemStorage};
#[cfg(feature = "std")]
pub use utils_mods::io_traits::{MockSocket, SocketFactory};
#[cfg(all(feature = "std", feature = "http-tracker"))]
pub use utils_mods::io_traits::{HttpClient, UreqHttpClient};
#[cfg(feature = "std")]
pub use session_mods::manager::Manager;
pub use core_mods::magnet::MagnetLink;
pub use core_mods::metainfo::{FileDetails, MetaInfoFile};
#[cfg(feature = "std")]
pub use storage_mods::assembler::Assembler;
#[cfg(feature = "std")]
pub use network_mods::peer::Peer;
#[cfg(feature = "std")]
pub use network_mods::peer_id::get as get_peer_id;
pub use network_mods::peer_message::PeerMessage;
#[cfg(feature = "std")]
pub use core_mods::selector::{PieceSelector, RarestFirstSelector, SequentialSelector};
#[cfg(feature = "std")]
pub use network_mods::dht::Dht;
#[cfg(feature = "std")]
pub use network_mods::utp::UtpSocketAdapter;
#[cfg(feature = "std")]
pub use session_mods::session::{TorrentSession, TorrentSessionBuilder};
#[cfg(feature = "std")]
pub use core_mods::torrent_context::{TorrentContext, TorrentStatus};
#[cfg(feature = "std")]
pub use network_mods::tracker::{AnnounceResponse, PeerDetails, Tracker, TrackerEvent, TrackerStatus, ScrapeResponse};
