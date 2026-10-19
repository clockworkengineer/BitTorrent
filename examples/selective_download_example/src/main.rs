use bittorrent_rs::{MetaInfoFile, TorrentSession};
use std::env;
use std::path::{Path, PathBuf};

fn print_usage() {
    eprintln!(
        "Usage: selective_download_example <torrent-file-or-magnet> <download-dir> [selected-indices]"
    );
    eprintln!();
    eprintln!("Arguments:");
    eprintln!("  <torrent-file-or-magnet> Path to a .torrent file or a magnet URI");
    eprintln!("  <download-dir>           Destination download folder");
    eprintln!(
        "  [selected-indices]       Comma-separated list or ranges of 0-based file indices to download (e.g. '0,2,4-6')"
    );
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  selective_download_example multi_episode.torrent ./downloads 0,3");
    eprintln!(
        "  selective_download_example \"magnet:?xt=urn:btih:...&dn=Archive\" ./downloads 1-3,5"
    );
}

fn parse_file_selection(spec: &str) -> Vec<usize> {
    let mut indices = Vec::new();
    for token in spec.split(',') {
        let token = token.trim();
        if let Some((start_str, end_str)) = token.split_once('-') {
            if let (Ok(start), Ok(end)) = (
                start_str.trim().parse::<usize>(),
                end_str.trim().parse::<usize>(),
            ) {
                for i in start..=end {
                    if !indices.contains(&i) {
                        indices.push(i);
                    }
                }
            }
        } else if let Ok(idx) = token.parse::<usize>()
            && !indices.contains(&idx)
        {
            indices.push(idx);
        }
    }
    indices.sort_unstable();
    indices
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let source = &args[0];
    let download_dir = PathBuf::from(&args[1]);
    let selection_arg = args.get(2).map(|s| s.as_str()).unwrap_or("0");

    let selected_indices = parse_file_selection(selection_arg);

    println!("=== BEP 53 Selective Download & WebSeed Mirror Manager ===");
    println!("Target Source:    {}", source);
    println!("Download Folder:  {}", download_dir.display());
    println!("Selected Files:   {:?}", selected_indices);
    println!();

    std::fs::create_dir_all(&download_dir)?;

    let mut session = if source.starts_with("magnet:") {
        println!("Configuring session from Magnet URI with BEP 53 selective file indices...");
        TorrentSession::from_magnet(source, &download_dir)
            .select_files(&selected_indices)
            .enable_webseed_health_tracker(true)
            .build()?
    } else {
        println!("Inspecting .torrent metainfo...");
        let mut meta = MetaInfoFile::new(Path::new(source))?;
        meta.parse()?;
        println!(
            "Torrent Name:   {}",
            meta.get_name().unwrap_or_else(|_| "unnamed".to_string())
        );
        let total_size = meta.get_total_size().unwrap_or(0);
        println!("Total Size:     {} bytes", total_size);
        let (_, files) = meta.local_files_to_download_list(Path::new("."))?;
        println!("\nFile Manifest (Total: {} files):", files.len());
        for (i, f) in files.iter().enumerate() {
            let is_selected = selected_indices.contains(&i);
            let mark = if is_selected {
                " [*] DOWNLOAD"
            } else {
                " [ ] SKIP"
            };
            println!(
                "  [{:>2}] {:<14} {:>10} bytes {}",
                i, f.name, f.length, mark
            );
        }

        println!("\nInitializing session with BEP 53 file filtering...");
        TorrentSession::builder(Path::new(source), &download_dir)
            .select_files(&selected_indices)
            .enable_webseed_health_tracker(true)
            .build()?
    };

    // Evaluate bandwidth and byte savings
    if let Some(eval) = session.bep53_evaluation() {
        println!("\n=== BEP 53 Selection Evaluation ===");
        println!(
            "Total Torrent Payload: {} bytes across {} files",
            eval.total_bytes,
            eval.unselected_files.len() + eval.selected_files.len()
        );
        println!("Selected Payload:      {} bytes", eval.selected_bytes);
        println!(
            "Piece Count Needed:    {} / {} pieces",
            eval.selected_piece_count, eval.total_piece_count
        );
        println!(
            "Bandwidth Saved:       {:.2}% bytes saved ({:.2}% fewer pieces)",
            eval.byte_savings_percent, eval.piece_savings_percent
        );
    }

    // Demonstrate adding dynamic WebSeed HTTP mirrors
    println!("\n=== Registering WebSeed HTTP Mirror Endpoints ===");
    session.add_webseed("https://mirror1.ch.openwebtorrent.com/files/");
    session.add_webseed("https://mirror2.us.openwebtorrent.com/files/");
    session.add_webseed("https://fallback.openwebtorrent.com/files/");
    println!("Registered 3 WebSeed endpoints with automatic circuit-breaker and latency tracking.");

    // Query mirror summaries
    let mirror_summaries = session.webseed_mirror_summary();
    println!("\nCurrent WebSeed Mirror Health Status:");
    for m in &mirror_summaries {
        println!(
            "  - {} | Score: {:.1}/100 | Latency: {}ms | Requests: ({} ok / {} fail) | Penalized: {}",
            m.endpoint_url,
            m.health_score,
            m.smoothed_latency_ms,
            m.successful_requests,
            m.failed_requests,
            m.is_penalized
        );
    }

    // Start download
    session.start_download()?;
    println!("\nDownload started! Press Ctrl+C to terminate.");
    println!("Only pieces covering selected files will be requested from peers and web seeds.");

    Ok(())
}
