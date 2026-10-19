use bittorrent_rs::TorrentSession;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let torrent_path = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("Usage: torrent_fast_resume_example <torrent-file> <download-dir>");
            std::process::exit(1);
        }
    };
    let download_dir = match args.next() {
        Some(dir) => dir,
        None => {
            eprintln!("Usage: torrent_fast_resume_example <torrent-file> <download-dir>");
            std::process::exit(1);
        }
    };

    let torrent_path = PathBuf::from(torrent_path);
    let download_dir = PathBuf::from(download_dir);

    // 1. Build session with standard full hash validation
    println!("--- Standard Startup (Full Hash Checking) ---");
    let start_std = Instant::now();
    let session_std = TorrentSession::builder(&torrent_path, &download_dir).build()?;
    println!("Standard session initialized in: {:?}", start_std.elapsed());
    println!("Standard initial progress: {:.2}%", session_std.progress());

    // 2. Save Fast Resume file (BEP 47 / libtorrent format)
    let resume_path = download_dir.join("state.fastresume");
    session_std.save_fast_resume(&resume_path)?;
    println!("Saved BEP 47 fast resume file to: {:?}", resume_path);

    // Cleanup standard session resources
    drop(session_std);

    // 3. Build session with BEP 47 fast resume file
    println!("\n--- Fast Resume Startup (BEP 47 Fast On-Disk Validation) ---");
    let start_fast = Instant::now();
    let session_fast = TorrentSession::builder(&torrent_path, &download_dir)
        .fast_resume_file(&resume_path)
        .build()?;
    println!(
        "Fast resume session initialized in: {:?}",
        start_fast.elapsed()
    );
    println!(
        "Fast resume initial progress: {:.2}%",
        session_fast.progress()
    );

    // Displaying piece selection info
    let context = session_fast.context();
    let ctx = context.lock().unwrap();
    println!(
        "Total pieces: {} | Missing pieces: {}",
        ctx.number_of_pieces, ctx.missing_pieces_count
    );

    Ok(())
}
