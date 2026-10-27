//! Asynchronous session event stream and subscription broadcaster.

use crate::core::torrent_context::TorrentStatus;
use crate::utils::util::LockExt;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// Strongly typed lifecycle and progress events emitted by an active `TorrentSession`.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// The session's overall state has changed (e.g. Downloading, Seeding, Paused, Stopped).
    StateChanged(TorrentStatus),
    /// A piece was successfully downloaded and verified.
    PieceCompleted {
        piece_index: usize,
        total_completed: usize,
        total_pieces: usize,
    },
    /// All pieces spanning a specific file have completed download and verification.
    FileCompleted { file_index: usize, path: String },
    /// A new peer connected and successfully completed the BitTorrent wire handshake.
    PeerConnected { addr: String, client: String },
    /// An existing peer disconnected or encountered a protocol error.
    PeerDisconnected { addr: String, reason: String },
    /// BEP 9 extension metadata for a magnet link was fully assembled and validated.
    MetadataResolved {
        name: String,
        total_size: u64,
        files_count: usize,
    },
    /// All requested pieces (or entire torrent) have finished downloading and verified.
    DownloadFinished,
    /// An error occurred during session execution.
    Error(String),
}

/// Thread-safe multi-subscriber event broadcaster.
#[derive(Clone, Default)]
pub struct EventBroadcaster {
    subscribers: Arc<Mutex<Vec<Sender<SessionEvent>>>>,
}

impl EventBroadcaster {
    /// Creates a new, empty event broadcaster.
    pub fn new() -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Subscribes to the event stream, returning a standard `Receiver<SessionEvent>`.
    pub fn subscribe(&self) -> Receiver<SessionEvent> {
        let (tx, rx) = channel();
        self.subscribers.lock_safe().push(tx);
        rx
    }

    /// Broadcasts an event to all active subscribers, automatically pruning disconnected receivers.
    pub fn emit(&self, event: SessionEvent) {
        let mut subs = self.subscribers.lock_safe();
        subs.retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Returns the number of currently active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock_safe().len()
    }
}
