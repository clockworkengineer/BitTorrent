//! NAT Port Mapping Protocol (NAT-PMP) client — RFC 6886
//!
//! Provides automatic port forwarding by sending mapping requests to the local
//! NAT router gateway. On `start_download()` the session maps port 6881 for both
//! TCP and UDP; on `stop()` the mappings are released.
//!
//! See [`NatPmpClient`] for the main entry point and
//! [docs/nat-pmp.md](https://github.com/clockworkengineer/BitTorrent/blob/main/docs/nat-pmp.md)
//! for the packet format and protocol walkthrough.

use crate::error::BitTorrentError;
use std::net::{UdpSocket, Ipv4Addr, SocketAddrV4};
use std::time::Duration;

/// A UDP client that communicates with the local NAT router using NAT-PMP (RFC 6886).
///
/// Create a client pointing at the router gateway, then call
/// [`request_mapping`](NatPmpClient::request_mapping) to open a port and
/// [`release_mapping`](NatPmpClient::release_mapping) to remove it.
pub struct NatPmpClient {
    gateway: Ipv4Addr,
}

/// Infers the default gateway IP address from the local machine's IP.
///
/// Replaces the last octet of the local IP with `.1` — the conventional
/// gateway address on home networks (e.g. `192.168.1.105` → `192.168.1.1`).
/// Falls back to `192.168.1.1` for loopback or unparseable addresses.
pub fn get_default_gateway() -> Ipv4Addr {
    let local_ip_str = crate::host::get_ip();
    if let Ok(local_ip) = local_ip_str.parse::<Ipv4Addr>() {
        let octets = local_ip.octets();
        if octets[0] == 127 {
            Ipv4Addr::new(192, 168, 1, 1)
        } else {
            Ipv4Addr::new(octets[0], octets[1], octets[2], 1)
        }
    } else {
        Ipv4Addr::new(192, 168, 1, 1)
    }
}

impl NatPmpClient {
    /// Creates a new `NatPmpClient` targeting the given gateway IP address.
    pub fn new(gateway: Ipv4Addr) -> Self {
        NatPmpClient { gateway }
    }

    /// Sends a port mapping request to the gateway and returns the assigned external port.
    ///
    /// # Arguments
    /// * `is_tcp`         – `true` for a TCP mapping (opcode 2), `false` for UDP (opcode 1).
    /// * `internal_port`  – The local port to expose (e.g. `6881`).
    /// * `external_port`  – The desired external port (`0` lets the router choose).
    /// * `lifetime_secs`  – How long the mapping should last in seconds (e.g. `3600`).
    ///
    /// Returns the external port number confirmed by the router on success.
    /// Fails with [`BitTorrentError`] if the socket times out, the response is malformed,
    /// or the router returns a non-zero result code.
    pub fn request_mapping(
        &self,
        is_tcp: bool,
        internal_port: u16,
        external_port: u16,
        lifetime_secs: u32,
    ) -> Result<u16, BitTorrentError> {
        let socket = UdpSocket::bind("0.0.0.0:0").map_err(BitTorrentError::Io)?;
        socket.set_read_timeout(Some(Duration::from_secs(2))).map_err(BitTorrentError::Io)?;

        let request = self.build_mapping_request(is_tcp, internal_port, external_port, lifetime_secs);
        let dest = SocketAddrV4::new(self.gateway, 5351);
        socket.send_to(&request, dest).map_err(BitTorrentError::Io)?;

        let mut buf = [0u8; 64];
        let (n, _src) = socket.recv_from(&mut buf).map_err(BitTorrentError::Io)?;

        let expected_op = if is_tcp { 130 } else { 129 };
        if n >= 16 && buf[1] == expected_op {
            let (_, mapped_external_port, _) = Self::parse_mapping_response(&buf[..n])?;
            Ok(mapped_external_port)
        } else {
            Err(BitTorrentError::Parse("Invalid NAT-PMP response OP code".into()))
        }
    }

    /// Removes a port mapping by sending a `lifetime = 0` request to the gateway.
    ///
    /// Errors from the gateway are silently ignored — a best-effort deletion is sufficient
    /// because mappings expire naturally after their configured lifetime.
    pub fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError> {
        let _ = self.request_mapping(is_tcp, internal_port, 0, 0);
        Ok(())
    }

    /// Builds a raw 12-byte NAT-PMP port mapping request packet.
    ///
    /// Useful for testing or for sending the packet manually via a custom socket.
    ///
    /// Packet layout:
    /// ```text
    /// [0]     version   = 0
    /// [1]     opcode    = 2 (TCP) or 1 (UDP)
    /// [2..4]  reserved  = 0
    /// [4..6]  internal port (big-endian)
    /// [6..8]  external port (big-endian)
    /// [8..12] lifetime  (big-endian u32, seconds)
    /// ```
    pub fn build_mapping_request(
        &self,
        is_tcp: bool,
        internal_port: u16,
        external_port: u16,
        lifetime_secs: u32,
    ) -> Vec<u8> {
        let mut pkt = vec![0u8; 12];
        pkt[0] = 0; // Version
        pkt[1] = if is_tcp { 2 } else { 1 };
        pkt[4..6].copy_from_slice(&internal_port.to_be_bytes());
        pkt[6..8].copy_from_slice(&external_port.to_be_bytes());
        pkt[8..12].copy_from_slice(&lifetime_secs.to_be_bytes());
        pkt
    }

    /// Parses a raw 16-byte NAT-PMP mapping response.
    ///
    /// Returns `(internal_port, external_port, lifetime_secs)` on success.
    /// Fails if the buffer is too short, the version field is non-zero, or the
    /// result code indicates an error.
    pub fn parse_mapping_response(buf: &[u8]) -> Result<(u16, u16, u32), BitTorrentError> {
        if buf.len() < 16 {
            return Err(BitTorrentError::Parse("NAT-PMP response too short".into()));
        }
        if buf[0] != 0 {
            return Err(BitTorrentError::Parse(format!("Unsupported NAT-PMP version: {}", buf[0])));
        }
        let result_code = u16::from_be_bytes([buf[2], buf[3]]);
        if result_code != 0 {
            return Err(BitTorrentError::Parse(format!(
                "NAT-PMP mapping failed with result code: {}",
                result_code
            )));
        }
        let internal_port = u16::from_be_bytes([buf[8], buf[9]]);
        let external_port = u16::from_be_bytes([buf[10], buf[11]]);
        let lifetime = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);
        Ok((internal_port, external_port, lifetime))
    }
}
