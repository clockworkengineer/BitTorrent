//! Tracker announcer implementations
//!
//! Provides the `Announcer` trait and concrete implementations for HTTP (`HttpAnnouncer`)
//! and UDP (`UdpAnnouncer`) protocols to announce client status to trackers and receive peer lists.

use crate::error::BitTorrentError;
use crate::tracker::{
    AnnounceResponse, PeerDetails, Tracker, TrackerAnnounceContext, TrackerEvent,
};
use crate::util::{pack_u32, pack_u64, unpack_u32, unpack_u64};
use std::io::Read;
use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;
use urlencoding::encode;
use urlencoding::encode_binary;

/// Trait defining the announcement interface to communicate with a BitTorrent tracker.
pub mod announcer_trait {
    use super::*;
    pub trait Announcer: Send {
        /// Performs an announce request to the tracker using the given context and returns the response.
        fn announce(
            &mut self,
            tracker: &crate::tracker::TrackerAnnounceContext,
        ) -> Result<AnnounceResponse, BitTorrentError>;
    }
}
pub use announcer_trait::Announcer;

/// An announcer that uses the HTTP/HTTPS protocol to communicate with trackers.
pub struct HttpAnnouncer;

impl HttpAnnouncer {
    /// Creates a new `HttpAnnouncer`.
    pub fn new() -> Self {
        HttpAnnouncer
    }

    /// Parses and decodes the HTTP Bencoded tracker announce response.
    fn decode_announce_response(
        tracker: &TrackerAnnounceContext,
        announce_response: &[u8],
    ) -> Result<AnnounceResponse, BitTorrentError> {
        let mut response = AnnounceResponse::default();
        if !announce_response.is_empty() {
            let decoded = crate::bencode::Bencode::decode(announce_response)?;
            response.status_message =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "failure reason")
                    .unwrap_or_default();
            if !response.status_message.is_empty() {
                response.failure = true;
                return Ok(response);
            }
            response.complete =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "complete")
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(0);
            response.incomplete =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "incomplete")
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(0);
            if let Some(field) =
                crate::bencode::Bencode::get_dictionary_entry(&decoded, "peers".as_bytes())
            {
                match field {
                    crate::bencode::BNode::String(bytes) => {
                        response.peer_list = tracker.get_compact_peer_list(bytes, 0);
                    }
                    crate::bencode::BNode::List(list) => {
                        for item in list {
                            if let crate::bencode::BNode::Dictionary(_peer_dict) = item {
                                let mut peer = PeerDetails {
                                    info_hash: tracker.info_hash.clone(),
                                    peer_id: None,
                                    ip: String::new(),
                                    port: 0,
                                };
                                if let Some(peer_field) =
                                    crate::bencode::Bencode::get_dictionary_entry(
                                        item,
                                        "ip".as_bytes(),
                                    )
                                {
                                    if let Some(ip_bytes) = peer_field.as_string() {
                                        peer.ip = String::from_utf8_lossy(ip_bytes).to_string();
                                    }
                                }
                                if peer.ip.contains(":") {
                                    if let Some((_, tail)) = peer.ip.rsplit_once(":") {
                                        peer.ip = tail.to_string();
                                    }
                                }
                                if let Some(peer_field) =
                                    crate::bencode::Bencode::get_dictionary_entry(
                                        item,
                                        "port".as_bytes(),
                                    )
                                {
                                    if let Some(port_bytes) = peer_field.as_number_bytes() {
                                        peer.port = String::from_utf8_lossy(port_bytes)
                                            .parse()
                                            .unwrap_or(0);
                                    }
                                }
                                if peer.ip != tracker.ip {
                                    response.peer_list.push(peer);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            response.interval =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "interval")
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(response.interval);
            response.min_interval =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "min interval")
                    .unwrap_or_default()
                    .parse()
                    .unwrap_or(response.min_interval);
            response.tracker_id =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "tracker id");
            response.status_message =
                crate::bencode::Bencode::get_dictionary_entry_string(&decoded, "warning message")
                    .unwrap_or_default();
            response.announce_count += 1;
        }
        Ok(response)
    }

    /// Constructs the full HTTP request URL including the query parameters for the tracker announce.
    fn build_announce_url(&self, tracker: &crate::tracker::TrackerAnnounceContext) -> String {
        let info_hash = encode_binary(&tracker.info_hash);
        let peer_id = encode(&tracker.peer_id);
        let mut announce_url = format!(
            "{}?info_hash={}&peer_id={}&port={}&compact={}&no_peer_id={}&uploaded={}&downloaded={}&left={}&ip={}&key={}&numwant={}",
            tracker.tracker_url,
            info_hash,
            peer_id,
            tracker.port,
            if tracker.compact { 1 } else { 0 },
            if tracker.no_peer_id { 1 } else { 0 },
            tracker.uploaded,
            tracker.downloaded,
            tracker.left,
            encode(&tracker.ip),
            encode(tracker.key.as_deref().unwrap_or("")),
            tracker.num_wanted,
        );
        if tracker.event != TrackerEvent::None {
            announce_url.push_str(&format!("&event={}", tracker.event.as_str()));
        }
        if let Some(tracker_id) = &tracker.tracker_id {
            if !tracker_id.is_empty() {
                announce_url.push_str(&format!("&trackerid={}", encode(tracker_id)));
            }
        }
        announce_url
    }
}

