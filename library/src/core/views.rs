//! Segregated role-based trait views (Interface Segregation Principle).
//!
//! Provides lean, read-only interfaces that decompose the God Object `TorrentContext`
//! into cohesive, single-responsibility views consumed by piece selectors, swarm
//! coordinators, metrics trackers, and disk schedulers.

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use crate::network::peer::Peer;

/// Read-only view of a remote peer's piece possession (ISP & DIP).
pub trait RemotePeerPieceView {
    /// Returns true if the remote peer possesses the specified piece.
    fn is_piece_on_remote_peer(&self, piece: u32) -> bool;
}

#[cfg(feature = "std")]
impl RemotePeerPieceView for Peer {
    fn is_piece_on_remote_peer(&self, piece: u32) -> bool {
        self.is_piece_on_remote_peer(piece)
    }
}

impl RemotePeerPieceView for [u8] {
    fn is_piece_on_remote_peer(&self, piece: u32) -> bool {
        let byte_index = (piece / 8) as usize;
        let bit_index = (piece % 8) as u8;
        if byte_index < self.len() {
            (self[byte_index] & (1 << (7 - bit_index))) != 0
        } else {
            false
        }
    }
}

impl RemotePeerPieceView for Vec<u8> {
    fn is_piece_on_remote_peer(&self, piece: u32) -> bool {
        self.as_slice().is_piece_on_remote_peer(piece)
    }
}

/// Read-only view for piece availability, verification status, and priority (ISP).
pub trait PieceAvailabilityView {
    /// Total number of pieces in the torrent.
    fn number_of_pieces(&self) -> usize;

    /// Length in bytes of the piece at the specified index.
    fn piece_length(&self, piece: u32) -> u32;

    /// Returns true if the piece has been downloaded and verified locally.
    fn is_piece_local(&self, piece: u32) -> bool;

    /// Returns true if the piece is desired according to file priorities.
    fn is_piece_wanted(&self, piece: u32) -> bool;

    /// Returns true if the piece is currently undergoing cryptographic hash verification.
    fn is_piece_verifying(&self, piece: u32) -> bool;

    /// Returns the number of peers in the swarm that possess this piece.
    fn peer_count_for_piece(&self, piece: u32) -> usize;

    /// Selects the rarest available piece matching remote peer availability.
    ///
    /// Default implementation performs a linear scan over available wanted pieces;
    /// `TorrentContext` overrides this using its O(log N) indexed priority queue.
    fn pop_rarest_piece(&self, peer: &dyn RemotePeerPieceView) -> Option<u32> {
        let num_pieces = self.number_of_pieces();
        let mut rarest_piece = None;
        let mut min_count = usize::MAX;

        for piece in 0..num_pieces as u32 {
            if self.is_piece_wanted(piece)
                && !self.is_piece_local(piece)
                && !self.is_piece_verifying(piece)
                && peer.is_piece_on_remote_peer(piece)
            {
                let count = self.peer_count_for_piece(piece);
                if count < min_count {
                    min_count = count;
                    rarest_piece = Some(piece);
                }
            }
        }
        rarest_piece
    }
}

#[cfg(feature = "std")]
impl<'a, T: RemotePeerPieceView + ?Sized> RemotePeerPieceView for std::sync::MutexGuard<'a, T> {
    fn is_piece_on_remote_peer(&self, piece: u32) -> bool {
        (**self).is_piece_on_remote_peer(piece)
    }
}

impl<T: RemotePeerPieceView + ?Sized> RemotePeerPieceView for &T {
    fn is_piece_on_remote_peer(&self, piece: u32) -> bool {
        (**self).is_piece_on_remote_peer(piece)
    }
}

#[cfg(feature = "std")]
impl<'a, T: PieceAvailabilityView + ?Sized> PieceAvailabilityView for std::sync::MutexGuard<'a, T> {
    fn number_of_pieces(&self) -> usize {
        (**self).number_of_pieces()
    }

    fn piece_length(&self, piece: u32) -> u32 {
        (**self).piece_length(piece)
    }

    fn is_piece_local(&self, piece: u32) -> bool {
        (**self).is_piece_local(piece)
    }

    fn is_piece_wanted(&self, piece: u32) -> bool {
        (**self).is_piece_wanted(piece)
    }

    fn is_piece_verifying(&self, piece: u32) -> bool {
        (**self).is_piece_verifying(piece)
    }

    fn peer_count_for_piece(&self, piece: u32) -> usize {
        (**self).peer_count_for_piece(piece)
    }

    fn pop_rarest_piece(&self, peer: &dyn RemotePeerPieceView) -> Option<u32> {
        (**self).pop_rarest_piece(peer)
    }
}

impl<T: PieceAvailabilityView + ?Sized> PieceAvailabilityView for &T {
    fn number_of_pieces(&self) -> usize {
        (**self).number_of_pieces()
    }

    fn piece_length(&self, piece: u32) -> u32 {
        (**self).piece_length(piece)
    }

