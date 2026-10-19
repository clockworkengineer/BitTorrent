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
    println!("  bt-cli super-seed <info-hash> <on|off>");
    println!("  bt-cli peers <info-hash>");
    println!("  bt-cli resume-info <path-to-fastresume-file>");
    println!("  bt-cli upgrade <input.torrent> <data-dir> [--format <hybrid|v2>] [--output <output.torrent>]");
    println!("  bt-cli v2-info <torrent-path>");
    println!("  bt-cli magnet-info <magnet-uri> [--inject-webtorrent] [--web-seed <url>]");
    println!("  bt-cli recheck <torrent-path> <data-dir> [--workers <N>]");
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

        let status_str = if t.is_super_seeding {
            format!("{} [SS]", t.status)
        } else {
            t.status.clone()
        };

        println!("{:<25} | {:<40} | {:<8} | {:<12} | {:<7} | {:<10} | {:<20} | {:<20}",
                 truncated_name, t.info_hash, progress_str, status_str, peers_str, speed_str, downloaded_str, t.download_dir);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        return;
    }

    let command = args[1].as_str();

    if command == "resume-info" {
        if args.len() < 3 {
            eprintln!("Error: 'resume-info' command requires a path to a fast-resume file.");
            eprintln!("Usage: bt-cli resume-info <path-to-fastresume-file>");
            return;
        }
        let resume_path = std::path::Path::new(&args[2]);
        match bittorrent_rs::FastResumeData::load_from_file(resume_path) {
            Ok(data) => {
                let info_hash_hex = data.info_hash.iter().map(|b| format!("{:02x}", b)).collect::<String>();
                println!("Fast Resume Metadata (BEP 47):");
                println!("  File:             {}", resume_path.display());
                println!("  Info Hash:        {}", info_hash_hex);
                println!("  Save Path:        {}", data.save_path.display());
                println!("  Pieces:           {} / {} ({:.1}%)",
                    data.pieces_have_count(),
                    data.num_pieces,
                    data.progress() * 100.0,
                );
                println!("  Piece Length:     {}", fmt_bytes(data.piece_length as u64));
                println!("  Downloaded:       {}", fmt_bytes(data.total_downloaded));
                println!("  Uploaded:         {}", fmt_bytes(data.total_uploaded));
                println!("  Active Time:      {}s", data.active_time);
                println!("  Seeding Time:     {}s", data.seeding_time);
                println!("  Seeding:          {}", if data.is_seeding { "YES" } else { "NO" });
                println!("  Files ({}) :", data.files.len());
                for (i, f) in data.files.iter().enumerate() {
                    let pri_str = match f.priority {
                        0 => "Skip",
                        1 => "Low",
                        7 => "High",
                        _ => "Normal",
                    };
                    let display_path = if f.path.as_os_str().is_empty() {
                        format!("file_{}", i + 1)
                    } else {
                        f.path.display().to_string()
                    };
                    println!("    {:>3}. {} ({} bytes, mtime: {}, priority: {})",
                        i + 1,
                        display_path,
                        f.length,
                        f.mtime,
                        pri_str
                    );
                }
                println!("  Known Peers ({}):", data.peers.len());
                for p in &data.peers {
                    println!("    - {}", p);
                }

                let verified = data.verify_on_disk(&data.save_path).unwrap_or(false);
                println!("\n  On-Disk Fast Verification: {}", if verified { "PASS (Valid)" } else { "FAIL (Disk mismatch or files missing)" });
            }
            Err(e) => {
                eprintln!("Failed to parse fast-resume file: {}", e);
            }
        }
        return;
    }

    if command == "upgrade" {
        if args.len() < 4 {
            eprintln!("Error: 'upgrade' command requires an input torrent file and data directory.");
            eprintln!("Usage: bt-cli upgrade <input.torrent> <data-dir> [--format <hybrid|v2>] [--output <output.torrent>]");
            return;
        }
        let input_path = std::path::Path::new(&args[2]);
        let data_dir = std::path::Path::new(&args[3]);
        let mut format = bittorrent_rs::TorrentExportFormat::Hybrid;
        let mut output_path = None;

        let mut idx = 4;
        while idx < args.len() {
            if args[idx] == "--format" && idx + 1 < args.len() {
                match args[idx + 1].to_ascii_lowercase().as_str() {
                    "v2" | "pure-v2" | "pure_v2" => format = bittorrent_rs::TorrentExportFormat::V2,
                    "hybrid" => format = bittorrent_rs::TorrentExportFormat::Hybrid,
                    other => {
                        eprintln!("Unknown format '{}'. Supported: hybrid, v2", other);
                        return;
                    }
                }
                idx += 2;
            } else if args[idx] == "--output" && idx + 1 < args.len() {
                output_path = Some(std::path::PathBuf::from(&args[idx + 1]));
                idx += 2;
            } else {
                idx += 1;
            }
        }

        let out_file = output_path.unwrap_or_else(|| {
            let stem = input_path.file_stem().and_then(|s| s.to_str()).unwrap_or("upgraded");
            let ext = match format {
                bittorrent_rs::TorrentExportFormat::V1 => "v1.torrent",
                bittorrent_rs::TorrentExportFormat::V2 => "v2.torrent",
                bittorrent_rs::TorrentExportFormat::Hybrid => "hybrid.torrent",
            };
            input_path.with_file_name(format!("{}.{}", stem, ext))
        });

        println!("Upgrading torrent metainfo...");
        println!("  Input:            {}", input_path.display());
        println!("  Data Dir:         {}", data_dir.display());
        println!("  Target Format:    {:?}", format);
        println!("  Output File:      {}", out_file.display());

        match bittorrent_rs::TorrentUpgrader::upgrade_from_file(input_path, data_dir, format) {
            Ok((torrent_bytes, report)) => {
                if let Err(e) = std::fs::write(&out_file, &torrent_bytes) {
                    eprintln!("Failed to write upgraded torrent file: {}", e);
                    return;
                }
                println!("\nUpgrade Complete!");
                println!("  Name:             {}", report.name);
                println!("  Format:           {:?}", report.format);
                println!("  Total Size:       {} ({} bytes)", fmt_bytes(report.total_size), report.total_size);
                println!("  Piece Length:     {}", fmt_bytes(report.piece_length as u64));
                println!("  v1 Infohash:      {}", report.v1_info_hash.as_deref().unwrap_or("N/A (Pure v2)"));
                println!("  v2 Infohash:      {}", report.v2_info_hash.as_deref().unwrap_or("N/A"));
                println!("  v2 Truncated:     {}", report.v2_truncated_info_hash.as_deref().unwrap_or("N/A"));
                println!("  Piece Layers:     {}", report.piece_layers_count);
                println!("  Files ({}):", report.file_pieces_roots.len());
                for (i, (path, pieces_root)) in report.file_pieces_roots.iter().enumerate() {
                    println!("    {:>3}. {} (pieces root: {})",
                        i + 1,
                        path,
                        pieces_root
                    );
                }
                println!("\n  Canonical Magnet URI:");
                println!("    {}", report.magnet_uri);
                println!("\nSuccessfully wrote upgraded torrent to: {}", out_file.display());
            }
            Err(e) => {
                eprintln!("Error upgrading torrent: {}", e);
            }
        }
        return;
    }

    if command == "v2-info" {
        if args.len() < 3 {
            eprintln!("Error: 'v2-info' command requires a path to a torrent file.");
            eprintln!("Usage: bt-cli v2-info <path-to-torrent-file>");
            return;
        }
        let torrent_path = std::path::Path::new(&args[2]);
        let mut metainfo = match bittorrent_rs::MetaInfoFile::new(torrent_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to open torrent file: {}", e);
                return;
            }
        };
        if let Err(e) = metainfo.parse() {
            eprintln!("Failed to parse torrent file: {}", e);
            return;
        }

        println!("BitTorrent Metainfo Inspection:");
        println!("  File:             {}", torrent_path.display());
        let name = metainfo.get_name().unwrap_or_else(|_| "Unknown".into());
        println!("  Name:             {}", name);

        let is_v2 = metainfo.is_v2();
        let is_hybrid = metainfo.is_hybrid();
        let version_str = if is_hybrid {
            "BitTorrent v2 Hybrid (v1 + v2 dual compatibility)"
        } else if is_v2 {
            "BitTorrent Pure v2 (BEP 52 Merkle trees)"
        } else {
            "BitTorrent v1 (Legacy SHA-1)"
        };
        println!("  Version:          {}", version_str);

        let total_size = metainfo.get_total_size().unwrap_or(0);
        println!("  Total Size:       {} ({} bytes)", fmt_bytes(total_size), total_size);

        if let Ok(pl) = metainfo.get_piece_length() {
            println!("  Piece Length:     {}", fmt_bytes(pl as u64));
        }

        if let Ok(v1_hash) = metainfo.get_v1_info_hash() {
            let v1_hex: String = v1_hash.iter().map(|b| format!("{:02x}", b)).collect();
            println!("  v1 Infohash:      {}", v1_hex);
        } else if !is_v2 {
            if let Ok(hash) = metainfo.get_info_hash() {
                let hex: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
                println!("  v1 Infohash:      {}", hex);
            }
        }

        if let Ok(v2_hash) = metainfo.get_v2_info_hash() {
            let v2_hex: String = v2_hash.iter().map(|b| format!("{:02x}", b)).collect();
            let v2_trunc: String = v2_hash[..20].iter().map(|b| format!("{:02x}", b)).collect();
            println!("  v2 Infohash:      {}", v2_hex);
            println!("  v2 Truncated:     {}", v2_trunc);
        }

        println!("  Piece Layers:     {}", metainfo.get_piece_layers().len());
        let piece_layers_cache_dir = std::path::Path::new(".piece_layers");
        if piece_layers_cache_dir.exists() {
            let mut cached_count = 0;
            let mut cached_bytes = 0u64;
            if let Ok(entries) = std::fs::read_dir(piece_layers_cache_dir) {
                for entry in entries.flatten() {
                    if entry.path().extension().map_or(false, |ext| ext == "layers") {
                        cached_count += 1;
                        if let Ok(meta) = entry.metadata() {
                            cached_bytes += meta.len();
                        }
                    }
                }
            }
            println!("  Layer Disk Cache: {} files cached ({}) in {}", cached_count, fmt_bytes(cached_bytes), piece_layers_cache_dir.display());
        }

        if let Ok((_, files)) = metainfo.local_files_to_download_list(".") {
            let non_pad_files: Vec<_> = files.iter().filter(|f| !f.is_pad_file()).collect();
            println!("  Files ({} non-pad):", non_pad_files.len());
            for (i, f) in non_pad_files.iter().enumerate() {
                let root_str = f.pieces_root_hex().unwrap_or_else(|| "N/A (v1 only)".into());
                println!("    {:>3}. {} ({} bytes, pieces root: {})",
                    i + 1,
                    f.torrent_path,
                    f.length,
                    root_str
                );
            }
        }

        if !is_v2 {
            println!("\n  Notice: This is a legacy v1 torrent. You can upgrade it to Hybrid or Pure v2 using:");
            println!("    bt-cli upgrade \"{}\" <data-dir>", torrent_path.display());
        }

        return;
    }

    if command == "magnet-info" {
        if args.len() < 3 {
            eprintln!("Error: 'magnet-info' command requires a magnet URI.");
            eprintln!("Usage: bt-cli magnet-info <magnet-uri> [--inject-webtorrent] [--web-seed <url>]");
            return;
        }
        let magnet_uri = &args[2];
        let mut magnet = match bittorrent_rs::MagnetLink::parse(magnet_uri) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to parse magnet URI: {}", e);
                return;
            }
        };

        let mut inject_webtorrent = false;
        let mut custom_web_seeds = Vec::new();
        let mut idx = 3;
        while idx < args.len() {
            if args[idx] == "--inject-webtorrent" {
                inject_webtorrent = true;
            } else if args[idx] == "--web-seed" && idx + 1 < args.len() {
                custom_web_seeds.push(args[idx + 1].clone());
                idx += 1;
            }
            idx += 1;
        }

        println!("BitTorrent Magnet URI Inspection:");
        println!("  Display Name:     {}", magnet.display_name.as_deref().unwrap_or("Unknown"));

        let mut v1_hex = String::with_capacity(40);
        for b in &magnet.info_hash {
            v1_hex.push_str(&format!("{:02x}", b));
        }
        println!("  v1 Infohash:      {}", v1_hex);

        if let Some(ref v2) = magnet.v2_info_hash {
            let mut v2_hex = String::with_capacity(64);
            for b in v2 {
                v2_hex.push_str(&format!("{:02x}", b));
            }
            println!("  v2 Infohash:      {}", v2_hex);
        } else {
            println!("  v2 Infohash:      N/A");
        }

        let format_str = if magnet.is_hybrid() {
            "Hybrid (v1 + v2)"
        } else if magnet.is_v2_only() {
            "Pure v2 (BEP 52)"
        } else {
            "v1 (Legacy SHA-1)"
        };
        println!("  Format:           {}", format_str);
        println!("  WebTorrent Cap:   {}", if magnet.is_webtorrent() { "Yes (WebSocket trackers present)" } else { "No" });

        let report = bittorrent_rs::WebTorrentMagnetInjector::extract_trackers(&magnet);
        println!("\n  Trackers ({} total):", report.total_trackers);
        println!("    WebSocket ({}):", report.websocket_trackers.len());
        for t in &report.websocket_trackers {
            println!("      - {}", t);
        }
        println!("    UDP ({}):", report.udp_trackers.len());
        for t in &report.udp_trackers {
            println!("      - {}", t);
        }
        println!("    HTTP/HTTPS ({}):", report.http_trackers.len());
        for t in &report.http_trackers {
            println!("      - {}", t);
        }

        println!("\n  WebSeeds ({}):", magnet.web_seeds.len());
        for ws in &magnet.web_seeds {
            println!("    - {}", ws);
        }

        if inject_webtorrent || !custom_web_seeds.is_empty() {
            let injected_ws = bittorrent_rs::WebTorrentMagnetInjector::inject_web_seeds(&mut magnet, &custom_web_seeds);
            let injected_trackers = if inject_webtorrent {
                bittorrent_rs::WebTorrentMagnetInjector::inject_websocket_trackers(&mut magnet, None)
            } else {
                0
            };
            println!("\n  Injected:");
            println!("    WebSeeds Added:       {}", injected_ws);
            println!("    WS Trackers Added:    {}", injected_trackers);
            println!("\n  Updated Magnet URI:");
            println!("    {}", magnet.to_uri());
        } else {
            println!("\n  Canonical Magnet URI:");
            println!("    {}", magnet.to_uri());
        }

        return;
    }

    if command == "recheck" {
        if args.len() < 4 {
            eprintln!("Error: 'recheck' command requires a path to a torrent file and download directory.");
            eprintln!("Usage: bt-cli recheck <path-to-torrent-file> <data-dir> [--workers <N>]");
            return;
        }
        let torrent_path = std::path::Path::new(&args[2]);
        let data_dir = std::path::Path::new(&args[3]);

        let mut workers = std::thread::available_parallelism()
            .map(|n| n.get().clamp(2, 8))
            .unwrap_or(4);
        let mut idx = 4;
        while idx < args.len() {
            if args[idx] == "--workers" && idx + 1 < args.len() {
                if let Ok(w) = args[idx + 1].parse::<usize>() {
                    workers = w.clamp(1, 32);
                }
                idx += 1;
            }
            idx += 1;
        }

        println!("Asynchronous Piece Hash Verification (Worker Pool):");
        println!("  Torrent File:     {}", torrent_path.display());
        println!("  Data Directory:   {}", data_dir.display());
        println!("  Worker Threads:   {}", workers);

        let mut metainfo = match bittorrent_rs::MetaInfoFile::new(torrent_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to open torrent file: {}", e);
                return;
            }
        };
        if let Err(e) = metainfo.parse() {
            eprintln!("Failed to parse torrent file: {}", e);
            return;
        }

        let piece_length = match metainfo.get_piece_length() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Failed to determine piece length: {}", e);
                return;
            }
        };
        let total_size = metainfo.get_total_size().unwrap_or(0);
        let pieces_hash = match metainfo.get_pieces_info_hash() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to retrieve piece hashes: {}", e);
                return;
            }
        };
        let num_pieces = pieces_hash.len() / 20;

        let (_size, files) = match metainfo.local_files_to_download_list(data_dir) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to resolve file paths: {}", e);
                return;
            }
        };

        let disk_io = std::sync::Arc::new(bittorrent_rs::storage::disk_io::DiskIO::new(
            data_dir,
            files,
            piece_length,
        ));

        let pool = bittorrent_rs::HashVerificationPool::new(workers, 1024);

        println!("  Total Pieces:     {} ({} total)", num_pieces, fmt_bytes(total_size));
        println!("  Piece Length:     {}", fmt_bytes(piece_length as u64));
        println!("\nCommencing parallel multi-threaded verification...");

        let start_time = std::time::Instant::now();
        let (_bitfield, progress) = match pool.recheck_pieces_blocking(
            num_pieces as u32,
            piece_length,
            total_size,
            &pieces_hash,
            disk_io,
        ) {
            Ok(res) => res,
            Err(e) => {
                eprintln!("Verification failed: {}", e);
                return;
            }
        };

        let elapsed = start_time.elapsed();
        let valid = progress.valid_pieces.load(std::sync::atomic::Ordering::Relaxed);
        let corrupt = progress.corrupt_pieces.load(std::sync::atomic::Ordering::Relaxed);
        let metrics = pool.metrics();

        println!("\nVerification Complete:");
        println!("  Valid Pieces:     {} / {}", valid, num_pieces);
        println!("  Corrupted Pieces: {}", corrupt);
        println!("  Bytes Hashed:     {}", fmt_bytes(metrics.total_bytes_verified));
        println!("  Elapsed Time:     {:.2?}", elapsed);
        println!("  Throughput:       {:.2} MB/s", metrics.throughput_mb_s());
        println!(
            "  Integrity Status: {}",
            if corrupt == 0 {
                "100% Passed (All pieces intact)"
            } else {
                "Incomplete/Corrupt (Pieces missing or modified)"
            }
        );

        return;
    }

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
        "super-seed" | "superseed" => {
            if args.len() < 4 {
                eprintln!("Error: 'super-seed' requires <info-hash> <on|off>");
                return;
            }
            let info_hash = args[2].clone();
            let enabled = match args[3].to_lowercase().as_str() {
                "on" | "true" | "1" | "enable" | "enabled" => true,
                "off" | "false" | "0" | "disable" | "disabled" => false,
                _ => {
                    eprintln!("Error: Expected 'on' or 'off' for super-seed state.");
                    return;
                }
            };
            IpcMessage::SetSuperSeeding { info_hash, enabled }
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
