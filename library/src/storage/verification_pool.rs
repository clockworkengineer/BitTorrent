//! Asynchronous Piece Hash Verification Worker Pool
//!
//! Provides a multi-threaded background worker pool for offloading CPU-intensive
//! cryptographic hash verifications (SHA-1 for v1 and SHA-256 Merkle tree verification
//! for BitTorrent v2 / Hybrid) away from the cooperative async executor and network loops.

use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::constants::BLOCK_SIZE;
use crate::error::BitTorrentError;
use crate::io_traits::BlockStorage;

/// A piece hash verification job dispatched to the worker pool.
#[derive(Clone)]
pub struct PieceVerificationJob {
    /// Zero-based piece index.
    pub piece_number: u32,
    /// Total length of the piece in bytes.
    pub piece_length: u32,
    /// Absolute byte offset where the piece begins in storage.
    pub piece_offset: u64,
    /// Expected 20-byte SHA-1 hash for v1 / hybrid torrents.
    pub expected_v1_hash: Option<[u8; 20]>,
    /// Expected 32-byte SHA-256 root hash for v2 torrents.
    pub expected_v2_root: Option<[u8; 32]>,
    /// Block storage reference to stream piece blocks from if direct_buffer is None.
    pub storage: Option<Arc<dyn BlockStorage>>,
    /// Optional in-memory buffer containing the complete piece data.
    pub direct_buffer: Option<Arc<Vec<u8>>>,
    /// List of peer IPs that contributed blocks to this piece.
    pub block_sources: Vec<Option<String>>,
}

/// The result of a background piece hash verification.
#[derive(Debug, Clone)]
pub struct PieceVerificationResult {
    /// Zero-based piece index.
    pub piece_number: u32,
    /// Total length of the piece in bytes.
    pub piece_length: u32,
    /// Absolute byte offset where the piece begins in storage.
    pub piece_offset: u64,
    /// Whether the piece data matches all expected cryptographic hashes.
    pub is_valid: bool,
    /// Computed 20-byte SHA-1 hash if evaluated.
    pub actual_v1_hash: Option<[u8; 20]>,
    /// Expected 20-byte SHA-1 hash if provided.
    pub expected_v1_hash: Option<[u8; 20]>,
    /// Computed 32-byte SHA-256 Merkle root hash if evaluated.
    pub actual_v2_root: Option<[u8; 32]>,
    /// Expected 32-byte SHA-256 root hash if provided.
    pub expected_v2_root: Option<[u8; 32]>,
    /// Contributing peer IPs for accountability if verification failed.
    pub block_sources: Vec<Option<String>>,
    /// Microseconds spent hashing and validating this piece.
    pub elapsed_us: u64,
    /// Optional error message (e.g. storage I/O read failure).
    pub error: Option<String>,
}

/// Telemetry metrics for the hash verification worker pool.
#[derive(Debug, Clone, Default)]
pub struct VerificationPoolMetrics {
    /// Total number of piece verification jobs submitted to the pool.
    pub jobs_submitted: u64,
    /// Total number of piece verification jobs finished.
    pub jobs_completed: u64,
    /// Total number of pieces verified as cryptographically valid.
    pub valid_pieces: u64,
    /// Total number of pieces that failed hash verification.
    pub corrupted_pieces: u64,
    /// Total raw bytes read and hashed across all workers.
    pub total_bytes_verified: u64,
    /// Cumulative microseconds spent on CPU hashing.
    pub total_verification_time_us: u64,
    /// Number of active background worker threads.
    pub active_workers: usize,
    /// Current number of jobs queued awaiting execution.
    pub queue_depth: usize,
}

impl VerificationPoolMetrics {
    /// Returns the average verification time per piece in microseconds.
    pub fn avg_verification_time_us(&self) -> u64 {
        if self.jobs_completed == 0 {
            0
        } else {
            self.total_verification_time_us / self.jobs_completed
        }
    }

