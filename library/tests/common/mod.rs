//! Shared test fixtures and helpers for integration tests.

use std::fs;
use std::path::PathBuf;

/// Returns the path to a test fixture file located in `tests/files/`.
#[allow(dead_code)]
pub fn sample_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}

/// Reads and returns the raw bytes of a test fixture file located in `tests/files/`.
#[allow(dead_code)]
pub fn sample_file_bytes(name: &str) -> Vec<u8> {
    let path = sample_file(name);
    fs::read(&path).unwrap_or_else(|e| panic!("Unable to read sample torrent file {:?}: {}", path, e))
}

pub mod storage_contract;
