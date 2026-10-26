//! Transfer bandwidth and completion metrics tracking.

use alloc::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

/// Tracks downloaded/uploaded byte counters and moving average speeds.
#[derive(Clone)]
pub struct TransferMetrics {
    pub total_bytes_downloaded: Arc<AtomicU64>,
    pub initial_bytes_downloaded: u64,
    pub total_bytes_to_download: Arc<AtomicU64>,
    pub total_bytes_uploaded: Arc<AtomicU64>,
    pub download_speed_tracker: Arc<Mutex<(Instant, u64, i64)>>,
    pub upload_speed_tracker: Arc<Mutex<(Instant, u64, i64)>>,
    pub cached_download_bps: Arc<AtomicI64>,
    pub cached_upload_bps: Arc<AtomicI64>,
}

impl TransferMetrics {
    pub fn new(total_bytes_to_download: u64) -> Self {
        TransferMetrics {
            total_bytes_downloaded: Arc::new(AtomicU64::new(0)),
            initial_bytes_downloaded: 0,
            total_bytes_to_download: Arc::new(AtomicU64::new(total_bytes_to_download)),
            total_bytes_uploaded: Arc::new(AtomicU64::new(0)),
            download_speed_tracker: Arc::new(Mutex::new((Instant::now(), 0, 0))),
            upload_speed_tracker: Arc::new(Mutex::new((Instant::now(), 0, 0))),
            cached_download_bps: Arc::new(AtomicI64::new(0)),
            cached_upload_bps: Arc::new(AtomicI64::new(0)),
        }
    }

    pub fn set_total_bytes_to_download(&self, total: u64) {
        self.total_bytes_to_download.store(total, Ordering::Release);
    }

    pub fn record_download(&self, bytes: u64) {
        self.total_bytes_downloaded
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_upload(&self, bytes: u64) {
        self.total_bytes_uploaded
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn progress_ppm(&self) -> u32 {
        let total = self.total_bytes_to_download.load(Ordering::Relaxed);
        if total == 0 {
            return 0;
        }
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let ppm = (downloaded * 10000) / total;
        ppm.min(10000) as u32
    }

    pub fn progress_percent(&self) -> f32 {
        self.progress_ppm() as f32 / 100.0
    }

    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let total = self.total_bytes_to_download.load(Ordering::Relaxed);
        if total == 0 {
            return Ok(0);
        }
        if total < downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(total - downloaded)
    }

    pub fn bytes_per_second(&self) -> i64 {
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

    /// Returns the most recently computed download bytes per second without acquiring tracker mutex.
    #[inline]
    pub fn current_download_bps(&self) -> i64 {
        self.cached_download_bps.load(Ordering::Relaxed)
    }

    /// Returns the most recently computed upload bytes per second without acquiring tracker mutex.
    #[inline]
    pub fn current_upload_bps(&self) -> i64 {
        self.cached_upload_bps.load(Ordering::Relaxed)
    }
}
