use crate::error::BitTorrentError;
use crate::peer::Peer;
use crate::peer_message::PeerMessage;
use crate::core::torrent_context::{TorrentContext, TorrentStatus};
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
        let net_opt = {
            let ctx = context.lock().unwrap();
            let mut pg = peer.lock().unwrap();
            for &(pn, bi) in &timed_out_requests {
                ctx.release_block_request(pn, bi);
                pg.outstanding_requests_count = pg.outstanding_requests_count.saturating_sub(1);
            }
            pg.network.clone()
        };

        if let Some(net) = net_opt {
            for (pn, bi) in timed_out_requests {
                let begin = bi * crate::constants::BLOCK_SIZE as u32;
                let length = {
                    let ctx = context.lock().unwrap();
                    std::cmp::min(
                        crate::constants::BLOCK_SIZE as u32,
                        ctx.get_piece_length(pn).saturating_sub(begin),
                    )
                };
                let _ = net.write_message(PeerMessage::Cancel {
                    index: pn,
                    begin,
                    length,
                }).await;
            }
        }
    }
}

// ─── Handshake ───────────────────────────────────────────────────────────────

/// Result type returned by [`perform_handshake`].
pub struct HandshakeResult {
    pub peer: Arc<Mutex<Peer>>,
    pub net: crate::peer_network::PeerNetwork,
    pub supports_extensions: bool,
}

/// Establishes a socket connection, runs MSE negotiation if configured, and
/// completes the BitTorrent handshake.
///
/// Returns `Ok(HandshakeResult)` on success or `Err(())` if the connection
/// should be abandoned (peer is already marked dead by the time this returns).
pub async fn perform_handshake(
    peer_details: &PeerDetails,
    context: &Arc<Mutex<TorrentContext>>,
    manager: &Option<Arc<Manager>>,
    info_hash: &[u8],
    local_peer_id: &str,
) -> Result<HandshakeResult, ()> {
    let config = {
        let ctx = context.lock().unwrap();
        ctx.config.clone()
    };

    let socket = match config.socket_factory.connect(&peer_details.ip, peer_details.port) {
        Ok(s) => s,
        Err(_) => {
            mark_peer_dead(manager, &peer_details.ip);
            return Err(());
        }
    };

    let peer = Arc::new(Mutex::new(Peer::new_with_socket(
        peer_details.ip.clone(),
        peer_details.port,
        socket,
    )));

    let mut net = {
        let mut pg = peer.lock().unwrap();
        pg.set_torrent_context(context.clone());
        match &pg.network {
            Some(n) => n.clone(),
            None => {
                mark_peer_dead(manager, &peer_details.ip);
                return Err(());
            }
        }
    };

    // MSE Diffie-Hellman handshake negotiation (only when enabled in config)
    #[cfg(feature = "mse")]
    if config.mse_enabled {
        let dh = crate::mse::DiffieHellman::new();
        let local_pub_bytes = dh.public_key;
        if net.write(&local_pub_bytes).await.is_err() {
            mark_peer_dead(manager, &peer_details.ip);
            return Err(());
        }
        let mut remote_pub_bytes = [0u8; 96];
        if net.read_exact(&mut remote_pub_bytes).await.is_err() {
            mark_peer_dead(manager, &peer_details.ip);
            return Err(());
        }
        let secret = dh.compute_shared_secret(remote_pub_bytes);

        // Derive RC4 cipher keys from the shared secret
        use sha1::Digest;
        let mut enc_hasher = sha1::Sha1::new();
        enc_hasher.update(&secret);
        enc_hasher.update(b"initiator");
        let enc_key = enc_hasher.finalize();

        let mut dec_hasher = sha1::Sha1::new();
        dec_hasher.update(&secret);
        dec_hasher.update(b"receiver");
        let dec_key = dec_hasher.finalize();

        let rc4_enc = crate::mse::Rc4::new(&enc_key);
        let rc4_dec = crate::mse::Rc4::new(&dec_key);
        net.set_mse_ciphers(rc4_enc, rc4_dec);
        peer.lock().unwrap().network = Some(net.clone());
    }

    #[cfg(not(feature = "mse"))]
    if config.mse_enabled {
        log_debug!("MSE is not compiled in this build!");
        mark_peer_dead(manager, &peer_details.ip);
        return Err(());
    }

    if net.write_handshake(info_hash, local_peer_id.as_bytes()).await.is_err() {
        mark_peer_dead(manager, &peer_details.ip);
        return Err(());
    }
    log_debug!("[peer {}:{}] handshake sent", peer_details.ip, peer_details.port);

    let (remote_info_hash, remote_peer_id, reserved) = match net.read_handshake().await {
        Ok(res) => res,
        Err(_) => {
            mark_peer_dead(manager, &peer_details.ip);
            return Err(());
        }
    };
    if remote_info_hash != info_hash {
        mark_peer_dead(manager, &peer_details.ip);
        return Err(());
    }
    // Reject trivially invalid all-zero info hashes.
    if remote_info_hash.iter().all(|&b| b == 0) {
        mark_peer_dead(manager, &peer_details.ip);
        return Err(());
    }

    let supports_extensions = (reserved[5] & 0x10) != 0;
    {
        let mut pg = peer.lock().unwrap();
        pg.connected = true;
        pg.remote_peer_id = Some(remote_peer_id);
        pg.supports_extensions = supports_extensions;
        pg.last_message_sent = Instant::now();
        pg.last_message_received = Instant::now();
    }

    // Send extension handshake advertising ut_metadata and ut_pex support
    if supports_extensions {
        let payload = b"d1:md11:ut_metadatai1e6:ut_pexi2eee";
        if net.write_message(PeerMessage::Extended { ext_id: 0, payload }).await.is_err() {
            mark_peer_dead(manager, &peer_details.ip);
            return Err(());
        }
        peer.lock().unwrap().update_last_message_sent();
    }

    // Send our current bitfield so the remote knows which pieces we have
    let bitfield = context.lock().unwrap().bitfield.clone();
    if !bitfield.is_empty() {
        if net.write_message(PeerMessage::Bitfield(&bitfield)).await.is_err() {
            mark_peer_dead(manager, &peer_details.ip);
            return Err(());
        }
        peer.lock().unwrap().update_last_message_sent();
        log_debug!("[peer {}:{}] sent Bitfield", peer_details.ip, peer_details.port);
    }
    if net.write_message(PeerMessage::Unchoke).await.is_err() {
        mark_peer_dead(manager, &peer_details.ip);
        return Err(());
    }
    peer.lock().unwrap().update_last_message_sent();
    peer.lock().unwrap().am_choking = false;
    log_debug!("[peer {}:{}] sent Unchoke", peer_details.ip, peer_details.port);

    if net.write_message(PeerMessage::Interested).await.is_err() {
        mark_peer_dead(manager, &peer_details.ip);
        return Err(());
    }
    peer.lock().unwrap().update_last_message_sent();
    log_debug!("[peer {}:{}] sent Interested", peer_details.ip, peer_details.port);

    Ok(HandshakeResult { peer, net, supports_extensions })
}

