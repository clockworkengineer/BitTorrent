//! BitTorrent Library
//!
//! A comprehensive BitTorrent protocol implementation in Rust, containing
//! metainfo parsing, tracker communication, peer wire protocol, piece selection,
//! disk I/O, and session management.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
pub mod announcer;
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
pub mod selector;
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
pub use io_traits::{AsyncSocket, BlockStorage};
#[cfg(feature = "std")]
pub use manager::Manager;
pub use metainfo::{FileDetails, MetaInfoFile};
#[cfg(feature = "std")]
pub use peer::Peer;
#[cfg(feature = "std")]
pub use peer_id::get as get_peer_id;
pub use peer_message::PeerMessage;
pub use selector::Selector;
#[cfg(feature = "std")]
pub use session::TorrentSession;
#[cfg(feature = "std")]
pub use torrent_context::{TorrentContext, TorrentStatus};
#[cfg(feature = "std")]
pub use tracker::{AnnounceResponse, PeerDetails, Tracker, TrackerEvent, TrackerStatus};
