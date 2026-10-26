//! Policy-based choking engine (Single Responsibility & Open/Closed Principles).
//!
//! Decouples the choking algorithm policy from asynchronous timers, channel dispatching,
//! and context synchronization. Enables independent, testable algorithm implementations
//! such as Tit-for-Tat, Round-Robin, and BitTyrant.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::core::views::{PeerSnapshot, SwarmView};
use crate::manager::Manager;
use crate::peer::{Peer, PeerAction};
use crate::session::config::ChokingStrategy;
use crate::session::worker::delay;

/// Evaluated output decisions from a choking policy evaluation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChokingDecision {
    /// Peer IPs or IDs selected for standard unchoke upload slots.
    pub unchoked_peers: Vec<String>,
    /// Peer IP or ID selected for optimistic unchoking, if any.
    pub optimistically_unchoked_peer: Option<String>,
    /// Peer IPs or IDs that should be choked.
    pub choked_peers: Vec<String>,
}

/// Algorithmic policy defining peer evaluation and upload slot allocation (SRP & OCP).
pub trait ChokingPolicy: Send + Sync + std::fmt::Debug {
    /// Evaluates connected peers in the swarm and determines choking/unchoking decisions.
    fn evaluate(
        &self,
        swarm: &dyn SwarmView,
        max_upload_slots: usize,
        is_seeding: bool,
    ) -> ChokingDecision;
}

/// Standard BitTorrent Tit-for-Tat choking policy (BEP 3).
///
/// In downloading mode: ranks interested peers by rolling download speed (reciprocity).
/// In seeding mode: ranks interested peers by rolling upload speed (speedy piece dispersal).
/// Periodically allocates an optimistic unchoke slot to explore newly connected peers.
#[derive(Debug, Default)]
pub struct StandardTitForTatPolicy {
    last_optimistic_ip: Mutex<Option<String>>,
    round_counter: AtomicUsize,
}

impl StandardTitForTatPolicy {
    /// Creates a new `StandardTitForTatPolicy`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChokingPolicy for StandardTitForTatPolicy {
    fn evaluate(
        &self,
        swarm: &dyn SwarmView,
        max_upload_slots: usize,
        is_seeding: bool,
    ) -> ChokingDecision {
        let mut interested: Vec<PeerSnapshot> = Vec::new();
        let mut disinterested: Vec<String> = Vec::new();

        swarm.for_each_peer(&mut |_ip, peer| {
            if peer.peer_interested {
                interested.push(peer.clone());
            } else {
                disinterested.push(peer.ip.clone());
            }
        });

        if interested.is_empty() {
            return ChokingDecision {
                unchoked_peers: Vec::new(),
                optimistically_unchoked_peer: None,
                choked_peers: disinterested,
            };
        }

        // Sort interested peers:
        // Seeding mode -> sort by upload rate descending (spread pieces faster).
        // Downloading mode -> sort by download rate descending (tit-for-tat reciprocal unchoking).
        interested.sort_by(|a, b| {
            if is_seeding {
                b.upload_rate
                    .partial_cmp(&a.upload_rate)
                    .unwrap_or(core::cmp::Ordering::Equal)
            } else {
                b.download_rate
                    .partial_cmp(&a.download_rate)
                    .unwrap_or(core::cmp::Ordering::Equal)
            }
        });

        let regular_slots = max_upload_slots.saturating_sub(1).min(interested.len());
        let (top_peers, remaining_interested) = interested.split_at(regular_slots);

        let unchoked_peers: Vec<String> = top_peers.iter().map(|p| p.ip.clone()).collect();

        // Optimistic unchoke selection
        let mut optimistic_peer = None;
        if !remaining_interested.is_empty() {
            let round = self.round_counter.fetch_add(1, Ordering::Relaxed);
            let rotate = (round % 3 == 0) || self.last_optimistic_ip.lock().unwrap().is_none();

            let mut last_opt = self.last_optimistic_ip.lock().unwrap();

            if !rotate {
                if let Some(ref current_ip) = *last_opt {
                    if let Some(found) = remaining_interested.iter().find(|p| p.ip == *current_ip) {
                        optimistic_peer = Some(found.ip.clone());
                    }
                }
            }

            if optimistic_peer.is_none() {
                let chosen_index = round % remaining_interested.len();
                let picked = &remaining_interested[chosen_index];
                *last_opt = Some(picked.ip.clone());
                optimistic_peer = Some(picked.ip.clone());
            }
        } else {
            *self.last_optimistic_ip.lock().unwrap() = None;
        }

        let mut choked_peers = disinterested;
        for p in remaining_interested {
            if optimistic_peer.as_ref() != Some(&p.ip) {
                choked_peers.push(p.ip.clone());
            }
        }

        ChokingDecision {
            unchoked_peers,
            optimistically_unchoked_peer: optimistic_peer,
            choked_peers,
        }
    }
}

