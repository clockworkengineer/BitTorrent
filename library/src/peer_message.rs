//! Peer wire protocol messages
//!
//! Defines the `PeerMessage` enum representing standard messages exchanged between
//! BitTorrent peers, alongside functions to encode and decode wire-format and handshake packets.

use crate::constants::{HASH_LENGTH, PEER_ID_LENGTH, SIZE_OF_U32};
use crate::error::BitTorrentError;

/// Enumeration of messages defined in the BitTorrent Peer Wire Protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port(u16),
}

impl PeerMessage {
    /// Encodes a `PeerMessage` into its protocol-specific wire byte representation.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            PeerMessage::KeepAlive => 0u32.to_be_bytes().to_vec(),
            PeerMessage::Choke => [1u32.to_be_bytes().as_ref(), &[0u8]].concat(),
            PeerMessage::Unchoke => [1u32.to_be_bytes().as_ref(), &[1u8]].concat(),
            PeerMessage::Interested => [1u32.to_be_bytes().as_ref(), &[2u8]].concat(),
            PeerMessage::NotInterested => [1u32.to_be_bytes().as_ref(), &[3u8]].concat(),
            PeerMessage::Have(index) => {
                let mut buffer = Vec::with_capacity(SIZE_OF_U32 + 1 + SIZE_OF_U32);
                buffer.extend_from_slice(&(1 + SIZE_OF_U32 as u32).to_be_bytes());
                buffer.push(4);
                buffer.extend_from_slice(&index.to_be_bytes());
                buffer
            }
            PeerMessage::Bitfield(bitfield) => {
                let mut buffer = Vec::with_capacity(SIZE_OF_U32 + 1 + bitfield.len());
                buffer.extend_from_slice(&((1 + bitfield.len()) as u32).to_be_bytes());
                buffer.push(5);
                buffer.extend_from_slice(bitfield);
                buffer
            }
            PeerMessage::Request {
                index,
                begin,
                length,
            } => Self::encode_triple_u32(6, *index, *begin, *length),
            PeerMessage::Piece {
                index,
                begin,
                block,
            } => {
                let mut buffer = Vec::with_capacity(SIZE_OF_U32 + 1 + 8 + block.len());
                buffer.extend_from_slice(&((1 + 8 + block.len()) as u32).to_be_bytes());
                buffer.push(7);
                buffer.extend_from_slice(&index.to_be_bytes());
                buffer.extend_from_slice(&begin.to_be_bytes());
                buffer.extend_from_slice(block);
                buffer
            }
            PeerMessage::Cancel {
                index,
                begin,
                length,
            } => Self::encode_triple_u32(8, *index, *begin, *length),
            PeerMessage::Port(port) => {
                let mut buffer = Vec::with_capacity(SIZE_OF_U32 + 1 + 2);
                buffer.extend_from_slice(&((1 + 2) as u32).to_be_bytes());
                buffer.push(9);
                buffer.extend_from_slice(&port.to_be_bytes());
                buffer
            }
        }
    }

    /// Decodes a wire-format message payload (excluding the 4-byte length prefix) into a `PeerMessage`.
    pub fn decode(buffer: &[u8]) -> Result<Self, BitTorrentError> {
        if buffer.is_empty() {
            return Ok(PeerMessage::KeepAlive);
        }
        let message_id = buffer[0];
        let payload = &buffer[1..];
        match message_id {
            0 => Ok(PeerMessage::Choke),
            1 => Ok(PeerMessage::Unchoke),
            2 => Ok(PeerMessage::Interested),
            3 => Ok(PeerMessage::NotInterested),
            4 => {
                if payload.len() != SIZE_OF_U32 {
                    return Err(BitTorrentError::Parse("Invalid HAVE payload length".into()));
                }
                let index = u32::from_be_bytes(payload.try_into().unwrap());
                Ok(PeerMessage::Have(index))
            }
            5 => Ok(PeerMessage::Bitfield(payload.to_vec())),
            6 => {
                let (index, begin, length) = Self::decode_triple_u32(payload)?;
                Ok(PeerMessage::Request {
                    index,
                    begin,
                    length,
                })
            }
            7 => {
                if payload.len() < 8 {
                    return Err(BitTorrentError::Parse(
                        "Invalid PIECE payload length".into(),
                    ));
                }
                let index = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                let begin = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                let block = payload[8..].to_vec();
                Ok(PeerMessage::Piece {
                    index,
                    begin,
                    block,
                })
            }
            8 => {
                let (index, begin, length) = Self::decode_triple_u32(payload)?;
                Ok(PeerMessage::Cancel {
                    index,
                    begin,
                    length,
                })
            }
            9 => {
                if payload.len() != 2 {
                    return Err(BitTorrentError::Parse("Invalid PORT payload length".into()));
                }
                let port = u16::from_be_bytes(payload.try_into().unwrap());
                Ok(PeerMessage::Port(port))
            }
            _ => Err(BitTorrentError::Parse("Unknown peer message ID".into())),
        }
    }

    /// Constructs the raw 68-byte handshake packet buffer for establishing a peer connection.
    pub fn handshake_encode(info_hash: &[u8], peer_id: &[u8]) -> Result<Vec<u8>, BitTorrentError> {
        if info_hash.len() != HASH_LENGTH {
            return Err(BitTorrentError::Parse("Info hash must be 20 bytes".into()));
        }
        if peer_id.len() != PEER_ID_LENGTH {
            return Err(BitTorrentError::Parse("Peer ID must be 20 bytes".into()));
        }

        let mut buffer = Vec::with_capacity(crate::constants::INITIAL_HANDSHAKE_LENGTH);
        buffer.push(19);
        buffer.extend_from_slice(b"BitTorrent protocol");
        buffer.extend_from_slice(&[0u8; 8]);
        buffer.extend_from_slice(info_hash);
        buffer.extend_from_slice(peer_id);
        Ok(buffer)
    }

    /// Decodes a 68-byte peer handshake packet and extracts the 20-byte `info_hash` and `peer_id`.
    pub fn handshake_decode(buffer: &[u8]) -> Result<(Vec<u8>, Vec<u8>), BitTorrentError> {
        if buffer.len() != crate::constants::INITIAL_HANDSHAKE_LENGTH {
            return Err(BitTorrentError::Parse("Invalid handshake length".into()));
        }
        let pstrlen = buffer[0];
        if pstrlen != 19 {
            return Err(BitTorrentError::Parse(
                "Invalid handshake protocol length".into(),
            ));
        }
        if &buffer[1..20] != b"BitTorrent protocol" {
            return Err(BitTorrentError::Parse(
                "Invalid handshake protocol string".into(),
            ));
        }
        let info_hash = buffer[28..48].to_vec();
        let peer_id = buffer[48..68].to_vec();
        Ok((info_hash, peer_id))
    }

    fn encode_triple_u32(message_id: u8, v1: u32, v2: u32, v3: u32) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(SIZE_OF_U32 + 1 + 12);
        buffer.extend_from_slice(&((1 + 12) as u32).to_be_bytes());
        buffer.push(message_id);
        buffer.extend_from_slice(&v1.to_be_bytes());
        buffer.extend_from_slice(&v2.to_be_bytes());
        buffer.extend_from_slice(&v3.to_be_bytes());
        buffer
    }

    fn decode_triple_u32(payload: &[u8]) -> Result<(u32, u32, u32), BitTorrentError> {
        if payload.len() != 12 {
            return Err(BitTorrentError::Parse("Invalid message payload length".into()));
        }
        let v1 = u32::from_be_bytes(payload[0..4].try_into().unwrap());
        let v2 = u32::from_be_bytes(payload[4..8].try_into().unwrap());
        let v3 = u32::from_be_bytes(payload[8..12].try_into().unwrap());
        Ok((v1, v2, v3))
    }
}