    /// Computes the aggregate hashing throughput in megabytes per second (MB/s).
    pub fn throughput_mb_s(&self) -> f64 {
        if self.total_verification_time_us == 0 {
            0.0
        } else {
            let bytes = self.total_bytes_verified as f64;
            let seconds = (self.total_verification_time_us as f64) / 1_000_000.0;
            (bytes / (1024.0 * 1024.0)) / seconds
        }
    }
}

/// Atomic progress tracker for parallel full-torrent verification and rechecks.
#[derive(Debug)]
pub struct RecheckProgress {
    /// Total number of pieces to verify.
    pub total_pieces: u32,
    /// Total byte count of the torrent.
    pub total_bytes: u64,
    /// Number of pieces verified so far.
    pub checked_pieces: AtomicU32,
    /// Number of pieces that passed hash verification.
    pub valid_pieces: AtomicU32,
    /// Number of pieces that failed hash verification.
    pub corrupt_pieces: AtomicU32,
    /// Total bytes verified so far.
    pub checked_bytes: AtomicU64,
    /// Whether all pieces have finished verification.
    pub is_complete: AtomicBool,
}

impl RecheckProgress {
    /// Creates a new progress tracker for the given piece and byte totals.
    pub fn new(total_pieces: u32, total_bytes: u64) -> Self {
        Self {
            total_pieces,
            total_bytes,
            checked_pieces: AtomicU32::new(0),
            valid_pieces: AtomicU32::new(0),
            corrupt_pieces: AtomicU32::new(0),
            checked_bytes: AtomicU64::new(0),
            is_complete: AtomicBool::new(total_pieces == 0),
        }
    }

    /// Returns the fractional completion progress in range `0.0..=1.0`.
    pub fn fraction_complete(&self) -> f32 {
        if self.total_pieces == 0 {
            1.0
        } else {
            let checked = self.checked_pieces.load(Ordering::Relaxed) as f32;
            (checked / self.total_pieces as f32).clamp(0.0, 1.0)
        }
    }

    /// Returns the progress percentage in range `0.0..=100.0`.
    pub fn percent_complete(&self) -> f32 {
        self.fraction_complete() * 100.0
    }

    /// Returns true if all pieces have finished verification.
    pub fn is_finished(&self) -> bool {
        self.is_complete.load(Ordering::Acquire)
    }
}

enum WorkerMessage {
    Verify(PieceVerificationJob, Option<Arc<RecheckProgress>>),
    Shutdown,
}

/// A multi-threaded worker pool for CPU-bound piece hash verifications.
pub struct HashVerificationPool {
    workers: Vec<JoinHandle<()>>,
    job_tx: Option<Sender<WorkerMessage>>,
    result_rx: Arc<Mutex<Receiver<PieceVerificationResult>>>,
    pending_jobs: Arc<AtomicUsize>,
    jobs_submitted: Arc<AtomicU64>,
    jobs_completed: Arc<AtomicU64>,
    valid_pieces: Arc<AtomicU64>,
    corrupted_pieces: Arc<AtomicU64>,
    total_bytes_verified: Arc<AtomicU64>,
    total_verification_time_us: Arc<AtomicU64>,
    worker_count: usize,
    queue_capacity: usize,
}

