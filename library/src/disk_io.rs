//! Disk I/O management
//!
//! Handles file system interactions including directory/file structure initialization,
//! scanning existing files to compute/verify the bitfield of local pieces, and writing downloaded pieces.

use crate::error::BitTorrentError;
use crate::piece_buffer::PieceBuffer;
use crate::piece_request::PieceRequest;
use crate::torrent_context::TorrentContext;
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;

/// Manager for orchestrating reading and writing torrent data blocks to and from disk.
#[derive(Debug)]
pub struct DiskIO {
    pub piece_write_queue: Sender<PieceBuffer>,
    pub piece_request_queue: Sender<PieceRequest>,
    _worker_handle: Option<thread::JoinHandle<()>>,
}

impl DiskIO {
    /// Creates a new `DiskIO` manager and starts a background worker thread.
    pub fn new() -> Self {
        let (write_sender, write_receiver) = mpsc::channel::<PieceBuffer>();
        let (request_sender, request_receiver) = mpsc::channel::<PieceRequest>();

        let worker_handle = thread::spawn(move || {
            let _ = write_receiver;
            let _ = request_receiver;
            // Background disk tasks can be implemented later.
        });

        DiskIO {
            piece_write_queue: write_sender,
            piece_request_queue: request_sender,
            _worker_handle: Some(worker_handle),
        }
    }

    /// Pre-creates the files and directories on disk for the torrent, pre-allocating the correct file sizes.
    pub fn create_local_torrent_structure(
        &self,
        tc: &TorrentContext,
    ) -> Result<(), BitTorrentError> {
        for file in &tc.files_to_download {
            let path = Path::new(&file.name);
            if let Some(dir) = path.parent() {
                create_dir_all(dir)?;
            }
            if !path.exists() {
                let handle = File::create(path)?;
                handle.set_len(file.length)?;
            }
        }
        Ok(())
    }

    /// Scans the local files to build and initialize the torrent session's bitfield based on what is already on disk.
    pub fn create_torrent_bitfield(&self, tc: &mut TorrentContext) -> Result<(), BitTorrentError> {
        let mut piece_buffer = vec![0u8; tc.piece_length as usize];
        let mut piece_number = 0u32;
        let mut bytes_in_buffer = 0usize;
        let files: Vec<_> = tc.files_to_download.clone();
        for file in files {
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

    /// Writes a single fully downloaded and verified piece to the appropriate offset in the local files on disk.
    pub fn write_piece(
        &self,
        tc: &TorrentContext,
        piece_number: u32,
        piece_data: &[u8],
    ) -> Result<(), BitTorrentError> {
        let global_offset = piece_number as u64 * tc.piece_length as u64;
        let ranges = resolve_file_ranges(tc, global_offset, piece_data.len());

        let total_resolved: usize = ranges.iter().map(|r| r.io_length).sum();
        if total_resolved < piece_data.len() {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Not enough space in torrent files to write the piece",
            )));
        }

        let mut data_offset = 0usize;
        for range in &ranges {
            let mut handle = OpenOptions::new().write(true).open(&range.file_path)?;
            handle.seek(SeekFrom::Start(range.seek_offset))?;
            handle.write_all(&piece_data[data_offset..data_offset + range.io_length])?;
            data_offset += range.io_length;
        }

        Ok(())
    }

    /// Reads a block of data from the disk for the given piece, begin offset, and length.
    /// Mirrors `write_piece` but returns the bytes instead of writing them.
    pub fn read_piece_block(
        &self,
        tc: &TorrentContext,
        piece_number: u32,
        begin: u32,
        length: u32,
    ) -> Result<Vec<u8>, BitTorrentError> {
        let piece_length = tc.get_piece_length(piece_number);
        if begin.checked_add(length).map_or(true, |end| end > piece_length) {
            return Err(BitTorrentError::Parse(
                "Requested block exceeds piece bounds".into(),
            ));
        }

        let global_offset = piece_number as u64 * tc.piece_length as u64 + begin as u64;
        let ranges = resolve_file_ranges(tc, global_offset, length as usize);

        let total_resolved: usize = ranges.iter().map(|r| r.io_length).sum();
        if total_resolved < length as usize {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "Not enough data in torrent files to read the block",
            )));
        }

        let mut result = vec![0u8; length as usize];
        let mut current_pos = 0;
        for range in &ranges {
            let mut handle = File::open(&range.file_path)?;
            handle.seek(SeekFrom::Start(range.seek_offset))?;
            handle.read_exact(&mut result[current_pos..current_pos + range.io_length])?;
            current_pos += range.io_length;
        }

        Ok(result)
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
            if total_bytes_to_download / tc.piece_length as i64 != 0 {
                tc.set_piece_length(piece_number, tc.piece_length);
            } else {
                tc.set_piece_length(piece_number, total_bytes_to_download as u32);
            }
            total_bytes_to_download -= tc.piece_length as i64;
        }
        Ok(())
    }
}

struct TargetFileRange {
    file_path: String,
    seek_offset: u64,
    io_length: usize,
}

fn resolve_file_ranges(
    tc: &TorrentContext,
    global_offset: u64,
    length: usize,
) -> Vec<TargetFileRange> {
    let mut ranges = Vec::new();
    let mut remaining = length;
    let mut current_offset = global_offset;

    for file in &tc.files_to_download {
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
