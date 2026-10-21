//! Abstract event listener interfaces for peer connection events.
//!
//! Replaces direct peer-to-coordinator cyclic coupling with decoupled event sinks (DIP & SRP).

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt::Debug;

/// Decoupled event notification sink emitted by a peer connection.
pub trait PeerEventListener: Send + Sync {
    /// Notifies that a remote peer has completed a block transfer.
    fn on_block_received(&self, _peer_ip: &str, _piece_index: u32, _begin: u32, _block: &[u8]) {}

    /// Notifies that a remote peer announced possession of a piece.
    fn on_peer_have(&self, _peer_ip: &str, _piece_index: u32) {}

    /// Notifies that a remote peer updated its bitfield.
    fn on_peer_bitfield(&self, _peer_ip: &str, _bitfield: &[u8]) {}

    /// Notifies that a remote peer choked this connection.
    fn on_peer_choked(&self, _peer_ip: &str) {}

    /// Notifies that a remote peer unchoked this connection.
    fn on_peer_unchoked(&self, _peer_ip: &str) {}

    /// Notifies that a peer disconnected or encountered a terminal socket error.
    fn on_peer_disconnected(&self, _peer_ip: &str) {}
}

/// No-op dummy event listener for standalone peer testing without a coordinator.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullPeerEventListener;

impl PeerEventListener for NullPeerEventListener {}

/// Thread-safe recording event listener for inspection in unit tests.
#[derive(Debug, Default)]
pub struct RecordingPeerEventListener {
    pub blocks: std::sync::Mutex<Vec<(String, u32, u32, usize)>>,
    pub haves: std::sync::Mutex<Vec<(String, u32)>>,
    pub bitfields: std::sync::Mutex<Vec<(String, Vec<u8>)>>,
    pub chokes: std::sync::Mutex<Vec<String>>,
    pub unchokes: std::sync::Mutex<Vec<String>>,
    pub disconnects: std::sync::Mutex<Vec<String>>,
}

impl RecordingPeerEventListener {
    pub fn new() -> Self {
        Self::default()
    }
}

impl PeerEventListener for RecordingPeerEventListener {
    fn on_block_received(&self, peer_ip: &str, piece_index: u32, begin: u32, block: &[u8]) {
        if let Ok(mut guard) = self.blocks.lock() {
            guard.push((peer_ip.into(), piece_index, begin, block.len()));
        }
    }

    fn on_peer_have(&self, peer_ip: &str, piece_index: u32) {
        if let Ok(mut guard) = self.haves.lock() {
            guard.push((peer_ip.into(), piece_index));
        }
    }

    fn on_peer_bitfield(&self, peer_ip: &str, bitfield: &[u8]) {
        if let Ok(mut guard) = self.bitfields.lock() {
            guard.push((peer_ip.into(), bitfield.to_vec()));
        }
    }

    fn on_peer_choked(&self, peer_ip: &str) {
        if let Ok(mut guard) = self.chokes.lock() {
            guard.push(peer_ip.into());
        }
    }

    fn on_peer_unchoked(&self, peer_ip: &str) {
        if let Ok(mut guard) = self.unchokes.lock() {
            guard.push(peer_ip.into());
        }
    }

    fn on_peer_disconnected(&self, peer_ip: &str) {
        if let Ok(mut guard) = self.disconnects.lock() {
            guard.push(peer_ip.into());
        }
    }
}

/// Event listener adapter forwarding peer events into TorrentContext.
#[cfg(feature = "std")]
pub struct ContextEventListener {
    context: Arc<std::sync::Mutex<crate::core::torrent_context::TorrentContext>>,
}

#[cfg(feature = "std")]
impl ContextEventListener {
    pub fn new(
        context: Arc<std::sync::Mutex<crate::core::torrent_context::TorrentContext>>,
    ) -> Self {
        Self { context }
    }
}

#[cfg(feature = "std")]
impl PeerEventListener for ContextEventListener {
    fn on_block_received(&self, _peer_ip: &str, _piece_index: u32, _begin: u32, _block: &[u8]) {
        // Block processing into TorrentContext is coordinated via handle_peer_message
        // and storage pipeline to prevent non-reentrant mutex deadlocks and duplicate processing.
    }

    fn on_peer_have(&self, _peer_ip: &str, _piece_index: u32) {
        // Peer piece availability updates are handled directly in handle_peer_message
        // to prevent non-reentrant mutex deadlocks and duplicate processing.
    }

    fn on_peer_bitfield(&self, _peer_ip: &str, _bitfield: &[u8]) {}

    fn on_peer_disconnected(&self, peer_ip: &str) {
        if let Ok(mut ctx) = self.context.try_lock() {
            ctx.remove_peer(peer_ip);
        }
    }
}
