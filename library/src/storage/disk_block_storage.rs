//! Pure on-disk block storage implementation (Single Responsibility Principle).
//!
//! Provides `BlockReader` and `BlockWriter` over a set of files on disk,
//! resolving global torrent offsets into local file offsets and boundary-spanning I/O.

use alloc::string::String;
use alloc::vec::Vec;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use crate::error::BitTorrentError;
use crate::io_traits::{BlockReader, BlockWriter};
use crate::metainfo::FileDetails;

/// Information about a resolved contiguous slice of a read or write operation within a specific file.
#[derive(Debug, Clone)]
pub struct TargetFileRange {
    pub file_path: String,
    pub seek_offset: u64,
    pub io_length: usize,
}

/// Pure block-level storage engine for reading and writing data across one or more disk files.
#[derive(Debug, Clone)]
pub struct DiskBlockStorage {
    pub files: Vec<FileDetails>,
}

impl DiskBlockStorage {
    /// Creates a new `DiskBlockStorage` managing the given list of files.
    pub fn new(files: Vec<FileDetails>) -> Self {
        Self { files }
    }

    /// Computes the total storage capacity represented by all files combined.
    pub fn total_capacity(&self) -> u64 {
        self.files.iter().map(|f| f.length).sum()
    }

    /// Resolves a global byte range into target files and their corresponding local file offsets.
    pub fn resolve_file_ranges(&self, global_offset: u64, length: usize) -> Vec<TargetFileRange> {
        let mut ranges = Vec::new();
        let mut remaining = length;
        let mut current_offset = global_offset;

        for file in &self.files {
            if current_offset >= file.length {
                current_offset -= file.length;
                continue;
            }

            let write_start = current_offset;
            let write_size = std::cmp::min(remaining as u64, file.length - write_start) as usize;

            ranges.push(TargetFileRange {
                file_path: file.name.clone(),
                seek_offset: write_start,
                io_length: write_size,
            });

            remaining -= write_size;
            current_offset = 0;

            if remaining == 0 {
                break;
            }
        }
        ranges
    }
}

impl BlockWriter for DiskBlockStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        if data.is_empty() {
            return Ok(());
        }

        let total_capacity = self.total_capacity();
        let end = offset.checked_add(data.len() as u64).ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "write offset + length overflowed u64",
            ))
        })?;

        if end > total_capacity {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "write offset + length exceeds storage capacity",
            )));
        }

        let ranges = self.resolve_file_ranges(offset, data.len());
        let total_resolved: usize = ranges.iter().map(|r| r.io_length).sum();
        if total_resolved < data.len() {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Not enough space in torrent files to write the block",
            )));
        }

        let mut data_offset = 0usize;
        for range in &ranges {
            let mut handle = OpenOptions::new().write(true).open(&range.file_path)?;
            handle.seek(SeekFrom::Start(range.seek_offset))?;
            handle.write_all(&data[data_offset..data_offset + range.io_length])?;
            data_offset += range.io_length;
        }

        Ok(())
    }

    fn flush(&self) -> Result<(), BitTorrentError> {
        // Disk files written with standard unbuffered file handles are committed upon write completion.
        Ok(())
    }

    fn discard_range(&self, _offset: u64, _length: u64) {
        // No-op for direct file writes.
    }
}

impl BlockReader for DiskBlockStorage {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        if buffer.is_empty() {
            return Ok(0);
        }

        let total_capacity = self.total_capacity();
        let end = offset.checked_add(buffer.len() as u64).ok_or_else(|| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "read offset + length overflowed u64",
            ))
        })?;

        if end > total_capacity {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "read offset + length exceeds storage capacity",
            )));
        }

        let ranges = self.resolve_file_ranges(offset, buffer.len());
        let total_resolved: usize = ranges.iter().map(|r| r.io_length).sum();
        if total_resolved < buffer.len() {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Not enough data in torrent files to read the block",
            )));
        }

        let mut current_pos = 0;
        for range in &ranges {
            let mut handle = File::open(&range.file_path)?;
            handle.seek(SeekFrom::Start(range.seek_offset))?;
            handle.read_exact(&mut buffer[current_pos..current_pos + range.io_length])?;
            current_pos += range.io_length;
        }

        Ok(buffer.len())
    }
}