impl Announcer for HttpAnnouncer {
    /// Executes the HTTP GET request to the tracker and decodes the response.
    fn announce(
        &mut self,
        tracker: &crate::tracker::TrackerAnnounceContext,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        Tracker::log_announce(tracker);
        let url = self.build_announce_url(tracker);
        let response = ureq::get(&url).call().map_err(|err| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                err.to_string(),
            ))
        })?;
        let mut body = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|err| {
                BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err.to_string(),
                ))
            })?;
        Self::decode_announce_response(tracker, &body)
    }
}

/// An announcer that uses the UDP protocol to communicate with trackers.
pub struct UdpAnnouncer {
    host_port: String,
    socket: Option<UdpSocket>,
    connected: bool,
    connection_id: u64,
}

impl UdpAnnouncer {
    /// Resolves the host name and port from the UDP tracker URL and creates a `UdpAnnouncer`.
    pub fn new(url: &str) -> Result<Self, BitTorrentError> {
        let parsed = url::Url::parse(url).map_err(|err| BitTorrentError::Parse(err.to_string()))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| BitTorrentError::Parse("Invalid UDP tracker URL".to_string()))?;
        let port = parsed
            .port_or_known_default()
            .ok_or_else(|| BitTorrentError::Parse("UDP tracker port missing".to_string()))?;
        Ok(UdpAnnouncer {
            host_port: format!("{}:{}", host, port),
            socket: None,
            connected: false,
            connection_id: 0,
        })
    }

    /// Instantiates and binds the `UdpSocket` if it hasn't been created yet.
    fn ensure_socket(&mut self) -> Result<(), BitTorrentError> {
        if self.socket.is_some() {
            return Ok(());
        }
        let mut addrs = self
            .host_port
            .to_socket_addrs()
            .map_err(|err| BitTorrentError::Parse(err.to_string()))?;
        let remote_addr = addrs
            .next()
            .ok_or_else(|| BitTorrentError::Parse("Failed to resolve tracker host".to_string()))?;
        let bind_addr = if remote_addr.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        let socket = UdpSocket::bind(bind_addr).map_err(BitTorrentError::Io)?;
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .map_err(BitTorrentError::Io)?;
        socket.connect(remote_addr).map_err(BitTorrentError::Io)?;
        self.socket = Some(socket);
        Ok(())
    }

    /// Sends a command payload over the UDP socket and waits for a response (supporting up to 1 retry on timeout).
    fn send_command(&self, command: &[u8]) -> Result<Vec<u8>, BitTorrentError> {
        let socket = self
            .socket
            .as_ref()
            .ok_or_else(|| BitTorrentError::Parse("UDP socket not initialised".to_string()))?;
        let mut retries = 0;
        loop {
            socket.send(command).map_err(BitTorrentError::Io)?;
            let mut buf = vec![0u8; 1500];
            match socket.recv(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    return Ok(buf);
                }
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut
                    {
                        if retries < 1 {
                            retries += 1;
                            continue;
                        }
                    }
                    return Err(BitTorrentError::Io(err));
                }
            }
        }
    }

    /// Builds the 16-byte UDP connect packet.
    fn build_connect_packet(&self, transaction_id: u32) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend_from_slice(&pack_u64(0x41727101980));
        packet.extend_from_slice(&pack_u32(0));
        packet.extend_from_slice(&pack_u32(transaction_id));
        packet
    }

    /// Builds the 98-byte UDP announce packet.
    fn build_announce_packet(
        &self,
        tracker: &crate::tracker::TrackerAnnounceContext,
        transaction_id: u32,
    ) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend_from_slice(&pack_u64(self.connection_id));
        packet.extend_from_slice(&pack_u32(1));
        packet.extend_from_slice(&pack_u32(transaction_id));
        packet.extend_from_slice(&tracker.info_hash);
        packet.extend_from_slice(tracker.peer_id.as_bytes());
        packet.extend_from_slice(&pack_u64(tracker.downloaded));
        packet.extend_from_slice(&pack_u64(tracker.left));
        packet.extend_from_slice(&pack_u64(tracker.uploaded));
        packet.extend_from_slice(&pack_u32(tracker.event as u32));
        packet.extend_from_slice(&pack_u32(0));
        packet.extend_from_slice(&pack_u32(0));
        packet.extend_from_slice(&pack_u32(tracker.num_wanted as u32));
        packet.extend_from_slice(&pack_u32(tracker.port as u32));
        packet.extend_from_slice(&pack_u32(0));
        packet
    }

    /// Performs the initial UDP connection handshake with the tracker to acquire a connection ID.
    fn connect(&mut self) -> Result<(), BitTorrentError> {
        let transaction_id: u32 = rand::random();
        let reply = self.send_command(&self.build_connect_packet(transaction_id))?;
        if transaction_id != unpack_u32(&reply, 4) {
            return Err(BitTorrentError::Parse(
                "UDP tracker transaction ID mismatch".into(),
            ));
        }
        if unpack_u32(&reply, 0) == 0 {
            self.connection_id = unpack_u64(&reply, 8);
            self.connected = true;
            Ok(())
        } else if unpack_u32(&reply, 0) == 3 {
            let message = String::from_utf8_lossy(&reply[8..]).into_owned();
            Err(BitTorrentError::Parse(format!(
                "UDP connect error: {}",
                message
            )))
        } else {
            Err(BitTorrentError::Parse(
                "Invalid UDP connect response".into(),
            ))
        }
    }
}

