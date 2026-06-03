//! BitTorrent protocol constants
//!
//! This module defines core constants used throughout the BitTorrent client,
//! such as block sizes, hash lengths, and limits on connection swarms.

pub const BLOCK_SIZE: usize = 1024 * 16;
pub const HASH_LENGTH: usize = 20;
pub const PEER_ID_LENGTH: usize = 20;
pub const SIZE_OF_U32: usize = 4;
pub const MAXIMUM_SWARM_SIZE: usize = 100;
pub const INITIAL_HANDSHAKE_LENGTH: usize = 68;
/// Number of missing pieces below which endgame mode activates.
pub const ENDGAME_THRESHOLD: u32 = 5;
/// How long a peer stays on the dead list before being reconsidered.
pub const DEAD_PEER_TTL_SECS: u64 = 600;
