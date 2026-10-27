//! Seekable streaming reader for torrent transfers.
//!
//! Provides [`TorrentStream`], implementing [`std::io::Read`] and [`std::io::Seek`]
//! over an active torrent session or arbitrary block storage. Supports sequential playback
//! and random seeking across whole torrents or individual files within multi-file torrents.

use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::error::BitTorrentError;
use crate::io_traits::{BlockReader, BlockStorage};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;
use crate::utils::util::LockExt;
use std::time::Duration;

/// Adapter allowing any [`BlockStorage`] implementation to be used as a [`BlockReader`].
#[derive(Clone)]
pub struct StorageBlockReader(pub Arc<dyn BlockStorage>);

impl std::fmt::Debug for StorageBlockReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageBlockReader").finish()
    }
}

impl BlockReader for StorageBlockReader {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        self.0.read_block(offset, buffer)
    }
}

/// A seekable streaming reader implementing [`std::io::Read`] and [`std::io::Seek`] over torrent data.
///
/// Designed for media players, video streamers, and on-demand random file access.
/// When seeking, it atomically updates an associated playback cursor piece so that the torrent's
/// piece selector can prioritize downloading pieces immediately around the read head.
pub struct TorrentStream {
    context: Arc<Mutex<TorrentContext>>,
    storage: Arc<dyn BlockReader>,
    start_offset: u64,
    end_offset: u64,
    position: u64,
    piece_length: u32,
    cursor_piece: Option<Arc<AtomicU32>>,
    read_timeout: Duration,
    poll_interval: Duration,
}

impl std::fmt::Debug for TorrentStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TorrentStream")
            .field("start_offset", &self.start_offset)
            .field("end_offset", &self.end_offset)
            .field("position", &self.position)
            .field("piece_length", &self.piece_length)
            .field("cursor_piece", &self.cursor_piece)
            .field("read_timeout", &self.read_timeout)
            .field("poll_interval", &self.poll_interval)
            .finish()
    }
}

impl TorrentStream {
    /// Creates a new seekable `TorrentStream` over a given byte range in the torrent.
    ///
    /// # Arguments
    /// * `context` - The torrent session context tracking piece availability and bitfields.
    /// * `storage` - The underlying block reader to fetch verified piece blocks from.
    /// * `start_offset` - The global byte offset where this stream begins.
    /// * `length` - The total byte length of this stream.
    /// * `cursor_piece` - Optional atomic cursor tracking the active playback piece index.
    pub fn new(
        context: Arc<Mutex<TorrentContext>>,
        storage: Arc<dyn BlockReader>,
        start_offset: u64,
        length: u64,
        cursor_piece: Option<Arc<AtomicU32>>,
    ) -> Self {
        let piece_length = {
            let ctx = context.lock_safe();
            ctx.piece_length
        };
        let end_offset = start_offset.saturating_add(length);
        if let Some(ref cursor) = cursor_piece {
            if piece_length > 0 {
                let piece = (start_offset / piece_length as u64) as u32;
                cursor.store(piece, Ordering::Relaxed);
            }
        }
        Self {
            context,
            storage,
            start_offset,
            end_offset,
            position: 0,
            piece_length,
            cursor_piece,
            read_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(50),
        }
    }

    /// Creates a seekable `TorrentStream` for a specific file index in the torrent,
    /// using the context's underlying storage.
    ///
    /// # Errors
    /// Returns [`BitTorrentError::Parse`] if `file_index` is out of bounds.
    pub fn for_file(
        context: Arc<Mutex<TorrentContext>>,
        file_index: usize,
        cursor_piece: Option<Arc<AtomicU32>>,
    ) -> Result<Self, BitTorrentError> {
        let (storage, start_offset, length) = {
            let ctx = context.lock_safe();
            let file = ctx.files_to_download.get(file_index).ok_or_else(|| {
                BitTorrentError::Parse(format!(
                    "File index {} out of bounds (total files: {})",
                    file_index,
                    ctx.files_to_download.len()
                ))
            })?;
            let reader: Arc<dyn BlockReader> = Arc::new(StorageBlockReader(ctx.storage.clone()));
            (reader, file.offset, file.length)
        };

        Ok(Self::new(
            context,
            storage,
            start_offset,
            length,
            cursor_piece,
        ))
    }

    /// Creates a seekable `TorrentStream` for a specific file index with a custom block reader.
    ///
    /// # Errors
    /// Returns [`BitTorrentError::Parse`] if `file_index` is out of bounds.
    pub fn for_file_with_storage(
        context: Arc<Mutex<TorrentContext>>,
        storage: Arc<dyn BlockReader>,
        file_index: usize,
        cursor_piece: Option<Arc<AtomicU32>>,
    ) -> Result<Self, BitTorrentError> {
        let (start_offset, length) = {
            let ctx = context.lock_safe();
            let file = ctx.files_to_download.get(file_index).ok_or_else(|| {
                BitTorrentError::Parse(format!(
                    "File index {} out of bounds (total files: {})",
                    file_index,
                    ctx.files_to_download.len()
                ))
            })?;
            (file.offset, file.length)
        };

        Ok(Self::new(
            context,
            storage,
            start_offset,
            length,
            cursor_piece,
        ))
    }

