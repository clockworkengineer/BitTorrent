//! Swarm topology and connected peer tracking.

use crate::network::tracker::PeerDetails;
use crate::peer::Peer;
use crate::utils::util::LockExt;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, RwLock};

/// Manages active peer connections, swarm limits, bad peer scoring, and PEX history.
pub struct SwarmTopology {
    pub peers: RwLock<HashMap<String, Arc<Mutex<Peer>>>>,
    pub bad_peer_scores: Mutex<HashMap<String, usize>>,
    pub maximum_swarm_size: usize,
    pub unchoked_peers_count: AtomicUsize,
    pub pex_dropped_peers: Mutex<Vec<PeerDetails>>,
    pub pex_added_peers_count: AtomicUsize,
    pub pex_dropped_peers_count: AtomicUsize,
}

impl SwarmTopology {
    pub fn new(maximum_swarm_size: usize) -> Self {
        SwarmTopology {
            peers: RwLock::new(HashMap::new()),
            bad_peer_scores: Mutex::new(HashMap::new()),
            maximum_swarm_size,
            unchoked_peers_count: AtomicUsize::new(0),
            pex_dropped_peers: Mutex::new(Vec::new()),
            pex_added_peers_count: AtomicUsize::new(0),
            pex_dropped_peers_count: AtomicUsize::new(0),
        }
    }

    pub fn connected_count(&self) -> usize {
        self.peers.read().map(|s| s.len()).unwrap_or(0)
    }

    pub fn unchoked_count(&self) -> usize {
        self.unchoked_peers_count.load(Ordering::Relaxed)
    }

    pub fn contains_peer(&self, ip: &str) -> bool {
        self.peers
            .read()
            .map(|s| s.contains_key(ip))
            .unwrap_or(false)
    }

    pub fn insert_peer(&self, ip: String, peer: Arc<Mutex<Peer>>) {
        if let Ok(mut guard) = self.peers.write() {
            guard.insert(ip, peer);
        }
    }

    pub fn remove_peer(&self, ip: &str) -> Option<Arc<Mutex<Peer>>> {
        self.peers.write().ok()?.remove(ip)
    }

    pub fn record_bad_peer(&self, ip: &str) -> usize {
        let mut scores = self.bad_peer_scores.lock_safe();
        let entry = scores.entry(ip.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    pub fn is_bad_peer(&self, ip: &str, threshold: usize) -> bool {
        let scores = self.bad_peer_scores.lock_safe();
        scores.get(ip).copied().unwrap_or(0) >= threshold
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.peers.write() {
            guard.clear();
        }
        if let Ok(mut scores) = self.bad_peer_scores.lock() {
            scores.clear();
        }
    }
}
