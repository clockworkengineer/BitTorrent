use std::net::{UdpSocket, Ipv4Addr, SocketAddrV4};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use crate::tracker::PeerDetails;

pub struct LsdListener {
    info_hash: Vec<u8>,
    peer_tx: Sender<PeerDetails>,
}

impl LsdListener {
    pub fn new(info_hash: Vec<u8>, peer_tx: Sender<PeerDetails>) -> Self {
        LsdListener {
            info_hash,
            peer_tx,
        }
    }

    pub fn start(self) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let multicast_ip = Ipv4Addr::new(239, 192, 152, 143);
            let port = 6771;
            
            // Allow multiple listeners on the same port for testing/running multiple clients
            let socket = match UdpSocket::bind(("0.0.0.0", port)) {
                Ok(s) => s,
                Err(e) => {
                    crate::log_debug!("LSD failed to bind UDP port 6771: {}", e);
                    return;
                }
            };

            if let Err(e) = socket.join_multicast_v4(&multicast_ip, &Ipv4Addr::new(0, 0, 0, 0)) {
                crate::log_debug!("LSD failed to join multicast group: {}", e);
                return;
            }

            let mut buf = [0u8; 1024];
            let expected_infohash_hex = crate::util::info_hash_to_string(&self.info_hash);

            loop {
                match socket.recv_from(&mut buf) {
                    Ok((n, src)) => {
                        if let Ok(msg) = std::str::from_utf8(&buf[..n]) {
                            let mut port = None;
                            let mut infohash_found = false;
                            for line in msg.lines() {
                                let line_trimmed = line.trim();
                                if line_trimmed.to_uppercase().starts_with("PORT:") {
                                    if let Some(p_str) = line_trimmed.split(':').nth(1) {
                                        if let Ok(p) = p_str.trim().parse::<u16>() {
                                            port = Some(p);
                                        }
                                    }
                                }
                                if line_trimmed.to_uppercase().starts_with("INFOHASH:") {
                                    if let Some(h_str) = line_trimmed.split(':').nth(1) {
                                        if h_str.trim().to_lowercase() == expected_infohash_hex.to_lowercase() {
                                            infohash_found = true;
                                        }
                                    }
                                }
                            }

                            if infohash_found {
                                if let Some(p) = port {
                                    let peer_ip = src.ip().to_string();
                                    let _ = self.peer_tx.send(PeerDetails {
                                        info_hash: self.info_hash.clone(),
                                        peer_id: None,
                                        ip: peer_ip,
                                        port: p,
                                    });
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    }
}

pub struct LsdAnnouncer {
    info_hash: Vec<u8>,
    local_port: u16,
}

impl LsdAnnouncer {
    pub fn new(info_hash: Vec<u8>, local_port: u16) -> Self {
        LsdAnnouncer {
            info_hash,
            local_port,
        }
    }

    pub fn start(self, context: Arc<Mutex<crate::core::torrent_context::TorrentContext>>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let socket = match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(_) => return,
            };
            let dest = SocketAddrV4::new(Ipv4Addr::new(239, 192, 152, 143), 6771);
            let infohash_hex = crate::util::info_hash_to_string(&self.info_hash);

            loop {
                {
                    let ctx = context.lock().unwrap();
                    if ctx.status == crate::core::torrent_context::TorrentStatus::Ended {
                        break;
                    }
                }

                let packet = format!(
                    "BT-SEARCH * HTTP/1.1\r\n\
                     Host: 239.192.152.143:6771\r\n\
                     Port: {}\r\n\
                     Infohash: {}\r\n\
                     \r\n",
                    self.local_port, infohash_hex
                );

                let _ = socket.send_to(packet.as_bytes(), dest);
                std::thread::sleep(Duration::from_secs(300));
            }
        })
    }
}
