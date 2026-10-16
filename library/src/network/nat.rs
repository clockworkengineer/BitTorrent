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

/// Connection details for a discovered or configured UPnP Internet Gateway Device (IGD).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpnpEndpoint {
    /// Hostname or IP address (e.g. "192.168.1.1").
    pub host: String,
    /// HTTP control port (e.g. 5000, 2189, or 49152).
    pub port: u16,
    /// Control path URL (e.g. "/ctl/IPConn" or "/xml/wanipc.xml").
    pub control_path: String,
    /// UPnP service type schema (e.g. "urn:schemas-upnp-org:service:WANIPConnection:1").
    pub service_type: String,
}

/// Builds an SSDP M-SEARCH multicast discovery request packet.
pub fn build_ssdp_msearch(target: &str) -> String {
    format!(
        "M-SEARCH * HTTP/1.1\r\n\
         HOST: 239.255.255.250:1900\r\n\
         ST: {}\r\n\
         MAN: \"ssdp:discover\"\r\n\
         MX: 2\r\n\
         \r\n",
        target
    )
}

/// Parses the `LOCATION` HTTP header from an SSDP discovery response.
pub fn parse_ssdp_location(response: &str) -> Option<String> {
    for line in response.lines() {
        let trimmed = line.trim();
        if let Some(colon_idx) = trimmed.find(':') {
            let header_name = trimmed[..colon_idx].trim();
            if header_name.eq_ignore_ascii_case("location") {
                let location_val = trimmed[colon_idx + 1..].trim();
                if !location_val.is_empty() {
                    return Some(location_val.to_string());
                }
            }
        }
    }
    None
}

/// Parses a URL into (host, port, path) components.
pub fn parse_url_components(url: &str) -> Result<(String, u16, String), BitTorrentError> {
    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);

    let (host_port, path) = match without_scheme.find('/') {
        Some(idx) => (&without_scheme[..idx], &without_scheme[idx..]),
        None => (without_scheme, "/"),
    };

    let (host, port) = match host_port.find(':') {
        Some(colon_idx) => {
            let host = &host_port[..colon_idx];
            let port = host_port[colon_idx + 1..]
                .parse::<u16>()
                .map_err(|e| BitTorrentError::Parse(format!("Invalid port in URL: {}", e)))?;
            (host.to_string(), port)
        }
        None => (host_port.to_string(), 80),
    };

    Ok((host, port, path.to_string()))
}

/// Extracts text content between an XML tag, ignoring namespace prefixes.
pub fn extract_xml_tag_content(xml: &str, tag: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(open_bracket) = xml[search_from..].find('<') {
        let tag_start = search_from + open_bracket + 1;
        if let Some(close_bracket) = xml[tag_start..].find('>') {
            let full_tag_content = &xml[tag_start..tag_start + close_bracket];
            let tag_name = full_tag_content.split_whitespace().next().unwrap_or("");
            let matches = tag_name.eq_ignore_ascii_case(tag)
                || (tag_name.ends_with(tag) && tag_name[..tag_name.len() - tag.len()].ends_with(':'));

            if matches && !tag_name.starts_with('/') {
                let content_start = tag_start + close_bracket + 1;
                if let Some(close_tag_pos) = xml[content_start..].find("</") {
                    let close_tag_start = content_start + close_tag_pos + 2;
                    if let Some(close_tag_end) = xml[close_tag_start..].find('>') {
                        let close_name = &xml[close_tag_start..close_tag_start + close_tag_end];
                        let close_matches = close_name.eq_ignore_ascii_case(tag)
                            || (close_name.ends_with(tag) && close_name[..close_name.len() - tag.len()].ends_with(':'));
                        if close_matches {
                            return Some(xml[content_start..content_start + close_tag_pos].trim().to_string());
                        }
                    }
                }
            }
            search_from = tag_start + close_bracket + 1;
        } else {
            break;
        }
    }
    None
}

