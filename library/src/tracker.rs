//! Tracker communication and status management
//!
//! Handles announcements to HTTP and UDP trackers, parsing responses, and maintaining
//! state information like upload/download statistics and discovered peers.

use crate::announcer::{Announcer, AnnouncerEnum, AnnouncerFactory};
use crate::error::BitTorrentError;
use crate::host;
use crate::manager::Manager;
use crate::peer_id;
use crate::torrent_context::TorrentContext;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Events sent to a tracker to indicate state changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerEvent {
    None,
    Started,
    Stopped,
    Completed,
}

impl TrackerEvent {
    /// Formats the tracker event as a string slice matching the protocol specification.
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackerEvent::None => "",
            TrackerEvent::Started => "started",
            TrackerEvent::Stopped => "stopped",
            TrackerEvent::Completed => "completed",
        }
    }
}

/// The connection status of the tracker client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerStatus {
    Running,
    Stopped,
    Stalled,
}

/// Contains contact information and identifiers for a remote peer.
#[derive(Debug, Clone)]
pub struct PeerDetails {
    pub info_hash: Vec<u8>,
    pub peer_id: Option<String>,
    pub ip: String,
    pub port: u16,
}

/// Structured response received from a tracker announce request.
#[derive(Debug, Clone)]
pub struct AnnounceResponse {
    pub announce_count: usize,
    pub failure: bool,
    pub status_message: String,
    pub interval: usize,
    pub min_interval: usize,
    pub tracker_id: Option<String>,
    pub complete: usize,
    pub incomplete: usize,
    pub peer_list: Vec<PeerDetails>,
}

impl Default for AnnounceResponse {
    /// Generates default announce response values with a standard 2000-second interval fallback.
    fn default() -> Self {
        AnnounceResponse {
            announce_count: 0,
            failure: false,
            status_message: String::new(),
            interval: 2000,
            min_interval: 0,
            tracker_id: None,
            complete: 0,
            incomplete: 0,
            peer_list: Vec::new(),
        }
    }
}

/// Request context containing all variables needed to perform a tracker announce.
pub struct TrackerAnnounceContext {
    pub info_hash: Vec<u8>,
    pub peer_id: String,
    pub port: u16,
    pub ip: String,
    pub compact: bool,
    pub no_peer_id: bool,
    pub key: Option<String>,
    pub tracker_id: Option<String>,
    pub num_wanted: usize,
    pub tracker_url: String,
    pub event: TrackerEvent,
    pub interval: usize,
    pub min_interval: usize,
    pub downloaded: u64,
    pub uploaded: u64,
    pub left: u64,
}

impl TrackerAnnounceContext {
    /// Extracts peer details from a compact-format peer byte list starting from a given offset.
    pub fn get_compact_peer_list(&self, peers: &[u8], offset: usize) -> Vec<PeerDetails> {
        let mut peer_list = Vec::new();
        let mut num = offset;
        while num + 6 <= peers.len() {
            let ip = format!(
                "{}.{}.{}.{}",
                peers[num],
                peers[num + 1],
                peers[num + 2],
                peers[num + 3]
            );
            let port = ((peers[num + 4] as u16) << 8) | peers[num + 5] as u16;
            if ip != self.ip {
                peer_list.push(PeerDetails {
                    info_hash: self.info_hash.clone(),
                    peer_id: None,
                    ip: ip.clone(),
                    port,
                });
            }
            num += 6;
        }
        peer_list
    }
}

/// Manages periodic communication with the tracker to announce status and retrieve peer swarms.
pub struct Tracker {
    tc: Arc<Mutex<TorrentContext>>,
    announcer: AnnouncerEnum,
    pub total_bytes_downloaded: Arc<std::sync::atomic::AtomicU64>,
    pub total_bytes_uploaded: Arc<std::sync::atomic::AtomicU64>,
    pub initial_bytes_downloaded: u64,
    pub total_bytes_to_download: u64,
    pub peer_id: String,
    pub port: u16,
    pub ip: String,
    pub compact: bool,
    pub no_peer_id: bool,
    pub key: Option<String>,
    pub tracker_id: Option<String>,
    pub num_wanted: usize,
    pub info_hash: Vec<u8>,
    pub tracker_url: String,
    pub tracker_urls: Vec<String>,
    pub interval: usize,
    pub min_interval: usize,
    pub tracker_status: TrackerStatus,
    pub event: TrackerEvent,
    pub peer_swarm_queue: Option<Sender<PeerDetails>>,
    pub peer_manager: Option<Arc<Manager>>,
    pub last_response: AnnounceResponse,
}

impl Tracker {
    /// Outputs debug log data detailing an outgoing announce request structure.
    pub fn log_announce(tracker: &TrackerAnnounceContext) {
        let info_hash = urlencoding::encode_binary(&tracker.info_hash);
        println!(
            "Announce: info_hash={} peer_id={} port={} compact={} no_peer_id={} uploaded={} downloaded={} left={} event={} ip={} key={:?} trackerid={:?} numwanted={}",
            info_hash,
            tracker.peer_id,
            tracker.port,
            if tracker.compact { 1 } else { 0 },
            if tracker.no_peer_id { 1 } else { 0 },
            tracker.uploaded,
            tracker.downloaded,
            tracker.left,
            tracker.event.as_str(),
            tracker.ip,
            tracker.key.as_deref().unwrap_or(""),
            tracker.tracker_id.as_deref().unwrap_or(""),
            tracker.num_wanted,
        );
    }



