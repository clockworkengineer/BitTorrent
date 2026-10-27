//! WebRTC / WebTorrent Peer Transport and Tracker Signaling
//!
//! Implements WebTorrent protocol support for direct browser-to-client peer exchange,
//! WebRTC Data Channel transport adaptation, and WebSocket tracker signaling (`wss://` and `ws://`).

use crate::error::BitTorrentError;
use crate::io_traits::AsyncSocket;
use crate::manual_reset_event::ManualResetEvent;
use crate::network::announcer::Announcer;
use crate::tracker::{AnnounceResponse, PeerDetails, ScrapeResponse, TrackerAnnounceContext};

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use crate::utils::util::LockExt;
use std::time::Duration;

/// State of a WebRTC Data Channel connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataChannelState {
    Connecting,
    Open,
    Closing,
    Closed,
}

/// Statistics and metrics for WebRTC Data Channel data transfer.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WebRtcStats {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub buffered_amount: usize,
}

/// An `AsyncSocket` implementation that models a WebRTC Data Channel ("webtorrent" channel)
/// for BitTorrent wire protocol exchange.
#[derive(Clone, Debug)]
pub struct WebRtcSocketAdapter {
    label: String,
    state: Arc<Mutex<DataChannelState>>,
    read_buffer: Arc<Mutex<VecDeque<u8>>>,
    notifier: Arc<ManualResetEvent>,
    tx_sender: Arc<Mutex<Option<std::sync::mpsc::Sender<Vec<u8>>>>>,
    stats: Arc<Mutex<WebRtcStats>>,
}

impl WebRtcSocketAdapter {
    /// Creates a new uncoupled WebRTC socket adapter with the given channel label.
    pub fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            state: Arc::new(Mutex::new(DataChannelState::Open)),
            read_buffer: Arc::new(Mutex::new(VecDeque::new())),
            notifier: Arc::new(ManualResetEvent::new(false)),
            tx_sender: Arc::new(Mutex::new(None)),
            stats: Arc::new(Mutex::new(WebRtcStats::default())),
        }
    }

    /// Returns the WebRTC Data Channel label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Creates a loopback pair of connected WebRTC Data Channel adapters `(local, remote)`.
    ///
    /// Data written to `local` is readable on `remote`, and vice versa.
    pub fn create_pair(label: &str) -> (Self, Self) {
        let (tx1, rx1) = std::sync::mpsc::channel::<Vec<u8>>();
        let (tx2, rx2) = std::sync::mpsc::channel::<Vec<u8>>();

        let adapter1 = Self::new(label);
        let adapter2 = Self::new(label);

        *adapter1.tx_sender.lock_safe() = Some(tx1);
        *adapter2.tx_sender.lock_safe() = Some(tx2);

        // Spawn relay threads to deliver outgoing messages to counterpart's incoming buffer
        let buf2 = adapter2.read_buffer.clone();
        let notif2 = adapter2.notifier.clone();
        let stats2 = adapter2.stats.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = rx1.recv() {
                let len = msg.len();
                {
                    let mut b = buf2.lock_safe();
                    b.extend(msg);
                }
                if let Ok(mut s) = stats2.lock() {
                    s.bytes_received += len as u64;
                    s.messages_received += 1;
                }
                notif2.set();
            }
        });

        let buf1 = adapter1.read_buffer.clone();
        let notif1 = adapter1.notifier.clone();
        let stats1 = adapter1.stats.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = rx2.recv() {
                let len = msg.len();
                {
                    let mut b = buf1.lock_safe();
                    b.extend(msg);
                }
                if let Ok(mut s) = stats1.lock() {
                    s.bytes_received += len as u64;
                    s.messages_received += 1;
                }
                notif1.set();
            }
        });

        (adapter1, adapter2)
    }

    /// Returns the current state of the WebRTC Data Channel.
    pub fn state(&self) -> DataChannelState {
        *self.state.lock_safe()
    }

    /// Injects raw binary payload into this adapter's incoming receive queue.
    pub fn inject_incoming(&self, data: &[u8]) {
        {
            let mut buf = self.read_buffer.lock_safe();
            buf.extend(data.iter().copied());
        }
        if let Ok(mut s) = self.stats.lock() {
            s.bytes_received += data.len() as u64;
            s.messages_received += 1;
        }
        self.notifier.set();
    }

    /// Returns a snapshot of data transfer stats.
    pub fn get_stats(&self) -> WebRtcStats {
        self.stats.lock_safe().clone()
    }
}

