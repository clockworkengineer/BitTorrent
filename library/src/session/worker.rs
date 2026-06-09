use crate::error::BitTorrentError;
use crate::peer::Peer;
use crate::peer_message::PeerMessage;
use crate::torrent_context::{TorrentContext, TorrentStatus};
use crate::tracker::PeerDetails;
use crate::manager::Manager;
use crate::log_debug;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Marks a peer IP address as dead in the peer manager registry.
pub fn mark_peer_dead(manager: &Option<Arc<Manager>>, ip: &str) {
    if let Some(mgr) = manager {
        mgr.add_to_dead_peer_list(ip);
    }
}

/// Helper task to cooperatively delay thread execution.
pub async fn delay(duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        std::thread::sleep(std::time::Duration::from_millis(5));
        crate::util::yield_now().await;
    }
}

/// Checks outstanding block requests for a peer and releases/cancels them on timeout (> 2 seconds).
async fn check_request_timeouts(
    peer: &Mutex<Peer>,
    context: &Mutex<TorrentContext>,
) {
    let timed_out_requests = {
        let mut pg = peer.lock().unwrap();
        let mut timed_out = Vec::new();
        pg.reserved_blocks.retain(|&(pn, bi, time)| {
            if time.elapsed() > Duration::from_secs(2) {
                timed_out.push((pn, bi));
                false
            } else {
                true
            }
        });
        timed_out
    };

    if !timed_out_requests.is_empty() {
        let ctx = context.lock().unwrap();
        for (pn, bi) in timed_out_requests {
            ctx.release_block_request(pn, bi);
            let begin = bi * crate::constants::BLOCK_SIZE as u32;
            let length = std::cmp::min(
                crate::constants::BLOCK_SIZE as u32,
                ctx.get_piece_length(pn).saturating_sub(begin),
            );
            let mut pg = peer.lock().unwrap();
            pg.outstanding_requests_count = pg.outstanding_requests_count.saturating_sub(1);
            let _ = pg.send_message(PeerMessage::Cancel {
                index: pn,
                begin,
                length,
            }).await;
        }
    }
}

