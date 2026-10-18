//! BEP 55: Holepunch Extension (`ut_holepunch`) for BitTorrent.
//!
//! Enables peer-to-peer NAT traversal by coordinating simultaneous connection
//! initiation (typically uTP over UDP) between two peers behind NATs/firewalls
//! via a mutually connected relay/rendezvous peer.

use crate::error::BitTorrentError;
use alloc::format;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// Message type for `ut_holepunch` messages (1 byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HolepunchMessageType {
    /// Sent by initiator to relaying peer with target endpoint.
    Rendezvous = 0x00,
    /// Sent by relaying peer to both initiator and target with peer endpoint.
    Connect = 0x01,
    /// Sent by relaying peer back to initiator if rendezvous cannot be completed.
    Error = 0x02,
}

impl HolepunchMessageType {
    pub fn from_u8(val: u8) -> Result<Self, BitTorrentError> {
        match val {
            0x00 => Ok(HolepunchMessageType::Rendezvous),
            0x01 => Ok(HolepunchMessageType::Connect),
            0x02 => Ok(HolepunchMessageType::Error),
            _ => Err(BitTorrentError::Parse(format!(
                "Invalid holepunch message type: 0x{:02x}",
                val
            ))),
        }
    }

    #[inline]
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }
}

/// Address type in `ut_holepunch` messages (1 byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HolepunchAddrType {
    IPv4 = 0x00,
    IPv6 = 0x01,
}

/// Error codes in `ut_holepunch` error messages (4 bytes big-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HolepunchErrorCode {
    None = 0x00,
    /// The target endpoint is unknown or invalid.
    NoSuchPeer = 0x01,
    /// The relaying peer is not currently connected to the target peer.
    NotConnected = 0x02,
    /// The target peer does not support the ut_holepunch extension.
    NoSupport = 0x03,
    /// The target endpoint specified belongs to the relaying peer itself.
    NoSelf = 0x04,
}

impl HolepunchErrorCode {
    pub fn try_from_u32(val: u32) -> Result<Self, BitTorrentError> {
        match val {
            0x00 => Ok(HolepunchErrorCode::None),
            0x01 => Ok(HolepunchErrorCode::NoSuchPeer),
            0x02 => Ok(HolepunchErrorCode::NotConnected),
            0x03 => Ok(HolepunchErrorCode::NoSupport),
            0x04 => Ok(HolepunchErrorCode::NoSelf),
            _ => Err(BitTorrentError::Parse(format!(
                "Invalid holepunch error code: {}",
                val
            ))),
        }
    }

    pub fn from_u32(val: u32) -> Self {
        Self::try_from_u32(val).unwrap_or(HolepunchErrorCode::None)
    }

    #[inline]
    pub fn as_u32(&self) -> u32 {
        *self as u32
    }
}

/// Represents a parsed BEP 55 `ut_holepunch` extension message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HolepunchMessage {
    pub msg_type: HolepunchMessageType,
    pub endpoint: SocketAddr,
    pub err_code: HolepunchErrorCode,
}

impl HolepunchMessage {
    /// Returns the address type (IPv4 or IPv6) of the message endpoint.
    #[inline]
    pub fn addr_type(&self) -> HolepunchAddrType {
        match self.endpoint.ip() {
            IpAddr::V4(_) => HolepunchAddrType::IPv4,
            IpAddr::V6(_) => HolepunchAddrType::IPv6,
        }
    }

    /// Creates a new `Rendezvous` message to request a connection bridge to `target`.
    pub fn rendezvous(target: SocketAddr) -> Self {
        Self {
            msg_type: HolepunchMessageType::Rendezvous,
            endpoint: target,
            err_code: HolepunchErrorCode::None,
        }
    }

    /// Creates a new `Connect` message instructing a peer to connect to `endpoint`.
    pub fn connect(endpoint: SocketAddr) -> Self {
        Self {
            msg_type: HolepunchMessageType::Connect,
            endpoint,
            err_code: HolepunchErrorCode::None,
        }
    }