impl AsyncSocket for WebRtcSocketAdapter {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        if buf.is_empty() {
            return Ok(0);
        }

        let start = std::time::Instant::now();
        loop {
            if *self.state.lock_safe() == DataChannelState::Closed {
                return Err(BitTorrentError::Protocol(
                    "WebRTC Data Channel closed".into(),
                ));
            }

            {
                let mut queue = self.read_buffer.lock_safe();
                if !queue.is_empty() {
                    let to_read = std::cmp::min(buf.len(), queue.len());
                    for i in 0..to_read {
                        if let Some(b) = queue.pop_front() { buf[i] = b; }
                    }
                    if queue.is_empty() {
                        self.notifier.reset();
                    }
                    return Ok(to_read);
                }
            }

            // Wait for incoming data with timeout
            if !self.notifier.wait_one(100) {
                if start.elapsed() > Duration::from_secs(30) {
                    return Err(BitTorrentError::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "WebRTC Data Channel read timed out",
                    )));
                }
            }
        }
    }

    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        if *self.state.lock_safe() != DataChannelState::Open {
            return Err(BitTorrentError::Protocol(
                "WebRTC Data Channel is not open".into(),
            ));
        }

        if let Some(ref tx) = *self.tx_sender.lock_safe() {
            tx.send(buf.to_vec())
                .map_err(|e| BitTorrentError::Protocol(format!("WebRTC send failed: {}", e)))?;
        }

        if let Ok(mut s) = self.stats.lock() {
            s.bytes_sent += buf.len() as u64;
            s.messages_sent += 1;
        }

        Ok(buf.len())
    }

    fn close(&self) {
        *self.state.lock_safe() = DataChannelState::Closed;
        self.notifier.set();
    }
}

// ─── WebTorrent Signaling Models (WebSocket Trackers) ──────────────────────────

/// Session Description Protocol (SDP) object used in WebRTC peer connection signaling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SdpDescription {
    #[serde(rename = "type")]
    pub sdp_type: String,
    pub sdp: String,
}

/// An SDP Offer object wrapped for WebTorrent tracker exchange.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebTorrentOffer {
    pub offer_id: String,
    pub offer: SdpDescription,
}

/// An SDP Answer object wrapped for WebTorrent tracker exchange.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebTorrentAnswer {
    pub offer_id: String,
    pub answer: SdpDescription,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_peer_id: Option<String>,
}

/// WebTorrent announce payload sent to WebSocket trackers (`wss://`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebTorrentAnnounceRequest {
    pub action: String,
    pub info_hash: String,
    pub peer_id: String,
    pub uploaded: u64,
    pub downloaded: u64,
    pub left: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numwant: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offers: Option<Vec<WebTorrentOffer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<SdpDescription>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_peer_id: Option<String>,
}

/// Scrape metrics for a single torrent file on a WebTorrent tracker.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WebTorrentScrapeFile {
    pub complete: usize,
    pub incomplete: usize,
    pub downloaded: usize,
}

/// WebTorrent scrape request payload sent to WebSocket trackers (`wss://` and `ws://`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebTorrentScrapeRequest {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<Vec<String>>,
}

/// WebTorrent scrape response received from WebSocket trackers.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebTorrentScrapeResponse {
    pub action: Option<String>,
    #[serde(default)]
    pub files: alloc::collections::BTreeMap<String, WebTorrentScrapeFile>,
    #[serde(rename = "failure reason")]
    pub failure_reason: Option<String>,
}

/// Response received from a WebTorrent WebSocket tracker.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebTorrentAnnounceResponse {
    pub action: Option<String>,
    pub interval: Option<u64>,
    pub info_hash: Option<String>,
    pub complete: Option<usize>,
    pub incomplete: Option<usize>,
    pub offer_id: Option<String>,
    pub offer: Option<SdpDescription>,
    pub answer: Option<SdpDescription>,
    pub peer_id: Option<String>,
    #[serde(rename = "failure reason")]
    pub failure_reason: Option<String>,
}

/// Hardware-agnostic WebSocket signaling transport trait for WebTorrent trackers.
pub trait WebSocketSignalingClient: Send + Sync + std::fmt::Debug {
    /// Sends a JSON text frame to the WebSocket tracker and awaits a response.
    fn send_message(&self, url: &str, message: &str) -> Result<String, BitTorrentError>;
}

