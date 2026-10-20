//! Bitfield scanning and local piece hash verification (Single Responsibility Principle).
//!
//! Provides pure, side-effect-free calculation of piece verification bitfields
//! across disk files or abstract `BlockStorage` sources without mutating `TorrentContext`.

use alloc::vec::Vec;
use sha1::Digest;
use std::fs::File;
use std::io::Read;

use crate::constants::HASH_LENGTH;
use crate::core::torrent_context::TorrentContext;
use crate::error::BitTorrentError;
use crate::io_traits::BlockStorage;
use crate::metainfo::FileDetails;
use crate::util::get_bitfield_index_and_mask;

/// Status result for an individual piece after hash verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PieceScanStatus {
    pub piece_index: u32,
    pub is_valid: bool,
    pub length: u32,
}

/// Aggregated result of a complete torrent bitfield scan.
#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    /// Bitfield byte vector indicating piece availability (1 = verified local piece).
    pub bitfield: Vec<u8>,
    /// Per-piece status entries detailing validation status and length in bytes.
    pub piece_statuses: Vec<PieceScanStatus>,
    /// Total number of valid downloaded bytes identified across all verified pieces.
    pub downloaded_bytes: u64,
}

/// Pure scanner for verifying local pieces and computing bitfield states.
#[derive(Debug, Default, Clone, Copy)]
pub struct BitfieldScanner;

impl BitfieldScanner {
    /// Scans disk files sequentially and verifies each piece's SHA-1 hash against expected info hashes.
    pub fn scan_local_files(
        files: &[FileDetails],
        piece_length: u32,
        expected_hashes: &[u8],
    ) -> Result<ScanResult, BitTorrentError> {
        if piece_length == 0 || expected_hashes.is_empty() {
            return Ok(ScanResult::default());
        }

        let num_pieces = expected_hashes.len() / HASH_LENGTH;
        let bitfield_len = (num_pieces + 7) / 8;
        let mut bitfield = vec![0u8; bitfield_len];
        let mut piece_statuses = Vec::with_capacity(num_pieces);
        let mut downloaded_bytes = 0u64;

        let mut piece_buffer = vec![0u8; piece_length as usize];
        let mut piece_number = 0u32;
        let mut bytes_in_buffer = 0usize;

        for file in files {
            let mut handle = match File::open(&file.name) {
                Ok(h) => h,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // File has not been created or written to yet; skip reading.
                    continue;
                }
                Err(e) => return Err(BitTorrentError::Io(e)),
            };

            loop {
                let bytes_read = handle.read(&mut piece_buffer[bytes_in_buffer..])?;
                if bytes_read == 0 {
                    break;
                }
                bytes_in_buffer += bytes_read;
                if bytes_in_buffer == piece_buffer.len() {
                    let is_valid = Self::verify_hash(
                        piece_number,
                        &piece_buffer[..bytes_in_buffer],
                        expected_hashes,
                    );
                    if is_valid {
                        let (b_idx, mask) = get_bitfield_index_and_mask(piece_number);
                        if b_idx < bitfield.len() {
                            bitfield[b_idx] |= mask;
                        }
                        downloaded_bytes += bytes_in_buffer as u64;
                    }
                    piece_statuses.push(PieceScanStatus {
                        piece_index: piece_number,
                        is_valid,
                        length: bytes_in_buffer as u32,
                    });
                    bytes_in_buffer = 0;
                    piece_number += 1;
                }
            }
        }

        if bytes_in_buffer > 0 {
            let is_valid = Self::verify_hash(
                piece_number,
                &piece_buffer[..bytes_in_buffer],
                expected_hashes,
            );
            if is_valid {
                let (b_idx, mask) = get_bitfield_index_and_mask(piece_number);
                if b_idx < bitfield.len() {
                    bitfield[b_idx] |= mask;
                }
                downloaded_bytes += bytes_in_buffer as u64;
            }
            piece_statuses.push(PieceScanStatus {
                piece_index: piece_number,
                is_valid,
                length: bytes_in_buffer as u32,
            });
        }

        Ok(ScanResult {
            bitfield,
            piece_statuses,
            downloaded_bytes,
        })
    }

    /// Scans an abstract `BlockStorage` source block-by-block without filesystem dependencies.
    pub fn scan_storage(
        storage: &dyn BlockStorage,
        number_of_pieces: u32,
        piece_length: u32,
        total_bytes: u64,
        expected_hashes: &[u8],
    ) -> Result<ScanResult, BitTorrentError> {
        let bitfield_len = ((number_of_pieces as usize) + 7) / 8;
        let mut bitfield = vec![0u8; bitfield_len];
        let mut piece_statuses = Vec::with_capacity(number_of_pieces as usize);
        let mut downloaded_bytes = 0u64;

        let mut buffer = vec![0u8; piece_length as usize];

        for piece in 0..number_of_pieces {
            let offset = piece as u64 * piece_length as u64;
            let length = if piece == number_of_pieces - 1 {
                let rem = (total_bytes % piece_length as u64) as u32;
                if rem == 0 { piece_length } else { rem }
            } else {
                piece_length
            };

            let read_slice = &mut buffer[..length as usize];
            let is_valid = match storage.read_block(offset, read_slice) {
                Ok(n) if n == length as usize => {
                    Self::verify_hash(piece, read_slice, expected_hashes)
                }
                _ => false,
            };

            if is_valid {
                let (b_idx, mask) = get_bitfield_index_and_mask(piece);
                if b_idx < bitfield.len() {
                    bitfield[b_idx] |= mask;
                }
                downloaded_bytes += length as u64;
            }

            piece_statuses.push(PieceScanStatus {
                piece_index: piece,
                is_valid,
                length,
            });
        }

        Ok(ScanResult {
            bitfield,
            piece_statuses,
            downloaded_bytes,
        })
    }

    /// Verifies the SHA-1 hash of a piece buffer against the expected hash in the info hash slice.
    fn verify_hash(piece_number: u32, buffer: &[u8], expected_hashes: &[u8]) -> bool {
        let hash = sha1::Sha1::digest(buffer);
        let offset = piece_number as usize * HASH_LENGTH;
        if offset + HASH_LENGTH > expected_hashes.len() {
            return false;
        }
        hash.as_slice() == &expected_hashes[offset..offset + HASH_LENGTH]
    }

    /// Applies a scan result to a `TorrentContext`, synchronizing piece statuses, bitfield, and speed trackers.
    pub fn apply_to_context(tc: &mut TorrentContext, scan_result: &ScanResult) {
        for status in &scan_result.piece_statuses {
            tc.update_bitfield_status(status.piece_index, status.is_valid, status.length);
        }
    }
}
