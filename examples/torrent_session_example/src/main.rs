use bittorrent_rs::{SessionEvent, TorrentSession, TorrentStatus};
use std::env;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let torrent_path = args
        .next()
        .expect("Usage: torrent_session_example <torrent-file> <download-dir>");
    let download_dir = args
        .next()
        .expect("Usage: torrent_session_example <torrent-file> <download-dir>");

    let torrent_path = PathBuf::from(torrent_path);
    let download_dir = PathBuf::from(download_dir);

    println!("=== BitTorrent Session Modern Facade & Event Stream Example ===");
    println!("Loaded torrent from: {}", torrent_path.display());
    println!("Download directory: {}", download_dir.display());

    // 1. Build session using fluent typestate builder and subscribe to events
    let builder = TorrentSession::builder(&torrent_path, &download_dir).with_seeding(false);

    let mut session = builder.build().expect("Failed to create torrent session");

    // Subscribe to strongly typed asynchronous events
    let events = session.events();

    println!("Initial status: {:?}", session.status());

    // 2. Start the session using the modern high-level session.start() facade
    session.start().expect("Failed to start session");
    println!("Session started via high-level facade.");
    println!("Current progress: {:.2}%", session.progress());

    // Read the initial event from the channel stream
    if let Ok(SessionEvent::StateChanged(st)) = events.try_recv() {
        println!("Received StateChanged event: {:?}", st);
    }

    if session.status() == TorrentStatus::Downloading {
        println!("Torrent is ready for peer download operations.");
    }

    // 3. Demonstrate lifecycle control and event observation
    session.pause().expect("Failed to pause");
    if let Ok(SessionEvent::StateChanged(st)) = events.try_recv() {
        println!("Event after pause: StateChanged({:?})", st);
    }
    println!("Paused. Status={:?}", session.status());

    session.resume().expect("Failed to resume");
    if let Ok(SessionEvent::StateChanged(st)) = events.try_recv() {
        println!("Event after resume: StateChanged({:?})", st);
    }
    println!("Resumed. Status={:?}", session.status());

    session.stop().expect("Failed to stop");
    if let Ok(event) = events.try_recv() {
        println!("Event after stop: {:?}", event);
    }
    println!("Stopped. Final status={:?}", session.status());
}