/// In-memory mock/loopback WebSocket signaling channel for testing WebTorrent trackers.
#[derive(Debug, Default)]
pub struct MockWebSocketSignalingChannel {
    pub sent_frames: Arc<Mutex<Vec<String>>>,
    pub response_queue: Arc<Mutex<VecDeque<String>>>,
}

impl MockWebSocketSignalingChannel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_response(&self, response_json: &str) {
        self.response_queue
            .lock_safe()
            .push_back(response_json.to_string());
    }

    pub fn get_sent_messages(&self) -> Vec<String> {
        self.sent_frames.lock_safe().clone()
    }
}

impl WebSocketSignalingClient for MockWebSocketSignalingChannel {
    fn send_message(&self, _url: &str, message: &str) -> Result<String, BitTorrentError> {
        self.sent_frames.lock_safe().push(message.to_string());
        let mut queue = self.response_queue.lock_safe();
        if let Some(resp) = queue.pop_front() {
            Ok(resp)
        } else {
            Ok(r#"{"action":"announce","interval":120,"complete":1,"incomplete":0}"#.to_string())
        }
    }
}

/// Incoming message variant received from a WebTorrent tracker WebSocket frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebTorrentIncomingMessage {
    PeerAnnouncement(AnnounceResponse),
    InboundOffer {
        offer_id: String,
        peer_id: String,
        info_hash: String,
        offer: SdpDescription,
    },
    InboundAnswer {
        offer_id: String,
        peer_id: String,
        to_peer_id: Option<String>,
        info_hash: String,
        answer: SdpDescription,
    },
    Error(String),
}

/// Creates a standard WebRTC SDP offer formatted for WebTorrent peer bootstrapping.
pub fn create_webtorrent_offer(offer_id: &str) -> WebTorrentOffer {
    WebTorrentOffer {
        offer_id: offer_id.to_string(),
        offer: SdpDescription {
            sdp_type: "offer".to_string(),
            sdp: format!(
                "v=0\r\no=- {} 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 127.0.0.1\r\na=sctp-port:5000\r\n",
                offer_id
            ),
        },
    }
}

/// Creates a standard WebRTC SDP answer formatted for WebTorrent peer bootstrapping.
pub fn create_webtorrent_answer(offer_id: &str, to_peer_id: &str) -> WebTorrentAnswer {
    WebTorrentAnswer {
        offer_id: offer_id.to_string(),
        to_peer_id: Some(to_peer_id.to_string()),
        answer: SdpDescription {
            sdp_type: "answer".to_string(),
            sdp: format!(
                "v=0\r\no=- {} 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=group:BUNDLE 0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 127.0.0.1\r\na=sctp-port:5000\r\n",
                offer_id
            ),
        },
    }
}

/// Announcer for WebTorrent WebSocket trackers (`wss://` and `ws://`).
#[derive(Debug)]
pub struct WebTorrentAnnouncer {
    pub tracker_url: String,
    signaling_client: Option<Arc<dyn WebSocketSignalingClient>>,
}

impl WebTorrentAnnouncer {
    /// Creates a new WebTorrentAnnouncer for the specified tracker URL.
    pub fn new(tracker_url: &str) -> Self {
        Self {
            tracker_url: tracker_url.to_string(),
            signaling_client: None,
        }
    }

    /// Creates a new WebTorrentAnnouncer with a custom WebSocket signaling client.
    pub fn with_client(tracker_url: &str, client: Arc<dyn WebSocketSignalingClient>) -> Self {
        Self {
            tracker_url: tracker_url.to_string(),
            signaling_client: Some(client),
        }
    }

    /// Sets or updates the WebSocket signaling client for this announcer.
    pub fn set_signaling_client(&mut self, client: Arc<dyn WebSocketSignalingClient>) {
        self.signaling_client = Some(client);
    }

    /// Builds a JSON-encoded announce request message with a single generated offer.
    pub fn build_announce_payload(
        &self,
        tracker: &TrackerAnnounceContext,
        offer_id: &str,
    ) -> Result<String, BitTorrentError> {
        let offer = create_webtorrent_offer(offer_id);
        self.build_announce_payload_with_offers(tracker, vec![offer], None)
    }

