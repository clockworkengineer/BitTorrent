//! Peer wire protocol messages
//!
//! Defines the `PeerMessage` enum representing standard messages exchanged between
//! BitTorrent peers, alongside functions to encode and decode wire-format and handshake packets.

use crate::constants::{HASH_LENGTH, PEER_ID_LENGTH, SIZE_OF_U32};
use crate::error::BitTorrentError;
use alloc::vec;
use alloc::vec::Vec;

/// Enumeration of messages defined in the BitTorrent Peer Wire Protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerMessage<'a> {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(&'a [u8]),
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: &'a [u8],
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port(u16),
    Extended {
        ext_id: u8,
        payload: &'a [u8],
    },
    Suggest(u32),
    HaveAll,
    HaveNone,
    Reject {
        index: u32,
        begin: u32,
        length: u32,
    },
    AllowedFast(u32),
    HashRequest {
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    },
    Hashes {
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
        hashes: &'a [u8],
    },
    HashReject {
        pieces_root: [u8; 32],
        base_layer: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    },
}

impl<'a> PeerMessage<'a> {
    /// Encodes a `PeerMessage` into its protocol-specific wire byte representation.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            PeerMessage::KeepAlive => vec![0, 0, 0, 0],
            PeerMessage::Choke => vec![0, 0, 0, 1, 0],
            PeerMessage::Unchoke => vec![0, 0, 0, 1, 1],
            PeerMessage::Interested => vec![0, 0, 0, 1, 2],
            PeerMessage::NotInterested => vec![0, 0, 0, 1, 3],
            PeerMessage::Have(index) => Self::encode_single_u32(4, *index),
            PeerMessage::Bitfield(bitfield) => {
                let mut buffer = Vec::with_capacity(5 + bitfield.len());
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
                let mut buffer = Vec::with_capacity(13 + block.len());
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
                let mut buffer = Vec::with_capacity(7);
                buffer.extend_from_slice(&3u32.to_be_bytes());
                buffer.push(9);
                buffer.extend_from_slice(&port.to_be_bytes());
                buffer
            }
            PeerMessage::Extended { ext_id, payload } => {
                let mut buffer = Vec::with_capacity(6 + payload.len());
                buffer.extend_from_slice(&((1 + 1 + payload.len()) as u32).to_be_bytes());
                buffer.push(20);
                buffer.push(*ext_id);
                buffer.extend_from_slice(payload);
                buffer
            }
            PeerMessage::Suggest(index) => Self::encode_single_u32(13, *index),
            PeerMessage::HaveAll => vec![0, 0, 0, 1, 14],
            PeerMessage::HaveNone => vec![0, 0, 0, 1, 15],
            PeerMessage::Reject {
                index,
                begin,
                length,
            } => Self::encode_triple_u32(16, *index, *begin, *length),
            PeerMessage::AllowedFast(index) => Self::encode_single_u32(17, *index),
            PeerMessage::HashRequest {
                pieces_root,
                base_layer,
                index,
                length,
                proof_layers,
            } => Self::encode_hash_control(
                21,
                pieces_root,
                *base_layer,
                *index,
                *length,
                *proof_layers,
            ),
            PeerMessage::Hashes {
                pieces_root,
                base_layer,
                index,
                length,
                proof_layers,
                hashes,
            } => {
                let msg_len = (1 + 32 + 16 + hashes.len()) as u32;
                let mut buffer = Vec::with_capacity(4 + msg_len as usize);
                buffer.extend_from_slice(&msg_len.to_be_bytes());
                buffer.push(22);
                buffer.extend_from_slice(pieces_root);
                buffer.extend_from_slice(&base_layer.to_be_bytes());
                buffer.extend_from_slice(&index.to_be_bytes());
                buffer.extend_from_slice(&length.to_be_bytes());
                buffer.extend_from_slice(&proof_layers.to_be_bytes());
                buffer.extend_from_slice(hashes);
                buffer
            }
            PeerMessage::HashReject {
                pieces_root,
                base_layer,
                index,
                length,
                proof_layers,
            } => Self::encode_hash_control(
                23,
                pieces_root,
                *base_layer,
                *index,
                *length,
                *proof_layers,
            ),
        }
    }

    /// Decodes a wire-format message payload (excluding the 4-byte length prefix) into a `PeerMessage`.
    pub fn decode(buffer: &'a [u8]) -> Result<Self, BitTorrentError> {
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
            4 => Ok(PeerMessage::Have(Self::decode_single_u32(payload, "HAVE")?)),
            5 => Ok(PeerMessage::Bitfield(payload)),
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
                let index = u32::from_be_bytes(
                    payload[0..4]
                        .try_into()
                        .map_err(|_| BitTorrentError::Parse("Invalid index bytes".into()))?,
                );
                let begin = u32::from_be_bytes(
                    payload[4..8]
                        .try_into()
                        .map_err(|_| BitTorrentError::Parse("Invalid begin bytes".into()))?,
                );
                let block = &payload[8..];
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
                let port = u16::from_be_bytes(
                    payload
                        .try_into()
                        .map_err(|_| BitTorrentError::Parse("Invalid port bytes".into()))?,
                );
                Ok(PeerMessage::Port(port))
            }
            20 => {
                if payload.is_empty() {
                    return Err(BitTorrentError::Parse(
                        "Invalid EXTENDED payload length".into(),
                    ));
                }
                let ext_id = payload[0];
                let ext_payload = &payload[1..];
                Ok(PeerMessage::Extended {
                    ext_id,
                    payload: ext_payload,
                })
            }
            13 => Ok(PeerMessage::Suggest(Self::decode_single_u32(
                payload, "SUGGEST",
            )?)),
            14 => Ok(PeerMessage::HaveAll),
            15 => Ok(PeerMessage::HaveNone),
            16 => {
                let (index, begin, length) = Self::decode_triple_u32(payload)?;
                Ok(PeerMessage::Reject {
                    index,
                    begin,
                    length,
                })
            }
            17 => Ok(PeerMessage::AllowedFast(Self::decode_single_u32(
                payload,
                "ALLOWED FAST",
            )?)),
            21 => {
                let (pieces_root, base_layer, index, length, proof_layers) =
                    Self::decode_hash_control(payload, "HASH_REQUEST")?;
                Ok(PeerMessage::HashRequest {
                    pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                })
            }
            22 => {
                if payload.len() < 48 || (payload.len() - 48) % 32 != 0 {
                    return Err(BitTorrentError::Parse(
                        "Invalid HASHES payload length".into(),
                    ));
                }
                let (pieces_root, base_layer, index, length, proof_layers) =
                    Self::decode_hash_control(&payload[..48], "HASHES")?;
                let hashes = &payload[48..];
                Ok(PeerMessage::Hashes {
                    pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                    hashes,
                })
            }
            23 => {
                let (pieces_root, base_layer, index, length, proof_layers) =
                    Self::decode_hash_control(payload, "HASH_REJECT")?;
                Ok(PeerMessage::HashReject {
                    pieces_root,
                    base_layer,
                    index,
                    length,
                    proof_layers,
                })
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
        let mut reserved = [0u8; 8];
        reserved[5] |= 0x10; // Support Extension Protocol (BEP 10)
        reserved[7] |= 0x04; // Support Fast Extension (BEP 6)
        buffer.extend_from_slice(&reserved);
        buffer.extend_from_slice(info_hash);
        buffer.extend_from_slice(peer_id);
        Ok(buffer)
    }

    /// Decodes a 68-byte peer handshake packet into fixed-size byte arrays with zero heap allocations.
    pub fn handshake_decode_arrays(
        buffer: &[u8],
    ) -> Result<([u8; 20], [u8; 20], [u8; 8]), BitTorrentError> {
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
        let mut reserved = [0u8; 8];
        reserved.copy_from_slice(&buffer[20..28]);
        let mut info_hash = [0u8; 20];
        info_hash.copy_from_slice(&buffer[28..48]);
        let mut peer_id = [0u8; 20];
        peer_id.copy_from_slice(&buffer[48..68]);
        Ok((info_hash, peer_id, reserved))
    }

    /// Decodes a 68-byte peer handshake packet and extracts the 20-byte `info_hash`, `peer_id`, and 8 reserved bytes.
    pub fn handshake_decode(buffer: &[u8]) -> Result<(Vec<u8>, Vec<u8>, [u8; 8]), BitTorrentError> {
        let (info_hash, peer_id, reserved) = Self::handshake_decode_arrays(buffer)?;
        Ok((info_hash.to_vec(), peer_id.to_vec(), reserved))
    }

    #[inline]
    fn encode_single_u32(message_id: u8, val: u32) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(SIZE_OF_U32 + 1 + 4);
        buffer.extend_from_slice(&5u32.to_be_bytes());
        buffer.push(message_id);
        buffer.extend_from_slice(&val.to_be_bytes());
        buffer
    }

    #[inline]
    fn decode_single_u32(payload: &[u8], name: &'static str) -> Result<u32, BitTorrentError> {
        if payload.len() != SIZE_OF_U32 {
            return Err(BitTorrentError::Parse(alloc::format!(
                "Invalid {} payload length",
                name
            )));
        }
        let val = u32::from_be_bytes(
            payload
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid payload bytes".into()))?,
        );
        Ok(val)
    }

    #[inline]
    fn encode_hash_control(
        message_id: u8,
        root: &[u8; 32],
        base: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    ) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(53);
        buffer.extend_from_slice(&49u32.to_be_bytes());
        buffer.push(message_id);
        buffer.extend_from_slice(root);
        buffer.extend_from_slice(&base.to_be_bytes());
        buffer.extend_from_slice(&index.to_be_bytes());
        buffer.extend_from_slice(&length.to_be_bytes());
        buffer.extend_from_slice(&proof_layers.to_be_bytes());
        buffer
    }

    #[inline]
    fn decode_hash_control(
        payload: &[u8],
        name: &'static str,
    ) -> Result<([u8; 32], u32, u32, u32, u32), BitTorrentError> {
        if payload.len() != 48 {
            return Err(BitTorrentError::Parse(alloc::format!(
                "Invalid {} payload length",
                name
            )));
        }
        let mut pieces_root = [0u8; 32];
        pieces_root.copy_from_slice(&payload[0..32]);
        let base_layer = u32::from_be_bytes(
            payload[32..36]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid base_layer".into()))?,
        );
        let index = u32::from_be_bytes(
            payload[36..40]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid index".into()))?,
        );
        let length = u32::from_be_bytes(
            payload[40..44]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid length".into()))?,
        );
        let proof_layers = u32::from_be_bytes(
            payload[44..48]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid proof_layers".into()))?,
        );
        Ok((pieces_root, base_layer, index, length, proof_layers))
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
            return Err(BitTorrentError::Parse(
                "Invalid message payload length".into(),
            ));
        }
        let v1 = u32::from_be_bytes(
            payload[0..4]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid payload bytes".into()))?,
        );
        let v2 = u32::from_be_bytes(
            payload[4..8]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid payload bytes".into()))?,
        );
        let v3 = u32::from_be_bytes(
            payload[8..12]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Invalid payload bytes".into()))?,
        );
        Ok((v1, v2, v3))
    }
}
