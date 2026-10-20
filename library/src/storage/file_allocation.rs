//! File allocation and directory hierarchy management (Single Responsibility Principle).
//!
//! Handles creating directory trees and pre-allocating sparse/sized files on disk
//! independently of block read/write operations or bitfield hashing.

use crate::error::BitTorrentError;
use crate::metainfo::FileDetails;
use std::fs::{create_dir_all, File};
use std::path::Path;

/// Responsible exclusively for filesystem directory tree creation and file pre-allocation.
#[derive(Debug, Default, Clone, Copy)]
pub struct FileAllocationManager;

impl FileAllocationManager {
    /// Pre-creates the files and directories on disk for the torrent, pre-allocating the correct file sizes.
    pub fn allocate_torrent_structure(
        download_path: &Path,
        files: &[FileDetails],
    ) -> Result<(), BitTorrentError> {
        for file in files {
            let path = Path::new(&file.name);
            if let Some(dir) = path.parent() {
                create_dir_all(dir)?;
            }
            if !path.exists() {
                let handle = File::create(path)?;
                handle.set_len(file.length)?;
            }
        }
        create_dir_all(download_path)?;
        Ok(())
    }

    /// Verifies whether all files in the torrent structure exist on disk with expected sizes.
    pub fn verify_files_exist(files: &[FileDetails]) -> bool {
        files.iter().all(|file| {
            let path = Path::new(&file.name);
            match path.metadata() {
                Ok(meta) => meta.is_file() && meta.len() >= file.length,
                Err(_) => false,
            }
        })
    }

    /// Pre-allocates a single file with the specified length.
    pub fn preallocate_file(path: &Path, length: u64) -> Result<(), BitTorrentError> {
        if let Some(dir) = path.parent() {
            create_dir_all(dir)?;
        }
        let handle = File::create(path)?;
        handle.set_len(length)?;
        Ok(())
    }
}
