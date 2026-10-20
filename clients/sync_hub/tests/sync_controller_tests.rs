use std::path::PathBuf;
use sync_hub::{SyncManager, SyncSessionController};

#[test]
fn test_sync_hub_session_controller_separation() {
    let sync_dir = PathBuf::from("downloads/test_sync_separation");
    let torrent_path = sync_dir.join(".sync.torrent");

    let controller = SyncSessionController::new(sync_dir.clone(), torrent_path, 6881);
    assert!(controller.session().is_none());

    let manager = SyncManager::new(sync_dir, 6881);
    assert!(manager.controller().session().is_none());
}
