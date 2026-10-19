//! Disk I/O management
//!
//! Handles file system interactions including directory/file structure initialization,
//! scanning existing files to compute/verify the bitfield of local pieces, and writing downloaded pieces.

use crate::core::torrent_context::TorrentContext;
use crate::error::BitTorrentError;
use crate::io_traits::{BlockReader, BlockWriter};
use alloc::string::String;
use alloc::vec::Vec;
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Manager for orchestrating reading and writing torrent data blocks to and from disk.
#[derive(Debug)]
pub struct DiskIO {
    pub download_path: PathBuf,
    pub files_to_download: Vec<crate::metainfo::FileDetails>,
    pub piece_length: u32,
}

impl DiskIO {
    /// Creates a new `DiskIO` manager.
    pub fn new(
        download_path: impl AsRef<Path>,
        files_to_download: Vec<crate::metainfo::FileDetails>,
        piece_length: u32,
    ) -> Self {
        DiskIO {
            download_path: download_path.as_ref().to_path_buf(),
            files_to_download,
            piece_length,
        }
    }

    /// Pre-creates the files and directories on disk for the torrent, pre-allocating the correct file sizes.
    pub fn create_local_torrent_structure(&self) -> Result<(), BitTorrentError> {
        for file in &self.files_to_download {
            let path = Path::new(&file.name);
            if let Some(dir) = path.parent() {
                create_dir_all(dir)?;
            }
            if !path.exists() {
                let handle = File::create(path)?;
                handle.set_len(file.length)?;
            }
        }
        create_dir_all(&self.download_path)?;
        Ok(())
    }

    /// Scans the local files to build and initialize the torrent session's bitfield based on what is already on disk.
    pub fn create_torrent_bitfield(&self, tc: &mut TorrentContext) -> Result<(), BitTorrentError> {
        let mut piece_buffer = vec![0u8; self.piece_length as usize];
        let mut piece_number = 0u32;
        let mut bytes_in_buffer = 0usize;
        for file in &self.files_to_download {
            let mut handle = File::open(&file.name)?;
            loop {
                let bytes_read = handle.read(&mut piece_buffer[bytes_in_buffer..])?;
                if bytes_read == 0 {
                    break;
                }
                bytes_in_buffer += bytes_read;
                if bytes_in_buffer == piece_buffer.len() {
                    tc.update_bitfield_from_buffer(
                        piece_number,
                        &piece_buffer,
                        bytes_in_buffer as u32,
                    );
                    bytes_in_buffer = 0;
                    piece_number += 1;
                }
            }
        }
        if bytes_in_buffer > 0 {
            tc.update_bitfield_from_buffer(piece_number, &piece_buffer, bytes_in_buffer as u32);
        }
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
        let ranges = resolve_file_ranges_internal(&self.files_to_download, offset, data.len());

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
}

impl BlockReader for DiskIO {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        let ranges = resolve_file_ranges_internal(&self.files_to_download, offset, buffer.len());

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

struct TargetFileRange {
    file_path: String,
    seek_offset: u64,
    io_length: usize,
}

fn resolve_file_ranges_internal(
    files_to_download: &[crate::metainfo::FileDetails],
    global_offset: u64,
    length: usize,
) -> Vec<TargetFileRange> {
    let mut ranges = Vec::new();
    let mut remaining = length;
    let mut current_offset = global_offset;

    for file in files_to_download {
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