impl Announcer for UdpAnnouncer {
    /// Executes the UDP announce process by ensuring connection, transmitting announce command, and parsing the response.
    fn announce(
        &mut self,
        tracker: &crate::tracker::TrackerAnnounceContext,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        Tracker::log_announce(tracker);
        self.ensure_socket()?;
        if !self.connected {
            if let Err(e) = self.connect() {
                self.socket = None;
                return Err(e);
            }
        }
        let transaction_id: u32 = rand::random();
        let reply = match self.send_command(&self.build_announce_packet(tracker, transaction_id)) {
            Ok(r) => r,
            Err(e) => {
                self.socket = None;
                self.connected = false;
                return Err(e);
            }
        };
        if transaction_id != unpack_u32(&reply, 4) {
            return Err(BitTorrentError::Parse(
                "UDP announce transaction ID mismatch".into(),
            ));
        }
        let mut response = AnnounceResponse::default();
        let action = unpack_u32(&reply, 0);
        if action == 1 {
            response.interval = unpack_u32(&reply, 8) as usize;
            response.incomplete = unpack_u32(&reply, 12) as usize;
            response.complete = unpack_u32(&reply, 16) as usize;
            response.peer_list = tracker.get_compact_peer_list(&reply, 20);
        } else if action == 3 {
            response.failure = true;
            response.status_message = String::from_utf8_lossy(&reply[8..]).into_owned();
        } else {
            return Err(BitTorrentError::Parse(
                "Invalid UDP announce response action".into(),
            ));
        }
        Ok(response)
    }
}

/// Factory helper to build `Announcer` trait objects dynamically based on the tracker URL protocol schema.
pub struct AnnouncerFactory;

impl AnnouncerFactory {
    /// Instantiates the appropriate `Announcer` concrete implementation (`HttpAnnouncer` or `UdpAnnouncer`) for a given URL.
    pub fn create(url: &str) -> Result<Box<dyn Announcer>, BitTorrentError> {
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(Box::new(HttpAnnouncer::new()))
        } else if url.starts_with("udp://") {
            Ok(Box::new(UdpAnnouncer::new(url)?))
        } else {
            Err(BitTorrentError::Parse("Invalid tracker URL".into()))
        }
    }
}
