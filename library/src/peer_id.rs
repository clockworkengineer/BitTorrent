use base64::{Engine as _, engine::general_purpose};
use rand::RngCore;

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
