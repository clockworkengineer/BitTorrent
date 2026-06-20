//! Global torrent and peer manager
//!
//! Maintains active torrent contexts, tracks dead peers to avoid reconnection loops,
//! and orchestrates peer discovery queues.

use crate::peer::Peer;
use crate::core::torrent_context::TorrentContext;
use crate::tracker::PeerDetails;
use crate::util::info_hash_to_string;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

/// Global orchestrator keeping track of active torrent contexts and network peers.
pub struct Manager {
    torrents: RwLock<HashMap<String, Arc<Mutex<TorrentContext>>>>,
    dead_peers: RwLock<HashMap<String, Instant>>,
    peer_discovery_queue: RwLock<Option<Sender<PeerDetails>>>,
    /// Maximum number of queued candidate peers (default: 1000).
    max_candidates: std::sync::atomic::AtomicUsize,
    /// Current count of candidates sent on the discovery channel.
    candidate_count: std::sync::atomic::AtomicUsize,
}

impl Manager {
    /// Creates a new `Manager` with empty torrent contexts, dead peer lists, and no discovery queue.
    pub fn new() -> Self {
        Manager {
            torrents: RwLock::new(HashMap::new()),
            dead_peers: RwLock::new(HashMap::new()),
            peer_discovery_queue: RwLock::new(None),
            max_candidates: std::sync::atomic::AtomicUsize::new(1000),
            candidate_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Sets the maximum number of peers that may be queued for discovery (default: 1000).
    pub fn set_max_candidates(&self, max: usize) {
        self.max_candidates.store(max, std::sync::atomic::Ordering::Relaxed);
        self.candidate_count.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Retrieves an active torrent context matching the provided info hash, if present.
    pub fn get_torrent_context(&self, info_hash: &[u8]) -> Option<Arc<Mutex<TorrentContext>>> {
        let key = info_hash_to_string(info_hash);
        self.torrents.read().unwrap().get(&key).cloned()
    }

    /// Adds a new torrent context to the manager's registry. Returns `true` if added successfully (i.e. did not exist).
    pub fn add_torrent_context(&self, tc: Arc<Mutex<TorrentContext>>) -> bool {
        let key = info_hash_to_string(&tc.lock().unwrap().info_hash);
        self.torrents.write().unwrap().insert(key, tc).is_none()
    }

    /// Removes a torrent context from the registry. Returns `true` if it was present.
    pub fn remove_torrent_context(&self, tc: &TorrentContext) -> bool {
        let key = info_hash_to_string(&tc.info_hash);
        self.torrents.write().unwrap().remove(&key).is_some()
    }

    /// Retrieves a peer matching the given info hash and IP address.
    pub fn get_peer(&self, info_hash: &[u8], ip: &str) -> Option<Arc<Mutex<Peer>>> {
        let tc = self.get_torrent_context(info_hash)?;
        tc.lock()
            .unwrap()
            .peer_swarm
            .read()
            .unwrap()
            .get(ip)
            .cloned()
    }

    /// Adds an IP address to the dead peer list to suppress reconnection attempts.
    pub fn add_to_dead_peer_list(&self, ip: &str) {
        self.dead_peers
            .write()
            .unwrap()
            .insert(ip.to_string(), Instant::now());
    }

    /// Removes an IP address from the dead peer list.
    pub fn remove_from_dead_peer_list(&self, ip: &str) {
        self.dead_peers.write().unwrap().remove(ip);
    }

    /// Checks if a peer IP is marked as dead.
    pub fn is_peer_dead(&self, ip: &str) -> bool {
        let mut dead_peers = self.dead_peers.write().unwrap();
        if let Some(&timestamp) = dead_peers.get(ip) {
            if timestamp.elapsed() > crate::constants::DEAD_PEER_TTL {
                dead_peers.remove(ip);
                return false;
            }
            return true;
        }
        false
    }

    /// Configures the sender channel for queueing newly discovered peers.
    pub fn set_peer_discovery_queue(&self, sender: Sender<PeerDetails>) {
        *self.peer_discovery_queue.write().unwrap() = Some(sender);
    }

    /// Clears/disables the peer discovery queue channel.
    pub fn clear_peer_discovery_queue(&self) {
        *self.peer_discovery_queue.write().unwrap() = None;
    }

    /// Pushes a newly discovered peer details block into the discovery queue, ignoring it if marked dead or the queue is full.
    pub fn queue_peer_for_discovery(&self, peer_details: PeerDetails) {
        if self.is_peer_dead(&peer_details.ip) {
            return;
        }
        if self.candidate_count.load(std::sync::atomic::Ordering::Relaxed)
            >= self.max_candidates.load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if let Some(sender) = &*self.peer_discovery_queue.read().unwrap() {
            if sender.send(peer_details).is_ok() {
                self.candidate_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Resets the candidate count (e.g., after the queue consumer drains it).
    pub fn reset_candidate_count(&self) {
        self.candidate_count.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}
