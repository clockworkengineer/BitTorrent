//! Swarm Latency Profiler & Peer ECT Estimator
//!
//! Tracks exponential moving average RTT, throughput, and jitter per connected peer,
//! identifying stragglers and computing Expected Completion Times (ECT) for adaptive
//! endgame block scheduling.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Exponential Moving Average smoothing factor for RTT updates (alpha = 0.125, standard TCP RFC 6298).
const RTT_ALPHA: f64 = 0.125;

/// Jitter / variance smoothing factor (beta = 0.25).
const RTT_BETA: f64 = 0.25;

/// Minimum RTT sample clamp (1 millisecond) to avoid zero-division.
const MIN_RTT: Duration = Duration::from_millis(1);

/// Maximum RTT sample clamp (30 seconds) to bound runaway measurements.
const MAX_RTT: Duration = Duration::from_secs(30);

/// Latency and throughput profile for a single peer connection.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerLatencyProfile {
    /// Peer IP address or identifier.
    pub peer_ip: String,
    /// Smoothed round-trip time (EMA).
    pub smoothed_rtt: Duration,
    /// RTT variance / jitter.
    pub rtt_variance: Duration,
    /// Rolling download transfer rate in bytes per second.
    pub rolling_download_rate: f64,
    /// Current number of outstanding block requests pending on this peer.
    pub in_flight_requests: usize,
    /// Total number of blocks successfully delivered by this peer.
    pub delivered_blocks: usize,
    /// Total number of block requests that timed out on this peer.
    pub timed_out_blocks: usize,
    /// Total bytes delivered by this peer.
    pub total_bytes_delivered: u64,
    /// Timestamp of the last successful block delivery.
    pub last_delivery: Option<Instant>,
}

impl PeerLatencyProfile {
    /// Creates a new profile with baseline initial assumptions.
    pub fn new(peer_ip: impl Into<String>) -> Self {
        Self {
            peer_ip: peer_ip.into(),
            smoothed_rtt: Duration::from_millis(100), // 100ms baseline default
            rtt_variance: Duration::from_millis(50),
            rolling_download_rate: 65536.0, // 64 KiB/s baseline default
            in_flight_requests: 0,
            delivered_blocks: 0,
            timed_out_blocks: 0,
            total_bytes_delivered: 0,
            last_delivery: None,
        }
    }

    /// Records a new RTT sample and delivered block byte length.
    pub fn update_sample(&mut self, sample_rtt: Duration, bytes: usize) {
        let sample = sample_rtt.clamp(MIN_RTT, MAX_RTT);

        if self.delivered_blocks == 0 {
            self.smoothed_rtt = sample;
            self.rtt_variance = sample / 2;
        } else {
            let sample_secs = sample.as_secs_f64();
            let srtt_secs = self.smoothed_rtt.as_secs_f64();
            let diff = (srtt_secs - sample_secs).abs();

            let new_var = (1.0 - RTT_BETA) * self.rtt_variance.as_secs_f64() + RTT_BETA * diff;
            let new_srtt = (1.0 - RTT_ALPHA) * srtt_secs + RTT_ALPHA * sample_secs;

            self.rtt_variance = Duration::from_secs_f64(new_var).clamp(MIN_RTT, MAX_RTT);
            self.smoothed_rtt = Duration::from_secs_f64(new_srtt).clamp(MIN_RTT, MAX_RTT);
        }

        // Update rolling download speed
        let instant_speed = bytes as f64 / sample.as_secs_f64().max(0.001);
        self.rolling_download_rate = 0.8 * self.rolling_download_rate + 0.2 * instant_speed;

        self.delivered_blocks += 1;
        self.total_bytes_delivered += bytes as u64;
        self.in_flight_requests = self.in_flight_requests.saturating_sub(1);
        self.last_delivery = Some(Instant::now());
    }

    /// Records that an in-flight block request timed out on this peer.
    pub fn record_timeout(&mut self) {
        self.timed_out_blocks += 1;
        self.in_flight_requests = self.in_flight_requests.saturating_sub(1);
        // Penalize smoothed RTT and download speed on timeout
        self.smoothed_rtt = (self.smoothed_rtt * 3 / 2).min(MAX_RTT);
        self.rolling_download_rate = (self.rolling_download_rate * 0.7).max(1024.0);
    }

    /// Records a new block request dispatched to this peer.
    pub fn record_request_dispatched(&mut self) {
        self.in_flight_requests += 1;
    }

    /// Calculates the Estimated Completion Time (ECT) for a block of the given size.
    ///
    /// ECT = RTT + transmission_delay + queueing_delay
    pub fn estimated_completion_time(&self, block_size: usize) -> Duration {
        let speed = self.rolling_download_rate.max(1024.0);
        let transmission_secs = (block_size as f64) / speed;
        let queueing_secs = (self.in_flight_requests as f64) * transmission_secs * 0.5;
        let total_secs = self.smoothed_rtt.as_secs_f64() + transmission_secs + queueing_secs;
        Duration::from_secs_f64(total_secs).clamp(MIN_RTT, Duration::from_secs(60))
    }

