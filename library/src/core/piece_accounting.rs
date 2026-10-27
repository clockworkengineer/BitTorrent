//! Piece accounting, bitfield state management, and file selection priorities.

use std::cmp::min;
use std::sync::atomic::Ordering;

use sha1::Digest;

use crate::constants::{BLOCK_SIZE, ENDGAME_THRESHOLD};
use crate::core::piece_store::PieceRarity;
use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::error::BitTorrentError;
use crate::metainfo::FilePriority;
use crate::peer::Peer;
use crate::util::get_bitfield_index_and_mask;

impl TorrentContext {
    /// Sets or clears a specific piece completion bit in the local bitfield.
    pub fn mark_piece_local(&mut self, piece_number: u32, local: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.bitfield.len() {
            if local {
                self.bitfield[byte_index] |= bit_mask;
            } else {
                self.bitfield[byte_index] &= !bit_mask;
            }
        }
        self.piece_store.mark_piece_local(piece_number, local);
        self.piece_manager.mark_piece_local(piece_number, local);

        if local {
            if let Some(ref eb) = self.event_broadcaster {
                let completed = self.piece_manager.completed_pieces_count();
                eb.emit(crate::session::events::SessionEvent::PieceCompleted {
                    piece_index: piece_number as usize,
                    total_completed: completed,
                    total_pieces: self.number_of_pieces,
                });
            }
        }
    }

    /// Checks if a specific piece has been fully downloaded and verified locally.
    pub fn is_piece_local(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.bitfield.len() {
            return self.bitfield[byte_index] & bit_mask != 0;
        }
        self.piece_store.is_piece_local(piece_number)
    }

    /// Sets or clears a specific piece index in the missing pieces tracking vector.
    pub fn mark_piece_missing(&mut self, piece_number: u32, missing: bool) {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.pieces_missing.len() {
            if missing {
                if self.pieces_missing[byte_index] & bit_mask == 0 {
                    self.pieces_missing[byte_index] |= bit_mask;
                    self.missing_pieces_count += 1;
                }
            } else if self.pieces_missing[byte_index] & bit_mask != 0 {
                self.pieces_missing[byte_index] &= !bit_mask;
                self.missing_pieces_count = self.missing_pieces_count.saturating_sub(1);
            }
        }
        self.piece_store.mark_piece_missing(piece_number, missing);
        self.piece_manager.mark_piece_missing(piece_number, missing);
    }

    /// Checks if a specific piece index is currently marked as missing.
    pub fn is_piece_missing(&self, piece_number: u32) -> bool {
        let (byte_index, bit_mask) = get_bitfield_index_and_mask(piece_number);
        if byte_index < self.pieces_missing.len() {
            return self.pieces_missing[byte_index] & bit_mask != 0;
        }
        self.piece_store.is_piece_missing(piece_number)
    }

    pub(crate) fn update_piece_peer_counts(&mut self, remote_peer: &Peer, increment: bool) {
        if self.piece_data.is_empty() || self.number_of_pieces == 0 {
            return;
        }
        let mut piece_number = 0u32;
        let mut pq = match self.piece_priority_queue.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        for byte in &remote_peer.remote_piece_bitfield {
            for bit in &[0x80u8, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01] {
                if (piece_number as usize) >= self.number_of_pieces
                    || (piece_number as usize) >= self.piece_data.len()
                {
                    return;
                }
                if byte & bit != 0 {
                    let idx = piece_number as usize;
                    if increment {
                        self.piece_data[idx].peer_count += 1;
                    } else {
                        self.piece_data[idx].peer_count =
                            self.piece_data[idx].peer_count.saturating_sub(1);
                    }
                    pq.push(PieceRarity {
                        peer_count: self.piece_data[idx].peer_count,
                        piece_index: piece_number,
                    });
                }
                piece_number += 1;
            }
        }
    }

    /// Increments peer counts for all pieces present in a newly connected peer's bitfield.
    pub fn merge_piece_bitfield(&mut self, remote_peer: &Peer) {
        self.update_piece_peer_counts(remote_peer, true);
    }

    /// Decrements piece peer counts when a peer disconnects.
    pub fn unmerge_piece_bitfield(&mut self, remote_peer: &Peer) {
        self.update_piece_peer_counts(remote_peer, false);
    }

    /// Returns the length of the specified piece.
    pub fn get_piece_length(&self, piece_number: u32) -> u32 {
        self.piece_data
            .get(piece_number as usize)
            .map(|p| p.piece_length)
            .unwrap_or(self.piece_length)
    }

