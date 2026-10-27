//! Storage engine, file layout mapping, block assembly, and verification coordinator (SRP & ISP).
//!
//! Encapsulates physical file storage, multi-file boundary offsets, file selection priority,
//! parallel cryptographic verification pooling, and endgame block staging.

use alloc::sync::Arc;
use alloc::vec::Vec;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::BitTorrentError;
use crate::io_traits::BlockStorage;
use crate::metainfo::{FileDetails, FilePriority};
use crate::storage::assembler::Assembler;
use crate::storage::erasure_staging::AdaptiveErasureStaging;
use crate::storage::verification_pool::HashVerificationPool;

/// Coordinates file mapping, block I/O operations, piece assembly, and verification pools.
pub struct StorageManager {
    /// Underlying block storage implementation (e.g. disk file, memory, or cached storage).
    pub storage: Arc<dyn BlockStorage>,
    /// Base destination directory for downloaded files on disk.
    pub download_path: PathBuf,
    /// Detailed metadata for every file in the torrent payload.
    pub files_to_download: Vec<FileDetails>,
    /// Selected 0-based file indices configured for selective download (BEP 53 `so=`).
    pub selected_file_indices: Option<Vec<usize>>,
    /// Assembler coordinating active block downloads and partial piece buffers.
    pub assembler: Assembler,
    /// Multi-worker thread pool for cryptographic SHA1 and BEP 52 v2 Merkle hash verification.
    pub verification_pool: Arc<HashVerificationPool>,
    /// Adaptive endgame cache for absorbing redundant blocks and straggler preemption.
    pub endgame_staging: Mutex<AdaptiveErasureStaging>,
}

impl StorageManager {
    /// Creates a new `StorageManager` with the specified storage driver and file catalog.
    pub fn new(
        storage: Arc<dyn BlockStorage>,
        download_path: impl AsRef<Path>,
        files_to_download: Vec<FileDetails>,
        verification_pool: Arc<HashVerificationPool>,
    ) -> Self {
        Self {
            storage,
            download_path: download_path.as_ref().to_path_buf(),
            files_to_download,
            selected_file_indices: None,
            assembler: Assembler::new(),
            verification_pool,
            endgame_staging: Mutex::new(AdaptiveErasureStaging::new(4, 1.5)),
        }
    }

    /// Reads a block of data from storage at the global torrent byte offset.
    pub fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        self.storage.read_block(offset, buffer)
    }

    /// Writes a block of data into storage at the global torrent byte offset.
    pub fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        self.storage.write_block(offset, data)
    }

    /// Flushes any buffered or cached writes to underlying persistent storage.
    pub fn flush(&self) -> Result<(), BitTorrentError> {
        self.storage.flush()
    }

    /// Total cumulative byte size across all files in the torrent payload.
    pub fn total_files_size(&self) -> u64 {
        self.files_to_download.iter().map(|f| f.length).sum()
    }

    /// Total number of distinct files in the torrent.
    pub fn file_count(&self) -> usize {
        self.files_to_download.len()
    }

    /// Configures the list of selected file indices, updating their priorities.
    pub fn set_selected_files(&mut self, indices: &[usize]) {
        let mut sorted = indices.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        self.selected_file_indices = Some(sorted.clone());

        for (idx, file) in self.files_to_download.iter_mut().enumerate() {
            if sorted.contains(&idx) {
                if file.priority == FilePriority::Skip {
                    file.priority = FilePriority::Normal;
                }
            } else {
                file.priority = FilePriority::Skip;
            }
        }
    }

    /// Returns the active selected file indices, if any selection is applied.
    pub fn selected_files(&self) -> Option<&[usize]> {
        self.selected_file_indices.as_deref()
    }

    /// Returns `true` if the file at `file_idx` is selected for download.
    pub fn is_file_selected(&self, file_idx: usize) -> bool {
        match &self.selected_file_indices {
            Some(indices) => indices.contains(&file_idx),
            None => true,
        }
    }

    /// Returns the priority assigned to the specified file.
    pub fn file_priority(&self, file_idx: usize) -> Option<FilePriority> {
        self.files_to_download.get(file_idx).map(|f| f.priority)
    }

    /// Sets the download priority for a specific file index.
    pub fn set_file_priority(&mut self, file_idx: usize, priority: FilePriority) -> bool {
        if let Some(file) = self.files_to_download.get_mut(file_idx) {
            file.priority = priority;
            true
        } else {
            false
        }
    }
}