impl HashVerificationPool {
    /// Creates a new `HashVerificationPool` with the specified number of worker threads.
    ///
    /// Clamps worker count between 1 and 32 threads.
    pub fn new(workers: usize, queue_capacity: usize) -> Self {
        let worker_count = workers.clamp(1, 32);
        let queue_capacity = queue_capacity.max(16);

        let (job_tx, job_rx) = channel::<WorkerMessage>();
        let (result_tx, result_rx) = channel::<PieceVerificationResult>();

        let job_rx = Arc::new(Mutex::new(job_rx));
        let pending_jobs = Arc::new(AtomicUsize::new(0));
        let jobs_submitted = Arc::new(AtomicU64::new(0));
        let jobs_completed = Arc::new(AtomicU64::new(0));
        let valid_pieces = Arc::new(AtomicU64::new(0));
        let corrupted_pieces = Arc::new(AtomicU64::new(0));
        let total_bytes_verified = Arc::new(AtomicU64::new(0));
        let total_verification_time_us = Arc::new(AtomicU64::new(0));

        let mut worker_handles = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let rx = job_rx.clone();
            let tx = result_tx.clone();
            let p_jobs = pending_jobs.clone();
            let j_comp = jobs_completed.clone();
            let v_pieces = valid_pieces.clone();
            let c_pieces = corrupted_pieces.clone();
            let b_verified = total_bytes_verified.clone();
            let t_us = total_verification_time_us.clone();

            let handle = thread::Builder::new()
                .name(format!("bt-verifier-{}", worker_id))
                .spawn(move || {
                    loop {
                        let msg = {
                            let lock = rx.lock().unwrap();
                            lock.recv()
                        };

                        match msg {
                            Ok(WorkerMessage::Verify(job, progress_opt)) => {
                                let result = execute_verification_job(&job);

                                p_jobs.fetch_sub(1, Ordering::Relaxed);
                                j_comp.fetch_add(1, Ordering::Relaxed);
                                b_verified.fetch_add(result.piece_length as u64, Ordering::Relaxed);
                                t_us.fetch_add(result.elapsed_us, Ordering::Relaxed);

                                if result.is_valid {
                                    v_pieces.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    c_pieces.fetch_add(1, Ordering::Relaxed);
                                }

                                if let Some(ref progress) = progress_opt {
                                    let prev = progress.checked_pieces.fetch_add(1, Ordering::Relaxed);
                                    progress.checked_bytes.fetch_add(result.piece_length as u64, Ordering::Relaxed);
                                    if result.is_valid {
                                        progress.valid_pieces.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        progress.corrupt_pieces.fetch_add(1, Ordering::Relaxed);
                                    }
                                    if prev + 1 >= progress.total_pieces {
                                        progress.is_complete.store(true, Ordering::Release);
                                    }
                                }

                                let _ = tx.send(result);
                            }
                            Ok(WorkerMessage::Shutdown) | Err(_) => {
                                break;
                            }
                        }
                    }
                })
                .expect("Failed to spawn verification worker thread");

            worker_handles.push(handle);
        }

