//! bt_daemon library components (SRP & DIP).
//!
//! Provides modular services for daemon state persistence, session pooling,
//! HTTP range media streaming, and IPC command handling.

pub mod engine;
pub mod ipc;
pub mod ipc_handler;
pub mod repository;
pub mod session_pool;
pub mod streaming;

pub use engine::Engine;
pub use ipc::IpcServer;
pub use ipc_handler::handle_ipc_message;
pub use repository::{DaemonState, DaemonStateRepository, SavedTorrent};
pub use session_pool::{info_hash_hex, TorrentSessionPool};
pub use streaming::{handle_stream_client, start_stream_server};