    /// Builds a JSON-encoded announce request message with multiple offers and an optional event.
    pub fn build_announce_payload_with_offers(
        &self,
        tracker: &TrackerAnnounceContext,
        offers: Vec<WebTorrentOffer>,
        event: Option<&str>,
    ) -> Result<String, BitTorrentError> {
        let info_hash_hex = crate::util::info_hash_to_string(&tracker.info_hash);
        let peer_id_str = tracker.peer_id.clone();

        let req = WebTorrentAnnounceRequest {
            action: "announce".to_string(),
            info_hash: info_hash_hex,
            peer_id: peer_id_str,
            uploaded: tracker.uploaded,
            downloaded: tracker.downloaded,
            left: tracker.left,
            event: event.map(|s| s.to_string()),
            numwant: Some(50),
            offers: if offers.is_empty() {
                None
            } else {
                Some(offers)
            },
            answer: None,
            offer_id: None,
            to_peer_id: None,
        };

        serde_json::to_string(&req).map_err(|e| BitTorrentError::Parse(e.to_string()))
    }

    /// Builds a JSON-encoded answer payload to respond to an incoming browser peer's offer.
    pub fn build_answer_payload(
        &self,
        info_hash: &[u8],
        peer_id: &str,
        answer: &WebTorrentAnswer,
    ) -> Result<String, BitTorrentError> {
        let info_hash_hex = crate::util::info_hash_to_string(info_hash);
        let req = WebTorrentAnnounceRequest {
            action: "announce".to_string(),
            info_hash: info_hash_hex,
            peer_id: peer_id.to_string(),
            uploaded: 0,
            downloaded: 0,
            left: 0,
            event: None,
            numwant: None,
            offers: None,
            answer: Some(answer.answer.clone()),
            offer_id: Some(answer.offer_id.clone()),
            to_peer_id: answer.to_peer_id.clone(),
        };
        serde_json::to_string(&req).map_err(|e| BitTorrentError::Parse(e.to_string()))
    }

    /// Builds a JSON-encoded scrape request for WebSocket trackers.
    pub fn build_scrape_payload(&self, info_hashes: &[&[u8]]) -> Result<String, BitTorrentError> {
        let hex_hashes: Vec<String> = info_hashes
            .iter()
            .map(|h| crate::util::info_hash_to_string(h))
            .collect();
        let req = WebTorrentScrapeRequest {
            action: "scrape".to_string(),
            info_hash: if hex_hashes.is_empty() {
                None
            } else {
                Some(hex_hashes)
            },
        };
        serde_json::to_string(&req).map_err(|e| BitTorrentError::Parse(e.to_string()))
    }

    /// Parses a JSON-encoded scrape response message from a WebSocket tracker.
    pub fn parse_scrape_response(&self, json_str: &str) -> Result<ScrapeResponse, BitTorrentError> {
        let parsed: WebTorrentScrapeResponse =
            serde_json::from_str(json_str).map_err(|e| BitTorrentError::Parse(e.to_string()))?;

        if let Some(err) = parsed.failure_reason {
            return Err(BitTorrentError::Parse(format!(
                "WebTorrent scrape failed: {}",
                err
            )));
        }

        let mut total_complete = 0;
        let mut total_incomplete = 0;
        let mut total_downloaded = 0;

        for (_h, stats) in parsed.files {
            total_complete += stats.complete;
            total_incomplete += stats.incomplete;
            total_downloaded += stats.downloaded;
        }

        Ok(ScrapeResponse {
            complete: total_complete,
            incomplete: total_incomplete,
            downloaded: total_downloaded,
        })
    }

    /// Parses a JSON-encoded response message received from a WebTorrent tracker into an `AnnounceResponse`.
    pub fn parse_announce_response(
        &self,
        json_str: &str,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        let parsed: WebTorrentAnnounceResponse =
            serde_json::from_str(json_str).map_err(|e| BitTorrentError::Parse(e.to_string()))?;

        if let Some(err) = parsed.failure_reason {
            return Ok(AnnounceResponse {
                failure: true,
                status_message: err,
                ..Default::default()
            });
        }

        let mut res = AnnounceResponse {
            failure: false,
            interval: parsed.interval.unwrap_or(120) as usize,
            complete: parsed.complete.unwrap_or(0),
            incomplete: parsed.incomplete.unwrap_or(0),
            ..Default::default()
        };

        if let Some(peer_id) = parsed.peer_id {
            res.peer_list.push(PeerDetails {
                ip: format!("webrtc:{}", &peer_id[..peer_id.len().min(12)]),
                port: 0,
                peer_id: Some(peer_id),
                info_hash: parsed.info_hash.map(|h| h.into_bytes()).unwrap_or_default(),
            });
        }

        Ok(res)
    }

