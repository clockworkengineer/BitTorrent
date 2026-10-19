//! BEP 38 / WebTorrent Dynamic WebSeed Failover & Mirror Health Tracker
//!
//! Provides multi-source HTTP fallback routing, circuit-breaker backoff penalties,
//! real-time latency/error rate health scoring, and dynamic mirror ranking across WebSeed endpoints.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const MIN_RTT: Duration = Duration::from_millis(5);
const MAX_RTT: Duration = Duration::from_secs(30);
const LATENCY_ALPHA: f64 = 0.125;
const LATENCY_BETA: f64 = 0.25;

/// Health and latency performance profile for an individual WebSeed mirror endpoint.
#[derive(Debug, Clone)]
pub struct MirrorHealthProfile {
    /// Mirror endpoint URL.
    pub endpoint_url: String,
    /// Total HTTP requests dispatched to this mirror.
    pub total_requests: usize,
    /// Successful HTTP requests delivered from this mirror.
    pub successful_requests: usize,
    /// Failed or timed out requests on this mirror.
    pub failed_requests: usize,
    /// Current consecutive failed requests.
    pub consecutive_failures: usize,
    /// Smoothed round-trip latency estimate (EMA).
    pub smoothed_latency: Duration,
    /// Latency variance/jitter estimate.
    pub latency_variance: Duration,
    /// Rolling transfer speed estimate in bytes per second.
    pub rolling_download_rate: f64,
    /// Total bytes successfully downloaded from this mirror.
    pub total_bytes_downloaded: u64,
    /// Timestamp of the last attempted request.
    pub last_attempt: Option<Instant>,
    /// Timestamp of the last successful request.
    pub last_success: Option<Instant>,
    /// Active circuit-breaker penalty expiration timestamp, if penalized.
    pub penalty_until: Option<Instant>,
    /// Number of times this endpoint has been penalized.
    pub penalty_count: usize,
}

impl MirrorHealthProfile {
    /// Creates a new profile for a WebSeed endpoint URL.
    pub fn new(endpoint_url: impl Into<String>) -> Self {
        Self {
            endpoint_url: endpoint_url.into(),
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            consecutive_failures: 0,
            smoothed_latency: Duration::from_millis(250),
            latency_variance: Duration::from_millis(100),
            rolling_download_rate: 128.0 * 1024.0, // 128 KiB/s initial baseline
            total_bytes_downloaded: 0,
            last_attempt: None,
            last_success: None,
            penalty_until: None,
            penalty_count: 0,
        }
    }

    /// Records a successful block delivery from this mirror.
    pub fn record_success(&mut self, latency: Duration, bytes: usize) {
        let sample = latency.clamp(MIN_RTT, MAX_RTT);
        let now = Instant::now();

        if self.successful_requests == 0 {
            self.smoothed_latency = sample;
            self.latency_variance = sample / 2;
        } else {
            let sample_secs = sample.as_secs_f64();
            let srtt_secs = self.smoothed_latency.as_secs_f64();
            let diff = (srtt_secs - sample_secs).abs();

            let new_var =
                (1.0 - LATENCY_BETA) * self.latency_variance.as_secs_f64() + LATENCY_BETA * diff;
            let new_srtt = (1.0 - LATENCY_ALPHA) * srtt_secs + LATENCY_ALPHA * sample_secs;

            self.latency_variance = Duration::from_secs_f64(new_var).clamp(MIN_RTT, MAX_RTT);
            self.smoothed_latency = Duration::from_secs_f64(new_srtt).clamp(MIN_RTT, MAX_RTT);
        }

        let instant_speed = bytes as f64 / sample.as_secs_f64().max(0.001);
        self.rolling_download_rate = 0.75 * self.rolling_download_rate + 0.25 * instant_speed;

        self.total_requests += 1;
        self.successful_requests += 1;
        self.consecutive_failures = 0;
        self.total_bytes_downloaded += bytes as u64;
        self.last_attempt = Some(now);
        self.last_success = Some(now);
        self.penalty_until = None; // clear any active penalty
    }

