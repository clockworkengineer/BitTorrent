//! Swarm topology, peer connections, and choking state manager (SRP & ISP).
//!
//! Encapsulates connected peer mappings, swarm capacity ceilings, reputation scoring,
//! super-seeding distribution state, and round-trip latency profiling.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

use crate::core::super_seeding::SuperSeedingState;
use crate::core::views::{PeerSnapshot, SwarmView};
use crate::network::latency_profiler::SwarmLatencyProfiler;
use crate::network::tracker::PeerDetails;
use crate::peer::Peer;
use crate::utils::util::{LockExt, RwLockExt};

/// Manages active peer connections, swarm admission limits, and peer reputation scoring.
pub struct SwarmManager {
    /// Active peer connections indexed by IP/identifier string.
    pub peer_swarm: Arc<RwLock<HashMap<String, Arc<Mutex<Peer>>>>>,
    /// Upper limit on simultaneously connected peers in this swarm.
    pub maximum_swarm_size: usize,
    /// Malicious or protocol-violating peer scores indexed by peer IP.
    pub bad_peer_scores: Mutex<HashMap<String, usize>>,
    /// BEP 16 super-seeding distribution and piece suggestion state.
    pub super_seeding: SuperSeedingState,
    /// Queue of disconnected peers ready to be advertised in outgoing PEX messages.
    pub pex_dropped_peers: Mutex<Vec<PeerDetails>>,
    /// Latency profiler tracking smoothed RTT and ECT per peer endpoint.
    pub latency_profiler: Mutex<SwarmLatencyProfiler>,
}

impl SwarmManager {
    /// Creates a new `SwarmManager` with the specified maximum swarm capacity.
    pub fn new(maximum_swarm_size: usize) -> Self {
        Self {
            peer_swarm: Arc::new(RwLock::new(HashMap::new())),
            maximum_swarm_size,
            bad_peer_scores: Mutex::new(HashMap::new()),
            super_seeding: SuperSeedingState::default(),
            pex_dropped_peers: Mutex::new(Vec::new()),
            latency_profiler: Mutex::new(SwarmLatencyProfiler::new()),
        }
    }

    /// Attempts to insert an active peer into the swarm. Returns `false` if the swarm is full.
    pub fn insert_peer(&self, ip: String, peer: Arc<Mutex<Peer>>) -> bool {
        let mut swarm = self.peer_swarm.write_safe();
        if swarm.len() >= self.maximum_swarm_size && !swarm.contains_key(&ip) {
            return false;
        }
        swarm.insert(ip, peer);
        true
    }

    /// Removes a peer from the active swarm, returning its handle if it was connected.
    pub fn remove_peer(&self, ip: &str) -> Option<Arc<Mutex<Peer>>> {
        let mut swarm = self.peer_swarm.write_safe();
        swarm.remove(ip)
    }

    /// Retrieves an `Arc` handle to a connected peer by IP or ID.
    pub fn get_peer(&self, ip: &str) -> Option<Arc<Mutex<Peer>>> {
        let swarm = self.peer_swarm.read_safe();
        swarm.get(ip).cloned()
    }

    /// Returns `true` if a peer with the given IP is currently connected.
    pub fn has_peer(&self, ip: &str) -> bool {
        let swarm = self.peer_swarm.read_safe();
        swarm.contains_key(ip)
    }

    /// Returns the number of currently connected peers in the swarm.
    pub fn connected_peers_count(&self) -> usize {
        let swarm = self.peer_swarm.read_safe();
        swarm.len()
    }

    /// Records a protocol violation score for an offending peer and returns their updated penalty count.
    pub fn record_bad_peer(&self, ip: &str) -> usize {
        let mut scores = self.bad_peer_scores.lock_safe();
        let entry = scores.entry(ip.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Checks if a peer has exceeded the bad peer infraction threshold.
    pub fn is_bad_peer(&self, ip: &str, threshold: usize) -> bool {
        let scores = self.bad_peer_scores.lock_safe();
        scores.get(ip).copied().unwrap_or(0) >= threshold
    }

    /// Clears all connected peers and reputation scores.
    pub fn clear(&self) {
        let mut swarm = self.peer_swarm.write_safe();
        swarm.clear();
        let mut scores = self.bad_peer_scores.lock_safe();
        scores.clear();
    }

    /// Enqueues a peer to be broadcasted as dropped in the next BEP 11 PEX frame.
    pub fn queue_pex_dropped(&self, peer: PeerDetails) {
        let mut guard = self.pex_dropped_peers.lock_safe();
        guard.push(peer);
    }

    /// Drains all currently queued PEX dropped peers.
    pub fn drain_pex_dropped(&self) -> Vec<PeerDetails> {
        let mut guard = self.pex_dropped_peers.lock_safe();
        std::mem::take(&mut *guard)
    }

    /// Generates point-in-time snapshots of all currently connected peers.
    pub fn peer_snapshots(&self) -> Vec<PeerSnapshot> {
        let swarm = self.peer_swarm.read_safe();
        let mut snapshots = Vec::with_capacity(swarm.len());
        for (ip, peer_arc) in swarm.iter() {
            let p = peer_arc.lock_safe();
            snapshots.push(PeerSnapshot {
                ip: ip.clone(),
                port: p.port,
                am_choking: p.am_choking,
                am_interested: p.am_interested,
                peer_choking: p.peer_choking.wait_one(0),
                peer_interested: p.peer_interested,
                download_rate: p.rolling_download_rate,
                upload_rate: p.rolling_upload_rate,
            });
        }
        snapshots
    }
}

impl SwarmView for SwarmManager {
    fn connected_peers_count(&self) -> usize {
        self.connected_peers_count()
    }

    fn unchoked_peers_count(&self) -> usize {
        let swarm = self.peer_swarm.read_safe();
        let mut count = 0;
        for peer_arc in swarm.values() {
            let p = peer_arc.lock_safe();
            if !p.peer_choking.wait_one(0) {
                count += 1;
            }
        }
        count
    }

    fn for_each_peer(&self, f: &mut dyn FnMut(&str, &PeerSnapshot)) {
        let snapshots = self.peer_snapshots();
        for snapshot in &snapshots {
            f(&snapshot.ip, snapshot);
        }
    }
}
