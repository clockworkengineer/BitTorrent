//! Piece selection strategy traits and implementations
//!
//! Provides the pluggable `PieceSelector` trait to customize piece selection
//! strategies (e.g. rarest-first vs. sequential download).

use crate::core::views::{PieceAvailabilityView, RemotePeerPieceView};

/// Pluggable piece selection strategy (ISP & DIP).
pub trait PieceSelector: std::fmt::Debug + Send + Sync {
    /// Selects the next piece index to request from the remote peer using segregated views.
    fn select_piece(
        &self,
        availability: &dyn PieceAvailabilityView,
        peer: &dyn RemotePeerPieceView,
    ) -> Option<u32>;
}

/// Rarest-First piece selection strategy (default).
#[derive(Debug, Clone, Copy)]
pub struct RarestFirstSelector;

impl PieceSelector for RarestFirstSelector {
    /// Selects the next piece index to request from the remote peer using the availability view's
    /// prioritized index (O(log N) priority queue on TorrentContext or linear scan on mocks).
    fn select_piece(
        &self,
        availability: &dyn PieceAvailabilityView,
        peer: &dyn RemotePeerPieceView,
    ) -> Option<u32> {
        availability.pop_rarest_piece(peer)
    }
}

/// Sequential piece selection strategy (ideal for media streaming).
#[derive(Debug, Clone, Copy)]
pub struct SequentialSelector;

impl PieceSelector for SequentialSelector {
    fn select_piece(
        &self,
        availability: &dyn PieceAvailabilityView,
        peer: &dyn RemotePeerPieceView,
    ) -> Option<u32> {
        (0..availability.number_of_pieces() as u32).find(|&piece| {
            availability.is_piece_wanted(piece)
                && !availability.is_piece_local(piece)
                && !availability.is_piece_verifying(piece)
                && peer.is_piece_on_remote_peer(piece)
        })
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
    pub fn new(
        cursor_piece: alloc::sync::Arc<core::sync::atomic::AtomicU32>,
        lookahead_window: u32,
    ) -> Self {
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
        self.cursor_piece
            .store(piece, core::sync::atomic::Ordering::Relaxed);
    }

    /// Returns the lookahead window size in pieces.
    pub fn lookahead_window(&self) -> u32 {
        self.lookahead_window
    }
}

impl PieceSelector for StreamingPieceSelector {
    fn select_piece(
        &self,
        availability: &dyn PieceAvailabilityView,
        peer: &dyn RemotePeerPieceView,
    ) -> Option<u32> {
        let cursor = self
            .cursor_piece
            .load(core::sync::atomic::Ordering::Relaxed);
        let total_pieces = availability.number_of_pieces() as u32;

        if total_pieces == 0 {
            return None;
        }

        let window_end = (cursor.saturating_add(self.lookahead_window)).min(total_pieces);

        // 1. Check lookahead window [cursor .. cursor + lookahead_window)
        for piece in cursor..window_end {
            if availability.is_piece_wanted(piece)
                && !availability.is_piece_local(piece)
                && !availability.is_piece_verifying(piece)
                && peer.is_piece_on_remote_peer(piece)
            {
                return Some(piece);
            }
        }

        // 2. Check remaining pieces from window_end to end of torrent
        for piece in window_end..total_pieces {
            if availability.is_piece_wanted(piece)
                && !availability.is_piece_local(piece)
                && !availability.is_piece_verifying(piece)
                && peer.is_piece_on_remote_peer(piece)
            {
                return Some(piece);
            }
        }

        // 3. Wrap around [0 .. cursor)
        for piece in 0..cursor.min(total_pieces) {
            if availability.is_piece_wanted(piece)
                && !availability.is_piece_local(piece)
                && !availability.is_piece_verifying(piece)
                && peer.is_piece_on_remote_peer(piece)
            {
                return Some(piece);
            }
        }

        None
    }
}