        Self {
            workers: worker_handles,
            job_tx: Some(job_tx),
            result_rx: Arc::new(Mutex::new(result_rx)),
            pending_jobs,
            jobs_submitted,
            jobs_completed,
            valid_pieces,
            corrupted_pieces,
            total_bytes_verified,
            total_verification_time_us,
            worker_count,
            queue_capacity,
        }
    }

    /// Submits a piece verification job to the background pool.
    ///
    /// # Errors
    /// Returns [`BitTorrentError::Capacity`](BitTorrentError::Capacity) if the queue is full,
    /// or [`BitTorrentError::Protocol`](BitTorrentError::Protocol) if the pool has been shut down.
    pub fn submit(&self, job: PieceVerificationJob) -> Result<(), BitTorrentError> {
        let current_depth = self.pending_jobs.load(Ordering::Relaxed);
        if current_depth >= self.queue_capacity {
            return Err(BitTorrentError::Protocol(
                "Hash verification worker pool queue is full".into(),
            ));
        }

        if let Some(ref tx) = self.job_tx {
            self.pending_jobs.fetch_add(1, Ordering::Relaxed);
            self.jobs_submitted.fetch_add(1, Ordering::Relaxed);
            tx.send(WorkerMessage::Verify(job, None))
                .map_err(|_| BitTorrentError::Protocol("Worker pool has stopped".into()))?;
            Ok(())
        } else {
            Err(BitTorrentError::Protocol("Worker pool is closed".into()))
        }
    }

    /// Drains and returns all currently available completed verification results without blocking.
    pub fn poll_results(&self) -> Vec<PieceVerificationResult> {
        let mut results = Vec::new();
        if let Ok(rx) = self.result_rx.lock() {
            while let Ok(res) = rx.try_recv() {
                results.push(res);
            }
        }
        results
    }

    /// Attempts to receive a single completed result without blocking.
    pub fn try_recv_result(&self) -> Option<PieceVerificationResult> {
        if let Ok(rx) = self.result_rx.lock() {
            rx.try_recv().ok()
        } else {
            None
        }
    }

    /// Waits up to `timeout` for a single completed result to become available.
    pub fn recv_result_timeout(&self, timeout: Duration) -> Option<PieceVerificationResult> {
        if let Ok(rx) = self.result_rx.lock() {
            rx.recv_timeout(timeout).ok()
        } else {
            None
        }
    }

    /// Captures a point-in-time snapshot of telemetry metrics.
    pub fn metrics(&self) -> VerificationPoolMetrics {
        VerificationPoolMetrics {
            jobs_submitted: self.jobs_submitted.load(Ordering::Relaxed),
            jobs_completed: self.jobs_completed.load(Ordering::Relaxed),
            valid_pieces: self.valid_pieces.load(Ordering::Relaxed),
            corrupted_pieces: self.corrupted_pieces.load(Ordering::Relaxed),
            total_bytes_verified: self.total_bytes_verified.load(Ordering::Relaxed),
            total_verification_time_us: self.total_verification_time_us.load(Ordering::Relaxed),
            active_workers: self.worker_count,
            queue_depth: self.pending_jobs.load(Ordering::Relaxed),
        }
    }

    /// Returns the number of background worker threads in this pool.
    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    /// Dispatches parallel verification of all torrent pieces across the worker pool,
    /// returning an atomic [`RecheckProgress`] tracking handle and the result receiver.
    pub fn recheck_pieces_parallel(
        &self,
        num_pieces: u32,
        piece_length: u32,
        total_bytes: u64,
        pieces_info_hash: &[u8],
        storage: Arc<dyn BlockStorage>,
    ) -> Result<Arc<RecheckProgress>, BitTorrentError> {
        let progress = Arc::new(RecheckProgress::new(num_pieces, total_bytes));

        if num_pieces == 0 {
            return Ok(progress);
        }

        let tx = self
            .job_tx
            .as_ref()
            .ok_or_else(|| BitTorrentError::Protocol("Worker pool is closed".into()))?;

        for piece in 0..num_pieces {
            let piece_offset = (piece as u64) * (piece_length as u64);
            let piece_len = if piece == num_pieces - 1 {
                let rem = (total_bytes % piece_length as u64) as u32;
                if rem == 0 {
                    piece_length
                } else {
                    rem
                }
            } else {
                piece_length
            };

            let expected_v1_hash = if (piece as usize * 20 + 20) <= pieces_info_hash.len() {
                let start = piece as usize * 20;
                let mut h = [0u8; 20];
                h.copy_from_slice(&pieces_info_hash[start..start + 20]);
                Some(h)
            } else {
                None
            };

            let job = PieceVerificationJob {
                piece_number: piece,
                piece_length: piece_len,
                piece_offset,
                expected_v1_hash,
                expected_v2_root: None,
                storage: Some(storage.clone()),
                direct_buffer: None,
                block_sources: Vec::new(),
            };

            self.pending_jobs.fetch_add(1, Ordering::Relaxed);
            self.jobs_submitted.fetch_add(1, Ordering::Relaxed);

            let _ = tx.send(WorkerMessage::Verify(job, Some(progress.clone())));
        }

        Ok(progress)
    }

    /// Performs a synchronous blocking recheck of all pieces using the worker pool in parallel,
    /// returning a packed bitfield and the completed progress tracker.
    pub fn recheck_pieces_blocking(
        &self,
        num_pieces: u32,
        piece_length: u32,
        total_bytes: u64,
        pieces_info_hash: &[u8],
        storage: Arc<dyn BlockStorage>,
    ) -> Result<(Vec<u8>, Arc<RecheckProgress>), BitTorrentError> {
        let progress = self.recheck_pieces_parallel(
            num_pieces,
            piece_length,
            total_bytes,
            pieces_info_hash,
            storage,
        )?;

        let num_bytes = (num_pieces as usize + 7) / 8;
        let mut bitfield = vec![0u8; num_bytes];

        let mut collected = 0;
        while collected < num_pieces {
            let res = self
                .recv_result_timeout(Duration::from_millis(100))
                .or_else(|| self.try_recv_result());

            if let Some(res) = res {
                collected += 1;
                if res.is_valid {
                    let (b_idx, mask) = crate::util::get_bitfield_index_and_mask(res.piece_number);
                    if b_idx < bitfield.len() {
                        bitfield[b_idx] |= mask;
                    }
                }
            } else if progress.is_finished() {
                // Drain any remaining buffered results
                for res in self.poll_results() {
                    if res.is_valid {
                        let (b_idx, mask) = crate::util::get_bitfield_index_and_mask(res.piece_number);
                        if b_idx < bitfield.len() {
                            bitfield[b_idx] |= mask;
                        }
                    }
                }
                break;
            }
        }

        Ok((bitfield, progress))
    }

    /// Shuts down all worker threads gracefully.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.job_tx.take() {
            for _ in 0..self.worker_count {
                let _ = tx.send(WorkerMessage::Shutdown);
            }
        }
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Default for HashVerificationPool {
    fn default() -> Self {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 8))
            .unwrap_or(2);
        Self::new(workers, 1024)
    }
}