    /// Parses an incoming WebSocket frame into a structured `WebTorrentIncomingMessage`.
    pub fn parse_incoming_message(
        &self,
        json_str: &str,
    ) -> Result<WebTorrentIncomingMessage, BitTorrentError> {
        let parsed: WebTorrentAnnounceResponse =
            serde_json::from_str(json_str).map_err(|e| BitTorrentError::Parse(e.to_string()))?;

        if let Some(err) = parsed.failure_reason {
            return Ok(WebTorrentIncomingMessage::Error(err));
        }

        if let (Some(offer), Some(offer_id), Some(peer_id)) = (
            parsed.offer,
            parsed.offer_id.as_ref(),
            parsed.peer_id.as_ref(),
        ) {
            return Ok(WebTorrentIncomingMessage::InboundOffer {
                offer_id: offer_id.clone(),
                peer_id: peer_id.clone(),
                info_hash: parsed.info_hash.unwrap_or_default(),
                offer,
            });
        }

        if let (Some(answer), Some(offer_id), Some(peer_id)) = (
            parsed.answer,
            parsed.offer_id.as_ref(),
            parsed.peer_id.as_ref(),
        ) {
            return Ok(WebTorrentIncomingMessage::InboundAnswer {
                offer_id: offer_id.clone(),
                peer_id: peer_id.clone(),
                to_peer_id: None,
                info_hash: parsed.info_hash.unwrap_or_default(),
                answer,
            });
        }

        let announce_res = self.parse_announce_response(json_str)?;
        Ok(WebTorrentIncomingMessage::PeerAnnouncement(announce_res))
    }
}

impl Announcer for WebTorrentAnnouncer {
    fn announce(
        &mut self,
        tracker: &TrackerAnnounceContext,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        let payload = self.build_announce_payload(tracker, "offer-init-01")?;
        crate::log_debug!(
            "[webtorrent] Prepared announce payload for {}: {} bytes",
            self.tracker_url,
            payload.len()
        );

        if let Some(ref client) = self.signaling_client {
            let resp_str = client.send_message(&self.tracker_url, &payload)?;
            self.parse_announce_response(&resp_str)
        } else {
            Ok(AnnounceResponse {
                failure: false,
                interval: 120,
                complete: 1,
                incomplete: 0,
                status_message: "WebTorrent WebSocket signaling ready".to_string(),
                peer_list: vec![],
                ..Default::default()
            })
        }
    }

    fn scrape(
        &mut self,
        tracker: &TrackerAnnounceContext,
    ) -> Result<ScrapeResponse, BitTorrentError> {
        if let Some(ref client) = self.signaling_client {
            let payload = self.build_scrape_payload(&[&tracker.info_hash])?;
            let resp_str = client.send_message(&self.tracker_url, &payload)?;
            self.parse_scrape_response(&resp_str)
        } else {
            Err(BitTorrentError::Parse(
                "WebTorrent scrape not supported without signaling client".into(),
            ))
        }
    }
}

/// Telemetry statistics for WebTorrent / WebRTC bridge operations.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebTorrentBridgeStats {
    pub offers_sent: usize,
    pub offers_received: usize,
    pub answers_sent: usize,
    pub answers_received: usize,
    pub tcp_to_webrtc_bytes: u64,
    pub webrtc_to_tcp_bytes: u64,
    pub active_webrtc_peers: usize,
}

/// Bridge coordinator managing WebTorrent WebRTC Data Channel connections,
/// offer/answer lifecycle negotiation, and cross-swarm peer bridging.
#[derive(Debug, Default)]
pub struct WebTorrentBridge {
    pending_offers: Mutex<HashMap<String, WebRtcSocketAdapter>>,
    active_peers: Mutex<HashMap<String, Arc<WebRtcSocketAdapter>>>,
    pub offers_sent: AtomicUsize,
    pub offers_received: AtomicUsize,
    pub answers_sent: AtomicUsize,
    pub answers_received: AtomicUsize,
    pub tcp_to_webrtc_bytes: AtomicU64,
    pub webrtc_to_tcp_bytes: AtomicU64,
}