    /// Configures the maximum timeout duration to wait for missing pieces during read operations.
    ///
    /// If set to [`Duration::ZERO`], reads will immediately return a [`std::io::ErrorKind::WouldBlock`]
    /// error if the required piece is not available locally.
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// Configures the polling interval when waiting for missing pieces to finish downloading.
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Returns a reference to the active atomic cursor piece, if streaming selection is configured.
    pub fn cursor_piece(&self) -> Option<&Arc<AtomicU32>> {
        self.cursor_piece.as_ref()
    }

    /// Returns the total stream length in bytes.
    pub fn total_len(&self) -> u64 {
        self.end_offset.saturating_sub(self.start_offset)
    }

    /// Alias for [`total_len`](Self::total_len).
    pub fn stream_len(&self) -> u64 {
        self.total_len()
    }

    /// Returns the current stream read position in bytes relative to the start of this stream.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Returns the global absolute byte position within the torrent data.
    pub fn global_position(&self) -> u64 {
        self.start_offset.saturating_add(self.position)
    }

    fn wait_for_piece(&self, piece: u32) -> std::io::Result<()> {
        let start = std::time::Instant::now();

        loop {
            {
                let ctx = self.context.lock_safe();
                if ctx.is_piece_local(piece) {
                    return Ok(());
                }
                if ctx.status == TorrentStatus::Ended {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionAborted,
                        "Torrent session has ended while waiting for piece",
                    ));
                }
            }

            if self.read_timeout.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    format!("Piece {} is not currently available locally", piece),
                ));
            }

            if start.elapsed() >= self.read_timeout {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "Timed out waiting for piece {} after {:?}",
                        piece, self.read_timeout
                    ),
                ));
            }

            std::thread::sleep(self.poll_interval);
        }
    }
}

impl Read for TorrentStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let stream_len = self.total_len();
        if self.position >= stream_len {
            return Ok(0); // EOF
        }

        let max_to_read = (buf.len() as u64).min(stream_len - self.position) as usize;
        let mut total_bytes_read = 0;

        while total_bytes_read < max_to_read {
            let current_global_pos = self.start_offset + self.position;
            let current_piece = if self.piece_length > 0 {
                (current_global_pos / self.piece_length as u64) as u32
            } else {
                0
            };

            // Update streaming cursor piece so peer workers focus here
            if let Some(ref cursor) = self.cursor_piece {
                cursor.store(current_piece, Ordering::Relaxed);
            }

            // Check if piece is local. If not, wait for it or return early if partial read done.
            let is_local = {
                let ctx = self.context.lock_safe();
                ctx.is_piece_local(current_piece)
            };

            if !is_local {
                if total_bytes_read > 0 {
                    // Return bytes already read without blocking on the next missing piece
                    break;
                }

                self.wait_for_piece(current_piece)?;
            }

            // Piece is local, calculate remaining bytes in this piece
            let piece_start = (current_piece as u64) * self.piece_length as u64;
            let piece_offset = current_global_pos.saturating_sub(piece_start);
            let piece_len = {
                let ctx = self.context.lock_safe();
                ctx.get_piece_length(current_piece) as u64
            };

            let piece_remaining = piece_len.saturating_sub(piece_offset);
            if piece_remaining == 0 {
                // Reached the end of this piece boundary
                break;
            }

            let needed = max_to_read - total_bytes_read;
            let chunk_size = (needed as u64).min(piece_remaining) as usize;

            let bytes_read = self
                .storage
                .read_block(
                    current_global_pos,
                    &mut buf[total_bytes_read..total_bytes_read + chunk_size],
                )
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

            if bytes_read == 0 {
                break;
            }

            total_bytes_read += bytes_read;
            self.position += bytes_read as u64;

            if bytes_read < chunk_size {
                break;
            }
        }

        // Update cursor piece after advancing position
        if let Some(ref cursor) = self.cursor_piece {
            if self.piece_length > 0 {
                let next_global = self.start_offset + self.position;
                let next_piece = (next_global / self.piece_length as u64) as u32;
                cursor.store(next_piece, Ordering::Relaxed);
            }
        }

        Ok(total_bytes_read)
    }
}

impl Seek for TorrentStream {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let stream_len = self.total_len();
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(offset) => {
                let cur = self.position as i128;
                let target = cur + (offset as i128);
                if target < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Cannot seek to a negative position",
                    ));
                }
                target as u64
            }
            SeekFrom::End(offset) => {
                let end = stream_len as i128;
                let target = end + (offset as i128);
                if target < 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "Cannot seek to a negative position",
                    ));
                }
                target as u64
            }
        };

        self.position = new_pos;

        if let Some(ref cursor) = self.cursor_piece {
            if self.piece_length > 0 {
                let global_pos = self.start_offset + self.position;
                let current_piece = (global_pos / self.piece_length as u64) as u32;
                cursor.store(current_piece, Ordering::Relaxed);
            }
        }

        Ok(self.position)
    }
}

