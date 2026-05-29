use crate::peer::Peer;
use crate::torrent_context::TorrentContext;
use crate::tracker::PeerDetails;
use crate::util::info_hash_to_string;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock};

pub struct Manager {
    torrents: RwLock<HashMap<String, Arc<Mutex<TorrentContext>>>>,
    dead_peers: RwLock<HashSet<String>>,
    peer_discovery_queue: RwLock<Option<Sender<PeerDetails>>>,
}

impl Manager {
    pub fn new() -> Self {
        Manager {
            torrents: RwLock::new(HashMap::new()),
            dead_peers: RwLock::new(HashSet::new()),
            peer_discovery_queue: RwLock::new(None),
        }
    }

    pub fn get_torrent_context(&self, info_hash: &[u8]) -> Option<Arc<Mutex<TorrentContext>>> {
        let key = info_hash_to_string(info_hash);
        self.torrents.read().unwrap().get(&key).cloned()
    }

    pub fn add_torrent_context(&self, tc: Arc<Mutex<TorrentContext>>) -> bool {
        let key = info_hash_to_string(&tc.lock().unwrap().info_hash);
        self.torrents.write().unwrap().insert(key, tc).is_none()
    }

    pub fn remove_torrent_context(&self, tc: &TorrentContext) -> bool {
        let key = info_hash_to_string(&tc.info_hash);
        self.torrents.write().unwrap().remove(&key).is_some()
    }

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

    pub fn add_to_dead_peer_list(&self, ip: &str) {
        self.dead_peers.write().unwrap().insert(ip.to_string());
    }

    pub fn remove_from_dead_peer_list(&self, ip: &str) {
        self.dead_peers.write().unwrap().remove(ip);
    }

    pub fn is_peer_dead(&self, ip: &str) -> bool {
        self.dead_peers.read().unwrap().contains(ip)
    }

    pub fn set_peer_discovery_queue(&self, sender: Sender<PeerDetails>) {
        *self.peer_discovery_queue.write().unwrap() = Some(sender);
    }

    pub fn clear_peer_discovery_queue(&self) {
        *self.peer_discovery_queue.write().unwrap() = None;
    }

    pub fn queue_peer_for_discovery(&self, peer_details: PeerDetails) {
        if self.is_peer_dead(&peer_details.ip) {
            return;
        }
        if let Some(sender) = &*self.peer_discovery_queue.read().unwrap() {
            let _ = sender.send(peer_details);
        }
    }
}
