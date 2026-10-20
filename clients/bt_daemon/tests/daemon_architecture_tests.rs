use std::sync::{mpsc, Arc, Mutex};

use bt_daemon::engine::Engine;
use bt_daemon::ipc_handler::handle_ipc_message;
use bt_daemon::repository::{DaemonState, DaemonStateRepository, SavedTorrent};
use bt_daemon::session_pool::TorrentSessionPool;
use torrent_client_shared::{IpcMessage, IpcReply};

#[test]
fn test_daemon_state_repository_roundtrip() {
    let temp_dir = std::env::temp_dir().join(format!(
        "bt_daemon_repo_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let state_file = temp_dir.join("test_state.json");
    let repo = DaemonStateRepository::new(state_file.clone());

    // 1. Initial load on non-existent file returns default empty state
    let initial = repo.load().expect("should return default on missing file");
    assert_eq!(initial, DaemonState::default());

    // 2. Save structured state
    let state = DaemonState {
        download_dir: "/path/to/downloads".to_string(),
        torrents: vec![
            SavedTorrent {
                torrent_path: "/path/to/sample1.torrent".to_string(),
                bitfield_hex: "ff00aa".to_string(),
                fast_resume_file: Some("/path/to/sample1.fastresume".to_string()),
            },
            SavedTorrent {
                torrent_path: "magnet:?xt=urn:btih:abcdef".to_string(),
                bitfield_hex: "1234".to_string(),
                fast_resume_file: None,
            },
        ],
    };

    repo.save(&state).expect("save should succeed");
    assert!(state_file.exists());

    // 3. Load back and verify equality
    let loaded = repo.load().expect("load should succeed");
    assert_eq!(loaded, state);

    // 4. Fast resume path computation
    let fr_path = repo.fast_resume_path_for_hash("0123456789abcdef0123456789abcdef01234567");
    assert_eq!(
        fr_path.file_name().unwrap(),
        "0123456789abcdef0123456789abcdef01234567.fastresume"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_torrent_session_pool_empty_and_queries() {
    let mut pool = TorrentSessionPool::new();
    assert_eq!(pool.len(), 0);
    assert!(pool.is_empty());
    assert!(!pool.contains_path("/nonexistent.torrent"));

    // Queries on unknown hash
    assert!(pool.find_by_hash("00112233445566778899aabbccddeeff00112233").is_none());
    assert!(pool.find_by_hash_mut("00112233445566778899aabbccddeeff00112233").is_none());

    // Pause/resume/remove on unknown hash return clean error
    let pause_err = pool.pause("00112233445566778899aabbccddeeff00112233");
    assert!(pause_err.is_err());
    assert_eq!(pause_err.unwrap_err(), "Torrent not found");

    let resume_err = pool.resume("00112233445566778899aabbccddeeff00112233");
    assert!(resume_err.is_err());
    assert_eq!(resume_err.unwrap_err(), "Torrent not found");

    let remove_err = pool.remove("00112233445566778899aabbccddeeff00112233", false);
    assert!(remove_err.is_err());
    assert_eq!(remove_err.unwrap_err(), "Torrent not found");

    let status = pool.get_status_info();
    assert!(status.is_empty());
}

#[test]
fn test_ipc_message_dispatch() {
    let temp_dir = std::env::temp_dir().join(format!(
        "bt_daemon_ipc_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let state_file = temp_dir.join("daemon_state.json");
    let repo = DaemonStateRepository::new(state_file);
    let engine = Arc::new(Mutex::new(Engine::new(repo, "downloads".to_string())));
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    // 1. Status query
    let status_reply = handle_ipc_message(IpcMessage::Status, &engine, &shutdown_tx);
    match status_reply {
        IpcReply::StatusList { torrents } => assert!(torrents.is_empty()),
        other => panic!("expected StatusList, got {:?}", other),
    }

    // 2. Pause non-existent
    let pause_reply = handle_ipc_message(
        IpcMessage::Pause {
            info_hash: "abcd".to_string(),
        },
        &engine,
        &shutdown_tx,
    );
    match pause_reply {
        IpcReply::Error { reason } => assert_eq!(reason, "Torrent not found"),
        other => panic!("expected Error, got {:?}", other),
    }

    // 3. Resume non-existent
    let resume_reply = handle_ipc_message(
        IpcMessage::Resume {
            info_hash: "abcd".to_string(),
        },
        &engine,
        &shutdown_tx,
    );
    match resume_reply {
        IpcReply::Error { reason } => assert_eq!(reason, "Torrent not found"),
        other => panic!("expected Error, got {:?}", other),
    }

    // 4. SetSuperSeeding non-existent
    let ss_reply = handle_ipc_message(
        IpcMessage::SetSuperSeeding {
            info_hash: "abcd".to_string(),
            enabled: true,
        },
        &engine,
        &shutdown_tx,
    );
    match ss_reply {
        IpcReply::Error { reason } => assert_eq!(reason, "Torrent not found"),
        other => panic!("expected Error, got {:?}", other),
    }

    // 5. Shutdown dispatch
    let shutdown_reply = handle_ipc_message(IpcMessage::Shutdown, &engine, &shutdown_tx);
    match shutdown_reply {
        IpcReply::Success { message } => assert!(message.contains("shutting down")),
        other => panic!("expected Success, got {:?}", other),
    }
    assert!(shutdown_rx.try_recv().is_ok());

    // Cleanup
    let _ = std::fs::remove_dir_all(temp_dir);
}