    /// Returns true if this peer is behaving as a straggler compared to the swarm median RTT.
    pub fn is_straggler(&self, swarm_median_rtt: Duration, straggler_multiplier: f64) -> bool {
        let timeout_ratio = if self.delivered_blocks + self.timed_out_blocks > 0 {
            self.timed_out_blocks as f64 / (self.delivered_blocks + self.timed_out_blocks) as f64
        } else {
            0.0
        };

        if timeout_ratio > 0.4 {
            return true;
        }

        let threshold = swarm_median_rtt.mul_f64(straggler_multiplier);
        self.smoothed_rtt > threshold
    }
}

/// Lightweight snapshot of a peer's latency status for telemetry/CLI.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerLatencySummary {
    pub peer_ip: String,
    pub smoothed_rtt_ms: u64,
    pub rtt_variance_ms: u64,
    pub download_rate_kbps: f64,
    pub in_flight: usize,
    pub delivered: usize,
    pub timed_out: usize,
    pub is_straggler: bool,
}

/// Swarm-wide latency profiler tracking responsiveness across all connected peers.
#[derive(Debug, Default)]
pub struct SwarmLatencyProfiler {
    profiles: HashMap<String, PeerLatencyProfile>,
}

impl SwarmLatencyProfiler {
    /// Creates a new empty `SwarmLatencyProfiler`.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Gets or creates a profile for the specified peer IP.
    pub fn get_or_create(&mut self, peer_ip: &str) -> &mut PeerLatencyProfile {
        self.profiles
            .entry(peer_ip.to_string())
            .or_insert_with(|| PeerLatencyProfile::new(peer_ip))
    }

    /// Records a block delivery RTT sample for a peer.
    pub fn record_sample(&mut self, peer_ip: &str, sample_rtt: Duration, bytes: usize) {
        self.get_or_create(peer_ip).update_sample(sample_rtt, bytes);
    }

    /// Records a block timeout on a peer.
    pub fn record_timeout(&mut self, peer_ip: &str) {
        self.get_or_create(peer_ip).record_timeout();
    }

    /// Records that a request was dispatched to a peer.
    pub fn record_request_sent(&mut self, peer_ip: &str) {
        self.get_or_create(peer_ip).record_request_dispatched();
    }

    /// Calculates the median smoothed RTT across all active profiles in the swarm.
    pub fn median_rtt(&self) -> Duration {
        if self.profiles.is_empty() {
            return Duration::from_millis(100);
        }
        let mut rtts: Vec<Duration> = self.profiles.values().map(|p| p.smoothed_rtt).collect();
        rtts.sort();
        rtts[rtts.len() / 2]
    }

    /// Ranks candidate peers by lowest Estimated Completion Time (ECT) for a given block size.
    pub fn rank_peers_by_ect<'a>(
        &self,
        candidates: &'a [&'a str],
        block_size: usize,
    ) -> Vec<(&'a str, Duration)> {
        let mut ranked: Vec<(&'a str, Duration)> = candidates
            .iter()
            .map(|&ip| {
                let ect = self
                    .profiles
                    .get(ip)
                    .map(|p| p.estimated_completion_time(block_size))
                    .unwrap_or_else(|| Duration::from_millis(250));
                (ip, ect)
            })
            .collect();

        ranked.sort_by_key(|&(_, ect)| ect);
        ranked
    }

    /// Returns the top N fastest peers based on ECT for the standard 16 KiB block.
    pub fn fastest_peers(&self, top_n: usize) -> Vec<String> {
        let mut peers: Vec<(&String, Duration)> = self
            .profiles
            .iter()
            .map(|(ip, p)| (ip, p.estimated_completion_time(crate::constants::BLOCK_SIZE)))
            .collect();

        peers.sort_by_key(|&(_, ect)| ect);
        peers.into_iter().take(top_n).map(|(ip, _)| ip.clone()).collect()
    }

    /// Generates summary snapshots of all peer latency profiles.
    pub fn summary(&self, straggler_multiplier: f64) -> Vec<PeerLatencySummary> {
        let median = self.median_rtt();
        let mut summaries: Vec<PeerLatencySummary> = self
            .profiles
            .values()
            .map(|p| PeerLatencySummary {
                peer_ip: p.peer_ip.clone(),
                smoothed_rtt_ms: p.smoothed_rtt.as_millis() as u64,
                rtt_variance_ms: p.rtt_variance.as_millis() as u64,
                download_rate_kbps: p.rolling_download_rate / 1024.0,
                in_flight: p.in_flight_requests,
                delivered: p.delivered_blocks,
                timed_out: p.timed_out_blocks,
                is_straggler: p.is_straggler(median, straggler_multiplier),
            })
            .collect();

        summaries.sort_by_key(|s| s.smoothed_rtt_ms);
        summaries
    }
}
