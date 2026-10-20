//! Disk I/O management
//!
//! Orchestrates file system interactions including directory/file structure initialization,
//! scanning existing files to compute/verify the bitfield of local pieces, and writing downloaded pieces.
//!
//! Refactored under SOLID principles to delegate responsibilities to dedicated components:
//! - [`FileAllocationManager`]: Filesystem directory creation and file sizing.
//! - [`BitfieldScanner`]: Pure calculation of local piece hashes and bitfield verification.
//! - [`DiskBlockStorage`]: Boundary-crossing block-level read and write I/O.

use crate::core::torrent_context::TorrentContext;
use crate::error::BitTorrentError;
use crate::io_traits::{BlockReader, BlockWriter};
use alloc::vec::Vec;
use std::path::{Path, PathBuf};

pub use crate::storage::bitfield_scanner::{BitfieldScanner, PieceScanStatus, ScanResult};
pub use crate::storage::disk_block_storage::{DiskBlockStorage, TargetFileRange};
pub use crate::storage::file_allocation::FileAllocationManager;

/// Manager for orchestrating reading and writing torrent data blocks to and from disk.
///
/// Decomposed under the Single Responsibility Principle into focused domain services
/// while maintaining complete backward compatibility.
#[derive(Debug)]
pub struct DiskIO {
    pub download_path: PathBuf,
    pub files_to_download: Vec<crate::metainfo::FileDetails>,
    pub piece_length: u32,
    block_storage: DiskBlockStorage,
}

impl DiskIO {
    /// Creates a new `DiskIO` manager.
    pub fn new(
        download_path: impl AsRef<Path>,
        files_to_download: Vec<crate::metainfo::FileDetails>,
        piece_length: u32,
    ) -> Self {
        let block_storage = DiskBlockStorage::new(files_to_download.clone());
        DiskIO {
            download_path: download_path.as_ref().to_path_buf(),
            files_to_download,
            piece_length,
            block_storage,
        }
    }

    /// Returns a reference to the underlying pure block storage engine.
    pub fn block_storage(&self) -> &DiskBlockStorage {
        &self.block_storage
    }

    /// Pre-creates the files and directories on disk for the torrent, pre-allocating the correct file sizes.
    ///
    /// Delegates directly to [`FileAllocationManager`] (SRP).
    pub fn create_local_torrent_structure(&self) -> Result<(), BitTorrentError> {
        FileAllocationManager::allocate_torrent_structure(&self.download_path, &self.files_to_download)
    }

    /// Scans the local files to build and initialize the torrent session's bitfield based on what is already on disk.
    ///
    /// Delegates calculation to [`BitfieldScanner`] (SRP).
    pub fn create_torrent_bitfield(&self, tc: &mut TorrentContext) -> Result<(), BitTorrentError> {
        let scan_result = BitfieldScanner::scan_local_files(
            &self.files_to_download,
            self.piece_length,
            &tc.pieces_info_hash,
        )?;
        BitfieldScanner::apply_to_context(tc, &scan_result);
        Ok(())
    }

    /// Scans and verifies local files using the multi-threaded verification worker pool in parallel,
    /// updating the context bitfield and returning the completed progress tracker.
    pub fn create_torrent_bitfield_parallel(
        &self,
        tc: &mut TorrentContext,
        pool: &crate::storage::verification_pool::HashVerificationPool,
    ) -> Result<std::sync::Arc<crate::storage::verification_pool::RecheckProgress>, BitTorrentError>
    {
        use std::sync::atomic::Ordering;
        let storage = tc.storage.clone();
        let (bitfield, progress) = pool.recheck_pieces_blocking(
            tc.number_of_pieces as u32,
            self.piece_length,
            tc.total_bytes_to_download,
            &tc.pieces_info_hash,
            storage,
        )?;

        tc.bitfield = bitfield;
        tc.pieces_missing = vec![0u8; tc.bitfield.len()];
        let mut downloaded = 0u64;
        let mut missing_count = 0usize;

        for p in 0..tc.number_of_pieces as u32 {
            let (b_idx, mask) = crate::util::get_bitfield_index_and_mask(p);
            let local = tc.bitfield[b_idx] & mask != 0;
            let len = if p == tc.number_of_pieces as u32 - 1 {
                let rem = (tc.total_bytes_to_download % self.piece_length as u64) as u32;
                if rem == 0 { self.piece_length } else { rem }
            } else {
                self.piece_length
            };
            tc.set_piece_length(p, len);
            if local {
                downloaded += len as u64;
            } else {
                tc.pieces_missing[b_idx] |= mask;
                missing_count += 1;
            }
        }

        tc.missing_pieces_count = missing_count;
        tc.total_bytes_downloaded
            .store(downloaded, Ordering::Relaxed);
        tc.initial_bytes_downloaded = downloaded;

        Ok(progress)
    }

    /// Marks all pieces of the torrent as locally complete in the context (forcing a fully downloaded state).
    pub fn fully_downloaded_torrent_bitfield(
        &self,
        tc: &mut TorrentContext,
    ) -> Result<(), BitTorrentError> {
        let mut total_bytes_to_download = tc.total_bytes_to_download as i64;
        for piece_number in 0..tc.number_of_pieces as u32 {
            tc.mark_piece_local(piece_number, true);
            tc.mark_piece_missing(piece_number, false);
            if total_bytes_to_download / self.piece_length as i64 != 0 {
                tc.set_piece_length(piece_number, self.piece_length);
            } else {
                tc.set_piece_length(piece_number, total_bytes_to_download as u32);
            }
            total_bytes_to_download -= self.piece_length as i64;
        }
        Ok(())
    }
}

impl BlockWriter for DiskIO {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        self.block_storage.write_block(offset, data)
    }

    fn flush(&self) -> Result<(), BitTorrentError> {
        self.block_storage.flush()
    }

    fn discard_range(&self, offset: u64, length: u64) {
        self.block_storage.discard_range(offset, length);
    }
}

impl BlockReader for DiskIO {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        self.block_storage.read_block(offset, buffer)
    }
}
