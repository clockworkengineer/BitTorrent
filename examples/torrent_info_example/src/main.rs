use bittorrent_rs::MetaInfoFile;
use std::env;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let torrent_path = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("Usage: torrent_info_example <torrent-file-path>");
            std::process::exit(1);
        }
    };

    let path = Path::new(&torrent_path);
    if !path.exists() {
        eprintln!("Error: File does not exist: {}", path.display());
        std::process::exit(1);
    }

    // 1. Load the torrent metainfo file
    let mut metainfo = MetaInfoFile::new(path)?;

    // 2. Parse the bencoded content
    metainfo.parse()?;

    // 3. Print parsed details
    println!("=== Torrent Metadata Info ===");
    println!("File: {}", path.display());
    println!("Is BitTorrent V2: {}", metainfo.is_v2());
    println!("Is Private: {}", metainfo.is_private());

    if let Ok(info_hash) = metainfo.get_info_hash() {
        println!("Info Hash: {}", bittorrent_rs::util::info_hash_to_string(&info_hash));
    }

    if let Ok(piece_length) = metainfo.get_piece_length() {
        println!("Piece Length: {} bytes ({:.1} KB)", piece_length, piece_length as f32 / 1024.0);
    }

    if let Ok(tracker) = metainfo.get_tracker() {
        println!("Primary Tracker: {}", tracker);
    }

    if let Ok(trackers) = metainfo.get_tracker_urls() {
        println!("All Trackers ({}):", trackers.len());
        for t in trackers {
            println!("  - {}", t);
        }
    }

    let web_seeds = metainfo.get_web_seeds();
    if !web_seeds.is_empty() {
        println!("Web Seeds ({}):", web_seeds.len());
        for ws in web_seeds {
            println!("  - {}", ws);
        }
    }

    // 4. Resolve download file list
    println!("\n=== Download Files ===");
    match metainfo.local_files_to_download_list(Path::new("downloads")) {
        Ok((total_size, files)) => {
            println!("Total Size: {} bytes ({:.2} MB)", total_size, total_size as f64 / (1024.0 * 1024.0));
            println!("Files ({}):", files.len());
            for file in files {
                println!("  - {} ({} bytes, offset: {})", file.name, file.length, file.offset);
            }
        }
        Err(e) => {
            println!("Failed to resolve files list: {}", e);
        }
    }

    Ok(())
}
