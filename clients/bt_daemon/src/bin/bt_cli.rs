//! BitTorrent CLI Companion Tool (bt_cli)
//!
//! Connects to the running bt_daemon via a Unix socket or Windows Named Pipe,
//! sends control commands, and displays statuses/results.

use torrent_client_shared::{IpcMessage, IpcReply, TorrentStatusInfo, fmt_bytes};
use std::env;

#[cfg(unix)]
mod ipc_client {
    use std::os::unix::net::UnixStream;
    use std::io::{Write, BufRead, BufReader};

    pub fn send_message(name: &str, msg: &str) -> std::io::Result<String> {
        let path = format!("/tmp/{}.sock", name);
        let mut stream = UnixStream::connect(path)?;
        let mut msg_bytes = msg.as_bytes().to_vec();
        msg_bytes.push(b'\n');
        stream.write_all(&msg_bytes)?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        reader.read_line(&mut reply)?;
        Ok(reply)
    }
}

#[cfg(windows)]
mod ipc_client {
    use std::fs::OpenOptions;
    use std::io::{Write, BufReader, BufRead};

    pub fn send_message(name: &str, msg: &str) -> std::io::Result<String> {
        let pipe_path = format!("\\\\.\\pipe\\{}", name);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)?;
        let mut msg_bytes = msg.as_bytes().to_vec();
        msg_bytes.push(b'\n');
        file.write_all(&msg_bytes)?;
        file.flush()?;

        let mut reader = BufReader::new(file);
        let mut reply = String::new();
        reader.read_line(&mut reply)?;
        Ok(reply)
    }
}

fn print_usage() {
    println!("BitTorrent CLI Companion Tool (bt-cli)\n");
    println!("Usage:");
    println!("  bt-cli add <torrent-or-magnet-path> [--dir <download-dir>]");
    println!("  bt-cli status");
    println!("  bt-cli pause <info-hash>");
    println!("  bt-cli resume <info-hash>");
    println!("  bt-cli remove <info-hash> [--purge]");
    println!("  bt-cli files <info-hash>");
    println!("  bt-cli set-priority <info-hash> <file-index> <skip|low|normal|high>");
    println!("  bt-cli peers <info-hash>");
    println!("  bt-cli pieces <info-hash>");
    println!("  bt-cli shutdown");
}

