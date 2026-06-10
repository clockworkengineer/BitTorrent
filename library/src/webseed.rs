use std::sync::{Arc, Mutex};
use std::time::Duration;
use crate::error::BitTorrentError;
use crate::torrent_context::{TorrentContext, TorrentStatus};
use crate::constants::BLOCK_SIZE;

pub fn start_webseed_loop(
    context: Arc<Mutex<TorrentContext>>,
    web_seeds: Vec<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if web_seeds.is_empty() {
            return;
        }

        loop {
            let (status, num_pieces, piece_length) = {
                let ctx = context.lock().unwrap();
                (ctx.status, ctx.number_of_pieces, ctx.piece_length)
            };

            if status == TorrentStatus::Ended {
                break;
            }

            if status != TorrentStatus::Downloading {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            // Find next missing block to request
            let next_block = {
                let ctx = context.lock().unwrap();
                let mut missing = None;
                for p in 0..num_pieces as u32 {
                    if !ctx.is_piece_local(p) {
                        if let Some((begin, length)) = ctx.next_pending_block(p) {
                            let block_index = begin / BLOCK_SIZE as u32;
                            if ctx.reserve_block_request(p, block_index) {
                                missing = Some((p, begin, length));
                                break;
                            }
                        }
                    }
                }
                missing
            };

            if let Some((piece, begin, length)) = next_block {
                let global_offset = (piece as u64) * (piece_length as u64) + begin as u64;
                let mut success = false;

                // Try downloading from each web seed mirror
                for url in &web_seeds {
                    match download_block_from_webseed(&context, url, global_offset, length) {
                        Ok(data) => {
                            let mut ctx = context.lock().unwrap();
                            let storage = ctx.storage.clone();
                            if ctx.process_piece_block(&*storage, piece, begin, &data, "webseed").is_ok() {
                                success = true;
                                break;
                            }
                        }
                        Err(e) => {
                            crate::log_debug!("WebSeed download failed from {}: {}", url, e);
                        }
                    }
                }

                if !success {
                    let ctx = context.lock().unwrap();
                    let block_index = begin / BLOCK_SIZE as u32;
                    ctx.release_block_request(piece, block_index);
                    // Sleep briefly on failure to avoid busy looping
                    std::thread::sleep(Duration::from_millis(100));
                }
            } else {
                // No blocks available or download complete
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    })
}

fn download_block_from_webseed(
    context: &Mutex<TorrentContext>,
    web_seed_url: &str,
    global_offset: u64,
    length: u32,
) -> Result<Vec<u8>, BitTorrentError> {
    let (files, download_path) = {
        let ctx = context.lock().unwrap();
        (ctx.files_to_download.clone(), ctx.download_path.clone())
    };

    let is_multi_file = files.len() > 1;
    let mut block_data = Vec::with_capacity(length as usize);
    let mut current_offset = global_offset;
    let mut remaining = length;

    for file in &files {
        if remaining == 0 {
            break;
        }
        let file_start = file.offset;
        let file_end = file.offset + file.length;

        if current_offset >= file_start && current_offset < file_end {
            let file_segment_len = std::cmp::min(remaining as u64, file_end - current_offset) as u32;
            let file_offset = current_offset - file_start;
            let rel_path = std::path::Path::new(&file.name)
                .strip_prefix(&download_path)
                .unwrap_or(std::path::Path::new(&file.name));

            let segment_data = download_segment(
                web_seed_url,
                rel_path,
                is_multi_file,
                file_offset,
                file_segment_len,
            )?;
            block_data.extend_from_slice(&segment_data);

            current_offset += file_segment_len as u64;
            remaining -= file_segment_len;
        }
    }

    if block_data.len() != length as usize {
        return Err(BitTorrentError::Parse(format!(
            "WebSeed mapped length mismatch: expected {}, got {}",
            length, block_data.len()
        )));
    }

    Ok(block_data)
}

fn download_segment(
    web_seed_url: &str,
    rel_path: &std::path::Path,
    is_multi_file: bool,
    file_offset: u64,
    length: u32,
) -> Result<Vec<u8>, BitTorrentError> {
    let mut target_url = web_seed_url.to_string();
    if is_multi_file {
        if !target_url.ends_with('/') {
            target_url.push('/');
        }
        target_url.push_str(&rel_path.to_string_lossy().replace('\\', "/"));
    }

    let response = ureq::get(&target_url)
        .set("Range", &format!("bytes={}-{}", file_offset, file_offset + length as u64 - 1))
        .call()
        .map_err(|e| BitTorrentError::Parse(format!("WebSeed HTTP error: {}", e)))?;

    let mut data = Vec::new();
    response.into_reader().read_to_end(&mut data)?;

    if data.len() != length as usize {
        return Err(BitTorrentError::Parse(format!(
            "WebSeed segment length mismatch: expected {}, got {}",
            length, data.len()
        )));
    }

    Ok(data)
}