/// Parses UPnP device description XML to discover WAN connection service type and control URL.
pub fn parse_device_description(xml: &str) -> Option<(String, String)> {
    let mut search_from = 0;
    let mut candidates = Vec::new();

    while let Some(start_idx) = xml[search_from..].find("<service>") {
        let service_start = search_from + start_idx + 9;
        if let Some(end_idx) = xml[service_start..].find("</service>") {
            let service_xml = &xml[service_start..service_start + end_idx];
            if let Some(service_type) = extract_xml_tag_content(service_xml, "serviceType") {
                if let Some(control_url) = extract_xml_tag_content(service_xml, "controlURL") {
                    let control_path = if control_url.starts_with('/')
                        || control_url.starts_with("http://")
                        || control_url.starts_with("https://")
                    {
                        control_url
                    } else {
                        format!("/{}", control_url)
                    };
                    candidates.push((service_type, control_path));
                }
            }
            search_from = service_start + end_idx + 10;
        } else {
            break;
        }
    }

    // Prioritize WANIPConnection:1, then WANIPConnection:2, then WANPPPConnection:1
    for (st, path) in &candidates {
        if st.contains("WANIPConnection:1") {
            return Some((st.clone(), path.clone()));
        }
    }
    for (st, path) in &candidates {
        if st.contains("WANIPConnection") {
            return Some((st.clone(), path.clone()));
        }
    }
    for (st, path) in &candidates {
        if st.contains("WANPPPConnection") {
            return Some((st.clone(), path.clone()));
        }
    }

    candidates.into_iter().next()
}

/// Fetches an HTTP resource and returns its body as a String.
pub fn fetch_http_resource(host: &str, port: u16, path: &str, timeout: Duration) -> Result<String, BitTorrentError> {
    use std::net::ToSocketAddrs;
    let addr_str = format!("{}:{}", host, port);
    let socket_addr = addr_str
        .to_socket_addrs()
        .map_err(BitTorrentError::Io)?
        .next()
        .ok_or_else(|| BitTorrentError::Protocol(format!("Could not resolve host: {}", host)))?;

    let mut stream = TcpStream::connect_timeout(&socket_addr, timeout).map_err(BitTorrentError::Io)?;
    stream.set_read_timeout(Some(timeout)).map_err(BitTorrentError::Io)?;
    stream.set_write_timeout(Some(timeout)).map_err(BitTorrentError::Io)?;

    let request = format!(
        "GET {} HTTP/1.1\r\n\
         Host: {}:{}\r\n\
         Connection: close\r\n\
         \r\n",
        path, host, port
    );
    stream.write_all(request.as_bytes()).map_err(BitTorrentError::Io)?;

    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);

    let resp_str = String::from_utf8_lossy(&response);
    if let Some(body_start) = resp_str.find("\r\n\r\n") {
        Ok(resp_str[body_start + 4..].to_string())
    } else {
        Ok(resp_str.to_string())
    }
}