fn print_status_table(torrents: &[TorrentStatusInfo]) {
    if torrents.is_empty() {
        println!("No active torrents in daemon.");
        return;
    }

    println!("{:<25} | {:<40} | {:<8} | {:<12} | {:<7} | {:<10} | {:<20} | {:<20}",
             "Name", "Info Hash", "Progress", "Status", "Peers", "Speed", "Downloaded / Total", "Download Dir");
    println!("{}", "-".repeat(155));
    for t in torrents {
        let progress_str = format!("{:.1}%", t.progress * 100.0);
        let peers_str = format!("{}({})", t.peers_active, t.peers_connected);
        let speed_str = format!("{}/s", fmt_bytes(t.download_rate));
        let downloaded_str = format!("{} / {}", fmt_bytes(t.downloaded), fmt_bytes(t.total_size));
        
        let truncated_name = if t.name.len() > 22 {
            format!("{}...", &t.name[..22])
        } else {
            t.name.clone()
        };

        println!("{:<25} | {:<40} | {:<8} | {:<12} | {:<7} | {:<10} | {:<20} | {:<20}",
                 truncated_name, t.info_hash, progress_str, t.status, peers_str, speed_str, downloaded_str, t.download_dir);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    let command = args[1].as_str();
    let message = match command {
        "add" => {
            if args.len() < 3 {
                eprintln!("Error: 'add' command requires a torrent file path or magnet link.");
                return;
            }
            let torrent_path = args[2].clone();
            let mut download_dir = None;
            let mut idx = 3;
            while idx < args.len() {
                if args[idx] == "--dir" && idx + 1 < args.len() {
                    download_dir = Some(args[idx + 1].clone());
                    break;
                }
                idx += 1;
            }
            IpcMessage::Add { torrent_path, download_dir }
        }
        "status" => IpcMessage::Status,
        "pause" => {
            if args.len() < 3 {
                eprintln!("Error: 'pause' command requires an info hash.");
                return;
            }
            IpcMessage::Pause { info_hash: args[2].clone() }
        }
        "resume" => {
            if args.len() < 3 {
                eprintln!("Error: 'resume' command requires an info hash.");
                return;
            }
            IpcMessage::Resume { info_hash: args[2].clone() }
        }
        "remove" => {
            if args.len() < 3 {
                eprintln!("Error: 'remove' command requires an info hash.");
                return;
            }
            let delete_data = args.iter().any(|arg| arg == "--purge");
            IpcMessage::Remove { info_hash: args[2].clone(), delete_data }
        }
        "files" => {
            if args.len() < 3 {
                eprintln!("Error: 'files' command requires an info hash.");
                return;
            }
            IpcMessage::Files { info_hash: args[2].clone() }
        }
        "set-priority" | "priority" => {
            if args.len() < 5 {
                eprintln!("Error: 'set-priority' requires <info-hash> <file-index> <skip|low|normal|high>");
                return;
            }
            let info_hash = args[2].clone();
            let file_index = match args[3].parse::<usize>() {
                Ok(i) => i,
                Err(_) => {
                    eprintln!("Error: <file-index> must be an integer");
                    return;
                }
            };
            let priority = args[4].clone();
            IpcMessage::SetFilePriority { info_hash, file_index, priority }
        }
        "peers" => {
            if args.len() < 3 {
                eprintln!("Error: 'peers' command requires an info hash.");
                return;
            }
            IpcMessage::Peers { info_hash: args[2].clone() }
        }
        "pieces" => {
            if args.len() < 3 {
                eprintln!("Error: 'pieces' command requires an info hash.");
                return;
            }
            IpcMessage::Pieces { info_hash: args[2].clone() }
        }
        "shutdown" => IpcMessage::Shutdown,
        _ => {
            eprintln!("Unknown command: {}", command);
            print_usage();
            return;
        }
    };

    let serialized = match serde_json::to_string(&message) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to serialize command: {}", e);
            return;
        }
    };

    println!("Connecting to daemon...");
    match ipc_client::send_message("bt-daemon", &serialized) {
        Ok(response_str) => {
            let reply: IpcReply = match serde_json::from_str(&response_str) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Received invalid response from daemon: {}", e);
                    eprintln!("Raw response: {}", response_str);
                    return;
                }
            };

            match reply {
                IpcReply::Success { message } => {
                    println!("Success: {}", message);
                }
                IpcReply::StatusList { torrents } => {
                    print_status_table(&torrents);
                }
                IpcReply::FileList { files } => {
                    println!("{:<6} {:<45} {:<12} {:<7} {}", "INDEX", "NAME", "SIZE", "MEDIA", "PRIORITY");
                    println!("{}", "-".repeat(82));
                    for f in files {
                        println!(
                            "{:<6} {:<45} {:<12} {:<7} {}",
                            f.index,
                            if f.name.len() > 43 { format!("{}...", &f.name[..40]) } else { f.name },
                            torrent_client_shared::fmt_bytes(f.length),
                            if f.is_media { "YES" } else { "NO" },
                            f.priority
                        );
                    }
                }
                IpcReply::PeerList { peers } => {
                    if peers.is_empty() {
                        println!("No peers currently connected to this torrent.");
                    } else {
                        println!("{:<22} | {:<24} | {:<8} | {:<12} | {:<12} | {}",
                                 "IP:PORT", "CLIENT", "PROGRESS", "DOWN RATE", "UP RATE", "FLAGS");
                        println!("{}", "-".repeat(95));
                        for p in peers {
                            let endpoint = format!("{}:{}", p.ip, p.port);
                            let progress_str = format!("{:.1}%", p.progress * 100.0);
                            let down_str = format!("{}/s", fmt_bytes(p.down_rate));
                            let up_str = format!("{}/s", fmt_bytes(p.up_rate));
                            println!("{:<22} | {:<24} | {:<8} | {:<12} | {:<12} | {}",
                                     endpoint, p.client, progress_str, down_str, up_str, p.flags);
                        }
                    }
                }
                IpcReply::PieceStates { info } => {
                    let pct = if info.total_pieces > 0 {
                        (info.have_pieces as f64 / info.total_pieces as f64) * 100.0
                    } else {
                        0.0
                    };
                    println!("Piece Summary: {} / {} completed ({:.1}%) • Piece Size: {}",
                             info.have_pieces, info.total_pieces, pct, fmt_bytes(info.piece_length as u64));
                    
                    let max_rarity = info.swarm_rarity.iter().copied().max().unwrap_or(0);
                    println!("Swarm Availability: Max peer copies for a piece: {}", max_rarity);

                    // Render mini ASCII availability / completion bar (50 characters wide)
                    let bar_width = 50.min(info.total_pieces);
                    if bar_width > 0 {
                        let mut bar = String::with_capacity(bar_width);
                        for i in 0..bar_width {
                            let piece_idx = (i * info.total_pieces) / bar_width;
                            if info.local_have.get(piece_idx).copied().unwrap_or(false) {
                                bar.push('█'); // Completed locally
                            } else if info.swarm_rarity.get(piece_idx).copied().unwrap_or(0) > 0 {
                                bar.push('▒'); // Available in swarm
                            } else {
                                bar.push('░'); // Missing everywhere
                            }
                        }
                        println!("\nProgress: [{}]", bar);
                        println!("Legend:   █ = Completed locally   ▒ = In Swarm   ░ = Missing\n");
                    }
                }
                IpcReply::Error { reason } => {
                    eprintln!("Error from daemon: {}", reason);
                }
            }
        }
        Err(e) => {
            eprintln!("Could not connect to daemon or transmit command: {}", e);
            eprintln!("Is the daemon running? Start it first.");
        }
    }
}