    /// Returns the number of peers that have announced possession of the specified piece.
    pub fn get_piece_peer_count(&self, piece_number: u32) -> usize {
        self.piece_data
            .get(piece_number as usize)
            .map(|p| p.peer_count)
            .unwrap_or(0)
    }

    /// Sets the byte length for a given piece index.
    ///
    /// If `piece_length` is larger than the torrent's standard piece length, the call is
    /// ignored and an error is logged — the piece length is left unchanged.
    pub fn set_piece_length(&mut self, piece_number: u32, piece_length: u32) {
        if piece_length <= self.piece_length {
            if let Some(p) = self.piece_data.get_mut(piece_number as usize) {
                p.piece_length = piece_length;
            }
        } else {
            crate::log_debug!(
                "[context] set_piece_length: piece {} length {} exceeds torrent max {}; ignoring",
                piece_number,
                piece_length,
                self.piece_length
            );
        }
    }

    /// Computes the SHA-1 checksum of the piece buffer and compares it to the expected metainfo hash.
    pub fn check_piece_hash(
        &self,
        piece_number: u32,
        piece_buffer: &[u8],
        number_of_bytes: u32,
    ) -> bool {
        let hash = sha1::Sha1::digest(&piece_buffer[..number_of_bytes as usize]);
        let offset = piece_number as usize * crate::constants::HASH_LENGTH;
        hash.iter()
            .zip(&self.pieces_info_hash[offset..offset + crate::constants::HASH_LENGTH])
            .all(|(a, b)| a == b)
    }

    /// Computes the SHA-1 checksum of the piece by reading it block-by-block from storage.
    pub fn check_piece_hash_streaming(
        &self,
        storage: &dyn crate::io_traits::BlockStorage,
        piece_number: u32,
        number_of_bytes: u32,
    ) -> bool {
        let mut hasher = sha1::Sha1::new();
        let mut read_offset = piece_number as u64 * self.piece_length as u64;
        let mut remaining = number_of_bytes as usize;

        let mut temp_buf = [0u8; BLOCK_SIZE];
        while remaining > 0 {
            let to_read = std::cmp::min(remaining, BLOCK_SIZE);
            match storage.read_block(read_offset, &mut temp_buf[..to_read]) {
                Ok(n) if n == to_read => {
                    hasher.update(&temp_buf[..to_read]);
                    read_offset += to_read as u64;
                    remaining -= to_read;
                }
                _ => return false,
            }
        }

        let hash = hasher.finalize();
        let offset = piece_number as usize * crate::constants::HASH_LENGTH;
        hash.iter()
            .zip(&self.pieces_info_hash[offset..offset + crate::constants::HASH_LENGTH])
            .all(|(a, b)| a == b)
    }

    /// Parameter-driven update of bitfield status without buffer reference.
    pub fn update_bitfield_status(
        &mut self,
        piece_number: u32,
        piece_there: bool,
        number_of_bytes: u32,
    ) {
        if piece_there && !self.is_piece_local(piece_number) {
            self.total_bytes_downloaded
                .fetch_add(number_of_bytes as u64, Ordering::Relaxed);
        }
        self.set_piece_length(piece_number, number_of_bytes);
        self.mark_piece_local(piece_number, piece_there);
        self.mark_piece_missing(piece_number, !piece_there);
    }

    /// Returns the number of bytes remaining to be downloaded.
    ///
    /// # Errors
    /// Returns a [`BitTorrentError::Parse`] if the computed remaining bytes turn negative.
    pub fn bytes_left_to_download(&self) -> Result<u64, crate::error::BitTorrentError> {
        let downloaded = self.total_bytes_downloaded.load(Ordering::Relaxed);
        let total = if self.total_bytes_to_download > 0 {
            self.total_bytes_to_download
        } else {
            self.transfer_metrics
                .total_bytes_to_download
                .load(Ordering::Relaxed)
        };
        if total == 0 {
            return Ok(0);
        }
        if total < downloaded {
            return Err(crate::error::BitTorrentError::Parse(
                "Bytes left to download turned negative.".to_string(),
            ));
        }
        Ok(total - downloaded)
    }

    /// Integrates a verified piece buffer, updating local bitfields, download speeds, and completion metrics.
    pub fn update_bitfield_from_buffer(
        &mut self,
        piece_number: u32,
        piece_buffer: &[u8],
        number_of_bytes: u32,
    ) {
        let piece_there = self.check_piece_hash(piece_number, piece_buffer, number_of_bytes);
        self.update_bitfield_status(piece_number, piece_there, number_of_bytes);
    }

