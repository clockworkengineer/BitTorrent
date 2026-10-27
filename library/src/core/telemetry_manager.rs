//! Lock-free telemetry and performance metrics manager (SRP & ISP).
//!
//! Tracks real-time bandwidth speeds, uploaded and downloaded totals,
//! completion progress, and network protocol counters without coarse mutex contention.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicI64, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::core::views::TransferStatsView;
use crate::error::BitTorrentError;

/// Manages real-time bandwidth metrics, progress tracking, and protocol operation counters.
#[derive(Clone)]
pub struct TelemetryManager {
    /// Total bytes downloaded from remote peers and WebSeeds.
    pub total_bytes_downloaded: Arc<AtomicU64>,
    /// Bytes downloaded prior to this session (e.g. from fast resume).
    pub initial_bytes_downloaded: u64,
    /// Total bytes required to complete all selected files.
    pub total_bytes_to_download: Arc<AtomicU64>,
    /// Total bytes uploaded to remote peers.
    pub total_bytes_uploaded: Arc<AtomicU64>,
    /// Atomic status code representing the active session state.
    pub atomic_status: Arc<AtomicU8>,
    /// Speed tracking state for incoming data: `(timestamp, bytes_at_timestamp, calculated_bps)`.
    pub download_speed_tracker: Arc<Mutex<(Instant, u64, i64)>>,
    /// Speed tracking state for outgoing data: `(timestamp, bytes_at_timestamp, calculated_bps)`.
    pub upload_speed_tracker: Arc<Mutex<(Instant, u64, i64)>>,
    /// Lock-free cache of the most recently computed download rate in bytes per second.
    pub cached_download_bps: Arc<AtomicI64>,
    /// Lock-free cache of the most recently computed upload rate in bytes per second.
    pub cached_upload_bps: Arc<AtomicI64>,
    /// Count of currently unchoked peers in the swarm.
    pub unchoked_peers_count: Arc<AtomicUsize>,
    /// Discovered peers added via PEX.
    pub pex_added_peers_count: Arc<AtomicUsize>,
    /// Dropped peers announced via PEX.
    pub pex_dropped_peers_count: Arc<AtomicUsize>,
    /// BEP 52 v2 hash requests sent.
    pub v2_hash_requests_sent: Arc<AtomicUsize>,
    /// BEP 52 v2 hash requests received.
    pub v2_hash_requests_received: Arc<AtomicUsize>,
    /// BEP 52 v2 Merkle hashes sent.
    pub v2_hashes_sent: Arc<AtomicUsize>,
    /// BEP 52 v2 Merkle hashes received.
    pub v2_hashes_received: Arc<AtomicUsize>,
    /// BEP 52 v2 hash rejects sent.
    pub v2_hash_rejects_sent: Arc<AtomicUsize>,
    /// BEP 52 v2 hash rejects received.
    pub v2_hash_rejects_received: Arc<AtomicUsize>,
    /// BEP 55 holepunch rendezvous requests sent.
    pub holepunch_rendezvous_sent: Arc<AtomicUsize>,
    /// BEP 55 holepunch rendezvous requests received.
    pub holepunch_rendezvous_received: Arc<AtomicUsize>,
    /// BEP 55 holepunch connect attempts sent.
    pub holepunch_connect_sent: Arc<AtomicUsize>,
    /// BEP 55 holepunch connect attempts received.
    pub holepunch_connect_received: Arc<AtomicUsize>,
    /// BEP 55 holepunch error messages sent.
    pub holepunch_error_sent: Arc<AtomicUsize>,
    /// BEP 55 holepunch error messages received.
    pub holepunch_error_received: Arc<AtomicUsize>,
}

