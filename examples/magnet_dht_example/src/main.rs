use bittorrent_rs::{MagnetLink, TorrentSession};
use std::env;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let magnet_uri = match args.next() {
        Some(uri) => uri,
        None => {
            eprintln!("Usage: magnet_dht_example <magnet-uri> [download-dir]");
            eprintln!("Example: magnet_dht_example \"magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel\"");
            std::process::exit(1);
        }
    };
    let download_dir = args.next().unwrap_or_else(|| "downloads".to_string());

    // 1. Parse the Magnet Link
    println!("Parsing magnet link...");
    let magnet = MagnetLink::parse(&magnet_uri)?;

    println!("\n=== Magnet Link Details ===");
    println!("Info Hash (Hex): {}", bittorrent_rs::util::info_hash_to_string(&magnet.info_hash));
    println!("Display Name:   {:?}", magnet.display_name);
    println!("Trackers List:");
    if magnet.trackers.is_empty() {
        println!("  - None specified in magnet link.");
    } else {
        for t in &magnet.trackers {
            println!("  - {}", t);
        }
    }

    // 2. Initialize a Magnet-based TorrentSession
    println!("\nInitializing torrent session with magnet link...");
    let download_path = Path::new(&download_dir);
    let session = TorrentSession::from_magnet(&magnet_uri, download_path)
        .build()?;

    println!("Session status: {:?}", session.status());
    println!("This session is ready to announce to trackers and download via magnet metadata bootstrap.");

    Ok(())
}