/// Discovers a UPnP IGD endpoint via SSDP multicast.
pub fn discover_upnp_endpoint(timeout: Duration) -> Result<UpnpEndpoint, BitTorrentError> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(BitTorrentError::Io)?;
    socket.set_read_timeout(Some(timeout)).map_err(BitTorrentError::Io)?;

    let search_targets = [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "urn:schemas-upnp-org:service:WANIPConnection:1",
        "urn:schemas-upnp-org:service:WANPPPConnection:1",
        "upnp:rootdevice",
    ];

    let dest = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);

    for target in &search_targets {
        let msg = build_ssdp_msearch(target);
        let _ = socket.send_to(msg.as_bytes(), dest);
    }

    let mut buf = [0u8; 4096];
    let start_time = std::time::Instant::now();

    while start_time.elapsed() < timeout {
        if let Ok((n, _src)) = socket.recv_from(&mut buf) {
            let resp_str = String::from_utf8_lossy(&buf[..n]);
            if let Some(loc) = parse_ssdp_location(&resp_str) {
                if let Ok((host, port, path)) = parse_url_components(&loc) {
                    if let Ok(desc_xml) = fetch_http_resource(&host, port, &path, timeout) {
                        if let Some((service_type, control_url)) = parse_device_description(&desc_xml) {
                            let (control_host, control_port, control_path) = if control_url.starts_with("http://")
                                || control_url.starts_with("https://")
                            {
                                parse_url_components(&control_url).unwrap_or((host, port, control_url))
                            } else {
                                (host, port, control_url)
                            };

                            return Ok(UpnpEndpoint {
                                host: control_host,
                                port: control_port,
                                control_path,
                                service_type,
                            });
                        }
                    }
                }
            }
        }
    }

    Err(BitTorrentError::Protocol("No UPnP IGD responded to SSDP discovery".into()))
}

/// A UPnP client that discovers router control endpoints via SSDP and manages port mappings using SOAP.
pub struct UpnpClient {
    gateway: Option<Ipv4Addr>,
    endpoint: std::sync::RwLock<Option<UpnpEndpoint>>,
}

impl UpnpClient {
    /// Creates a UPnP client associated with a default router gateway IP.
    pub fn new(gateway: Ipv4Addr) -> Self {
        Self {
            gateway: Some(gateway),
            endpoint: std::sync::RwLock::new(None),
        }
    }

    /// Creates a UPnP client with a pre-configured endpoint (useful for explicit router setups or testing).
    pub fn with_endpoint(endpoint: UpnpEndpoint) -> Self {
        Self {
            gateway: None,
            endpoint: std::sync::RwLock::new(Some(endpoint)),
        }
    }

    /// Automatically discovers a UPnP IGD via SSDP multicast.
    pub fn discover() -> Result<Self, BitTorrentError> {
        let ep = discover_upnp_endpoint(Duration::from_secs(2))?;
        Ok(Self::with_endpoint(ep))
    }

    /// Resolves or retrieves the cached UPnP control endpoint.
    pub fn resolve_endpoint(&self) -> Result<UpnpEndpoint, BitTorrentError> {
        if let Some(ep) = self.endpoint.read().unwrap().as_ref() {
            return Ok(ep.clone());
        }

        // 1. Attempt SSDP multicast discovery
        if let Ok(ep) = discover_upnp_endpoint(Duration::from_millis(500)) {
            *self.endpoint.write().unwrap() = Some(ep.clone());
            return Ok(ep);
        }

        // 2. Fallback: If gateway was provided, use fallback endpoint
        if let Some(gw) = self.gateway {
            let fallback_ep = UpnpEndpoint {
                host: gw.to_string(),
                port: 2189,
                control_path: "/xml/wanipc.xml".to_string(),
                service_type: "urn:schemas-upnp-org:service:WANIPConnection:1".to_string(),
            };
            *self.endpoint.write().unwrap() = Some(fallback_ep.clone());
            return Ok(fallback_ep);
        }

        Err(BitTorrentError::Protocol("No UPnP IGD endpoint discovered".into()))
    }

    /// Queries the router for its external public IP address via UPnP SOAP.
    pub fn get_external_ip(&self) -> Result<Ipv4Addr, BitTorrentError> {
        let endpoint = self.resolve_endpoint()?;
        let body = format!("<u:GetExternalIPAddress xmlns:u=\"{}\"></u:GetExternalIPAddress>", endpoint.service_type);
        let resp = self.perform_soap_request_to(&endpoint, "GetExternalIPAddress", &body)?;
        if let Some(ip_str) = extract_xml_tag_content(&resp, "NewExternalIPAddress") {
            ip_str.trim().parse::<Ipv4Addr>().map_err(|e| BitTorrentError::Parse(e.to_string()))
        } else {
            Err(BitTorrentError::Parse("Missing NewExternalIPAddress in SOAP response".into()))
        }
    }

