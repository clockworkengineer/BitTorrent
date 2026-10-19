//! Contact information and identifiers for a remote peer.

use alloc::string::String;
use alloc::vec::Vec;

/// Contains contact information and identifiers for a remote peer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerDetails {
    pub info_hash: Vec<u8>,
    pub peer_id: Option<String>,
    pub ip: String,
    pub port: u16,
}

impl PeerDetails {
    /// Formats the peer IP address and port into a standard socket address string (e.g. `1.2.3.4:6881` or `[2001:db8::1]:6881`).
    pub fn endpoint(&self) -> String {
        crate::util::format_socket_addr(&self.ip, self.port)
    }

    /// Returns `true` if this peer represents an IPv6 endpoint.
    pub fn is_ipv6(&self) -> bool {
        self.ip.contains(':') && !self.ip.starts_with("webrtc:")
    }

    /// Returns `true` if this peer represents an IPv4 endpoint.
    pub fn is_ipv4(&self) -> bool {
        !self.is_ipv6() && !self.ip.starts_with("webrtc:")
    }

    /// Returns `true` if this peer represents a WebRTC DataChannel browser peer.
    pub fn is_webrtc(&self) -> bool {
        self.ip.starts_with("webrtc:")
    }

    /// Returns a clean display endpoint string without transport-specific URI schemes.
    pub fn display_endpoint(&self) -> String {
        if self.is_webrtc() {
            self.ip.clone()
        } else {
            self.endpoint()
        }
    }
}