    /// Creates and configures a new `Tracker` manager by mapping urls, selecting primary announcer protocol handlers, and initializing defaults.
    pub fn new(tc: Arc<Mutex<TorrentContext>>) -> Result<Self, BitTorrentError> {
        let guard = tc.lock().unwrap();
        let info_hash = guard.info_hash.clone();
        let tracker_url = guard.tracker_url.clone();
        let tracker_urls = guard.tracker_urls.clone();
        let (announcer, active_url) = {
            let mut result =
                AnnouncerFactory::create(&tracker_url).map(|a| (a, tracker_url.clone()));
            if result.is_err() {
                for url in &tracker_urls {
                    if url == &tracker_url {
                        continue;
                    }
                    if let Ok(a) = AnnouncerFactory::create(url) {
                        result = Ok((a, url.clone()));
                        break;
                    }
                }
            }
            result?
        };
        let total_bytes_downloaded = guard.total_bytes_downloaded.clone();
        let total_bytes_uploaded = guard.total_bytes_uploaded.clone();
        let initial_bytes_downloaded = guard.initial_bytes_downloaded;
        let total_bytes_to_download = guard.total_bytes_to_download;
        Ok(Tracker {
            tc: tc.clone(),
            announcer,
            total_bytes_downloaded,
            total_bytes_uploaded,
            initial_bytes_downloaded,
            total_bytes_to_download,
            peer_id: peer_id::get(),
            port: 6881,
            ip: host::get_ip(),
            compact: true,
            no_peer_id: false,
            key: None,
            tracker_id: None,
            num_wanted: 50,
            info_hash,
            tracker_url: active_url,
            tracker_urls,
            interval: 2000,
            min_interval: 0,
            tracker_status: TrackerStatus::Stopped,
            event: TrackerEvent::None,
            peer_swarm_queue: None,
            peer_manager: None,
            last_response: AnnounceResponse::default(),
        })
    }

    /// Creates a new `Tracker` utilizing a pre-constructed custom announcer handler.
    pub fn new_with_announcer(
        tc: Arc<Mutex<TorrentContext>>,
        announcer: Box<dyn Announcer>,
    ) -> Result<Self, BitTorrentError> {
        let guard = tc.lock().unwrap();
        let info_hash = guard.info_hash.clone();
        let tracker_url = guard.tracker_url.clone();
        let tracker_urls = guard.tracker_urls.clone();
        let total_bytes_downloaded = guard.total_bytes_downloaded.clone();
        let total_bytes_uploaded = guard.total_bytes_uploaded.clone();
        let initial_bytes_downloaded = guard.initial_bytes_downloaded;
        let total_bytes_to_download = guard.total_bytes_to_download;
        Ok(Tracker {
            tc: tc.clone(),
            announcer: AnnouncerEnum::Custom(announcer),
            total_bytes_downloaded,
            total_bytes_uploaded,
            initial_bytes_downloaded,
            total_bytes_to_download,
            peer_id: peer_id::get(),
            port: 6881,
            ip: host::get_ip(),
            compact: true,
            no_peer_id: false,
            key: None,
            tracker_id: None,
            num_wanted: 50,
            info_hash,
            tracker_url,
            tracker_urls,
            interval: 2000,
            min_interval: 0,
            tracker_status: TrackerStatus::Stopped,
            event: TrackerEvent::None,
            peer_swarm_queue: None,
            peer_manager: None,
            last_response: AnnounceResponse::default(),
        })
    }

    /// Sets the target sender channel for newly discovered peer details blocks.
    pub fn set_peer_swarm_queue(&mut self, sender: Sender<PeerDetails>) {
        self.peer_swarm_queue = Some(sender);
    }

    /// Links a global `Manager` to dispatch discovered peers.
    pub fn set_peer_manager(&mut self, manager: Arc<Manager>) {
        self.peer_manager = Some(manager);
    }

    /// Retrieves downloaded byte statistics cached in the torrent context.
    pub fn downloaded(&self) -> u64 {
        self.total_bytes_downloaded
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_sub(self.initial_bytes_downloaded)
    }

