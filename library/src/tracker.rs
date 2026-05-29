use crate::announcer::{Announcer, AnnouncerFactory};
use crate::error::BitTorrentError;
use crate::host;
use crate::peer_id;
use crate::torrent_context::TorrentContext;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerEvent {
    None,
    Started,
    Stopped,
    Completed,
}

impl TrackerEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackerEvent::None => "",
            TrackerEvent::Started => "started",
            TrackerEvent::Stopped => "stopped",
            TrackerEvent::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerStatus {
    Running,
    Stopped,
    Stalled,
}

#[derive(Debug, Clone)]
pub struct PeerDetails {
    pub info_hash: Vec<u8>,
    pub peer_id: Option<String>,
    pub ip: String,
    pub port: u16,
}

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

pub type TrackerCallback = Arc<dyn Fn(&TrackerAnnounceContext) + Send + Sync>;

pub struct Tracker {
    tc: Arc<Mutex<TorrentContext>>,
    announcer: Box<dyn Announcer>,
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
    pub interval: usize,
    pub min_interval: usize,
    pub tracker_status: TrackerStatus,
    pub event: TrackerEvent,
    pub callback: Option<TrackerCallback>,
    pub peer_swarm_queue: Option<Sender<PeerDetails>>,
    pub last_response: AnnounceResponse,
}

impl Tracker {
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

    pub fn new(tc: Arc<Mutex<TorrentContext>>) -> Result<Self, BitTorrentError> {
        let guard = tc.lock().unwrap();
        let info_hash = guard.info_hash.clone();
        let tracker_url = guard.tracker_url.clone();
        let announcer = AnnouncerFactory::create(&tracker_url)?;
        Ok(Tracker {
            tc: tc.clone(),
            announcer,
            peer_id: peer_id::get(),
            port: 6881,
            ip: host::get_ip(),
            compact: true,
            no_peer_id: false,
            key: None,
            tracker_id: None,
            num_wanted: 5,
            info_hash,
            tracker_url,
            interval: 2000,
            min_interval: 0,
            tracker_status: TrackerStatus::Stopped,
            event: TrackerEvent::None,
            callback: None,
            peer_swarm_queue: None,
            last_response: AnnounceResponse::default(),
        })
    }

    pub fn set_peer_swarm_queue(&mut self, sender: Sender<PeerDetails>) {
        self.peer_swarm_queue = Some(sender);
    }

    pub fn downloaded(&self) -> u64 {
        self.tc.lock().unwrap().total_bytes_downloaded
    }

    pub fn uploaded(&self) -> u64 {
        self.tc.lock().unwrap().total_bytes_uploaded
    }

    pub fn left(&self) -> u64 {
        self.tc
            .lock()
            .unwrap()
            .bytes_left_to_download()
            .unwrap_or(0)
    }

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

    pub fn announce_once(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        if self.tracker_status == TrackerStatus::Stalled {
            return Err(BitTorrentError::Parse("Tracker is stalled".to_string()));
        }
        let announce_context = self.build_announce_context();
        let response = self.announcer.announce(&announce_context)?;
        if !response.failure {
            self.update_running_status_from_announce(&response);
            self.queue_new_peers(&response);
        }
        self.last_response = response.clone();
        Ok(response)
    }

    fn queue_new_peers(&self, response: &AnnounceResponse) {
        if let Some(sender) = &self.peer_swarm_queue {
            if self.tc.lock().unwrap().status == crate::torrent_context::TorrentStatus::Downloading
                && !response.failure
            {
                for peer_details in &response.peer_list {
                    let _ = sender.send(peer_details.clone());
                }
            }
        }
    }

    fn update_running_status_from_announce(&mut self, response: &AnnounceResponse) {
        self.tracker_id = response.tracker_id.clone();
        self.min_interval = response.min_interval;
        if self.tc.lock().unwrap().status == crate::torrent_context::TorrentStatus::Downloading
            && response.interval > self.min_interval
        {
            self.interval = response.interval;
        }
    }

    pub fn change_status(
        &mut self,
        event: TrackerEvent,
    ) -> Result<AnnounceResponse, BitTorrentError> {
        self.event = event;
        let response = self.announce_once()?;
        self.event = TrackerEvent::None;
        if !response.failure {
            self.tracker_status = TrackerStatus::Running;
        }
        Ok(response)
    }

    pub fn start_announcing(&mut self) -> Result<AnnounceResponse, BitTorrentError> {
        if self.tracker_status == TrackerStatus::Running {
            return Err(BitTorrentError::Parse("Tracker is already running".into()));
        }
        if self.tc.lock().unwrap().bytes_left_to_download()? == 0 {
            self.tc.lock().unwrap().total_bytes_downloaded = 0;
            self.tc.lock().unwrap().total_bytes_to_download = 0;
            self.change_status(TrackerEvent::None)
        } else {
            self.change_status(TrackerEvent::Started)
        }
    }

    pub fn stop_announcing(&mut self) {
        self.tracker_status = TrackerStatus::Stopped;
    }

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
