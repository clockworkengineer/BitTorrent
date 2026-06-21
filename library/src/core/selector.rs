//! Piece selection strategy traits and implementations
//!
//! Provides the pluggable `PieceSelector` trait to customize piece selection
//! strategies (e.g. rarest-first vs. sequential download).

use crate::peer::Peer;
use crate::core::torrent_context::TorrentContext;

/// Pluggable piece selection strategy.
pub trait PieceSelector: std::fmt::Debug + Send + Sync {
    /// Selects the next piece index to request from the remote peer.
    fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32>;
}

/// Rarest-First piece selection strategy (default).
#[derive(Debug, Clone, Copy)]
pub struct RarestFirstSelector;

impl PieceSelector for RarestFirstSelector {
    /// Selects the next piece index to request from the remote peer using a priority queue (binary heap).
    ///
    /// # Performance Trade-off
    /// - **Time Complexity**: Pushing/updating the priority queue takes O(log N) where N is the number of pieces.
    ///   Peeking and popping from the heap takes O(log N) amortized.
    /// - **Stale entries**: Populated incrementally when peer counts change, keeping selection efficient.
    ///   This eliminates the previous linear O(N) scan per peer request.
    /// - **Memory Trade-off**: Requires a small amount of extra memory to maintain the heap structure.
    fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32> {
        let mut pq = context.piece_priority_queue.lock().unwrap();
        let mut temporary_list = Vec::new();
        let mut selected = None;

        while let Some(rarity) = pq.peek() {
            let piece = rarity.piece_index;
            // Check if this entry is stale (piece already completed locally)
            if context.is_piece_local(piece) {
                pq.pop();
                continue;
            }
            // Check if peer count has changed (lazy deletion of stale entries)
            if context.get_piece_peer_count(piece) != rarity.peer_count {
                pq.pop();
                continue;
            }
            // It's valid! Now check if the remote peer actually has this piece.
            if peer.is_piece_on_remote_peer(piece) {
                selected = Some(piece);
                break;
            } else {
                // Stash it temporarily because we can't request it from this peer,
                // but other peers might have it.
                temporary_list.push(pq.pop().unwrap());
            }
        }

        // Put back the items we popped that this peer didn't have
        for item in temporary_list {
            pq.push(item);
        }

        selected
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
