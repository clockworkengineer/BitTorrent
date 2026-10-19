use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use crate::error::BitTorrentError;
use crate::core::torrent_context::{TorrentContext, TorrentStatus};
use crate::constants::BLOCK_SIZE;

/// Starts a background thread driving WebSeed HTTP mirror block requests.
pub fn start_webseed_loop(
    context: Arc<Mutex<TorrentContext>>,
    web_seeds: Vec<String>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // Register initial WebSeeds into the context's mirror tracker if not already present
        {
            let ctx = context.lock().unwrap();
            let mut tracker = ctx.webseed_tracker.lock().unwrap();
            for ws in &web_seeds {
                tracker.register_mirror(ws);
            }
        }

        loop {
            let (status, num_pieces, piece_length, enable_tracker, timeout_secs) = {
                let ctx = context.lock().unwrap();
                (
                    ctx.status,
                    ctx.number_of_pieces,
                    ctx.piece_length,
                    ctx.config.enable_webseed_health_tracker,
                    ctx.config.webseed_timeout_secs,
                )
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
                    if !ctx.is_piece_local(p) && ctx.is_piece_wanted(p) {
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

                // Select ranked mirrors dynamically based on health scores and latency
                let candidate_mirrors = {
                    let ctx = context.lock().unwrap();
                    if enable_tracker {
                        let ranked = ctx.webseed_tracker.lock().unwrap().select_ranked_mirrors();
                        if ranked.is_empty() {
                            ctx.web_seeds.clone()
                        } else {
                            ranked
                        }
                    } else {
                        ctx.web_seeds.clone()
                    }
                };

                // Try downloading from candidate web seed mirrors in ranked order
                for url in &candidate_mirrors {
                    let req_start = Instant::now();
                    match download_block_from_webseed(&context, url, global_offset, length, timeout_secs) {
                        Ok(data) => {
                            let req_duration = req_start.elapsed();
                            let mut ctx = context.lock().unwrap();
                            if enable_tracker {
                                ctx.webseed_tracker
                                    .lock()
                                    .unwrap()
                                    .record_success(url, req_duration, data.len());
                            }
                            let storage = ctx.storage.clone();
                            if ctx.process_piece_block(&*storage, piece, begin, &data, url).is_ok() {
                                success = true;
                                break;
                            }
                        }
                        Err(e) => {
                            crate::log_debug!("WebSeed download failed from {}: {}", url, e);
                            let ctx = context.lock().unwrap();
                            if enable_tracker {
                                ctx.webseed_tracker
                                    .lock()
                                    .unwrap()
                                    .record_failure(url);
                            }
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
    timeout_secs: u64,
) -> Result<Vec<u8>, BitTorrentError> {
    let (files, is_multi_file) = {
        let ctx = context.lock().unwrap();
        let multi = ctx.files_to_download.len() > 1;
        (ctx.files_to_download.clone(), multi)
    };

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

            // Use pre-computed torrent-relative path (e.g. "Sintel/Sintel.mp4").
            let segment_data = download_segment(
                web_seed_url,
                &file.torrent_path,
                is_multi_file,
                file_offset,
                file_segment_len,
                timeout_secs,
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
    rel_path: &str,
    is_multi_file: bool,
    file_offset: u64,
    length: u32,
    timeout_secs: u64,
) -> Result<Vec<u8>, BitTorrentError> {
    let target_url = if is_multi_file {
        // BEP 19: base URL + '/' + relative path (no leading slash)
        let base = web_seed_url.trim_end_matches('/');
        let path = rel_path.trim_start_matches('/');
        format!("{}/{}", base, path)
    } else {
        // Single-file: use the web-seed URL directly (it IS the file URL)
        web_seed_url.to_string()
    };

    let response = ureq::get(&target_url)
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .set("Range", &format!("bytes={}-{}", file_offset, file_offset + length as u64 - 1))
        .call()
        .map_err(|e| BitTorrentError::Parse(format!("WebSeed HTTP error: {}: {}", target_url, e)))?;

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
