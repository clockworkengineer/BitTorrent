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
