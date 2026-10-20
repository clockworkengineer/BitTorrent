//! Transfer bandwidth and completion metrics tracking.

use alloc::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Tracks downloaded/uploaded byte counters and moving average speeds.
pub struct TransferMetrics {
    pub total_bytes_downloaded: Arc<AtomicU64>,
    pub initial_bytes_downloaded: u64,
    pub total_bytes_to_download: u64,
    pub total_bytes_uploaded: Arc<AtomicU64>,
    pub download_speed_tracker: Mutex<(Instant, u64, i64)>,
    pub upload_speed_tracker: Mutex<(Instant, u64, i64)>,
}

impl TransferMetrics {
    pub fn new(total_bytes_to_download: u64) -> Self {
        TransferMetrics {
            total_bytes_downloaded: Arc::new(AtomicU64::new(0)),
            initial_bytes_downloaded: 0,
            total_bytes_to_download,
            total_bytes_uploaded: Arc::new(AtomicU64::new(0)),
            download_speed_tracker: Mutex::new((Instant::now(), 0, 0)),
            upload_speed_tracker: Mutex::new((Instant::now(), 0, 0)),
        }
    }

    pub fn record_download(&self, bytes: u64) {
        self.total_bytes_downloaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_upload(&self, bytes: u64) {
        self.total_bytes_uploaded.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn progress_ppm(&self) -> u32 {
        if self.total_bytes_to_download == 0 {
            return 10000;
        }
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let ppm = (downloaded * 10000) / self.total_bytes_to_download;
        ppm.min(10000) as u32
    }

    pub fn progress_percent(&self) -> f32 {
        self.progress_ppm() as f32 / 100.0
    }

    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        if self.total_bytes_to_download < downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(self.total_bytes_to_download - downloaded)
    }

    pub fn bytes_per_second(&self) -> i64 {
        let (elapsed, delta_bytes, needs_update, now, current_bytes, old_bps) = {
            if let Ok(guard) = self.download_speed_tracker.lock() {
                let now = Instant::now();
                let elapsed = now.duration_since(guard.0);
                let current_bytes = self.total_bytes_downloaded.load(Ordering::Relaxed);
                if elapsed.as_millis() >= 500 {
                    (
                        elapsed,
                        current_bytes.saturating_sub(guard.1),
                        true,
                        now,
                        current_bytes,
                        guard.2,
                    )
                } else {
                    (elapsed, 0, false, now, current_bytes, guard.2)
                }
            } else {
                return 0;
            }
        };

        if needs_update {
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

            if let Ok(mut guard) = self.download_speed_tracker.lock() {
                *guard = (now, current_bytes, bps);
            }
            bps
        } else {
            old_bps
        }
    }

    pub fn upload_bytes_per_second(&self) -> i64 {
        let (elapsed, delta_bytes, needs_update, now, current_bytes, old_bps) = {
            if let Ok(guard) = self.upload_speed_tracker.lock() {
                let now = Instant::now();
                let elapsed = now.duration_since(guard.0);
                let current_bytes = self.total_bytes_uploaded.load(Ordering::Relaxed);
                if elapsed.as_millis() >= 500 {
                    (
                        elapsed,
                        current_bytes.saturating_sub(guard.1),
                        true,
                        now,
                        current_bytes,
                        guard.2,
                    )
                } else {
                    (elapsed, 0, false, now, current_bytes, guard.2)
                }
            } else {
                return 0;
            }
        };

        if needs_update {
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

            if let Ok(mut guard) = self.upload_speed_tracker.lock() {
                *guard = (now, current_bytes, bps);
            }
            bps
        } else {
            old_bps
        }
    }
}
