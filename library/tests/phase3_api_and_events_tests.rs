//! Phase 3 API Modernization & Event Streaming Integration Tests
//!
//! Validates:
//! 1. `TorrentSessionBuilder<State>` typestate transitions (`Initial` -> `Ready`).
//! 2. Single-call `session.start()?` and `builder.start()?` high-level facades.
//! 3. Asynchronous `session.events()` stream broadcasting `SessionEvent` notifications.
//! 4. Strongly typed `BitTorrentError` variants and standard conversions.

mod common;

use bittorrent_rs::{
    BencodeError, BitTorrentError, LockExt, SessionEvent, TorrentSession, TorrentSessionBuilder,
    TorrentStatus,
};
use common::sample_file;
use std::path::PathBuf;

fn temp_download_dir(suffix: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("bittorrent_phase3_tests")
        .join(suffix);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("Failed to create temp download dir");
    dir
}

#[test]
fn test_typestate_builder_transition() {
    let tmp = temp_download_dir("typestate_transition");
    let torrent_path = sample_file("singlefile.torrent");

    // 1. Start in uninitialized Initial typestate
    let initial_builder = TorrentSessionBuilder::initial();

    // 2. Set source
    let builder_with_source = initial_builder.with_source(torrent_path);

    // 3. Set download path -> Transitions to Ready typestate!
    let ready_builder = builder_with_source.with_download_path(&tmp);

    // 4. Configure options on Ready builder
    let session = ready_builder
        .with_seeding(false)
        .build()
        .expect("Failed to build session from typestate builder");

    assert_eq!(session.status(), TorrentStatus::Initialised);
    assert_eq!(session.progress(), 0.0);
}

#[test]
fn test_single_call_builder_start_facade() {
    let tmp = temp_download_dir("single_call_start");
    let torrent_path = sample_file("singlefile.torrent");

    // Start in a single intuitive call
    let mut session = TorrentSession::builder(torrent_path, &tmp)
        .with_seeding(false)
        .start()
        .expect("Failed to start session via builder facade");

    assert_eq!(session.status(), TorrentStatus::Downloading);

    // Stop cleanly
    session.stop().expect("Failed to stop session");
    assert_eq!(session.status(), TorrentStatus::Ended);
}

#[test]
fn test_session_event_channel_stream() {
    let tmp = temp_download_dir("event_stream");
    let torrent_path = sample_file("singlefile.torrent");

    let mut session = TorrentSession::builder(torrent_path, &tmp)
        .with_seeding(false)
        .build()
        .expect("Failed to build session");

    // Subscribe to session event stream
    let rx = session.events();
    assert_eq!(session.event_broadcaster.subscriber_count(), 1);

    // 1. Start session -> emits StateChanged(Downloading)
    session.start().expect("Failed to start session");
    let ev1 = rx.recv().expect("Failed to receive start event");
    assert_eq!(ev1, SessionEvent::StateChanged(TorrentStatus::Downloading));

    // 2. Pause session -> emits StateChanged(Paused)
    session.pause().expect("Failed to pause session");
    let ev2 = rx.recv().expect("Failed to receive pause event");
    assert_eq!(ev2, SessionEvent::StateChanged(TorrentStatus::Paused));

    // 3. Resume session -> emits StateChanged(Downloading)
    session.resume().expect("Failed to resume session");
    let ev3 = rx.recv().expect("Failed to receive resume event");
    assert_eq!(ev3, SessionEvent::StateChanged(TorrentStatus::Downloading));

    // 4. Stop session -> emits StateChanged(Ended)
    session.stop().expect("Failed to stop session");
    let ev4 = rx.recv().expect("Failed to receive stop event");
    assert_eq!(ev4, SessionEvent::StateChanged(TorrentStatus::Ended));
}

#[test]
fn test_piece_completed_event_emission() {
    let tmp = temp_download_dir("piece_completed_events");
    let torrent_path = sample_file("singlefile.torrent");

    let session = TorrentSession::builder(torrent_path, &tmp)
        .build()
        .expect("Failed to build session");

    let rx = session.events();

    // Mark piece 0 verified local
    session.context.lock_safe().mark_piece_local(0, true);

    let event = rx.recv().expect("Expected PieceCompleted event");
    match event {
        SessionEvent::PieceCompleted {
            piece_index,
            total_completed,
            total_pieces,
        } => {
            assert_eq!(piece_index, 0);
            assert!(total_completed >= 1);
            assert!(total_pieces >= 1);
        }
        other => panic!("Unexpected event received: {:?}", other),
    }
}

#[test]
fn test_strongly_typed_bittorrent_error_variants() {
    // Storage variant
    let err = BitTorrentError::Storage {
        path: "/mnt/fast/file.iso".to_string(),
        reason: "No space left on device".to_string(),
    };
    assert!(
        err.to_string()
            .contains("Storage failure on '/mnt/fast/file.iso'")
    );

    // Tracker variant
    let err = BitTorrentError::Tracker("HTTP 503 Service Unavailable".to_string());
    assert!(err.to_string().contains("Tracker error: HTTP 503"));

    // ProtocolViolation variant
    let err = BitTorrentError::ProtocolViolation {
        peer: "192.168.1.50:6881".to_string(),
        reason: "Malformed handshake preamble".to_string(),
    };
    assert!(
        err.to_string()
            .contains("Protocol violation from peer 192.168.1.50:6881")
    );

    // SessionClosed variant
    let err = BitTorrentError::SessionClosed;
    assert_eq!(err.to_string(), "Torrent session is closed");

    // From<BencodeError> conversion
    let bencode_err = BencodeError::NestingDepthExceeded;
    let berr: BitTorrentError = bencode_err.into();
    match berr {
        BitTorrentError::Bencode(BencodeError::NestingDepthExceeded) => {}
        other => panic!("Expected Bencode(NestingDepthExceeded), got {:?}", other),
    }
}
