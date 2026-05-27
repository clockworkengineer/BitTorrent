use crate::peer::Peer;
use crate::torrent_context::TorrentContext;
use crate::util::info_hash_to_string;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

pub struct Manager {
    torrents: RwLock<HashMap<String, Arc<Mutex<TorrentContext>>>>,
    dead_peers: RwLock<HashSet<String>>,
}

impl Manager {
    pub fn new() -> Self {
        Manager {
            torrents: RwLock::new(HashMap::new()),
            dead_peers: RwLock::new(HashSet::new()),
        }
    }

    pub fn get_torrent_context(&self, info_hash: &[u8]) -> Option<Arc<Mutex<TorrentContext>>> {
        let key = info_hash_to_string(info_hash);
        self.torrents.read().unwrap().get(&key).cloned()
    }

    pub fn add_torrent_context(&self, tc: Arc<Mutex<TorrentContext>>) -> bool {
        let key = info_hash_to_string(&tc.lock().unwrap().info_hash);
        self.torrents
            .write()
            .unwrap()
            .insert(key, tc)
            .is_none()
    }

    pub fn remove_torrent_context(&self, tc: &TorrentContext) -> bool {
        let key = info_hash_to_string(&tc.info_hash);
        self.torrents.write().unwrap().remove(&key).is_some()
    }

    pub fn get_peer(&self, info_hash: &[u8], ip: &str) -> Option<Arc<Mutex<Peer>>> {
        let tc = self.get_torrent_context(info_hash)?;
        tc.lock().unwrap().peer_swarm.read().unwrap().get(ip).cloned()
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
}
