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

#[cfg(feature = "std")]
pub mod announcer;
#[cfg(feature = "std")]
pub mod assembler;
pub mod average;
pub mod bencode;
pub mod constants;
#[cfg(feature = "std")]
pub mod disk_io;
pub mod error;
pub mod io_traits;
#[cfg(feature = "std")]
pub mod host;
#[cfg(feature = "std")]
pub mod manager;
#[cfg(feature = "std")]
pub mod manual_reset_event;
pub mod magnet;
pub mod metainfo;
#[cfg(feature = "std")]
pub mod peer;
#[cfg(feature = "std")]
pub mod peer_id;
pub mod peer_message;
#[cfg(feature = "std")]
pub mod peer_network;
#[cfg(feature = "std")]
pub mod piece_buffer;
#[cfg(feature = "std")]
pub mod piece_request;
#[cfg(feature = "std")]
pub mod selector;
#[cfg(feature = "std")]
pub mod dht;
#[cfg(feature = "std")]
pub mod session;
#[cfg(feature = "std")]
pub mod torrent_context;
#[cfg(feature = "std")]
pub mod tracker;
pub mod util;

pub use average::Average;
pub use bencode::{BNode, Bencode};
pub use error::BitTorrentError;
pub use io_traits::{AsyncSocket, BlockStorage, MemStorage};
#[cfg(feature = "std")]
pub use io_traits::{MockSocket, SocketFactory};
#[cfg(all(feature = "std", feature = "http-tracker"))]
pub use io_traits::{HttpClient, UreqHttpClient};
#[cfg(feature = "std")]
pub use manager::Manager;
pub use magnet::MagnetLink;
pub use metainfo::{FileDetails, MetaInfoFile};
#[cfg(feature = "std")]
pub use assembler::Assembler;
#[cfg(feature = "std")]
pub use peer::Peer;
#[cfg(feature = "std")]
pub use peer_id::get as get_peer_id;
pub use peer_message::PeerMessage;
#[cfg(feature = "std")]
pub use selector::{PieceSelector, RarestFirstSelector, SequentialSelector};
#[cfg(feature = "std")]
pub use dht::Dht;
#[cfg(feature = "std")]
pub use session::{TorrentSession, TorrentSessionBuilder};
#[cfg(feature = "std")]
pub use torrent_context::{TorrentContext, TorrentStatus};
#[cfg(feature = "std")]
pub use tracker::{AnnounceResponse, PeerDetails, Tracker, TrackerEvent, TrackerStatus};