    /// Records a failed or timed out request from this mirror.
    pub fn record_failure(
        &mut self,
        failure_threshold: usize,
        base_cooldown_secs: u64,
        max_cooldown_secs: u64,
    ) {
        let now = Instant::now();
        self.total_requests += 1;
        self.failed_requests += 1;
        self.consecutive_failures += 1;
        self.last_attempt = Some(now);

        // Penalize smoothed latency and download rate
        self.smoothed_latency = (self.smoothed_latency * 3 / 2).min(MAX_RTT);
        self.rolling_download_rate = (self.rolling_download_rate * 0.6).max(1024.0);

        // Trigger circuit breaker backoff if consecutive failures hit threshold
        if self.consecutive_failures >= failure_threshold.max(1) {
            let exponent = (self.consecutive_failures - failure_threshold).min(6) as u32;
            let multiplier = 1u64.checked_shl(exponent).unwrap_or(64);
            let cooldown_secs =
                (base_cooldown_secs.saturating_mul(multiplier)).min(max_cooldown_secs);

            self.penalty_until = Some(now + Duration::from_secs(cooldown_secs));
            self.penalty_count += 1;
        }
    }

    /// Returns true if this mirror is currently under a circuit-breaker backoff penalty.
    pub fn is_penalized(&self) -> bool {
        if let Some(until) = self.penalty_until {
            Instant::now() < until
        } else {
            false
        }
    }

    /// Calculates composite health score (0.0 to 100.0).
    ///
    /// Higher is better. Factors in success rate, latency, and active penalties.
    pub fn health_score(&self) -> f64 {
        if self.is_penalized() {
            return 0.0;
        }

        let success_rate = if self.total_requests == 0 {
            1.0 // Untried mirror receives optimistic baseline
        } else {
            self.successful_requests as f64 / self.total_requests as f64
        };

        // Latency penalty: 100ms -> 1.0, 1000ms -> 0.1, clamped
        let lat_ms = self.smoothed_latency.as_millis().max(50) as f64;
        let latency_factor = (500.0 / lat_ms).clamp(0.1, 1.0);

        let consecutive_penalty = match self.consecutive_failures {
            0 => 1.0,
            1 => 0.8,
            2 => 0.5,
            _ => 0.2,
        };

        (success_rate * 60.0 + latency_factor * 30.0 + consecutive_penalty * 10.0).clamp(0.0, 100.0)
    }

    /// Generates a telemetry summary of this mirror profile.
    pub fn summary(&self) -> MirrorHealthSummary {
        MirrorHealthSummary {
            endpoint_url: self.endpoint_url.clone(),
            total_requests: self.total_requests,
            successful_requests: self.successful_requests,
            failed_requests: self.failed_requests,
            consecutive_failures: self.consecutive_failures,
            smoothed_latency_ms: self.smoothed_latency.as_millis() as u64,
            download_rate_kbps: self.rolling_download_rate / 1024.0,
            total_bytes_downloaded: self.total_bytes_downloaded,
            health_score: self.health_score(),
            is_penalized: self.is_penalized(),
        }
    }
}

/// Telemetry summary snapshot for an individual WebSeed mirror endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorHealthSummary {
    pub endpoint_url: String,
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub consecutive_failures: usize,
    pub smoothed_latency_ms: u64,
    pub download_rate_kbps: f64,
    pub total_bytes_downloaded: u64,
    pub health_score: f64,
    pub is_penalized: bool,
}

/// Swarm WebSeed mirror tracker managing endpoint health and dynamic fallback routing.
#[derive(Debug, Clone)]
pub struct WebSeedMirrorTracker {
    profiles: HashMap<String, MirrorHealthProfile>,
    failure_threshold: usize,
    base_cooldown_secs: u64,
    max_cooldown_secs: u64,
}