impl TelemetryManager {
    /// Creates a new `TelemetryManager` initialized with the given total download payload.
    pub fn new(total_bytes_to_download: u64) -> Self {
        Self {
            total_bytes_downloaded: Arc::new(AtomicU64::new(0)),
            initial_bytes_downloaded: 0,
            total_bytes_to_download: Arc::new(AtomicU64::new(total_bytes_to_download)),
            total_bytes_uploaded: Arc::new(AtomicU64::new(0)),
            atomic_status: Arc::new(AtomicU8::new(0)),
            download_speed_tracker: Arc::new(Mutex::new((Instant::now(), 0, 0))),
            upload_speed_tracker: Arc::new(Mutex::new((Instant::now(), 0, 0))),
            cached_download_bps: Arc::new(AtomicI64::new(0)),
            cached_upload_bps: Arc::new(AtomicI64::new(0)),
            unchoked_peers_count: Arc::new(AtomicUsize::new(0)),
            pex_added_peers_count: Arc::new(AtomicUsize::new(0)),
            pex_dropped_peers_count: Arc::new(AtomicUsize::new(0)),
            v2_hash_requests_sent: Arc::new(AtomicUsize::new(0)),
            v2_hash_requests_received: Arc::new(AtomicUsize::new(0)),
            v2_hashes_sent: Arc::new(AtomicUsize::new(0)),
            v2_hashes_received: Arc::new(AtomicUsize::new(0)),
            v2_hash_rejects_sent: Arc::new(AtomicUsize::new(0)),
            v2_hash_rejects_received: Arc::new(AtomicUsize::new(0)),
            holepunch_rendezvous_sent: Arc::new(AtomicUsize::new(0)),
            holepunch_rendezvous_received: Arc::new(AtomicUsize::new(0)),
            holepunch_connect_sent: Arc::new(AtomicUsize::new(0)),
            holepunch_connect_received: Arc::new(AtomicUsize::new(0)),
            holepunch_error_sent: Arc::new(AtomicUsize::new(0)),
            holepunch_error_received: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Updates the total bytes expected to be downloaded.
    pub fn set_total_bytes_to_download(&self, total: u64) {
        self.total_bytes_to_download.store(total, Ordering::Release);
    }

    /// Records newly downloaded bytes.
    pub fn record_download(&self, bytes: u64) {
        self.total_bytes_downloaded
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Records newly uploaded bytes.
    pub fn record_upload(&self, bytes: u64) {
        self.total_bytes_uploaded
            .fetch_add(bytes, Ordering::Relaxed);
    }

    /// Computes download progress in parts-per-ten-thousand (0..10,000).
    pub fn progress_ppm(&self) -> u32 {
        let total = self.total_bytes_to_download.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let ppm = (downloaded * 10000) / total;
        ppm.min(10000) as u32
    }

    /// Computes percentage of completion (0.0% to 100.0%).
    pub fn progress_percent(&self) -> f32 {
        self.progress_ppm() as f32 / 100.0
    }

    /// Computes bytes remaining to download.
    pub fn bytes_left_to_download(&self) -> Result<u64, BitTorrentError> {
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let total = self.total_bytes_to_download.load(Ordering::Relaxed);
        if total == 0 {
            return Ok(0);
        }
        if total < downloaded {
            return Err(BitTorrentError::Parse(
                "Bytes left to download turned negative.".into(),
            ));
        }
        Ok(total - downloaded)
    }

    /// Calculates or returns exponential moving average download rate in bytes per second.
    pub fn download_bytes_per_second(&self) -> i64 {
        let Ok(mut guard) = self.download_speed_tracker.try_lock() else {
            return self.cached_download_bps.load(Ordering::Relaxed);
        };

        let now = Instant::now();
        let elapsed = now.duration_since(guard.0);
        if elapsed.as_millis() >= 500 {
            let current_bytes = self.total_bytes_downloaded.load(Ordering::Relaxed);
            let delta_bytes = current_bytes.saturating_sub(guard.1);
            let old_bps = guard.2;
            let instant_bps = (delta_bytes as f64 / elapsed.as_secs_f64()) as i64;
            let bps = if delta_bytes > 0 {
                if old_bps > 0 {
                    ((old_bps as f64 * 0.7) + (instant_bps as f64 * 0.3)).round() as i64
                } else {
                    instant_bps
                }
            } else if old_bps > 0 {
                let decayed = (old_bps as f64 * 0.75).round() as i64;
                if decayed < 50 { 0 } else { decayed }
            } else {
                0
            };

            *guard = (now, current_bytes, bps);
            self.cached_download_bps.store(bps, Ordering::Relaxed);
            bps
        } else {
            guard.2
        }
    }

    /// Calculates or returns exponential moving average upload rate in bytes per second.
    pub fn upload_bytes_per_second(&self) -> i64 {
        let Ok(mut guard) = self.upload_speed_tracker.try_lock() else {
            return self.cached_upload_bps.load(Ordering::Relaxed);
        };

        let now = Instant::now();
        let elapsed = now.duration_since(guard.0);
        if elapsed.as_millis() >= 500 {
            let current_bytes = self.total_bytes_uploaded.load(Ordering::Relaxed);
            let delta_bytes = current_bytes.saturating_sub(guard.1);
            let old_bps = guard.2;
            let instant_bps = (delta_bytes as f64 / elapsed.as_secs_f64()) as i64;
            let bps = if delta_bytes > 0 {
                if old_bps > 0 {
                    ((old_bps as f64 * 0.7) + (instant_bps as f64 * 0.3)).round() as i64
                } else {
                    instant_bps
                }
            } else if old_bps > 0 {
                let decayed = (old_bps as f64 * 0.75).round() as i64;
                if decayed < 50 { 0 } else { decayed }
            } else {
                0
            };

            *guard = (now, current_bytes, bps);
            self.cached_upload_bps.store(bps, Ordering::Relaxed);
            bps
        } else {
            guard.2
        }
    }

    /// Returns the most recently computed download rate without blocking or acquiring mutexes.
    #[inline]
    pub fn current_download_bps(&self) -> i64 {
        self.cached_download_bps.load(Ordering::Relaxed)
    }

    /// Returns the most recently computed upload rate without blocking or acquiring mutexes.
    #[inline]
    pub fn current_upload_bps(&self) -> i64 {
        self.cached_upload_bps.load(Ordering::Relaxed)
    }

    /// Estimates seconds remaining to completion based on current download speed and remaining bytes.
    pub fn eta_seconds(&self) -> Option<u64> {
        let left = self.bytes_left_to_download().ok()?;
        if left == 0 {
            return Some(0);
        }
        let bps = self.current_download_bps();
        if bps <= 0 {
            return None;
        }
        Some(left / bps as u64)
    }
}

impl TransferStatsView for TelemetryManager {
    fn total_downloaded(&self) -> u64 {
        self.total_bytes_downloaded.load(Ordering::Relaxed)
    }

    fn total_uploaded(&self) -> u64 {
        self.total_bytes_uploaded.load(Ordering::Relaxed)
    }

    fn current_download_rate(&self) -> u64 {
        self.current_download_bps().max(0) as u64
    }

    fn current_upload_rate(&self) -> u64 {
        self.current_upload_bps().max(0) as u64
    }

    fn progress_percent(&self) -> f32 {
        self.progress_percent()
    }
}
