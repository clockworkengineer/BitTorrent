//! BEP 11: Peer Exchange (PEX) for BitTorrent.
//!
//! Provides support for the `ut_pex` extension protocol message, enabling
//! peers to exchange swarm membership updates (both IPv4 and IPv6) directly
//! with minimal tracker load.

use crate::bencode::{BNode, Bencode};
use crate::error::BitTorrentError;
use crate::network::tracker::PeerDetails;
use alloc::string::ToString;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::net::{Ipv4Addr, Ipv6Addr};

/// Bit flags for PEX peers as defined in BEP 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct PexFlags(pub u8);

impl PexFlags {
    /// Peer prefers encryption (MSE / header encryption).
    pub const PREF_ENCRYPT: u8 = 0x01;
    /// Peer is a seed / upload only.
    pub const SEED: u8 = 0x02;
    /// Peer supports uTP (BEP 29).
    pub const UTP: u8 = 0x04;
    /// Peer supports holepunching (BEP 55).
    pub const HOLEPUNCH: u8 = 0x08;
    /// Outgoing connection (the sender initiated the connection).
    pub const OUTGOING: u8 = 0x10;

    #[inline]
    pub fn new(flags: u8) -> Self {
        Self(flags)
    }

    #[inline]
    pub fn bits(&self) -> u8 {
        self.0
    }

    #[inline]
    pub fn is_pref_encrypt(&self) -> bool {
        (self.0 & Self::PREF_ENCRYPT) != 0
    }

    #[inline]
    pub fn is_seed(&self) -> bool {
        (self.0 & Self::SEED) != 0
    }

    #[inline]
    pub fn is_utp(&self) -> bool {
        (self.0 & Self::UTP) != 0
    }

    #[inline]
    pub fn is_holepunch(&self) -> bool {
        (self.0 & Self::HOLEPUNCH) != 0
    }

    #[inline]
    pub fn is_outgoing(&self) -> bool {
        (self.0 & Self::OUTGOING) != 0
    }
}

/// Represents an individual peer with BEP 11 flags within a PEX message.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PexPeer {
    pub details: PeerDetails,
    pub flags: PexFlags,
}

impl PexPeer {
    pub fn new(details: PeerDetails, flags: PexFlags) -> Self {
        Self { details, flags }
    }
}

/// A decoded or constructed BEP 11 Peer Exchange message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PexMessage {
    pub added_v4: Vec<PexPeer>,
    pub dropped_v4: Vec<PeerDetails>,
    pub added_v6: Vec<PexPeer>,
    pub dropped_v6: Vec<PeerDetails>,
}

impl PexMessage {
    /// Creates a new, empty PEX message.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the message contains no added or dropped peers.
    pub fn is_empty(&self) -> bool {
        self.added_v4.is_empty()
            && self.dropped_v4.is_empty()
            && self.added_v6.is_empty()
            && self.dropped_v6.is_empty()
    }

    /// Adds an IPv4 peer with BEP 11 flags.
    pub fn add_v4(&mut self, peer: PeerDetails, flags: PexFlags) {
        self.added_v4.push(PexPeer::new(peer, flags));
    }

    /// Adds a dropped IPv4 peer.
    pub fn drop_v4(&mut self, peer: PeerDetails) {
        self.dropped_v4.push(peer);
    }

    /// Adds an IPv6 peer with BEP 11 flags.
    pub fn add_v6(&mut self, peer: PeerDetails, flags: PexFlags) {
        self.added_v6.push(PexPeer::new(peer, flags));
    }

    /// Adds a dropped IPv6 peer.
    pub fn drop_v6(&mut self, peer: PeerDetails) {
        self.dropped_v6.push(peer);
    }