/// Establishes transport connection, performs handshakes, and executes read/write loops for a peer connection.
pub async fn handle_peer_session(
    peer_details: PeerDetails,
    context: Arc<Mutex<TorrentContext>>,
    manager: Option<Arc<Manager>>,
) {
    let info_hash = context.lock().unwrap().info_hash.clone();
    let local_peer_id = crate::peer_id::get();

    if let Some(ref mgr) = manager {
        if mgr.is_peer_dead(&peer_details.ip) {
            return;
        }
    }

    if context.lock().unwrap().is_peer_blacklisted(&peer_details.ip) {
        return;
    }

    let config = {
        let ctx = context.lock().unwrap();
        ctx.config.clone()
    };

    let socket = match config.socket_factory.connect(&peer_details.ip, peer_details.port) {
        Ok(s) => s,
        Err(_) => {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
    };

    let peer = Arc::new(Mutex::new(Peer::new_with_socket(
        peer_details.ip.clone(),
        peer_details.port,
        socket,
    )));

    let net = {
        let mut pg = peer.lock().unwrap();
        pg.set_torrent_context(context.clone());
        match &pg.network {
            Some(n) => n.clone(),
            None => {
                mark_peer_dead(&manager, &peer_details.ip);
                return;
            }
        }
    };

    if net.write_handshake(&info_hash, local_peer_id.as_bytes()).await.is_err() {
        mark_peer_dead(&manager, &peer_details.ip);
        return;
    }
    println!(
        "Handshake completed with peer {}:{}",
        peer_details.ip, peer_details.port
    );
    let (_, remote_peer_id) = match net.read_handshake().await {
        Ok(res) => res,
        Err(_) => {
            mark_peer_dead(&manager, &peer_details.ip);
            return;
        }
    };
    {
        let mut pg = peer.lock().unwrap();
        pg.connected = true;
        pg.remote_peer_id = Some(remote_peer_id);
    }
    
    let bitfield = context.lock().unwrap().bitfield.clone();
    if net.write_message(PeerMessage::Bitfield(&bitfield)).await.is_err() {
        mark_peer_dead(&manager, &peer_details.ip);
        return;
    }
    println!(
        "Sent Bitfield to peer {}:{}",
        peer_details.ip, peer_details.port
    );
    if net.write_message(PeerMessage::Unchoke).await.is_err() {
        mark_peer_dead(&manager, &peer_details.ip);
        return;
    }
    {
        peer.lock().unwrap().am_choking = false;
    }
    println!(
        "Sent Unchoke to peer {}:{}",
        peer_details.ip, peer_details.port
    );
    if net.write_message(PeerMessage::Interested).await.is_err() {
        mark_peer_dead(&manager, &peer_details.ip);
        return;
    }
    println!(
        "Sent Interested to peer {}:{}",
        peer_details.ip, peer_details.port
    );

    {
        let ctx = context.lock().unwrap();
        if !ctx.add_peer(peer.clone()) {
            return;
        }
    }

    let mut static_buf_opt = crate::util::acquire_buffer();
    let mut _fallback_buf = None;
    let read_buffer_backing = match &mut static_buf_opt {
        Some(buf) => buf.as_mut(),
        None => {
            _fallback_buf = Some(vec![0u8; crate::util::BUFFER_SIZE]);
            _fallback_buf.as_mut().unwrap().as_mut_slice()
        }
    };
    let mut last_progress = Instant::now();
    loop {
        let status = {
            let ctx = context.lock().unwrap();
            if ctx.status == TorrentStatus::Ended {
                break;
            }
            ctx.paused.wait_one(0)
        };
        if status {
            delay(Duration::from_millis(100)).await;
            continue;
        }

        let message = match net.read_message(&mut *read_buffer_backing).await {
            Ok(m) => m,
            Err(err) => {
                if let BitTorrentError::Io(ref io_err) = err {
                    if io_err.kind() == std::io::ErrorKind::WouldBlock
                        || io_err.kind() == std::io::ErrorKind::TimedOut
                    {
                        check_request_timeouts(&peer, &context).await;
                        if last_progress.elapsed() > Duration::from_secs(30) {
                            log_debug!("[peer {}:{}] 30s idle timeout, dropping",
                                peer_details.ip, peer_details.port);
                            mark_peer_dead(&manager, &peer_details.ip);
                            break;
                        }
                        continue;
                    }
                }
                log_debug!("[peer {}:{}] read error: {}", peer_details.ip, peer_details.port, err);
                mark_peer_dead(&manager, &peer_details.ip);
                break;
            }
        };
        
        let actions_res = {
            let mut pg = peer.lock().unwrap();
            let mut ctx = context.lock().unwrap();
            pg.handle_peer_message(message, &mut ctx)
        };
        let actions = match actions_res {
            Ok(a) => a,
            Err(_) => {
                mark_peer_dead(&manager, &peer_details.ip);
                break;
            }
        };

        if Peer::execute_actions(actions, &net, &context, &peer_details.ip).await.is_err() {
            mark_peer_dead(&manager, &peer_details.ip);
            break;
        }

        check_request_timeouts(&peer, &context).await;

        let (peer_choking_set, outstanding_requests_count, number_of_missing_pieces) = {
            let pg = peer.lock().unwrap();
            (pg.peer_choking.wait_one(0), pg.outstanding_requests_count, pg.number_of_missing_pieces)
        };

        let is_downloading = {
            context.lock().unwrap().status == TorrentStatus::Downloading
        };

        if peer_choking_set && is_downloading {
            let to_send = 10usize.saturating_sub(outstanding_requests_count);
            let mut send_error = false;
            let mut none_count = 0;
            for _ in 0..to_send {
                let next_req = {
                    let pg = peer.lock().unwrap();
                    let mut ctx = context.lock().unwrap();
                    ctx.next_block_request_for_peer(&pg)
                };
                match next_req {
                    Some((pn, begin, length)) => {
                        if net.write_message(PeerMessage::Request { index: pn, begin, length }).await.is_err() {
                            mark_peer_dead(&manager, &peer_details.ip);
                            send_error = true;
                            break;
                        }
                        {
                            let mut pg = peer.lock().unwrap();
                            let bi = begin / crate::constants::BLOCK_SIZE as u32;
                            pg.reserved_blocks.push((pn, bi, Instant::now()));
                            pg.outstanding_requests_count = pg.outstanding_requests_count.saturating_add(1);
                        }
                        last_progress = Instant::now();
                    }
                    None => { none_count += 1; break; }
                }
            }
            if none_count > 0 {
                log_debug!("[peer {}:{}] no blocks available (outstanding={} missing_pieces={})",
                    peer_details.ip, peer_details.port,
                    outstanding_requests_count,
                    number_of_missing_pieces);
            }
            if send_error {
                break;
            }
        }

        if context.lock().unwrap().is_download_complete() {
            break;
        }
    }

    log_debug!("[peer {}:{}] thread exiting", peer_details.ip, peer_details.port);
    {
        let mut ctx = context.lock().unwrap();
        if let Ok(pg) = peer.try_lock() {
            for &(pn, bi, _) in &pg.reserved_blocks {
                ctx.release_block_request(pn, bi);
            }
        }
        ctx.remove_peer(&peer_details.ip);
    }
}
