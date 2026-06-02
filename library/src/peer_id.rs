//! Peer ID generation
//!
//! Generates a unique peer ID for the local client using a client identifier prefix
//! and a base64-encoded random suffix.

use base64::{Engine as _, engine::general_purpose};
use rand::RngCore;

/// Generates a unique 20-byte BitTorrent Peer ID conforming to the Azureus-style client ID.
/// It uses the prefix `-AZ1000-` and appends a 12-character random base64 string.
pub fn get() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    let encoded = general_purpose::STANDARD.encode(&bytes);
    let suffix = if encoded.len() >= 12 {
        &encoded[..12]
    } else {
        &encoded
    };
    format!("-AZ1000-{}", suffix)
}
