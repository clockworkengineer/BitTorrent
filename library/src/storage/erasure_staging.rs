//! Predictive Erasure Block Staging & Adaptive Endgame Cache
//!
//! Provides multi-source block request staging, straggler preemption, and erasure
//! absorption for endgame mode, eliminating tail completion stalls caused by slow peer pipelines.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Outcome when a block arrives in the endgame erasure cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockArrivalOutcome {
    /// First time this block has arrived.
    /// It must be written to storage and committed to the piece buffer,
    /// and cancellation messages should be dispatched to all other peers who had it requested.
    CommitAndCancelOthers {
        peers_to_cancel: Vec<String>,
    },
    /// Redundant duplicate block arrival from a slower/preempted peer.
    /// The block was already committed; safely absorbed and dropped in memory
    /// without writing duplicate bytes to disk.
    AbsorbDuplicate {
        source_peer: String,
    },
}

/// Metadata for an in-flight block request staged across one or more swarm sources.
#[derive(Debug, Clone)]
pub struct StagedBlock {
    /// Torrent piece index.
    pub piece: u32,
    /// 0-based block index within the piece.
    pub block_index: u32,
    /// Peer IPs that have an active in-flight request for this block.
    pub sources: Vec<String>,
    /// Timestamp when the first request was dispatched.
    pub first_requested: Instant,
    /// Timestamp when the latest redundant clone request was dispatched.
    pub latest_requested: Instant,
    /// Estimated Completion Time expected from the primary source.
    pub expected_completion: Duration,
    /// True if a straggler preemption clone was scheduled for this block.
    pub is_preempted: bool,
}

/// Telemetry metrics for the adaptive endgame erasure staging cache.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EndgameStagingMetrics {
    /// Number of blocks currently in-flight in the staging cache.
    pub active_staged_blocks: usize,
    /// Total block requests tracked by the staging engine.
    pub total_staged_blocks: usize,
    /// Number of distinct blocks committed to storage on first arrival.
    pub committed_blocks: usize,
    /// Number of times a slow straggler peer was proactively preempted by cloning to a faster peer.
    pub preempted_stragglers: usize,
    /// Number of redundant duplicate blocks absorbed in-memory without disk I/O.
    pub absorbed_duplicate_blocks: usize,
    /// Number of Cancel messages sent to abort redundant block requests.
    pub cancelled_redundant_requests: usize,
    /// Estimated cumulative stall latency in milliseconds saved by preemption.
    pub estimated_stall_time_saved_ms: u64,
}

/// Adaptive erasure staging cache managing tail block requests and eliminating stalls.
#[derive(Debug)]
pub struct AdaptiveErasureStaging {
    staged: HashMap<(u32, u32), StagedBlock>,
    committed_blocks: HashSet<(u32, u32)>,
    max_clones_per_block: usize,
    straggler_ratio: f64,
    metrics: EndgameStagingMetrics,
}

impl Default for AdaptiveErasureStaging {
    fn default() -> Self {
        Self::new(2, 1.8)
    }
}

impl AdaptiveErasureStaging {
    /// Creates a new `AdaptiveErasureStaging` engine.
    ///
    /// - `max_clones_per_block`: Maximum concurrent peers requesting the same block (default: 2).
    /// - `straggler_ratio`: Multiplier on ECT before an outstanding request is considered stalled (default: 1.8).
    pub fn new(max_clones_per_block: usize, straggler_ratio: f64) -> Self {
        Self {
            staged: HashMap::new(),
            committed_blocks: HashSet::new(),
            max_clones_per_block: max_clones_per_block.max(1),
            straggler_ratio: straggler_ratio.max(1.1),
            metrics: EndgameStagingMetrics::default(),
        }
    }