    /// Creates a new `Error` message reporting failure code for the requested endpoint.
    pub fn error(endpoint: SocketAddr, err_code: HolepunchErrorCode) -> Self {
        Self {
            msg_type: HolepunchMessageType::Error,
            endpoint,
            err_code,
        }
    }

    /// Encodes this message into its BEP 55 binary representation.
    pub fn encode(&self) -> Vec<u8> {
        let is_ipv6 = self.endpoint.is_ipv6();
        let capacity = if is_ipv6 { 24 } else { 12 };
        let mut buffer = Vec::with_capacity(capacity);

        buffer.push(self.msg_type.as_u8());
        match self.endpoint.ip() {
            IpAddr::V4(v4) => {
                buffer.push(HolepunchAddrType::IPv4 as u8);
                buffer.extend_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                buffer.push(HolepunchAddrType::IPv6 as u8);
                buffer.extend_from_slice(&v6.octets());
            }
        }

        buffer.extend_from_slice(&self.endpoint.port().to_be_bytes());
        buffer.extend_from_slice(&self.err_code.as_u32().to_be_bytes());
        buffer
    }

    /// Decodes a BEP 55 binary payload into a `HolepunchMessage`.
    pub fn decode(payload: &[u8]) -> Result<Self, BitTorrentError> {
        if payload.len() < 12 {
            return Err(BitTorrentError::Parse(
                "Payload too short for ut_holepunch message".into(),
            ));
        }

        let msg_type = HolepunchMessageType::from_u8(payload[0])?;
        let addr_type = payload[1];

        let (endpoint, err_code_offset) = match addr_type {
            0x00 => {
                // IPv4: 1 (msg_type) + 1 (addr_type) + 4 (ip) + 2 (port) + 4 (err_code) = 12 bytes
                if payload.len() < 12 {
                    return Err(BitTorrentError::Parse(
                        "Payload too short for IPv4 ut_holepunch".into(),
                    ));
                }
                let octets: [u8; 4] = payload[2..6].try_into().map_err(|_| {
                    BitTorrentError::Parse("Failed to read IPv4 octets".into())
                })?;
                let port = u16::from_be_bytes(payload[6..8].try_into().map_err(|_| {
                    BitTorrentError::Parse("Failed to read port bytes".into())
                })?);
                (SocketAddr::new(IpAddr::V4(Ipv4Addr::from(octets)), port), 8)
            }
            0x01 => {
                // IPv6: 1 (msg_type) + 1 (addr_type) + 16 (ip) + 2 (port) + 4 (err_code) = 24 bytes
                if payload.len() < 24 {
                    return Err(BitTorrentError::Parse(
                        "Payload too short for IPv6 ut_holepunch".into(),
                    ));
                }
                let octets: [u8; 16] = payload[2..18].try_into().map_err(|_| {
                    BitTorrentError::Parse("Failed to read IPv6 octets".into())
                })?;
                let port = u16::from_be_bytes(payload[18..20].try_into().map_err(|_| {
                    BitTorrentError::Parse("Failed to read port bytes".into())
                })?);
                (SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port), 20)
            }
            _ => {
                return Err(BitTorrentError::Parse(format!(
                    "Unknown addr_type 0x{:02x} in ut_holepunch",
                    addr_type
                )));
            }
        };

        if payload.len() < err_code_offset + 4 {
            return Err(BitTorrentError::Parse(
                "Missing error code in ut_holepunch message".into(),
            ));
        }

        let err_code_raw = u32::from_be_bytes(
            payload[err_code_offset..err_code_offset + 4]
                .try_into()
                .map_err(|_| BitTorrentError::Parse("Failed to read err_code".into()))?,
        );
        let err_code = HolepunchErrorCode::try_from_u32(err_code_raw)?;

        Ok(Self {
            msg_type,
            endpoint,
            err_code,
        })
    }
}