/// Round-Robin choking policy rotating upload slots uniformly across interested peers.
#[derive(Debug, Default)]
pub struct RoundRobinPolicy {
    cursor: AtomicUsize,
}

impl RoundRobinPolicy {
    /// Creates a new `RoundRobinPolicy`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ChokingPolicy for RoundRobinPolicy {
    fn evaluate(
        &self,
        swarm: &dyn SwarmView,
        max_upload_slots: usize,
        _is_seeding: bool,
    ) -> ChokingDecision {
        let mut interested: Vec<String> = Vec::new();
        let mut disinterested: Vec<String> = Vec::new();

        swarm.for_each_peer(&mut |_ip, peer| {
            if peer.peer_interested {
                interested.push(peer.ip.clone());
            } else {
                disinterested.push(peer.ip.clone());
            }
        });

        if interested.is_empty() {
            return ChokingDecision {
                unchoked_peers: Vec::new(),
                optimistically_unchoked_peer: None,
                choked_peers: disinterested,
            };
        }

        let total = interested.len();
        let slots = max_upload_slots.min(total);
        let start = self.cursor.fetch_add(slots, Ordering::Relaxed) % total;

        let mut unchoked = Vec::with_capacity(slots);
        let mut choked = disinterested;

        for i in 0..total {
            let idx = (start + i) % total;
            let ip = interested[idx].clone();
            if i < slots {
                unchoked.push(ip);
            } else {
                choked.push(ip);
            }
        }

        ChokingDecision {
            unchoked_peers: unchoked,
            optimistically_unchoked_peer: None,
            choked_peers: choked,
        }
    }
}

/// Strategic choking policy prioritizing peers with highest reciprocity efficiency (BitTyrant).
#[derive(Debug, Default)]
pub struct BitTyrantPolicy;

impl BitTyrantPolicy {
    /// Creates a new `BitTyrantPolicy`.
    pub fn new() -> Self {
        Self
    }
}

impl ChokingPolicy for BitTyrantPolicy {
    fn evaluate(
        &self,
        swarm: &dyn SwarmView,
        max_upload_slots: usize,
        _is_seeding: bool,
    ) -> ChokingDecision {
        let mut interested: Vec<PeerSnapshot> = Vec::new();
        let mut disinterested: Vec<String> = Vec::new();

        swarm.for_each_peer(&mut |_ip, peer| {
            if peer.peer_interested {
                interested.push(peer.clone());
            } else {
                disinterested.push(peer.ip.clone());
            }
        });

        if interested.is_empty() {
            return ChokingDecision {
                unchoked_peers: Vec::new(),
                optimistically_unchoked_peer: None,
                choked_peers: disinterested,
            };
        }

        // Rank by reciprocity ratio: download_rate / upload_cost
        interested.sort_by(|a, b| {
            let ratio_a = a.download_rate / a.upload_rate.max(1.0);
            let ratio_b = b.download_rate / b.upload_rate.max(1.0);
            ratio_b
                .partial_cmp(&ratio_a)
                .unwrap_or(core::cmp::Ordering::Equal)
        });

        let slots = max_upload_slots.min(interested.len());
        let (top, remaining) = interested.split_at(slots);

        let unchoked_peers: Vec<String> = top.iter().map(|p| p.ip.clone()).collect();
        let mut choked_peers = disinterested;
        choked_peers.extend(remaining.iter().map(|p| p.ip.clone()));

        ChokingDecision {
            unchoked_peers,
            optimistically_unchoked_peer: None,
            choked_peers,
        }
    }
}

