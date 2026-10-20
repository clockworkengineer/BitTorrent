//! BEP 9 metadata exchange pieces collection and dictionary assembly.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use std::time::Instant;

/// Tracks incoming BEP 9 metadata pieces and builds the completed torrent info dictionary.
#[derive(Default)]
pub struct MetadataBootstrap {
    pub metadata_size: Option<usize>,
    pub metadata_pieces: BTreeMap<u32, Vec<u8>>,
    pub requested_metadata_pieces: BTreeMap<u32, Instant>,
    pub info_dict_bytes: Option<Vec<u8>>,
    pub expedited_manifest: Option<crate::network::metadata_fetcher::PartialMetadataManifest>,
}

impl MetadataBootstrap {
    pub fn new() -> Self {
        MetadataBootstrap::default()
    }

    pub fn set_metadata_size(&mut self, size: usize) {
        self.metadata_size = Some(size);
    }

    pub fn add_piece(&mut self, piece_index: u32, data: Vec<u8>) -> bool {
        self.metadata_pieces.insert(piece_index, data);
        self.requested_metadata_pieces.remove(&piece_index);
        self.is_complete()
    }

    pub fn is_complete(&self) -> bool {
        let size = match self.metadata_size {
            Some(s) if s > 0 => s,
            _ => return false,
        };
        let expected_pieces = (size + crate::constants::BLOCK_SIZE as usize - 1)
            / crate::constants::BLOCK_SIZE as usize;
        self.metadata_pieces.len() == expected_pieces
    }

    pub fn assemble_info_dict(&mut self) -> Option<Vec<u8>> {
        if !self.is_complete() {
            return None;
        }
        let total_size = self.metadata_size?;
        let mut buffer = Vec::with_capacity(total_size);
        for (_, piece) in &self.metadata_pieces {
            buffer.extend_from_slice(piece);
        }
        buffer.truncate(total_size);
        self.info_dict_bytes = Some(buffer.clone());
        Some(buffer)
    }
}