    /// Returns the download priority of a piece based on all files that overlap it.
    ///
    /// If multiple files overlap this piece, the highest priority among them is selected.
    /// If all overlapping files are `FilePriority::Skip`, the piece itself is skipped.
    pub fn get_piece_priority(&self, piece: u32) -> FilePriority {
        if self.files_to_download.is_empty() || self.piece_length == 0 {
            return FilePriority::Normal;
        }

        let piece_start = piece as u64 * self.piece_length as u64;
        let piece_end = if self.total_bytes_to_download > 0 {
            min(
                piece_start + self.piece_length as u64,
                self.total_bytes_to_download,
            )
        } else {
            piece_start + self.piece_length as u64
        };

        if piece_end <= piece_start {
            return FilePriority::Normal;
        }

        let mut max_priority = None;
        for file in &self.files_to_download {
            let file_start = file.offset;
            let file_end = file.offset + file.length;
            if file.length > 0 && file_start < piece_end && file_end > piece_start {
                max_priority = match max_priority {
                    None => Some(file.priority),
                    Some(p) => Some(std::cmp::max(p, file.priority)),
                };
            }
        }

        max_priority.unwrap_or(FilePriority::Normal)
    }

    /// Returns true if this piece is desired for download (i.e. not marked as `Skip`).
    pub fn is_piece_wanted(&self, piece: u32) -> bool {
        self.get_piece_priority(piece) != FilePriority::Skip
    }

    /// Sets the download priority for a file at the given index.
    ///
    /// If a file is skipped and this causes pieces to become un-wanted,
    /// any pending block reservations for those pieces are released.
    pub fn set_file_priority(
        &mut self,
        file_index: usize,
        priority: FilePriority,
    ) -> Result<(), BitTorrentError> {
        if file_index >= self.files_to_download.len() {
            return Err(BitTorrentError::Parse(format!(
                "File index {} out of range",
                file_index
            )));
        }

        self.files_to_download[file_index].priority = priority;

        // If priority is Skip, clear any pending reservations for pieces that are no longer wanted
        if priority == FilePriority::Skip {
            for piece in 0..self.number_of_pieces as u32 {
                if !self.is_piece_wanted(piece) {
                    self.clear_piece_requests(piece);
                }
            }
        } else {
            // Re-seed the priority queue with wanted missing pieces
            if let Ok(mut pq) = self.piece_priority_queue.lock() {
                pq.clear();
                for piece_index in 0..self.number_of_pieces as u32 {
                    if self.is_piece_wanted(piece_index) && !self.is_piece_local(piece_index) {
                        let count = self
                            .piece_data
                            .get(piece_index as usize)
                            .map(|p| p.peer_count)
                            .unwrap_or(0);
                        pq.push(PieceRarity {
                            peer_count: count,
                            piece_index,
                        });
                    }
                }
            }
        }

        Ok(())
    }

    /// Sets the list of selected 0-based file indices (BEP 53 `so=`).
    ///
    /// If files are already populated in this context, applies the selection immediately,
    /// setting unselected files to [`FilePriority::Skip`] and selected files to [`FilePriority::Normal`].
    pub fn set_selected_files(&mut self, indices: &[usize]) {
        let mut sorted = indices.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        self.selected_file_indices = Some(sorted.clone());
        self.storage_manager.set_selected_files(&sorted);
        if !self.files_to_download.is_empty() {
            self.apply_bep53_file_selection(&sorted);
        }
    }

    /// Applies BEP 53 file selection to existing files in this context.
    ///
    /// Elevates selected files to [`FilePriority::Normal`] if currently skipped,
    /// sets unselected files to [`FilePriority::Skip`], clears pending reservations
    /// for pieces that are no longer wanted, and re-seeds the piece priority queue.
    pub fn apply_bep53_file_selection(&mut self, selected_indices: &[usize]) {
        for (idx, file) in self.files_to_download.iter_mut().enumerate() {
            if selected_indices.contains(&idx) {
                if file.priority == FilePriority::Skip {
                    file.priority = FilePriority::Normal;
                }
            } else {
                file.priority = FilePriority::Skip;
            }
        }

        // Clear requests for pieces that became unwanted
        for piece in 0..self.number_of_pieces as u32 {
            if !self.is_piece_wanted(piece) {
                self.clear_piece_requests(piece);
            }
        }

        // Re-seed piece priority queue with wanted missing pieces
        if let Ok(mut pq) = self.piece_priority_queue.lock() {
            pq.clear();
            for piece_index in 0..self.number_of_pieces as u32 {
                if self.is_piece_wanted(piece_index) && !self.is_piece_local(piece_index) {
                    let count = self
                        .piece_data
                        .get(piece_index as usize)
                        .map(|p| p.peer_count)
                        .unwrap_or(0);
                    pq.push(PieceRarity {
                        peer_count: count,
                        piece_index,
                    });
                }
            }
        }
    }