/// Implementation of `ChokingStrategy` driving a pluggable `ChokingPolicy` inside an async task loop.
#[derive(Debug)]
pub struct PolicyChokingStrategy {
    pub policy: Arc<dyn ChokingPolicy>,
    pub max_upload_slots: usize,
}

impl PolicyChokingStrategy {
    /// Creates a new runner with the given policy and default 4 upload slots.
    pub fn new(policy: Arc<dyn ChokingPolicy>) -> Self {
        Self {
            policy,
            max_upload_slots: 4,
        }
    }

    /// Sets the maximum upload slots to allocate.
    pub fn with_max_upload_slots(mut self, slots: usize) -> Self {
        self.max_upload_slots = slots;
        self
    }
}

impl ChokingStrategy for PolicyChokingStrategy {
    fn spawn_choking_loop(
        &self,
        context: Arc<std::sync::Mutex<TorrentContext>>,
        task_tx: std::sync::mpsc::Sender<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
        manager: Option<Arc<Manager>>,
    ) {
        let policy = self.policy.clone();
        let max_slots = self.max_upload_slots;

        let atomic_status = {
            let ctx = context.lock().unwrap();
            ctx.atomic_status.clone()
        };

        let _ = task_tx.send(Box::pin(async move {
            loop {
                let start_time = Instant::now();
                let duration = Duration::from_secs(10);
                let mut ended = false;
                while start_time.elapsed() < duration {
                    if atomic_status.load(std::sync::atomic::Ordering::Relaxed)
                        == TorrentStatus::Ended as u8
                    {
                        ended = true;
                        break;
                    }
                    delay(Duration::from_millis(100)).await;
                }
                if ended {
                    break;
                }

                let (status, is_seeding) = {
                    let ctx = context.lock().unwrap();
                    (ctx.status, ctx.status == TorrentStatus::Seeding)
                };
                if status == TorrentStatus::Ended {
                    break;
                }

                let mut peers_to_action = Vec::new();
                {
                    let ctx = context.lock().unwrap();

                    // 1. Update rolling rate windows for peers in the swarm
                    if let Ok(swarm) = ctx.peer_swarm.read() {
                        for peer_arc in swarm.values() {
                            if let Ok(mut p) = peer_arc.lock() {
                                let dl_rate = p.bytes_downloaded_in_interval as f64 / 10.0;
                                p.rolling_download_rate =
                                    p.rolling_download_rate * 0.8 + dl_rate * 0.2;
                                p.bytes_downloaded_in_interval = 0;

                                let ul_rate = p.bytes_uploaded_in_interval as f64 / 10.0;
                                p.rolling_upload_rate = p.rolling_upload_rate * 0.8 + ul_rate * 0.2;
                                p.bytes_uploaded_in_interval = 0;
                            }
                        }
                    }

                    // 2. Evaluate choking policy against SwarmView
                    let decision = policy.evaluate(&*ctx, max_slots, is_seeding);

                    // 3. Map policy decision to concrete PeerActions
                    if let Ok(swarm) = ctx.peer_swarm.read() {
                        let unchoke_set: HashSet<&str> = decision
                            .unchoked_peers
                            .iter()
                            .map(|s| s.as_str())
                            .chain(decision.optimistically_unchoked_peer.as_deref())
                            .collect();

                        for (ip, peer_arc) in swarm.iter() {
                            if let Ok(mut p) = peer_arc.lock() {
                                if unchoke_set.contains(ip.as_str()) {
                                    if p.am_choking {
                                        p.am_choking = false;
                                        peers_to_action
                                            .push((peer_arc.clone(), PeerAction::SendUnchoke));
                                    }
                                } else if !p.am_choking {
                                    p.am_choking = true;
                                    peers_to_action.push((peer_arc.clone(), PeerAction::SendChoke));
                                }
                            }
                        }
                    }
                }

                // 4. Dispatch actions to remote peer network sockets
                for (peer_arc, action) in peers_to_action {
                    let net_opt = peer_arc.lock().unwrap().network.clone();
                    if let Some(net) = net_opt {
                        let peer_ip = peer_arc.lock().unwrap().ip.clone();
                        let _ = Peer::execute_actions(
                            &peer_arc,
                            vec![action],
                            &net,
                            &context,
                            &peer_ip,
                            &manager,
                        )
                        .await;
                    }
                }
            }
        }));
    }
}