// ─── Magnet bootstrap ────────────────────────────────────────────────────────

/// Handles the magnet-link metadata exchange phase (ut_metadata).
///
/// Requests missing metadata pieces from this peer and assembles them into the
/// full info dictionary when complete.  Transitions the context from
/// `metadata-bootstrap` mode to a normal torrent download on success.
///
/// Returns `true` if the session should break out of the main peer loop.
pub async fn handle_magnet_bootstrap(
    peer_details: &PeerDetails,
    peer: &Arc<Mutex<Peer>>,
    net: &crate::peer_network::PeerNetwork,
    context: &Arc<Mutex<TorrentContext>>,
    manager: &Option<Arc<Manager>>,
) -> bool {
    let info_hash = context.lock().unwrap().info_hash.clone();

    // Request the next un-fetched metadata piece from this peer
    let request_piece = {
        let mut ctx = context.lock().unwrap();
        let pg = peer.lock().unwrap();
        if pg.supports_extensions {
            if let Some(&peer_ext_id) = pg.extension_ids.get("ut_metadata") {
                if let Some(size) = ctx.metadata_size {
                    let num_pieces = (size + 16383) / 16384;
                    let mut target_piece = None;
                    for p in 0..num_pieces as u32 {
                        if !ctx.metadata_pieces.contains_key(&p) {
                            let is_requested = ctx.requested_metadata_pieces.get(&p)
                                .map(|t| t.elapsed() < Duration::from_secs(5))
                                .unwrap_or(false);
                            if !is_requested {
                                ctx.requested_metadata_pieces.insert(p, Instant::now());
                                target_piece = Some((p, peer_ext_id));
                                break;
                            }
                        }
                    }
                    target_piece
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some((p, peer_ext_id)) = request_piece {
        let payload = alloc::format!("d8:msg_typei0e5:piecei{}ee", p).into_bytes();
        if net.write_message(PeerMessage::Extended { ext_id: peer_ext_id, payload: &payload }).await.is_err() {
            mark_peer_dead(manager, &peer_details.ip);
            return true; // break
        }
        peer.lock().unwrap().update_last_message_sent();
    }

    // Check if we now have all metadata pieces and can assemble the info dict
    let transition_data = {
        let mut ctx = context.lock().unwrap();
        if let Some(size) = ctx.metadata_size {
            let num_pieces = (size + 16383) / 16384;
            if ctx.metadata_pieces.len() == num_pieces && ctx.info_dict_bytes.is_none() {
                let mut assembled = Vec::with_capacity(size);
                for p in 0..num_pieces as u32 {
                    if let Some(chunk) = ctx.metadata_pieces.get(&p) {
                        assembled.extend_from_slice(chunk);
                    }
                }
                use sha1::Digest;
                let mut hasher = sha1::Sha1::new();
                hasher.update(&assembled);
                let hash = hasher.finalize().to_vec();
                if hash == info_hash {
                    Some(assembled)
                } else {
                    ctx.metadata_pieces.clear();
                    ctx.requested_metadata_pieces.clear();
                    log_debug!("[magnet] metadata hash mismatch, re-downloading...");
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(assembled) = transition_data {
        let mut ctx = context.lock().unwrap();
        let dl_path = ctx.download_path.clone();
        if let Err(e) = ctx.transition_from_metadata(&assembled, &dl_path) {
            log_debug!("[peer {}:{}] failed transition from metadata: {}", peer_details.ip, peer_details.port, e);
            mark_peer_dead(manager, &peer_details.ip);
            return true; // break
        }
        log_debug!("[peer {}:{}] successfully transitioned from magnet to standard download", peer_details.ip, peer_details.port);
    }

    false // continue the main loop
}

// ─── Block requests ──────────────────────────────────────────────────────────

/// Issues up to 10 pipelined block `Request` messages to the peer when unchoked
/// and in downloading mode.
///
/// Returns `true` if a send error occurred and the caller should break the loop.
pub async fn send_block_requests(
    peer_details: &PeerDetails,
    peer: &Arc<Mutex<Peer>>,
    net: &crate::peer_network::PeerNetwork,
    context: &Arc<Mutex<TorrentContext>>,
    manager: &Option<Arc<Manager>>,
    outstanding_requests_count: usize,
    number_of_missing_pieces: usize,
) -> bool {
    let to_send = 10usize.saturating_sub(outstanding_requests_count);
    let mut none_count = 0;
    for _ in 0..to_send {
        let next_req = {
            let mut ctx = context.lock().unwrap();
            let pg = peer.lock().unwrap();
            ctx.next_block_request_for_peer(&pg)
        };
        match next_req {
            Some((pn, begin, length)) => {
                if net.write_message(PeerMessage::Request { index: pn, begin, length }).await.is_err() {
                    mark_peer_dead(manager, &peer_details.ip);
                    return true; // send error → break
                }
                {
                    let mut pg = peer.lock().unwrap();
                    let bi = begin / crate::constants::BLOCK_SIZE as u32;
                    pg.reserved_blocks.push((pn, bi, Instant::now()));
                    pg.outstanding_requests_count = pg.outstanding_requests_count.saturating_add(1);
                    pg.update_last_message_sent();
                }
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
    false
}

// ─── Main peer-loop ──────────────────────────────────────────────────────────

/// The inner read/write loop executed after a successful handshake.
///
/// Drives PEX, message dispatch, magnet bootstrap, block requesting,
/// keep-alive, and timeout handling until the session ends or the peer dies.
async fn run_peer_loop(
    peer_details: &PeerDetails,
    peer: &Arc<Mutex<Peer>>,
    net: &mut crate::peer_network::PeerNetwork,
    context: &Arc<Mutex<TorrentContext>>,
    manager: &Option<Arc<Manager>>,
) {
    let info_hash = context.lock().unwrap().info_hash.clone();

    let mut static_buf_opt = crate::util::acquire_buffer();
    let mut _fallback_buf = None;
    let read_buffer_backing = match &mut static_buf_opt {
        Some(buf) => buf.as_mut(),
        None => {
            _fallback_buf = Some(vec![0u8; crate::util::BUFFER_SIZE]);
            _fallback_buf.as_mut().unwrap().as_mut_slice()
        }
    };
    let mut last_pex_sent = Instant::now();
    let mut sent_peers: std::collections::HashMap<String, u16> = std::collections::HashMap::new();

    loop {
        // Pause / ended check
        let status = {
            let ctx = context.lock().unwrap();
            if ctx.status == TorrentStatus::Ended {
                break;
            }
            ctx.is_paused()
        };
        if status {
            delay(Duration::from_millis(100)).await;
            continue;
        }

        // ── Periodic PEX broadcast ────────────────────────────────────────
        let (supports_extensions, is_private) = {
            let ctx = context.lock().unwrap();
            let pg = peer.lock().unwrap();
            (pg.supports_extensions, ctx.is_private)
        };
        if supports_extensions && !is_private && last_pex_sent.elapsed() > Duration::from_secs(60) {
            let pex_ext_id = {
                let pg = peer.lock().unwrap();
                pg.extension_ids.get("ut_pex").cloned()
            };
            if let Some(ext_id) = pex_ext_id {
                let mut current_swarm = Vec::new();
                {
                    let ctx = context.lock().unwrap();
                    let swarm = ctx.peer_swarm.read().unwrap();
                    for (ip, p_arc) in swarm.iter() {
                        if ip != &peer_details.ip {
                            if let Ok(p_guard) = p_arc.try_lock() {
                                current_swarm.push(PeerDetails {
                                    info_hash: info_hash.clone(),
                                    peer_id: None,
                                    ip: ip.clone(),
                                    port: p_guard.port,
                                });
                            }
                        }
                    }
                }

                let mut added_bytes = Vec::new();
                let mut added6_bytes = Vec::new();
                let mut newly_sent = std::collections::HashMap::new();
                for detail in &current_swarm {
                    newly_sent.insert(detail.ip.clone(), detail.port);
                    if !sent_peers.contains_key(&detail.ip) {
                        if let Ok(ip_addr) = detail.ip.parse::<std::net::Ipv4Addr>() {
                            added_bytes.extend_from_slice(&ip_addr.octets());
                            added_bytes.extend_from_slice(&detail.port.to_be_bytes());
                        } else if let Ok(ip6_addr) = detail.ip.parse::<std::net::Ipv6Addr>() {
                            added6_bytes.extend_from_slice(&ip6_addr.octets());
                            added6_bytes.extend_from_slice(&detail.port.to_be_bytes());
                        }
                    }
                }

                let mut dropped_bytes = Vec::new();
                let mut dropped6_bytes = Vec::new();
                for (ip, &port) in &sent_peers {
                    if !newly_sent.contains_key(ip) {
                        if let Ok(ip_addr) = ip.parse::<std::net::Ipv4Addr>() {
                            dropped_bytes.extend_from_slice(&ip_addr.octets());
                            dropped_bytes.extend_from_slice(&port.to_be_bytes());
                        } else if let Ok(ip6_addr) = ip.parse::<std::net::Ipv6Addr>() {
                            dropped6_bytes.extend_from_slice(&ip6_addr.octets());
                            dropped6_bytes.extend_from_slice(&port.to_be_bytes());
                        }
                    }
                }

                sent_peers = newly_sent;

                if !added_bytes.is_empty() || !dropped_bytes.is_empty() || !added6_bytes.is_empty() || !dropped6_bytes.is_empty() {
                    let mut pex_items = Vec::new();
                    pex_items.push((b"added".as_slice(), crate::bencode::BNode::String(&added_bytes)));
                    pex_items.push((b"dropped".as_slice(), crate::bencode::BNode::String(&dropped_bytes)));
                    if !added6_bytes.is_empty() {
                        pex_items.push((b"added6".as_slice(), crate::bencode::BNode::String(&added6_bytes)));
                    }
                    if !dropped6_bytes.is_empty() {
                        pex_items.push((b"dropped6".as_slice(), crate::bencode::BNode::String(&dropped6_bytes)));
                    }
                    let pex_dict = crate::bencode::BNode::Dictionary(pex_items);
                    let pex_payload = crate::bencode::Bencode::encode(&pex_dict);

                    if net.write_message(PeerMessage::Extended { ext_id, payload: &pex_payload }).await.is_ok() {
                        peer.lock().unwrap().update_last_message_sent();
                    }
                }
            }
            last_pex_sent = Instant::now();
        }

        // ── Read next message ─────────────────────────────────────────────
        let message = match net.read_message(&mut *read_buffer_backing).await {
            Ok(m) => m,
            Err(err) => {
                if let BitTorrentError::Io(ref io_err) = err {
                    if io_err.kind() == std::io::ErrorKind::WouldBlock
                        || io_err.kind() == std::io::ErrorKind::TimedOut
                    {
                        check_request_timeouts(peer, context).await;

                        let (last_sent, last_recv) = {
                            let pg = peer.lock().unwrap();
                            (pg.last_message_sent, pg.last_message_received)
                        };

                        if last_recv.elapsed() > Duration::from_secs(120) {
                            log_debug!("[peer {}:{}] 120s idle timeout, dropping",
                                peer_details.ip, peer_details.port);
                            mark_peer_dead(manager, &peer_details.ip);
                            break;
                        }

                        if last_sent.elapsed() > Duration::from_secs(120) {
                            log_debug!("[peer {}:{}] sending keep-alive",
                                peer_details.ip, peer_details.port);
                            if net.write_message(PeerMessage::KeepAlive).await.is_ok() {
                                peer.lock().unwrap().update_last_message_sent();
                            }
                        }
                        continue;
                    }
                }
                log_debug!("[peer {}:{}] read error: {}", peer_details.ip, peer_details.port, err);
                mark_peer_dead(manager, &peer_details.ip);
                break;
            }
        };

        // ── Dispatch message ──────────────────────────────────────────────
        let actions_res = {
            let mut ctx = context.lock().unwrap();
            let mut pg = peer.lock().unwrap();
            pg.handle_peer_message(message, &mut ctx)
        };
        let actions = match actions_res {
            Ok(a) => a,
            Err(_) => {
                mark_peer_dead(manager, &peer_details.ip);
                break;
            }
        };

        if Peer::execute_actions(peer, actions, net, context, &peer_details.ip, manager).await.is_err() {
            mark_peer_dead(manager, &peer_details.ip);
            break;
        }

        check_request_timeouts(peer, context).await;

        let (peer_choking_set, outstanding_requests_count, number_of_missing_pieces) = {
            let pg = peer.lock().unwrap();
            (pg.peer_choking.wait_one(0), pg.outstanding_requests_count, pg.number_of_missing_pieces)
        };

        let is_downloading = context.lock().unwrap().status == TorrentStatus::Downloading;

        let is_magnet_bootstrap = context.lock().unwrap().pieces_info_hash.is_empty();

        // ── Magnet bootstrap ──────────────────────────────────────────────
        if is_magnet_bootstrap && is_downloading {
            if handle_magnet_bootstrap(peer_details, peer, net, context, manager).await {
                break;
            }
        }

        // ── Block requests ────────────────────────────────────────────────
        if !is_magnet_bootstrap && peer_choking_set && is_downloading {
            if send_block_requests(
                peer_details, peer, net, context, manager,
                outstanding_requests_count, number_of_missing_pieces,
            ).await {
                break;
            }
        }

        if context.lock().unwrap().is_download_complete() {
            break;
        }
    }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

/// Establishes transport connection, performs handshakes, and executes the
/// read/write loop for a peer connection.
///
/// This is the top-level function spawned by the session for each peer.
/// Internally it delegates to:
/// - [`perform_handshake`] — socket connect, MSE, BitTorrent handshake, initial messages
/// - [`run_peer_loop`] — the message dispatch / PEX / request loop
/// - [`handle_magnet_bootstrap`] (inside the loop) — metadata piece exchange
/// - [`send_block_requests`] (inside the loop) — pipelined block requests
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

    let hs = match perform_handshake(&peer_details, &context, &manager, &info_hash, &local_peer_id).await {
        Ok(hs) => hs,
        Err(()) => return,
    };

    let HandshakeResult { peer, mut net, .. } = hs;

    {
        let ctx = context.lock().unwrap();
        if !ctx.add_peer(peer.clone()) {
            return;
        }
    }

    run_peer_loop(&peer_details, &peer, &mut net, &context, &manager).await;

    log_debug!("[peer {}:{}] thread exiting", peer_details.ip, peer_details.port);
    {
        let mut ctx = context.lock().unwrap();
        {
            let pg = peer.lock().unwrap();
            for &(pn, bi, _) in &pg.reserved_blocks {
                ctx.release_block_request(pn, bi);
            }
        }
        ctx.remove_peer(&peer_details.ip);
    }
}
