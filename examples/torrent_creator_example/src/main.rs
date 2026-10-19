use bittorrent_rs::{MetaInfoFile, TorrentBuilder};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn print_usage() {
    eprintln!("Usage: torrent_creator_example <input-path> <output.torrent> [format] [piece-length-kb]");
    eprintln!();
    eprintln!("Arguments:");
    eprintln!("  <input-path>        Path to a single file or a directory to package into the torrent");
    eprintln!("  <output.torrent>    Destination path for the created .torrent file");
    eprintln!("  [format]            Torrent format: 'v1', 'v2', or 'hybrid' (default: 'hybrid')");
    eprintln!("  [piece-length-kb]   Piece size in KiB: e.g. 16, 32, 64, 128, 256, 512, 1024 (default: 64)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  torrent_creator_example ./my_movie.mp4 ./my_movie.torrent hybrid 128");
    eprintln!("  torrent_creator_example ./dataset/ ./dataset.torrent v2 256");
}

fn add_files_recursive(
    builder: TorrentBuilder,
    base_dir: &Path,
    current_dir: &Path,
) -> Result<TorrentBuilder, Box<dyn std::error::Error>> {
    let mut b = builder;
    for entry in fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            b = add_files_recursive(b, base_dir, &path)?;
        } else if path.is_file() {
            let rel_path = path
                .strip_prefix(base_dir)?
                .to_string_lossy()
                .replace('\\', "/");
            let data = fs::read(&path)?;
            println!("  + Adding file: {} ({} bytes)", rel_path, data.len());
            b = b.add_file(rel_path, data);
        }
    }
    Ok(b)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let input_path = PathBuf::from(&args[0]);
    let output_torrent_path = PathBuf::from(&args[1]);
    let format = args.get(2).map(|s| s.to_lowercase()).unwrap_or_else(|| "hybrid".to_string());
    let piece_kb: usize = args
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);

    if !input_path.exists() {
        eprintln!("Error: input path '{}' does not exist.", input_path.display());
        std::process::exit(1);
    }

    let piece_length = piece_kb * 1024;
    let torrent_name = input_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed_torrent");

    println!("=== BitTorrent File Generator Example ===");
    println!("Torrent Name:  {}", torrent_name);
    println!("Format Target: {}", format.to_uppercase());
    println!("Piece Length:  {} KiB ({} bytes)", piece_kb, piece_length);
    println!("Input Path:    {}", input_path.display());
    println!("Output Target: {}", output_torrent_path.display());
    println!();

    let mut builder = TorrentBuilder::new(torrent_name)
        .piece_length(piece_length)
        .comment("Created with bittorrent-rs TorrentBuilder")
        .tracker("udp://tracker.opentrackr.org:1337/announce")
        .tracker("udp://tracker.openbittorrent.com:6969/announce")
        .tracker("wss://tracker.openwebtorrent.com") // WebTorrent signaling
        .web_seed("https://webseed.example.com/files/");

    println!("Scanning and hashing files:");
    if input_path.is_file() {
        let file_name = input_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file.dat");
        let data = fs::read(&input_path)?;
        println!("  + Adding file: {} ({} bytes)", file_name, data.len());
        builder = builder.add_file(file_name, data);
    } else if input_path.is_dir() {
        builder = add_files_recursive(builder, &input_path, &input_path)?;
    }

    println!("\nBuilding {} torrent bytes...", format.to_uppercase());
    let torrent_bytes = match format.as_str() {
        "v1" => builder.build_v1()?,
        "v2" => builder.build_v2()?,
        "hybrid" => builder.build_hybrid()?,
        other => {
            eprintln!("Unknown format '{}'. Supported formats are 'v1', 'v2', 'hybrid'.", other);
            std::process::exit(1);
        }
    };

    fs::write(&output_torrent_path, &torrent_bytes)?;
    println!("Saved successfully: {} ({} bytes)", output_torrent_path.display(), torrent_bytes.len());

    // Verify and inspect the created .torrent file
    println!("\n=== Validating Generated Torrent ===");
    let mut metainfo = MetaInfoFile::from_bytes(&torrent_bytes);
    metainfo.parse()?;

    println!("Validation: OK!");
    if let Ok(total_size) = metainfo.get_total_size() {
        println!("Total Size:   {} bytes", total_size);
    }
    if let Ok(pieces) = metainfo.get_pieces_info_hash() {
        println!("v1 Pieces:    {}", pieces.len() / 20);
    }

    if let Ok(v1_hash) = metainfo.get_v1_info_hash() {
        println!("v1 InfoHash (Hex):   {}", bittorrent_rs::util::info_hash_to_string(&v1_hash));
    }
    if let Ok(v2_hash) = metainfo.get_v2_info_hash() {
        println!("v2 InfoHash (Hex):   {}", bittorrent_rs::util::info_hash_to_string(&v2_hash));
    }

    // Build sample Magnet URI
    let mut magnet_uri = format!("magnet:?dn={}", urlencoding::encode(torrent_name));
    if let Ok(v1_hash) = metainfo.get_v1_info_hash() {
        magnet_uri.push_str(&format!("&xt=urn:btih:{}", bittorrent_rs::util::info_hash_to_string(&v1_hash)));
    }
    if let Ok(v2_hash) = metainfo.get_v2_info_hash() {
        magnet_uri.push_str(&format!("&xt=urn:btmh:1220{}", bittorrent_rs::util::info_hash_to_string(&v2_hash)));
    }
    if let Ok(trackers) = metainfo.get_tracker_urls() {
        for tracker in trackers {
            magnet_uri.push_str(&format!("&tr={}", urlencoding::encode(&tracker)));
        }
    }

    println!("\nGenerated Magnet Link:\n{}", magnet_uri);

    Ok(())
}