    /// Encodes this PEX message into a bencoded byte vector.
    ///
    /// Respects BEP 11 dictionary key ordering:
    /// `added`, `added.f`, `added6`, `added6.f`, `dropped`, `dropped6`.
    pub fn encode(&self) -> Vec<u8> {
        let mut added_bytes = Vec::with_capacity(self.added_v4.len() * 6);
        let mut added_flags = Vec::with_capacity(self.added_v4.len());
        for p in &self.added_v4 {
            if let Some(octets) = parse_ipv4_octets(&p.details.ip) {
                added_bytes.extend_from_slice(&octets);
                added_bytes.extend_from_slice(&p.details.port.to_be_bytes());
                added_flags.push(p.flags.bits());
            }
        }

        let mut dropped_bytes = Vec::with_capacity(self.dropped_v4.len() * 6);
        for p in &self.dropped_v4 {
            if let Some(octets) = parse_ipv4_octets(&p.ip) {
                dropped_bytes.extend_from_slice(&octets);
                dropped_bytes.extend_from_slice(&p.port.to_be_bytes());
            }
        }

        let mut added6_bytes = Vec::with_capacity(self.added_v6.len() * 18);
        let mut added6_flags = Vec::with_capacity(self.added_v6.len());
        for p in &self.added_v6 {
            if let Some(octets) = parse_ipv6_octets(&p.details.ip) {
                added6_bytes.extend_from_slice(&octets);
                added6_bytes.extend_from_slice(&p.details.port.to_be_bytes());
                added6_flags.push(p.flags.bits());
            }
        }

        let mut dropped6_bytes = Vec::with_capacity(self.dropped_v6.len() * 18);
        for p in &self.dropped_v6 {
            if let Some(octets) = parse_ipv6_octets(&p.ip) {
                dropped6_bytes.extend_from_slice(&octets);
                dropped6_bytes.extend_from_slice(&p.port.to_be_bytes());
            }
        }

        let mut entries = Vec::new();
        // BEP 11: "added" and "dropped" are mandatory keys (may be empty strings)
        entries.push((b"added".as_slice(), BNode::String(&added_bytes)));
        if !added_flags.is_empty() {
            entries.push((b"added.f".as_slice(), BNode::String(&added_flags)));
        }
        if !added6_bytes.is_empty() {
            entries.push((b"added6".as_slice(), BNode::String(&added6_bytes)));
        }
        if !added6_flags.is_empty() {
            entries.push((b"added6.f".as_slice(), BNode::String(&added6_flags)));
        }
        entries.push((b"dropped".as_slice(), BNode::String(&dropped_bytes)));
        if !dropped6_bytes.is_empty() {
            entries.push((b"dropped6".as_slice(), BNode::String(&dropped6_bytes)));
        }

        // Bencode dictionaries MUST be sorted lexicographically
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let dict = BNode::Dictionary(entries);
        Bencode::encode(&dict)
    }

