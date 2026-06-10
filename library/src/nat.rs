//! NAT Port Mapping Protocol (NAT-PMP - BEP 27)
//!
//! Provides automatic port forwarding by querying the default gateway via NAT-PMP.

use crate::error::BitTorrentError;
use std::net::{UdpSocket, Ipv4Addr, SocketAddrV4};
use std::time::Duration;

pub struct NatPmpClient {
    gateway: Ipv4Addr,
}

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
    pub fn new(gateway: Ipv4Addr) -> Self {
        NatPmpClient { gateway }
    }

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

    pub fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError> {
        let _ = self.request_mapping(is_tcp, internal_port, 0, 0);
        Ok(())
    }

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
