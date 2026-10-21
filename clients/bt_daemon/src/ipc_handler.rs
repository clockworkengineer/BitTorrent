//! IPC Command Handler (Single Responsibility Principle).
//!
//! Maps incoming `IpcMessage` requests to engine operations and formats responses.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use torrent_client_shared::{
    FileInfo, IpcMessage, IpcReply, extract_peer_infos, extract_piece_availability,
    is_media_filename,
};

use crate::engine::Engine;
use crate::session_pool::info_hash_hex;

/// Dispatches an incoming `IpcMessage` request to the daemon engine and produces an `IpcReply`.
pub fn handle_ipc_message(
    req: IpcMessage,
    engine: &Arc<Mutex<Engine>>,
    shutdown_tx: &Sender<()>,
) -> IpcReply {
    match req {
        IpcMessage::Add {
            torrent_path,
            download_dir,
        } => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            if let Some(dir) = download_dir {
                e.download_dir = dir;
            }
            match e.add_session(torrent_path, None, None) {
                Ok(_) => IpcReply::Success {
                    message: "Torrent added successfully".to_string(),
                },
                Err(err) => IpcReply::Error { reason: err },
            }
        }
        IpcMessage::Status => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            IpcReply::StatusList {
                torrents: e.get_status_info(),
            }
        }
        IpcMessage::Pause { info_hash } => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            match e.pause_session(&info_hash) {
                Ok(_) => IpcReply::Success {
                    message: "Torrent paused".to_string(),
                },
                Err(err) => IpcReply::Error { reason: err },
            }
        }
        IpcMessage::Resume { info_hash } => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            match e.resume_session(&info_hash) {
                Ok(_) => IpcReply::Success {
                    message: "Torrent resumed".to_string(),
                },
                Err(err) => IpcReply::Error { reason: err },
            }
        }
        IpcMessage::Remove {
            info_hash,
            delete_data,
        } => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            match e.remove_session(&info_hash, delete_data) {
                Ok(_) => IpcReply::Success {
                    message: "Torrent removed".to_string(),
                },
                Err(err) => IpcReply::Error { reason: err },
            }
        }
        IpcMessage::Shutdown => {
            let _ = shutdown_tx.send(());
            IpcReply::Success {
                message: "Daemon shutting down...".to_string(),
            }
        }
        IpcMessage::Files { info_hash } => {
            let e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            let files = if let Some(s) = e
                .pool
                .sessions()
                .iter()
                .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash))
            {
                if let Ok(ctx) = s.session.context().lock() {
                    ctx.files_to_download
                        .iter()
                        .enumerate()
                        .map(|(idx, f)| FileInfo {
                            index: idx,
                            name: f.torrent_path.clone(),
                            length: f.length,
                            is_media: is_media_filename(&f.torrent_path),
                            priority: f.priority.as_str().to_string(),
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            IpcReply::FileList { files }
        }
        IpcMessage::Peers { info_hash } => {
            let e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            let peers = if let Some(s) = e
                .pool
                .sessions()
                .iter()
                .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash))
            {
                if let Ok(ctx) = s.session.context().lock() {
                    extract_peer_infos(&ctx)
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            IpcReply::PeerList { peers }
        }
        IpcMessage::Pieces { info_hash } => {
            let e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            if let Some(s) = e
                .pool
                .sessions()
                .iter()
                .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash))
            {
                if let Ok(ctx) = s.session.context().lock() {
                    let info = extract_piece_availability(&ctx);
                    IpcReply::PieceStates { info }
                } else {
                    IpcReply::Error {
                        reason: "Unable to lock session context".to_string(),
                    }
                }
            } else {
                IpcReply::Error {
                    reason: "Torrent not found".to_string(),
                }
            }
        }
        IpcMessage::SetFilePriority {
            info_hash,
            file_index,
            priority,
        } => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            if let Some(s) = e
                .pool
                .sessions_mut()
                .iter_mut()
                .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash))
            {
                let prio = bittorrent_rs::FilePriority::from_str_loose(&priority);
                match s.session.set_file_priority(file_index, prio) {
                    Ok(()) => IpcReply::Success {
                        message: format!("Set file {} priority to {}", file_index, prio.as_str()),
                    },
                    Err(err) => IpcReply::Error {
                        reason: err.to_string(),
                    },
                }
            } else {
                IpcReply::Error {
                    reason: "Torrent not found".to_string(),
                }
            }
        }
        IpcMessage::SetSuperSeeding { info_hash, enabled } => {
            let mut e = match engine.lock() {
                Ok(guard) => guard,
                Err(_) => {
                    return IpcReply::Error {
                        reason: "Lock poisoned".to_string(),
                    };
                }
            };
            if let Some(s) = e
                .pool
                .sessions_mut()
                .iter_mut()
                .find(|s| info_hash_hex(&s.session).eq_ignore_ascii_case(&info_hash))
            {
                s.session.set_super_seeding(enabled);
                IpcReply::Success {
                    message: format!("Super-seeding set to {} for {}", enabled, info_hash),
                }
            } else {
                IpcReply::Error {
                    reason: "Torrent not found".to_string(),
                }
            }
        }
    }
}