    /// Returns the currently active BEP 53 selected file indices, if configured.
    pub fn selected_files(&self) -> Option<&[usize]> {
        self.selected_file_indices.as_deref()
    }

    /// Returns a BEP 53 selection evaluation comparing selected files to total torrent payload.
    pub fn bep53_evaluation(
        &self,
    ) -> Option<crate::network::metadata_fetcher::BEP53SelectionEvaluation> {
        if let Some(ref manifest) = self.expedited_manifest {
            let selected = self.selected_file_indices.as_deref().unwrap_or(&[]);
            return Some(manifest.evaluate_bep53_selection(selected));
        }

        if !self.files_to_download.is_empty() {
            let files = self
                .files_to_download
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    let start_piece = if self.piece_length == 0 {
                        0
                    } else {
                        (f.offset / self.piece_length as u64) as u32
                    };
                    let end_piece = if f.length == 0 || self.piece_length == 0 {
                        start_piece
                    } else {
                        ((f.offset + f.length - 1) / self.piece_length as u64) as u32
                    };
                    crate::network::metadata_fetcher::PartialFileInfo {
                        index: idx,
                        name: f.name.clone(),
                        length: f.length,
                        piece_range: (start_piece, end_piece),
                    }
                })
                .collect();

            let manifest = crate::network::metadata_fetcher::PartialMetadataManifest {
                name: self.file_name.clone(),
                piece_length: self.piece_length as u64,
                total_bytes: self.total_bytes_to_download,
                total_pieces: self.number_of_pieces as u32,
                files,
                has_complete_file_list: true,
            };
            let selected = self.selected_file_indices.as_deref().unwrap_or(&[]);
            return Some(manifest.evaluate_bep53_selection(selected));
        }

        None
    }

    /// Checks if the session download is complete (all wanted pieces downloaded).
    pub fn is_download_complete(&self) -> bool {
        if self.number_of_pieces == 0 || self.total_bytes_to_download == 0 {
            return false;
        }
        let mut any_wanted = false;
        for piece in 0..self.number_of_pieces as u32 {
            if self.is_piece_wanted(piece) {
                any_wanted = true;
                if !self.is_piece_local(piece) {
                    return false;
                }
            }
        }
        any_wanted
    }

    /// Returns true when the torrent has entered endgame mode.
    pub fn is_endgame(&self) -> bool {
        let missing_wanted = (0..self.number_of_pieces as u32)
            .filter(|&p| self.is_piece_wanted(p) && !self.is_piece_local(p))
            .take(ENDGAME_THRESHOLD + 1)
            .count();
        missing_wanted <= ENDGAME_THRESHOLD
    }

    /// Checks and updates status to `Seeding` if downloading is completed.
    pub fn try_complete_download(&mut self) {
        if self.status == TorrentStatus::Downloading && self.is_download_complete() {
            self.set_status(TorrentStatus::Seeding);
            self.download_finished.set();
            if let Some(ref eb) = self.event_broadcaster {
                eb.emit(crate::session::events::SessionEvent::StateChanged(TorrentStatus::Seeding));
                eb.emit(crate::session::events::SessionEvent::DownloadFinished);
            }
        }
    }

    /// Returns a tuple containing:
    /// 1. A boolean vector indicating which pieces are verified and present locally.
    /// 2. A frequency vector counting how many connected peers currently hold each piece.
    pub fn get_piece_availability(&self) -> (alloc::vec::Vec<bool>, alloc::vec::Vec<u16>) {
        let total = self.number_of_pieces;
        let mut local_have = alloc::vec![false; total];
        for (i, have) in local_have.iter_mut().enumerate() {
            *have = self.is_piece_local(i as u32);
        }

        let mut swarm_rarity = alloc::vec![0u16; total];
        if let Ok(swarm) = self.peer_swarm.read() {
            for peer_arc in swarm.values() {
                if let Ok(peer) = peer_arc.try_lock() {
                    for piece_idx in 0..total {
                        let (byte_idx, mask) =
                            crate::util::get_bitfield_index_and_mask(piece_idx as u32);
                        if (byte_idx as usize) < peer.remote_piece_bitfield.len()
                            && (peer.remote_piece_bitfield[byte_idx as usize] & mask) != 0
                        {
                            swarm_rarity[piece_idx] = swarm_rarity[piece_idx].saturating_add(1);
                        }
                    }
                }
            }
        }

        (local_have, swarm_rarity)
    }
}