    /// Decodes a bencoded PEX message payload.
    pub fn decode(payload: &[u8], info_hash: &[u8]) -> Result<Self, BitTorrentError> {
        let bnode = Bencode::decode(payload)?;
        let mut msg = PexMessage::new();

        // 1. added (IPv4)
        let added_flags_opt = bnode.dict_get(b"added.f").and_then(|n| n.as_string());
        if let Some(added_node) = bnode.dict_get(b"added") {
            if let Some(bytes) = added_node.as_string() {
                for (idx, chunk) in bytes.chunks_exact(6).enumerate() {
                    let ip = format!("{}.{}.{}.{}", chunk[0], chunk[1], chunk[2], chunk[3]);
                    let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                    let flag = added_flags_opt.and_then(|f| f.get(idx).copied()).unwrap_or(0);
                    msg.added_v4.push(PexPeer::new(
                        PeerDetails {
                            info_hash: info_hash.to_vec(),
                            peer_id: None,
                            ip,
                            port,
                        },
                        PexFlags::new(flag),
                    ));
                }
            }
        }

        // 2. dropped (IPv4)
        if let Some(dropped_node) = bnode.dict_get(b"dropped") {
            if let Some(bytes) = dropped_node.as_string() {
                for chunk in bytes.chunks_exact(6) {
                    let ip = format!("{}.{}.{}.{}", chunk[0], chunk[1], chunk[2], chunk[3]);
                    let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                    msg.dropped_v4.push(PeerDetails {
                        info_hash: info_hash.to_vec(),
                        peer_id: None,
                        ip,
                        port,
                    });
                }
            }
        }

        // 3. added6 (IPv6)
        let added6_flags_opt = bnode.dict_get(b"added6.f").and_then(|n| n.as_string());
        if let Some(added6_node) = bnode.dict_get(b"added6") {
            if let Some(bytes) = added6_node.as_string() {
                for (idx, chunk) in bytes.chunks_exact(18).enumerate() {
                    let mut octets = [0u8; 16];
                    octets.copy_from_slice(&chunk[0..16]);
                    #[cfg(feature = "std")]
                    let ip = std::net::Ipv6Addr::from(octets).to_string();
                    #[cfg(not(feature = "std"))]
                    let ip = format_ipv6_octets(&octets);

                    let port = u16::from_be_bytes([chunk[16], chunk[17]]);
                    let flag = added6_flags_opt.and_then(|f| f.get(idx).copied()).unwrap_or(0);
                    msg.added_v6.push(PexPeer::new(
                        PeerDetails {
                            info_hash: info_hash.to_vec(),
                            peer_id: None,
                            ip,
                            port,
                        },
                        PexFlags::new(flag),
                    ));
                }
            }
        }

        // 4. dropped6 (IPv6)
        if let Some(dropped6_node) = bnode.dict_get(b"dropped6") {
            if let Some(bytes) = dropped6_node.as_string() {
                for chunk in bytes.chunks_exact(18) {
                    let mut octets = [0u8; 16];
                    octets.copy_from_slice(&chunk[0..16]);
                    #[cfg(feature = "std")]
                    let ip = std::net::Ipv6Addr::from(octets).to_string();
                    #[cfg(not(feature = "std"))]
                    let ip = format_ipv6_octets(&octets);

                    let port = u16::from_be_bytes([chunk[16], chunk[17]]);
                    msg.dropped_v6.push(PeerDetails {
                        info_hash: info_hash.to_vec(),
                        peer_id: None,
                        ip,
                        port,
                    });
                }
            }
        }

        Ok(msg)
    }
}

/// Parses an IPv4 address string into standard 4-byte network byte order octets.
pub fn parse_ipv4_octets(ip: &str) -> Option<[u8; 4]> {
    #[cfg(feature = "std")]
    {
        ip.trim().parse::<Ipv4Addr>().ok().map(|a| a.octets())
    }
    #[cfg(not(feature = "std"))]
    {
        let mut parts = ip.trim().split('.');
        let a = parts.next()?.parse::<u8>().ok()?;
        let b = parts.next()?.parse::<u8>().ok()?;
        let c = parts.next()?.parse::<u8>().ok()?;
        let d = parts.next()?.parse::<u8>().ok()?;
        if parts.next().is_some() { return None; }
        Some([a, b, c, d])
    }
}

/// Parses an IPv6 address string into standard 16-byte network byte order octets.
pub fn parse_ipv6_octets(ip: &str) -> Option<[u8; 16]> {
    let clean = ip.trim().trim_start_matches('[').trim_end_matches(']');
    #[cfg(feature = "std")]
    {
        clean.parse::<Ipv6Addr>().ok().map(|a| a.octets())
    }
    #[cfg(not(feature = "std"))]
    {
        None
    }
}

#[cfg(not(feature = "std"))]
fn format_ipv6_octets(octets: &[u8; 16]) -> String {
    alloc::format!(
        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
        u16::from_be_bytes([octets[0], octets[1]]),
        u16::from_be_bytes([octets[2], octets[3]]),
        u16::from_be_bytes([octets[4], octets[5]]),
        u16::from_be_bytes([octets[6], octets[7]]),
        u16::from_be_bytes([octets[8], octets[9]]),
        u16::from_be_bytes([octets[10], octets[11]]),
        u16::from_be_bytes([octets[12], octets[13]]),
        u16::from_be_bytes([octets[14], octets[15]])
    )
}
