//! Daemon state persistence repository (Single Responsibility Principle).
//!
//! Handles disk persistence and JSON serialization for daemon active torrent states
//! and fastresume file references.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Serializable snapshot of the daemon configuration and active torrents.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct DaemonState {
    pub download_dir: String,
    pub torrents: Vec<SavedTorrent>,
}

/// Metadata and resumption references for a single active torrent session.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SavedTorrent {
    pub torrent_path: String,
    pub bitfield_hex: String,
    #[serde(default)]
    pub fast_resume_file: Option<String>,
}

/// Repository responsible for loading and persisting daemon state files (SRP).
#[derive(Debug, Clone)]
pub struct DaemonStateRepository {
    state_file: PathBuf,
}

impl DaemonStateRepository {
    /// Creates a new `DaemonStateRepository` referencing the specified state file path.
    pub fn new(state_file: PathBuf) -> Self {
        Self { state_file }
    }

    /// Returns a reference to the underlying state file path.
    pub fn state_file(&self) -> &Path {
        &self.state_file
    }

    /// Loads the saved daemon state from disk, or returns default if the file does not exist.
    pub fn load(&self) -> Result<DaemonState, std::io::Error> {
        if !self.state_file.exists() {
            return Ok(DaemonState::default());
        }
        let content = std::fs::read_to_string(&self.state_file)?;
        let state = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(state)
    }

    /// Persists the daemon state to disk formatted with indentation.
    pub fn save(&self, state: &DaemonState) -> Result<(), std::io::Error> {
        if let Some(parent) = self.state_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = serde_json::to_string_pretty(state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&self.state_file, content)
    }

    /// Computes the dedicated fast-resume file path for a specific torrent info-hash.
    pub fn fast_resume_path_for_hash(&self, info_hash: &str) -> PathBuf {
        self.state_file
            .with_file_name(format!("{}.fastresume", info_hash))
    }
}