    /// Sends a SOAP action request to the specified UPnP control endpoint.
    pub fn perform_soap_request_to(
        &self,
        endpoint: &UpnpEndpoint,
        action: &str,
        body: &str,
    ) -> Result<String, BitTorrentError> {
        use std::net::ToSocketAddrs;

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
            "POST {} HTTP/1.1\r\n\
             Host: {}:{}\r\n\
             Content-Length: {}\r\n\
             Content-Type: text/xml; charset=\"utf-8\"\r\n\
             SOAPAction: \"{}#{}\"\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            endpoint.control_path,
            endpoint.host,
            endpoint.port,
            soap_payload.len(),
            endpoint.service_type,
            action,
            soap_payload
        );

        let dest = format!("{}:{}", endpoint.host, endpoint.port);
        let socket_addr = dest
            .to_socket_addrs()
            .map_err(BitTorrentError::Io)?
            .next()
            .ok_or_else(|| BitTorrentError::Protocol(format!("Could not resolve host: {}", endpoint.host)))?;

        let mut stream = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2))
            .map_err(BitTorrentError::Io)?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(BitTorrentError::Io)?;
        stream.set_write_timeout(Some(Duration::from_secs(2))).map_err(BitTorrentError::Io)?;

        stream.write_all(request_str.as_bytes()).map_err(BitTorrentError::Io)?;
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response);

        let resp_str = String::from_utf8_lossy(&response).to_string();
        if resp_str.contains("200 OK") || resp_str.contains("200") {
            Ok(resp_str)
        } else {
            Err(BitTorrentError::Parse(format!("UPnP SOAP action '{}' failed: {}", action, resp_str)))
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
        let endpoint = self.resolve_endpoint()?;
        let protocol = if is_tcp { "TCP" } else { "UDP" };
        let local_ip = crate::host::get_ip();
        let body = format!(
            "<u:AddPortMapping xmlns:u=\"{}\">\r\n\
             <NewRemoteHost></NewRemoteHost>\r\n\
             <NewExternalPort>{}</NewExternalPort>\r\n\
             <NewProtocol>{}</NewProtocol>\r\n\
             <NewInternalPort>{}</NewInternalPort>\r\n\
             <NewInternalClient>{}</NewInternalClient>\r\n\
             <NewEnabled>1</NewEnabled>\r\n\
             <NewPortMappingDescription>BitTorrent-rs</NewPortMappingDescription>\r\n\
             <NewLeaseDuration>{}</NewLeaseDuration>\r\n\
             </u:AddPortMapping>",
            endpoint.service_type, external_port, protocol, internal_port, local_ip, lifetime_secs
        );

        self.perform_soap_request_to(&endpoint, "AddPortMapping", &body)?;
        Ok(external_port)
    }

    fn release_mapping(&self, is_tcp: bool, internal_port: u16) -> Result<(), BitTorrentError> {
        let endpoint = self.resolve_endpoint()?;
        let protocol = if is_tcp { "TCP" } else { "UDP" };
        let body = format!(
            "<u:DeletePortMapping xmlns:u=\"{}\">\r\n\
             <NewRemoteHost></NewRemoteHost>\r\n\
             <NewExternalPort>{}</NewExternalPort>\r\n\
             <NewProtocol>{}</NewProtocol>\r\n\
             </u:DeletePortMapping>",
            endpoint.service_type, internal_port, protocol
        );

        self.perform_soap_request_to(&endpoint, "DeletePortMapping", &body)?;
        Ok(())
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

    /// Access the underlying UPnP client directly.
    pub fn upnp(&self) -> &UpnpClient {
        &self.upnp
    }

    /// Access the underlying NAT-PMP client directly.
    pub fn nat_pmp(&self) -> &NatPmpClient {
        &self.nat_pmp
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
