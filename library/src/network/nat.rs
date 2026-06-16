//! NAT Port Mapping (NAT-PMP / UPnP) client
//!
//! Provides automatic port forwarding by sending mapping requests to the local
//! NAT router gateway using either NAT-PMP (RFC 6886) or UPnP (SOAP over SSDP).

use crate::error::BitTorrentError;
use std::net::{UdpSocket, TcpStream, Ipv4Addr, SocketAddrV4};
use std::time::Duration;
use std::io::{Read, Write};

/// Infers the default gateway IP address from the local machine's IP.
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

/// Unified trait for port mapping mechanisms.
pub trait PortMapper: Send + Sync {
    fn request_mapping(
        &self,
        is_tcp: bool,
        internal_port: u16,
        external_port: u16,
        lifetime_secs: u32,
    ) -> Result<u16, BitTorrentError>;

    fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError>;
}

/// A UDP client that communicates with the local NAT router using NAT-PMP (RFC 6886).
pub struct NatPmpClient {
    gateway: Ipv4Addr,
}

impl NatPmpClient {
    pub fn new(gateway: Ipv4Addr) -> Self {
        NatPmpClient { gateway }
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

impl PortMapper for NatPmpClient {
    fn request_mapping(
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

    fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError> {
        let _ = self.request_mapping(is_tcp, internal_port, 0, 0);
        Ok(())
    }
}

/// A UPnP client that opens ports via SOAP requests.
pub struct UpnpClient {
    gateway: Ipv4Addr,
}

impl UpnpClient {
    pub fn new(gateway: Ipv4Addr) -> Self {
        Self { gateway }
    }

    fn perform_soap_request(&self, action: &str, body: &str) -> Result<(), BitTorrentError> {
        // Build SOAP HTTP request
        let soap_payload = format!(
            "<?xml version=\"1.0\"?>\r\n\
             <s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\">\r\n\
             <s:Body>\r\n\
             {}\r\n\
             </s:Body>\r\n\
             </s:Envelope>",
            body
        );

        let request_str = format!(
            "POST /xml/wanipc.xml HTTP/1.1\r\n\
             Host: {}:2189\r\n\
             Content-Length: {}\r\n\
             Content-Type: text/xml; charset=\"utf-8\"\r\n\
             SOAPAction: \"urn:schemas-upnp-org:service:WANIPConnection:1#{}\"\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            self.gateway,
            soap_payload.len(),
            action,
            soap_payload
        );

        let dest = SocketAddrV4::new(self.gateway, 2189);
        let mut stream = TcpStream::connect_timeout(&dest.into(), Duration::from_secs(2))
            .map_err(BitTorrentError::Io)?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(BitTorrentError::Io)?;

        stream.write_all(request_str.as_bytes()).map_err(BitTorrentError::Io)?;
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);

        // Simple validation that HTTP response is 200 OK
        let resp_str = String::from_utf8_lossy(&response);
        if resp_str.contains("200 OK") || resp_str.contains("200") {
            Ok(())
        } else {
            Err(BitTorrentError::Parse("UPnP SOAP action failed".into()))
        }
    }
}

impl PortMapper for UpnpClient {
    fn request_mapping(
        &self,
        is_tcp: bool,
        internal_port: u16,
        external_port: u16,
        lifetime_secs: u32,
    ) -> Result<u16, BitTorrentError> {
        let protocol = if is_tcp { "TCP" } else { "UDP" };
        let local_ip = crate::host::get_ip();
        let body = format!(
            "<u:AddPortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\r\n\
             <NewRemoteHost></NewRemoteHost>\r\n\
             <NewExternalPort>{}</NewExternalPort>\r\n\
             <NewProtocol>{}</NewProtocol>\r\n\
             <NewInternalPort>{}</NewInternalPort>\r\n\
             <NewInternalClient>{}</NewInternalClient>\r\n\
             <NewEnabled>1</NewEnabled>\r\n\
             <NewPortMappingDescription>BitTorrent-rs</NewPortMappingDescription>\r\n\
             <NewLeaseDuration>{}</NewLeaseDuration>\r\n\
             </u:AddPortMapping>",
            external_port, protocol, internal_port, local_ip, lifetime_secs
        );

        self.perform_soap_request("AddPortMapping", &body)?;
        Ok(external_port)
    }

    fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError> {
        let protocol = if is_tcp { "TCP" } else { "UDP" };
        let body = format!(
            "<u:DeletePortMapping xmlns:u=\"urn:schemas-upnp-org:service:WANIPConnection:1\">\r\n\
             <NewRemoteHost></NewRemoteHost>\r\n\
             <NewExternalPort>{}</NewExternalPort>\r\n\
             <NewProtocol>{}</NewProtocol>\r\n\
             </u:DeletePortMapping>",
            internal_port, protocol
        );

        self.perform_soap_request("DeletePortMapping", &body)
    }
}

/// Fallback port mapper that tries NAT-PMP first, then UPnP.
pub struct FallbackPortMapper {
    nat_pmp: NatPmpClient,
    upnp: UpnpClient,
}

impl FallbackPortMapper {
    pub fn new(gateway: Ipv4Addr) -> Self {
        Self {
            nat_pmp: NatPmpClient::new(gateway),
            upnp: UpnpClient::new(gateway),
        }
    }
}

impl PortMapper for FallbackPortMapper {
    fn request_mapping(
        &self,
        is_tcp: bool,
        internal_port: u16,
        external_port: u16,
        lifetime_secs: u32,
    ) -> Result<u16, BitTorrentError> {
        if let Ok(port) = self.nat_pmp.request_mapping(is_tcp, internal_port, external_port, lifetime_secs) {
            return Ok(port);
        }
        self.upnp.request_mapping(is_tcp, internal_port, external_port, lifetime_secs)
    }

    fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError> {
        let _ = self.nat_pmp.release_mapping(is_tcp, internal_port);
        let _ = self.upnp.release_mapping(is_tcp, internal_port);
        Ok(())
    }
}
