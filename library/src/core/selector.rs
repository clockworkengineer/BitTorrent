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
            // Check if this piece is desired (not skipped by selective download)
            if !context.is_piece_wanted(piece) {
                pq.pop();
                continue;
            }
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
            .find(|&piece| context.is_piece_wanted(piece) && !context.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece))
    }
}

/// Streaming piece selection strategy (ideal for media streaming / seekable reading).
///
/// Prioritizes pieces within a forward window around an atomic playback cursor (`cursor_piece`).
/// When selecting a piece to download:
/// 1. Prioritizes missing pieces in `[cursor .. cursor + lookahead_window)` that the remote peer possesses.
/// 2. If all pieces in the lookahead window are already local or unavailable from this peer,
///    checks pieces beyond the window (`[cursor + lookahead_window .. total_pieces)`),
///    then wraps around to (`[0 .. cursor)`).
#[derive(Debug, Clone)]
pub struct StreamingPieceSelector {
    cursor_piece: alloc::sync::Arc<core::sync::atomic::AtomicU32>,
    lookahead_window: u32,
}

impl StreamingPieceSelector {
    /// Creates a new `StreamingPieceSelector` with an atomic cursor and a lookahead window (in pieces).
    pub fn new(cursor_piece: alloc::sync::Arc<core::sync::atomic::AtomicU32>, lookahead_window: u32) -> Self {
        Self {
            cursor_piece,
            lookahead_window,
        }
    }

    /// Returns a reference to the atomic playback cursor piece index.
    pub fn cursor_piece(&self) -> &alloc::sync::Arc<core::sync::atomic::AtomicU32> {
        &self.cursor_piece
    }

    /// Sets or updates the playback cursor piece index.
    pub fn set_cursor_piece(&self, piece: u32) {
        self.cursor_piece.store(piece, core::sync::atomic::Ordering::Relaxed);
    }

    /// Returns the lookahead window size in pieces.
    pub fn lookahead_window(&self) -> u32 {
        self.lookahead_window
    }
}

impl PieceSelector for StreamingPieceSelector {
    fn select_piece(&self, context: &TorrentContext, peer: &Peer) -> Option<u32> {
        let cursor = self.cursor_piece.load(core::sync::atomic::Ordering::Relaxed);
        let total_pieces = context.number_of_pieces as u32;

        if total_pieces == 0 {
            return None;
        }

        let window_end = (cursor.saturating_add(self.lookahead_window)).min(total_pieces);

        // 1. Check lookahead window [cursor .. cursor + lookahead_window)
        for piece in cursor..window_end {
            if context.is_piece_wanted(piece) && !context.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece) {
                return Some(piece);
            }
        }

        // 2. Check remaining pieces from window_end to end of torrent
        for piece in window_end..total_pieces {
            if context.is_piece_wanted(piece) && !context.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece) {
                return Some(piece);
            }
        }

        // 3. Wrap around [0 .. cursor)
        for piece in 0..cursor.min(total_pieces) {
            if context.is_piece_wanted(piece) && !context.is_piece_local(piece) && peer.is_piece_on_remote_peer(piece) {
                return Some(piece);
            }
        }

        None
    }
}

