use crate::error::BitTorrentError;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;

pub struct MagnetLink {
    pub info_hash: Vec<u8>,
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
}

impl MagnetLink {
    /// Parses a magnet URI string. Supported format:
    /// magnet:?xt=urn:btih:<hex-or-base32-hash>&dn=<name>&tr=<tracker>
    pub fn parse(uri: &str) -> Result<Self, BitTorrentError> {
        if !uri.starts_with("magnet:?") {
            return Err(BitTorrentError::Parse("Invalid magnet link prefix".into()));
        }
        let query = &uri["magnet:?".len()..];
        let mut info_hash = None;
        let mut display_name = None;
        let mut trackers = Vec::new();

        for param in query.split('&') {
            let mut parts = param.splitn(2, '=');
            let key = parts.next().unwrap_or("");
            let val = parts.next().unwrap_or("");
            if val.is_empty() {
                continue;
            }

            match key {
                "xt" => {
                    if val.starts_with("urn:btih:") {
                        let hash_str = &val["urn:btih:".len()..];
                        if let Some(bytes) = parse_hex_hash(hash_str) {
                            info_hash = Some(bytes);
                        } else if let Some(bytes) = parse_base32_hash(hash_str) {
                            info_hash = Some(bytes);
                        } else {
                            return Err(BitTorrentError::Parse(format!("Invalid info hash format: {}", hash_str)));
                        }
                    }
                }
                "dn" => {
                    display_name = Some(percent_decode(val));
                }
                "tr" => {
                    trackers.push(percent_decode(val));
                }
                _ => {}
            }
        }

        let info_hash = info_hash.ok_or_else(|| BitTorrentError::Parse("Missing info hash (xt) in magnet link".into()))?;

        Ok(MagnetLink {
            info_hash,
            display_name,
            trackers,
        })
    }
}

fn percent_decode(s: &str) -> String {
    let mut res = String::new();
    let mut bytes = s.as_bytes().iter();
    while let Some(&b) = bytes.next() {
        if b == b'%' {
            if let (Some(&h1), Some(&h2)) = (bytes.next(), bytes.next()) {
                if let Some(c) = hex_chars_to_byte(h1, h2) {
                    res.push(c as char);
                    continue;
                }
            }
        } else if b == b'+' {
            res.push(' ');
            continue;
        }
        res.push(b as char);
    }
    res
}

fn hex_chars_to_byte(h1: u8, h2: u8) -> Option<u8> {
    let n1 = char::from(h1).to_digit(16)?;
    let n2 = char::from(h2).to_digit(16)?;
    Some(((n1 << 4) | n2) as u8)
}

fn parse_hex_hash(hex: &str) -> Option<Vec<u8>> {
    if hex.len() != 40 {
        return None;
    }
    let mut bytes = Vec::with_capacity(20);
    for i in 0..20 {
        let h1 = hex.as_bytes()[i * 2];
        let h2 = hex.as_bytes()[i * 2 + 1];
        let n1 = char::from(h1).to_digit(16)?;
        let n2 = char::from(h2).to_digit(16)?;
        bytes.push(((n1 << 4) | n2) as u8);
    }
    Some(bytes)
}

fn parse_base32_hash(b32: &str) -> Option<Vec<u8>> {
    if b32.len() != 32 {
        return None;
    }
    let mut bytes = Vec::with_capacity(20);
    let mut current_byte = 0u8;
    let mut bits_buffered = 0;
    
    for &c in b32.as_bytes() {
        let val = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        
        bits_buffered += 5;
        if bits_buffered >= 8 {
            bits_buffered -= 8;
            current_byte |= val >> bits_buffered;
            bytes.push(current_byte);
            current_byte = (((val as u16) << (8 - bits_buffered)) & 0xFF) as u8;
        } else {
            current_byte |= ((val as u16) << (8 - bits_buffered)) as u8;
        }
    }
    if bytes.len() == 20 {
        Some(bytes)
    } else {
        None
    }
}
