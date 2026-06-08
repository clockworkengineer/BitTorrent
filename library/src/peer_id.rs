//! Peer ID generation
//!
//! Generates a unique peer ID for the local client using a client identifier prefix
//! and a random alphanumeric suffix.

use rand::Rng;

/// Generates a unique 20-byte BitTorrent Peer ID conforming to the Azureus-style client ID.
/// It uses the prefix `-AZ1000-` and appends a 12-character random alphanumeric string.
pub fn get() -> String {
    let mut rng = rand::thread_rng();
    let chars: String = (0..12)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            let char_list = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            char_list[idx] as char
        })
        .collect();
    format!("-AZ1000-{}", chars)
}