impl Drop for HashVerificationPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Executes a single piece verification job, reading bytes and evaluating SHA-1 / SHA-256 hashes.
fn execute_verification_job(job: &PieceVerificationJob) -> PieceVerificationResult {
    let start = Instant::now();
    let mut actual_v1_hash = None;
    let mut actual_v2_root = None;
    let mut is_valid = true;
    let mut error = None;

    if let Some(ref buf) = job.direct_buffer {
        let len = (job.piece_length as usize).min(buf.len());
        let slice = &buf[..len];

        if let Some(expected_v1) = job.expected_v1_hash {
            use sha1::Digest;
            let digest = sha1::Sha1::digest(slice);
            let mut h = [0u8; 20];
            h.copy_from_slice(&digest);
            actual_v1_hash = Some(h);
            if h != expected_v1 {
                is_valid = false;
            }
        }

        if let Some(expected_v2) = job.expected_v2_root {
            let root = compute_v2_piece_merkle_root(slice);
            actual_v2_root = Some(root);
            if root != expected_v2 {
                is_valid = false;
            }
        }
    } else if let Some(ref storage) = job.storage {
        use sha1::Digest;
        let mut v1_hasher = if job.expected_v1_hash.is_some() {
            Some(sha1::Sha1::new())
        } else {
            None
        };

        let mut v2_leaf_hashes = if job.expected_v2_root.is_some() {
            Some(Vec::new())
        } else {
            None
        };

        let mut read_offset = job.piece_offset;
        let mut remaining = job.piece_length as usize;
        let mut temp_buf = [0u8; BLOCK_SIZE];
        let mut io_failed = false;

        while remaining > 0 {
            let to_read = remaining.min(BLOCK_SIZE);
            match storage.read_block(read_offset, &mut temp_buf[..to_read]) {
                Ok(n) if n == to_read => {
                    if let Some(ref mut h) = v1_hasher {
                        h.update(&temp_buf[..to_read]);
                    }
                    if let Some(ref mut leaves) = v2_leaf_hashes {
                        #[cfg(feature = "v2")]
                        {
                            use sha2::Digest as Sha2Digest;
                            let leaf = sha2::Sha256::digest(&temp_buf[..to_read]);
                            let mut lh = [0u8; 32];
                            lh.copy_from_slice(&leaf);
                            leaves.push(lh);
                        }
                    }
                    read_offset += to_read as u64;
                    remaining -= to_read;
                }
                Ok(n) => {
                    io_failed = true;
                    error = Some(format!(
                        "Short read from storage: expected {} bytes, got {}",
                        to_read, n
                    ));
                    break;
                }
                Err(e) => {
                    io_failed = true;
                    error = Some(format!("Storage read error: {}", e));
                    break;
                }
            }
        }

        if io_failed {
            is_valid = false;
        } else {
            if let Some(h) = v1_hasher {
                let digest = h.finalize();
                let mut actual = [0u8; 20];
                actual.copy_from_slice(&digest);
                actual_v1_hash = Some(actual);
                if let Some(expected) = job.expected_v1_hash {
                    if actual != expected {
                        is_valid = false;
                    }
                }
            }

            if let Some(leaves) = v2_leaf_hashes {
                let root = compute_merkle_root_from_leaves(&leaves);
                actual_v2_root = Some(root);
                if let Some(expected) = job.expected_v2_root {
                    if root != expected {
                        is_valid = false;
                    }
                }
            }
        }
    } else {
        is_valid = false;
        error = Some("No data source provided for verification job".into());
    }

    let elapsed_us = start.elapsed().as_micros() as u64;

    PieceVerificationResult {
        piece_number: job.piece_number,
        piece_length: job.piece_length,
        piece_offset: job.piece_offset,
        is_valid,
        actual_v1_hash,
        expected_v1_hash: job.expected_v1_hash,
        actual_v2_root,
        expected_v2_root: job.expected_v2_root,
        block_sources: job.block_sources.clone(),
        elapsed_us,
        error,
    }
}

