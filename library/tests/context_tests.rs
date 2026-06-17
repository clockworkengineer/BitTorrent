//! Unit Tests for TorrentContext Status State Transitions
//!
//! Verifies valid status transitions (such as Pause, Resume, Stop) and asserts
//! that invalid state changes (e.g. pausing when already paused) return correct error results.

use bittorrent_rs::{TorrentContext, TorrentStatus, RarestFirstSelector};
use bittorrent_rs::session::SessionConfig;
use std::sync::Arc;

/// Verifies that TorrentContext transitions correctly through a normal lifecycle:
/// Initialised -> Downloading -> Paused -> Downloading -> Ended.
#[test]
fn test_context_status_transitions_flow() {
    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        vec![0; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    tc.total_bytes_to_download = 1024;

    assert_eq!(tc.status, TorrentStatus::Initialised);

    tc.start_downloading().unwrap();
    assert_eq!(tc.status, TorrentStatus::Downloading);

    tc.pause().unwrap();
    assert_eq!(tc.status, TorrentStatus::Paused);

    tc.resume().unwrap();
    assert_eq!(tc.status, TorrentStatus::Downloading);

    tc.stop().unwrap();
    assert_eq!(tc.status, TorrentStatus::Ended);
}

/// Verifies that TorrentContext yields errors when transitioning from invalid states.
#[test]
fn test_context_status_transitions_errors() {
    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);
    let mut tc = TorrentContext::new_magnet_bootstrap(
        vec![0; 20],
        vec!["http://tracker.example.com/announce".to_string()],
        selector,
        &std::path::PathBuf::from("."),
        config,
    ).unwrap();
    tc.total_bytes_to_download = 1024;

    assert!(tc.pause().is_err());
    assert!(tc.resume().is_err());

    tc.start_downloading().unwrap();

    assert!(tc.start_downloading().is_err());
    assert!(tc.resume().is_err());

    tc.pause().unwrap();
    
    assert!(tc.pause().is_err());

    tc.resume().unwrap();
    tc.stop().unwrap();

    assert!(tc.stop().is_err());
    assert!(tc.start_downloading().is_err());
}
