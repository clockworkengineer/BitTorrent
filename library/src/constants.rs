//! BitTorrent protocol constants
//!
//! This module defines core constants used throughout the BitTorrent client,
//! such as block sizes, hash lengths, and limits on connection swarms.

use std::time::Duration;

pub const BLOCK_SIZE: usize = 1024 * 16;
pub const HASH_LENGTH: usize = 20;
pub const PEER_ID_LENGTH: usize = 20;
pub const SIZE_OF_U32: usize = 4;
pub const MAXIMUM_SWARM_SIZE: usize = 100;
pub const ENDGAME_THRESHOLD: usize = 5;
pub const DEAD_PEER_TTL: Duration = Duration::from_secs(600);
pub const INITIAL_HANDSHAKE_LENGTH: usize = 68;