/// Computes the root hash of a Merkle tree from an array of 32-byte leaf hashes.
fn compute_merkle_root_from_leaves(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    if leaves.len() == 1 {
        return leaves[0];
    }
    #[cfg(feature = "v2")]
    {
        use sha2::{Digest, Sha256};
        let p2 = leaves.len().next_power_of_two();
        let mut cur = leaves.to_vec();
        while cur.len() < p2 {
            cur.push([0u8; 32]);
        }
        while cur.len() > 1 {
            let mut next = Vec::with_capacity(cur.len() / 2);
            for pair in cur.chunks_exact(2) {
                let mut hasher = Sha256::new();
                hasher.update(&pair[0]);
                hasher.update(&pair[1]);
                let parent: [u8; 32] = hasher.finalize().into();
                next.push(parent);
            }
            cur = next;
        }
        cur[0]
    }
    #[cfg(not(feature = "v2"))]
    {
        [0u8; 32]
    }
}

/// Chunks raw piece data into 16 KiB blocks and computes the BEP 52 piece Merkle root.
fn compute_v2_piece_merkle_root(data: &[u8]) -> [u8; 32] {
    if data.is_empty() {
        return [0u8; 32];
    }
    #[cfg(feature = "v2")]
    {
        use sha2::{Digest, Sha256};
        let mut leaves = Vec::new();
        for chunk in data.chunks(BLOCK_SIZE) {
            let hash = Sha256::digest(chunk);
            let mut leaf = [0u8; 32];
            leaf.copy_from_slice(&hash);
            leaves.push(leaf);
        }
        compute_merkle_root_from_leaves(&leaves)
    }
    #[cfg(not(feature = "v2"))]
    {
        [0u8; 32]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::Digest;

    struct InMemoryStorage {
        data: Mutex<Vec<u8>>,
    }

    impl InMemoryStorage {
        fn new(size: usize) -> Self {
            Self {
                data: Mutex::new(vec![0x7A; size]),
            }
        }
    }

    impl crate::io_traits::BlockReader for InMemoryStorage {
        fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
            let data = self.data.lock().unwrap();
            let start = offset as usize;
            if start >= data.len() {
                return Ok(0);
            }
            let end = (start + buffer.len()).min(data.len());
            let to_copy = end - start;
            buffer[..to_copy].copy_from_slice(&data[start..end]);
            Ok(to_copy)
        }
    }

    impl crate::io_traits::BlockWriter for InMemoryStorage {
        fn write_block(&self, offset: u64, data_slice: &[u8]) -> Result<(), BitTorrentError> {
            let mut data = self.data.lock().unwrap();
            let start = offset as usize;
            let end = start + data_slice.len();
            if end > data.len() {
                data.resize(end, 0);
            }
            data[start..end].copy_from_slice(data_slice);
            Ok(())
        }
    }

    #[test]
    fn test_worker_pool_parallel_valid_verification() {
        let pool = HashVerificationPool::new(4, 64);
        let piece_len = 32768; // 32 KiB
        let storage = Arc::new(InMemoryStorage::new(piece_len * 4));

        let mut expected_hashes = Vec::new();
        for p in 0..4 {
            let offset = p * piece_len;
            let slice = &storage.data.lock().unwrap()[offset..offset + piece_len];
            let digest = sha1::Sha1::digest(slice);
            let mut h = [0u8; 20];
            h.copy_from_slice(&digest);
            expected_hashes.push(h);

            let job = PieceVerificationJob {
                piece_number: p as u32,
                piece_length: piece_len as u32,
                piece_offset: offset as u64,
                expected_v1_hash: Some(h),
                expected_v2_root: None,
                storage: Some(storage.clone()),
                direct_buffer: None,
                block_sources: vec![Some("192.168.1.1".into())],
            };
            pool.submit(job).expect("Submit job");
        }

        let mut completed = 0;
        let mut results = Vec::new();
        while completed < 4 {
            for res in pool.poll_results() {
                assert!(res.is_valid);
                assert_eq!(res.expected_v1_hash, Some(expected_hashes[res.piece_number as usize]));
                results.push(res);
                completed += 1;
            }
            thread::sleep(Duration::from_millis(5));
        }

        let metrics = pool.metrics();
        assert_eq!(metrics.jobs_completed, 4);
        assert_eq!(metrics.valid_pieces, 4);
        assert_eq!(metrics.corrupted_pieces, 0);
        assert_eq!(metrics.total_bytes_verified, (piece_len * 4) as u64);
        assert!(metrics.throughput_mb_s() >= 0.0);
    }

    #[test]
    fn test_worker_pool_corrupted_piece_detection() {
        let pool = HashVerificationPool::new(2, 32);
        let piece_len = 16384;
        let storage = Arc::new(InMemoryStorage::new(piece_len));

        let bogus_hash = [0xEE; 20];
        let job = PieceVerificationJob {
            piece_number: 0,
            piece_length: piece_len as u32,
            piece_offset: 0,
            expected_v1_hash: Some(bogus_hash),
            expected_v2_root: None,
            storage: Some(storage.clone()),
            direct_buffer: None,
            block_sources: vec![Some("10.0.0.1".into())],
        };

        pool.submit(job).expect("Submit job");

        let result = pool
            .recv_result_timeout(Duration::from_secs(1))
            .expect("Receive result");
        assert!(!result.is_valid);
        assert_eq!(result.expected_v1_hash, Some(bogus_hash));
        assert_ne!(result.actual_v1_hash, Some(bogus_hash));
        assert_eq!(result.block_sources, vec![Some("10.0.0.1".into())]);

        let metrics = pool.metrics();
        assert_eq!(metrics.corrupted_pieces, 1);
        assert_eq!(metrics.valid_pieces, 0);
    }

    #[test]
    fn test_direct_buffer_and_v2_merkle_verification() {
        let pool = HashVerificationPool::new(2, 32);
        let sample_data = vec![0xAB; 32768];
        let root = compute_v2_piece_merkle_root(&sample_data);

        let job = PieceVerificationJob {
            piece_number: 7,
            piece_length: sample_data.len() as u32,
            piece_offset: 0,
            expected_v1_hash: None,
            expected_v2_root: Some(root),
            storage: None,
            direct_buffer: Some(Arc::new(sample_data)),
            block_sources: Vec::new(),
        };

        pool.submit(job).expect("Submit job");
        let result = pool
            .recv_result_timeout(Duration::from_secs(1))
            .expect("Receive result");
        assert!(result.is_valid);
        assert_eq!(result.actual_v2_root, Some(root));
    }
}