    fn is_piece_local(&self, piece: u32) -> bool {
        (**self).is_piece_local(piece)
    }

    fn is_piece_wanted(&self, piece: u32) -> bool {
        (**self).is_piece_wanted(piece)
    }

    fn is_piece_verifying(&self, piece: u32) -> bool {
        (**self).is_piece_verifying(piece)
    }

    fn peer_count_for_piece(&self, piece: u32) -> usize {
        (**self).peer_count_for_piece(piece)
    }

    fn pop_rarest_piece(&self, peer: &dyn RemotePeerPieceView) -> Option<u32> {
        (**self).pop_rarest_piece(peer)
    }
}

/// Point-in-time snapshot of an individual peer connection state.
#[derive(Debug, Clone)]
pub struct PeerSnapshot {
    pub ip: String,
    pub port: u16,
    pub am_choking: bool,
    pub am_interested: bool,
    pub peer_choking: bool,
    pub peer_interested: bool,
    pub download_rate: f64,
    pub upload_rate: f64,
}

/// Read-only snapshot view of the swarm for choking and unchoking decisions (ISP).
pub trait SwarmView: Send + Sync {
    /// Returns the count of connected peers currently in the swarm.
    fn connected_peers_count(&self) -> usize;

    /// Returns the count of peers currently unchoked.
    fn unchoked_peers_count(&self) -> usize;

    /// Iterates over peer snapshots in the swarm without exposing internal synchronization primitives.
    fn for_each_peer(&self, f: &mut dyn FnMut(&str, &PeerSnapshot));
}

/// Read-only view of session bandwidth and transfer progress statistics (ISP).
pub trait TransferStatsView: Send + Sync {
    /// Total bytes downloaded from the swarm.
    fn total_downloaded(&self) -> u64;

    /// Total bytes uploaded to the swarm.
    fn total_uploaded(&self) -> u64;

    /// Current rolling download speed in bytes/sec.
    fn current_download_rate(&self) -> u64;

    /// Current rolling upload speed in bytes/sec.
    fn current_upload_rate(&self) -> u64;

    /// Download progress expressed as a percentage in [0.0, 100.0].
    fn progress_percent(&self) -> f32;
}

/// Lightweight mock implementation of `PieceAvailabilityView` for isolated unit tests.
#[derive(Debug, Clone, Default)]
pub struct MockPieceAvailabilityView {
    pub num_pieces: usize,
    pub piece_length: u32,
    pub local_pieces: Vec<bool>,
    pub wanted_pieces: Vec<bool>,
    pub verifying_pieces: Vec<bool>,
    pub peer_counts: Vec<usize>,
}

impl MockPieceAvailabilityView {
    /// Creates a mock availability view with all pieces wanted and none local.
    pub fn new(num_pieces: usize, piece_length: u32) -> Self {
        Self {
            num_pieces,
            piece_length,
            local_pieces: alloc::vec![false; num_pieces],
            wanted_pieces: alloc::vec![true; num_pieces],
            verifying_pieces: alloc::vec![false; num_pieces],
            peer_counts: alloc::vec![0; num_pieces],
        }
    }
}

impl PieceAvailabilityView for MockPieceAvailabilityView {
    fn number_of_pieces(&self) -> usize {
        self.num_pieces
    }

    fn piece_length(&self, _piece: u32) -> u32 {
        self.piece_length
    }

    fn is_piece_local(&self, piece: u32) -> bool {
        self.local_pieces
            .get(piece as usize)
            .copied()
            .unwrap_or(false)
    }

    fn is_piece_wanted(&self, piece: u32) -> bool {
        self.wanted_pieces
            .get(piece as usize)
            .copied()
            .unwrap_or(false)
    }

    fn is_piece_verifying(&self, piece: u32) -> bool {
        self.verifying_pieces
            .get(piece as usize)
            .copied()
            .unwrap_or(false)
    }

    fn peer_count_for_piece(&self, piece: u32) -> usize {
        self.peer_counts.get(piece as usize).copied().unwrap_or(0)
    }
}

/// Lightweight mock implementation of `SwarmView` for isolated unit tests.
#[derive(Debug, Clone, Default)]
pub struct MockSwarmView {
    pub peers: Vec<PeerSnapshot>,
    pub unchoked_count: usize,
}

impl MockSwarmView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_peer(mut self, snapshot: PeerSnapshot) -> Self {
        if !snapshot.am_choking {
            self.unchoked_count += 1;
        }
        self.peers.push(snapshot);
        self
    }
}

impl SwarmView for MockSwarmView {
    fn connected_peers_count(&self) -> usize {
        self.peers.len()
    }

    fn unchoked_peers_count(&self) -> usize {
        self.unchoked_count
    }

    fn for_each_peer(&self, f: &mut dyn FnMut(&str, &PeerSnapshot)) {
        for p in &self.peers {
            f(&p.ip, p);
        }
    }
}
