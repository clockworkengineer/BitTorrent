use crate::error::BitTorrentError;
use crate::piece_buffer::PieceBuffer;
use crate::piece_request::PieceRequest;
use crate::torrent_context::TorrentContext;
use std::fs::{create_dir_all, File};
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;

#[derive(Debug)]
pub struct DiskIO {
    pub piece_write_queue: Sender<PieceBuffer>,
    pub piece_request_queue: Sender<PieceRequest>,
    _worker_handle: Option<thread::JoinHandle<()>>,
}

impl DiskIO {
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

    pub fn create_local_torrent_structure(&self, tc: &TorrentContext) -> Result<(), BitTorrentError> {
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
                    tc.update_bitfield_from_buffer(piece_number, &piece_buffer, bytes_in_buffer as u32);
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

    pub fn fully_downloaded_torrent_bitfield(&self, tc: &mut TorrentContext) -> Result<(), BitTorrentError> {
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
