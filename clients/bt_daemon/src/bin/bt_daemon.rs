//! BitTorrent Seeding & Backup Daemon (bt_daemon)
//!
//! A background daemon that hosts a bittorrent session manager, persists active torrent state,
//! and listens on a local Unix socket or Named Pipe for command-line instructions.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bt_daemon::engine::Engine;
use bt_daemon::ipc::IpcServer;
use bt_daemon::ipc_handler::handle_ipc_message;
use bt_daemon::repository::DaemonStateRepository;
use bt_daemon::streaming::start_stream_server;
use torrent_client_shared::{IpcMessage, IpcReply};

fn main() {
    let mut stream_port = 8081u16;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        if (args[i] == "--stream-port" || args[i] == "-s") && i + 1 < args.len() {
            if let Ok(p) = args[i + 1].parse::<u16>() {
                stream_port = p;
            }
            i += 2;
        } else {
            i += 1;
        }
    }

    let mut state_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    state_dir.push("BitTorrent-rs");
    let _ = std::fs::create_dir_all(&state_dir);
    let state_file = state_dir.join("daemon_state.json");

    let repository = DaemonStateRepository::new(state_file);
    let engine = Arc::new(Mutex::new(Engine::new(repository, "downloads".to_string())));

    // Load existing state
    {
        let mut e = engine.lock().unwrap();
        e.load_state();
        println!(
            "Daemon initialized. Loaded {} active sessions.",
            e.pool.len()
        );
    }

    // Launch HTTP Media Stream Server
    let engine_stream = engine.clone();
    start_stream_server(engine_stream, stream_port);

    // Set up shutdown channel
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

    // Ctrl+C Handler
    let shutdown_tx_ctrlc = shutdown_tx.clone();
    ctrlc::set_handler(move || {
        println!("\nShutdown signal received (Ctrl+C)...");
        let _ = shutdown_tx_ctrlc.send(());
    })
    .expect("Error setting Ctrl-C handler");

    // Launch IPC Server
    let engine_ipc = engine.clone();
    let shutdown_tx_ipc = shutdown_tx.clone();
    std::thread::spawn(move || {
        let server = IpcServer::new("bt-daemon");
        println!("IPC server listening...");
        let _ = server.listen(move |req_str| {
            let req: IpcMessage = match serde_json::from_str(&req_str) {
                Ok(m) => m,
                Err(e) => {
                    return serde_json::to_string(&IpcReply::Error {
                        reason: format!("JSON Parse Error: {}", e),
                    })
                    .unwrap();
                }
            };

            let reply = handle_ipc_message(req, &engine_ipc, &shutdown_tx_ipc);

            serde_json::to_string(&reply).unwrap_or_else(|_| {
                "{\"type\":\"Error\",\"data\":{\"reason\":\"Serialization error\"}}".to_string()
            })
        });
    });

    // Wait for shutdown signal
    let _ = shutdown_rx.recv();
    println!("Saving state and stopping all torrent sessions... This may take a moment.");
    let mut e = engine.lock().unwrap();
    e.save_state();
    e.stop_all();
    println!("Daemon terminated cleanly.");
}
