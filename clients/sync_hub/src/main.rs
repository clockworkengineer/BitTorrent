use std::path::PathBuf;
use std::env;
use sync_hub::manager::SyncManager;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let sync_dir = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from("downloads/sync_folder")
    };

    let listen_port = if args.len() > 2 {
        args[2].parse::<u16>()?
    } else {
        6881
    };

    std::fs::create_dir_all(&sync_dir)?;

    let mut manager = SyncManager::new(sync_dir, listen_port);
    manager.start()?;

    println!("Sync Hub is running. Press Ctrl+C to stop.");
    // Keep main thread alive
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
