//! Super-seeding (initial seeding / BEP 16) state tracking.

use std::collections::HashMap;

/// Tracks the super-seeding (initial seeding / BEP 16) distribution state.
#[derive(Debug, Clone, Default)]
pub struct SuperSeedingState {
    /// Whether super-seeding mode is actively enabled.
    pub enabled: bool,
    /// Maps piece index to the peer IP it was exclusively offered to.
    pub piece_offered_to: HashMap<u32, String>,
    /// Maps each peer IP to the piece index currently offered to them pending swarm validation.
    pub peer_pending_piece: HashMap<String, u32>,
    /// Counts how many times each piece has been witnessed in the swarm (from peers other than the one it was offered to).
    pub piece_swarm_count: HashMap<u32, usize>,
    /// Cumulative count of pieces offered/suggested in super-seeding mode.
    pub total_pieces_offered: usize,
    /// Cumulative count of pieces verified as propagated across the swarm.
    pub total_pieces_propagated: usize,
}