    /// Retrieves uploaded byte statistics cached in the torrent context.
    pub fn uploaded(&self) -> u64 {
        self.total_bytes_uploaded
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Computes bytes remaining to download in the torrent.
    pub fn left(&self) -> u64 {
        let downloaded = self.total_bytes_downloaded
            .load(std::sync::atomic::Ordering::Relaxed);
        self.total_bytes_to_download.saturating_sub(downloaded)
    }

    /// Constructs the parameters object needed to make an announce request.
    pub fn build_announce_context(&self) -> TrackerAnnounceContext {
        TrackerAnnounceContext {
            info_hash: self.info_hash.clone(),
            peer_id: self.peer_id.clone(),
            port: self.port,
            ip: self.ip.clone(),
            compact: self.compact,
            no_peer_id: self.no_peer_id,
            key: self.key.clone(),
            tracker_id: self.tracker_id.clone(),
            num_wanted: self.num_wanted,
            tracker_url: self.tracker_url.clone(),
            event: self.event,
            interval: self.interval,
            min_interval: self.min_interval,
            downloaded: self.downloaded(),
            uploaded: self.uploaded(),
            left: self.left(),
        }
    }

    /// Performs the announce request, falling back to alternate URLs if the primary fails.
    fn announce_with_fallback(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        let mut last_error = None;
        let original_tracker_url = self.tracker_url.clone();
        for tracker_url in &self.tracker_urls.clone() {
            self.tracker_url = tracker_url.clone();
            if tracker_url != &original_tracker_url {
                match AnnouncerFactory::create(tracker_url) {
                    Ok(a) => self.announcer = a,
                    Err(_) => continue,
                }
            }
            let announce_context = self.build_announce_context();
            let response = self.announcer.announce(&announce_context);
            match response {
                Ok(response) => {
                    if response.failure {
                        last_error = Some(BitTorrentError::Parse(response.status_message.clone()));
                        continue;
                    }
                    return Ok(response);
                }
                Err(err) => {
                    last_error = Some(err);
                    continue;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            BitTorrentError::Parse("No tracker URLs could be reached.".to_string())
        }))
    }

    /// Triggers a single tracker announcement and queues returned peer addresses.
    pub fn announce_once(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        if self.tracker_status == TrackerStatus::Stalled {
            return Err(BitTorrentError::Parse("Tracker is stalled".to_string()));
        }
        let response = self.announce_with_fallback()?;
        if !response.failure {
            self.update_running_status_from_announce(&response);
            self.queue_new_peers(&response);
        }
        self.last_response = response.clone();
        Ok(response)
    }

    /// Returns a copy of the list of peers received in the last announce response.
    pub fn last_peer_list(&self) -> Vec<PeerDetails> {
        self.last_response.peer_list.clone()
    }

    /// Pushes list of newly discovered peer connections into discovery manager/queues.
    fn queue_new_peers(&self, response: &AnnounceResponse) {
        if response.failure {
            return;
        }

        if self.tc.lock().unwrap().status != crate::torrent_context::TorrentStatus::Downloading {
            return;
        }

        if let Some(sender) = &self.peer_swarm_queue {
            for peer_details in &response.peer_list {
                let _ = sender.send(peer_details.clone());
            }
            return;
        }

        if let Some(manager) = &self.peer_manager {
            for peer_details in &response.peer_list {
                manager.queue_peer_for_discovery(peer_details.clone());
            }
        }
    }

    /// Updates local interval, tracker IDs, and tracking limits based on the announce response.
    fn update_running_status_from_announce(&mut self, response: &AnnounceResponse) {
        self.tracker_id = response.tracker_id.clone();
        self.min_interval = response.min_interval;
        if self.tc.lock().unwrap().status == crate::torrent_context::TorrentStatus::Downloading
            && response.interval > self.min_interval
        {
            self.interval = response.interval;
        }
    }

    /// Modifies the tracker announce status and updates internal running state.
    pub fn change_status(
        &mut self,
        event: TrackerEvent,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        self.event = event;
        let response = self.announce_once()?;
        self.event = TrackerEvent::None;
        if !response.failure {
            if event == TrackerEvent::Stopped {
                self.tracker_status = TrackerStatus::Stopped;
            } else {
                self.tracker_status = TrackerStatus::Running;
            }
        }
        Ok(response)
    }

    /// Sends a 'started' event request to the tracker.
    pub fn announce_started(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        self.change_status(TrackerEvent::Started)
    }

    /// Sends a 'completed' event request to the tracker.
    pub fn announce_completed(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        self.change_status(TrackerEvent::Completed)
    }

    /// Sends a 'stopped' event request to the tracker.
    pub fn announce_stopped(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        self.change_status(TrackerEvent::Stopped)
    }

    /// Commences periodic tracker announcements, choosing started or completed events based on download progress.
    pub fn start_announcing(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        if self.tracker_status == TrackerStatus::Running {
            return Err(BitTorrentError::Parse("Tracker is already running".into()));
        }
        if self.tc.lock().unwrap().bytes_left_to_download()? == 0 {
            self.announce_completed()
        } else {
            self.announce_started()
        }
    }

    /// Ceases periodic tracker announcements by sending a stopped event.
    pub fn stop_announcing(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        self.announce_stopped()
    }

    /// Adjusts the announcement interval when the client enters seeding state.
    pub fn set_seeding_interval(&mut self, seeding_interval: usize) -> Result<(), BitTorrentError> {
        if self.tc.lock().unwrap().status == crate::torrent_context::TorrentStatus::Seeding {
            if seeding_interval > self.min_interval {
                self.interval = seeding_interval;
            }
            Ok(())
        } else {
            Err(BitTorrentError::Parse(
                "Cannot change interval as torrent is not seeding.".into(),
            ))
        }
    }
}
