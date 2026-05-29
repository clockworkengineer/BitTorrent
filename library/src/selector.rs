use crate::peer::Peer;
use crate::torrent_context::TorrentContext;
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;
use std::collections::HashSet;

#[derive(Debug)]
pub struct Selector {
    random_seed: StdRng,
}

impl Default for Selector {
    fn default() -> Self {
        Selector::new()
    }
}

impl Selector {
    pub fn new() -> Self {
        Selector {
            random_seed: StdRng::from_entropy(),
        }
    }

    pub fn next_piece(&mut self, tc: &TorrentContext) -> Option<u32> {
        if tc.number_of_pieces == 0 {
            return None;
        }
        let start_piece = self.random_seed.gen_range(0..tc.number_of_pieces as u32);
        let (suggested, piece_number) = tc.find_next_missing_piece(start_piece);
        if suggested { Some(piece_number) } else { None }
    }

    pub fn local_piece_suggestions(
        &mut self,
        remote_peer: &Peer,
        number_of_suggestions: usize,
    ) -> Vec<u32> {
        let mut suggestions = HashSet::new();
        let tc_guard = remote_peer.tc.as_ref().map(|tc| tc.lock().unwrap());
        let number_of_pieces = tc_guard.as_ref().map_or(0, |tc| tc.number_of_pieces as u32);
        if number_of_pieces == 0 {
            return Vec::new();
        }
        let start_piece = self.random_seed.gen_range(0..number_of_pieces);
        let max_suggestions = std::cmp::min(
            remote_peer.number_of_missing_pieces as usize,
            number_of_suggestions,
        );
        if max_suggestions == 0 {
            return Vec::new();
        }
        let mut current_piece = start_piece;
        while suggestions.len() < max_suggestions {
            if !remote_peer.is_piece_on_remote_peer(current_piece)
                && tc_guard
                    .as_ref()
                    .map_or(false, |tc| tc.is_piece_local(current_piece))
                && !suggestions.contains(&current_piece)
            {
                suggestions.insert(current_piece);
            }
            current_piece = (current_piece + 1) % number_of_pieces;
            if current_piece == start_piece {
                break;
            }
        }
        suggestions.into_iter().collect()
    }

    pub fn get_list_of_peers(
        &self,
        tc: &TorrentContext,
        piece_number: u32,
        max_peers: usize,
    ) -> Vec<String> {
        let mut peers: Vec<(i64, String)> = tc
            .peer_swarm
            .read()
            .unwrap()
            .iter()
            .filter_map(|(ip, peer)| {
                let peer = peer.lock().unwrap();
                if peer.connected
                    && peer.peer_choking.wait_one(0)
                    && peer.is_piece_on_remote_peer(piece_number)
                {
                    Some((peer.average_packet_response.get(), ip.clone()))
                } else {
                    None
                }
            })
            .collect();
        peers.sort_by_key(|(latency, _)| *latency);
        peers
            .into_iter()
            .take(max_peers)
            .map(|(_, ip)| ip)
            .collect()
    }
}