    /// Stages a block request dispatched to a peer with an estimated completion time.
    pub fn stage_request(&mut self, piece: u32, block_index: u32, peer_ip: &str, ect: Duration) {
        let key = (piece, block_index);
        let now = Instant::now();

        if let Some(existing) = self.staged.get_mut(&key) {
            if !existing.sources.contains(&peer_ip.to_string()) {
                existing.sources.push(peer_ip.to_string());
                existing.latest_requested = now;
            }
        } else {
            self.staged.insert(
                key,
                StagedBlock {
                    piece,
                    block_index,
                    sources: vec![peer_ip.to_string()],
                    first_requested: now,
                    latest_requested: now,
                    expected_completion: ect,
                    is_preempted: false,
                },
            );
            self.metrics.total_staged_blocks += 1;
        }
        self.metrics.active_staged_blocks = self.staged.len();
    }

    /// Returns true if this block can accept another redundant clone request.
    pub fn can_clone_block(&self, piece: u32, block_index: u32) -> bool {
        let key = (piece, block_index);
        if self.committed_blocks.contains(&key) {
            return false;
        }
        match self.staged.get(&key) {
            None => true,
            Some(staged) => staged.sources.len() < self.max_clones_per_block,
        }
    }

    /// Checks for in-flight requests that have exceeded their expected completion window,
    /// returning candidate blocks and their slow primary source peer for straggler preemption.
    pub fn detect_stalled_blocks(&mut self) -> Vec<(u32, u32, String)> {
        let now = Instant::now();
        let straggler_ratio = self.straggler_ratio;
        let max_clones = self.max_clones_per_block;
        let mut stalled = Vec::new();

        for (&(piece, block_index), staged) in self.staged.iter_mut() {
            if staged.sources.len() >= max_clones || staged.is_preempted {
                continue;
            }

            // Stall threshold: ECT * straggler_ratio, clamped to at least 400ms
            let stall_threshold = (staged.expected_completion.mul_f64(straggler_ratio))
                .max(Duration::from_millis(400));

            if now.duration_since(staged.first_requested) > stall_threshold {
                staged.is_preempted = true;
                if let Some(primary_peer) = staged.sources.first() {
                    stalled.push((piece, block_index, primary_peer.clone()));
                }
            }
        }

        if !stalled.is_empty() {
            self.metrics.preempted_stragglers += stalled.len();
        }

        stalled
    }

    /// Handles an incoming block payload, determining whether to commit or absorb as erasure duplicate.
    pub fn on_block_arrived(&mut self, piece: u32, block_index: u32, from_peer: &str) -> BlockArrivalOutcome {
        let key = (piece, block_index);

        if self.committed_blocks.contains(&key) {
            // Already committed earlier by another peer -> safely absorb redundant duplicate in-memory
            self.metrics.absorbed_duplicate_blocks += 1;
            return BlockArrivalOutcome::AbsorbDuplicate {
                source_peer: from_peer.to_string(),
            };
        }

        self.committed_blocks.insert(key);
        self.metrics.committed_blocks += 1;

        let peers_to_cancel = if let Some(staged) = self.staged.remove(&key) {
            let cancel_list: Vec<String> = staged
                .sources
                .into_iter()
                .filter(|p| p != from_peer)
                .collect();

            self.metrics.cancelled_redundant_requests += cancel_list.len();
            if staged.is_preempted {
                let saved = staged.expected_completion.as_millis() as u64;
                self.metrics.estimated_stall_time_saved_ms += saved;
            }
            cancel_list
        } else {
            Vec::new()
        };

        self.metrics.active_staged_blocks = self.staged.len();

        BlockArrivalOutcome::CommitAndCancelOthers { peers_to_cancel }
    }

    /// Clears any staged entries and committed markers for a piece when completed or reset.
    pub fn clear_piece(&mut self, piece: u32) {
        self.staged.retain(|&(p, _), _| p != piece);
        self.committed_blocks.retain(|&(p, _)| p != piece);
        self.metrics.active_staged_blocks = self.staged.len();
    }

    /// Returns a copy of current staging metrics.
    pub fn metrics(&self) -> EndgameStagingMetrics {
        let mut m = self.metrics.clone();
        m.active_staged_blocks = self.staged.len();
        m
    }
}
