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
pub use core::torrent_context;

#[cfg(feature = "std")]
pub use network::announcer;
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
pub use network::peer_message;
#[cfg(feature = "std")]
pub use network::peer_network;
#[cfg(feature = "std")]
pub use network::tracker;
#[cfg(all(feature = "std", feature = "utp"))]
pub use network::utp;

#[cfg(feature = "std")]
pub use storage::assembler;
#[cfg(feature = "std")]
pub use storage::disk_io;
#[cfg(feature = "std")]
pub use storage::piece_buffer;
#[cfg(feature = "std")]
pub use storage::piece_request;

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

pub use utils::average::Average;
pub use utils::bencode::{BNode, Bencode};
pub use utils::bencode_tokenizer::{BencodeToken, BencodeTokenizer};
pub use utils::error::{BitTorrentError, BencodeError};
pub use utils::io_traits::{AsyncSocket, BlockStorage, MemStorage, MockSocket, MockSender, MockReceiver, Socket};
#[cfg(feature = "std")]
pub use utils::io_traits::SocketFactory;
#[cfg(all(feature = "std", feature = "http-tracker"))]
pub use utils::io_traits::{HttpClient, UreqHttpClient};
#[cfg(feature = "std")]
pub use session::manager::Manager;
pub use core::magnet::MagnetLink;
pub use core::metainfo::{FileDetails, MetaInfoFile};
#[cfg(feature = "std")]
pub use storage::assembler::Assembler;
#[cfg(feature = "std")]
pub use network::peer::Peer;
#[cfg(feature = "std")]
pub use network::peer_id::get as get_peer_id;
pub use network::peer_message::PeerMessage;
#[cfg(feature = "std")]
pub use core::selector::{PieceSelector, RarestFirstSelector, SequentialSelector};
#[cfg(all(feature = "std", feature = "dht"))]
pub use network::dht::Dht;
#[cfg(all(feature = "std", feature = "utp"))]
pub use network::utp::UtpSocketAdapter;
#[cfg(feature = "std")]
pub use session::session::{TorrentSession, TorrentSessionBuilder};
#[cfg(feature = "std")]
pub use session::builder::MagnetSessionBuilder;
#[cfg(feature = "std")]
pub use session::config::SessionConfig;
#[cfg(feature = "std")]
pub use session::client::TorrentClient;
#[cfg(feature = "std")]
pub use core::torrent_context::{TorrentContext, TorrentStatus};
#[cfg(feature = "std")]
pub use network::tracker::{AnnounceResponse, PeerDetails, Tracker, TrackerEvent, TrackerStatus, ScrapeResponse};