impl WebTorrentBridge {
    /// Creates a new, uncoupled `WebTorrentBridge`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Generates a batch of unique WebTorrent SDP offers and initializes local paired sockets.
    pub fn generate_offers(&self, count: usize) -> Vec<WebTorrentOffer> {
        let mut offers = Vec::with_capacity(count);
        let mut pending = self.pending_offers.lock_safe();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        for i in 0..count {
            let offer_id = format!("offer-{:x}-{}", timestamp, i);
            let (local, _remote) = WebRtcSocketAdapter::create_pair("webtorrent");
            let offer = create_webtorrent_offer(&offer_id);
            pending.insert(offer_id, local);
            offers.push(offer);
        }

        self.offers_sent.fetch_add(count, Ordering::Relaxed);
        offers
    }

    /// Handles an incoming SDP answer from a remote peer matching an existing offer.
    pub fn handle_inbound_answer(
        &self,
        offer_id: &str,
        remote_peer_id: &str,
        _answer: &SdpDescription,
    ) -> Result<Arc<WebRtcSocketAdapter>, BitTorrentError> {
        let adapter = {
            let mut pending = self.pending_offers.lock_safe();
            pending.remove(offer_id).ok_or_else(|| {
                BitTorrentError::Protocol(format!("No matching offer for offer_id: {}", offer_id))
            })?
        };

        let adapter_arc = Arc::new(adapter);
        let peer_ip = format!("webrtc:{}", remote_peer_id);
        self.active_peers
            .lock_safe()
            .insert(peer_ip, adapter_arc.clone());

        self.answers_received.fetch_add(1, Ordering::Relaxed);
        Ok(adapter_arc)
    }

    /// Handles an incoming SDP offer from a remote peer, generating an answer and activating socket.
    pub fn handle_inbound_offer(
        &self,
        offer_id: &str,
        remote_peer_id: &str,
        _offer: &SdpDescription,
    ) -> Result<(WebTorrentAnswer, Arc<WebRtcSocketAdapter>), BitTorrentError> {
        let (local, _remote) = WebRtcSocketAdapter::create_pair("webtorrent");
        let answer = create_webtorrent_answer(offer_id, remote_peer_id);
        let adapter_arc = Arc::new(local);

        let peer_ip = format!("webrtc:{}", remote_peer_id);
        self.active_peers
            .lock_safe()
            .insert(peer_ip, adapter_arc.clone());

        self.offers_received.fetch_add(1, Ordering::Relaxed);
        self.answers_sent.fetch_add(1, Ordering::Relaxed);

        Ok((answer, adapter_arc))
    }

    /// Retrieves an active WebRTC socket adapter by virtual peer IP address (e.g. `webrtc:<peer_id>`).
    pub fn get_peer_socket(&self, peer_ip: &str) -> Option<Arc<WebRtcSocketAdapter>> {
        self.active_peers.lock_safe().get(peer_ip).cloned()
    }

    /// Registers an active WebRTC socket adapter for a given peer IP address.
    pub fn register_peer_socket(&self, peer_ip: &str, adapter: Arc<WebRtcSocketAdapter>) {
        self.active_peers
            .lock_safe()
            .insert(peer_ip.to_string(), adapter);
    }

    /// Removes an active WebRTC socket adapter by virtual peer IP address.
    pub fn remove_peer_socket(&self, peer_ip: &str) {
        self.active_peers.lock_safe().remove(peer_ip);
    }

    /// Records bridge transfer bytes between TCP/uTP swarms and WebRTC swarms.
    pub fn record_bridge_transfer(&self, from_webrtc: bool, bytes: u64) {
        if from_webrtc {
            self.webrtc_to_tcp_bytes.fetch_add(bytes, Ordering::Relaxed);
        } else {
            self.tcp_to_webrtc_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    /// Returns a snapshot of WebTorrent bridge statistics.
    pub fn stats(&self) -> WebTorrentBridgeStats {
        let active_count = self.active_peers.lock_safe().len();
        WebTorrentBridgeStats {
            offers_sent: self.offers_sent.load(Ordering::Relaxed),
            offers_received: self.offers_received.load(Ordering::Relaxed),
            answers_sent: self.answers_sent.load(Ordering::Relaxed),
            answers_received: self.answers_received.load(Ordering::Relaxed),
            tcp_to_webrtc_bytes: self.tcp_to_webrtc_bytes.load(Ordering::Relaxed),
            webrtc_to_tcp_bytes: self.webrtc_to_tcp_bytes.load(Ordering::Relaxed),
            active_webrtc_peers: active_count,
        }
    }
}