impl Default for WebSeedMirrorTracker {
    fn default() -> Self {
        Self::new(3, 10, 300)
    }
}

impl WebSeedMirrorTracker {
    /// Creates a new `WebSeedMirrorTracker`.
    pub fn new(failure_threshold: usize, base_cooldown_secs: u64, max_cooldown_secs: u64) -> Self {
        Self {
            profiles: HashMap::new(),
            failure_threshold: failure_threshold.max(1),
            base_cooldown_secs: base_cooldown_secs.max(1),
            max_cooldown_secs: max_cooldown_secs.max(base_cooldown_secs),
        }
    }

    /// Registers a new WebSeed mirror URL if not already tracked.
    pub fn register_mirror(&mut self, url: impl Into<String>) {
        let u = url.into();
        self.profiles
            .entry(u.clone())
            .or_insert_with(|| MirrorHealthProfile::new(u));
    }

    /// Removes a WebSeed mirror URL from tracking.
    pub fn remove_mirror(&mut self, url: &str) -> bool {
        self.profiles.remove(url).is_some()
    }

    /// Records a successful delivery on a mirror.
    pub fn record_success(&mut self, url: &str, latency: Duration, bytes: usize) {
        if let Some(profile) = self.profiles.get_mut(url) {
            profile.record_success(latency, bytes);
        } else {
            let mut profile = MirrorHealthProfile::new(url);
            profile.record_success(latency, bytes);
            self.profiles.insert(url.to_string(), profile);
        }
    }

    /// Records a failed request on a mirror, applying backoff penalties if threshold reached.
    pub fn record_failure(&mut self, url: &str) {
        let threshold = self.failure_threshold;
        let base_cd = self.base_cooldown_secs;
        let max_cd = self.max_cooldown_secs;

        if let Some(profile) = self.profiles.get_mut(url) {
            profile.record_failure(threshold, base_cd, max_cd);
        } else {
            let mut profile = MirrorHealthProfile::new(url);
            profile.record_failure(threshold, base_cd, max_cd);
            self.profiles.insert(url.to_string(), profile);
        }
    }

    /// Returns candidate WebSeed mirrors sorted by highest health score first.
    ///
    /// Non-penalized mirrors are prioritized. If all available mirrors are currently
    /// penalized, the mirror with the soonest expiring penalty is selected as a probe.
    pub fn select_ranked_mirrors(&self) -> Vec<String> {
        if self.profiles.is_empty() {
            return Vec::new();
        }

        let mut available: Vec<(&MirrorHealthProfile, f64)> = Vec::new();
        let mut penalized: Vec<&MirrorHealthProfile> = Vec::new();

        for profile in self.profiles.values() {
            if profile.is_penalized() {
                penalized.push(profile);
            } else {
                available.push((profile, profile.health_score()));
            }
        }

        // Sort available mirrors descending by health score, breaking ties by lowest latency
        available.sort_by(|(a, score_a), (b, score_b)| {
            score_b
                .partial_cmp(score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.smoothed_latency.cmp(&b.smoothed_latency))
        });

        let mut result: Vec<String> = available
            .into_iter()
            .map(|(p, _)| p.endpoint_url.clone())
            .collect();

        // If no healthy mirrors remain, allow probing penalized mirrors by earliest cooldown expiration
        if result.is_empty() && !penalized.is_empty() {
            penalized.sort_by_key(|p| p.penalty_until);
            result.extend(penalized.into_iter().map(|p| p.endpoint_url.clone()));
        }

        result
    }

    /// Returns a telemetry summary list for all registered mirrors.
    pub fn summaries(&self) -> Vec<MirrorHealthSummary> {
        let mut list: Vec<MirrorHealthSummary> =
            self.profiles.values().map(|p| p.summary()).collect();
        list.sort_by(|a, b| {
            b.health_score
                .partial_cmp(&a.health_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        list
    }

    /// Returns the number of registered mirrors.
    pub fn mirror_count(&self) -> usize {
        self.profiles.len()
    }
}
