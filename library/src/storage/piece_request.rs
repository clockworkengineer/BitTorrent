//! Piece request definitions
//!
//! Structs representing a request for a specific data block from a peer.

#[derive(Debug, Clone)]
pub struct PieceRequest {
    pub info_hash: Vec<u8>,
    pub ip: String,
    pub piece_number: u32,
    pub block_offset: u32,
    pub block_size: u32,
}
