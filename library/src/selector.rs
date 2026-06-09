//! Piece selection strategy traits and implementations
//!
//! Provides the pluggable `PieceSelector` trait to customize piece selection
//! strategies (e.g. rarest-first vs. sequential download).

use crate::peer::Peer;
use crate::torrent_context::TorrentContext;

/// Pluggable piece selection strategy.
pub trait PieceSelector: std::fmt::Debug + Send + Sync {
    /// Selects the next piece index to request from the remote peer.
    fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32>;
}

/// Rarest-First piece selection strategy (default).
#[derive(Debug, Clone, Copy)]
pub struct RarestFirstSelector;

impl PieceSelector for RarestFirstSelector {
    fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32> {
        (0..context.number_of_pieces as u32)
            .filter(|&piece| !context.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
            .min_by_key(|&piece| (context.get_piece_peer_count(piece), piece))
    }
}

/// Sequential piece selection strategy (ideal for media streaming).
#[derive(Debug, Clone, Copy)]
pub struct SequentialSelector;

impl PieceSelector for SequentialSelector {
    fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32> {
        (0..context.number_of_pieces as u32)
            .find(|&piece| !context.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
    }
}
