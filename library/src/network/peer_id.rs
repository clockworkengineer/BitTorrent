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

/// Identifies the client software and version string from a standard 20-byte BitTorrent Peer ID.
///
/// Recognizes Azureus-style (`-XX1234-`), Shadow-style, and custom vendor prefixes.
pub fn parse_peer_client(peer_id: &[u8]) -> String {
    if peer_id.len() < 8 {
        return "Unknown".to_string();
    }

    // Azureus-style: -XX1234-
    if peer_id[0] == b'-' && peer_id[7] == b'-' {
        let code = match std::str::from_utf8(&peer_id[1..3]) {
            Ok(s) => s,
            Err(_) => return "Unknown".to_string(),
        };

        let v_chars: Vec<char> = peer_id[3..7].iter().map(|&b| b as char).collect();
        let version = format!(
            "{}.{}.{}.{}",
            v_chars[0], v_chars[1], v_chars[2], v_chars[3]
        )
        .trim_end_matches(".0")
        .to_string();

        let client_name = match code {
            "qB" => "qBittorrent",
            "UT" | "uT" => "µTorrent",
            "UM" => "µTorrent Mac",
            "TR" => "Transmission",
            "DE" => "Deluge",
            "AZ" => "Vuze / Azureus",
            "BT" => "BitTorrent",
            "LT" => "libtorrent (Rasterbar)",
            "lt" => "libTorrent (Rakshasa)",
            "WD" => "WebTorrent",
            "WW" => "WebTorrent Desktop",
            "BC" => "BitComet",
            "KT" => "KTorrent",
            "TT" => "Tixati",
            "SD" | "XL" => "Xunlei",
            "MR" => "Miro",
            "MO" => "MonoTorrent",
            "IL" => "iLivid",
            "JS" => "JSTorrent",
            _ => return format!("{}-{}", code, version),
        };
        return format!("{} {}", client_name, version);
    }

    // Shadow's style: e.g. M3-4-2--... or S5-8-7--...
    if peer_id.len() >= 6 && peer_id[1].is_ascii_digit() && peer_id[2] == b'-' {
        let name = match peer_id[0] {
            b'M' => "Mainline",
            b'S' => "Shadow",
            b'T' => "BitTornado",
            b'A' => "ABC",
            b'O' => "Osprey",
            b'U' => "UPnP",
            _ => "Shadow-style",
        };
        if let Ok(s) = std::str::from_utf8(&peer_id[1..6]) {
            return format!("{} {}", name, s);
        }
    }

    // Fallback: ASCII representation if printable
    if let Ok(s) = std::str::from_utf8(&peer_id[..8]) {
        if s.chars().all(|c| c.is_ascii_graphic()) {
            return s.to_string();
        }
    }

    // Hex representation
    format!(
        "{:02X}{:02X}{:02X}{:02X}...",
        peer_id[0], peer_id[1], peer_id[2], peer_id[3]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_peer_client_known_clients() {
        assert_eq!(
            parse_peer_client(b"-qB4600-abcdefghijkl"),
            "qBittorrent 4.6"
        );
        assert_eq!(
            parse_peer_client(b"-TR4030-123456789012"),
            "Transmission 4.0.3"
        );
        assert_eq!(parse_peer_client(b"-UT3550-xxxxxxxxxxxx"), "µTorrent 3.5.5");
        assert_eq!(parse_peer_client(b"-DE2110-yyyyyyyyyyyy"), "Deluge 2.1.1");
        assert_eq!(
            parse_peer_client(b"-AZ1000-zzzzzzzzzzzz"),
            "Vuze / Azureus 1"
        );
        assert_eq!(
            parse_peer_client(b"-WD0120-wwwwwwwwwwww"),
            "WebTorrent 0.1.2"
        );
        assert_eq!(parse_peer_client(b"-XX9999-111111111111"), "XX-9.9.9.9");
    }

    #[test]
    fn test_parse_peer_client_fallback() {
        assert_eq!(parse_peer_client(b"M4-3-6--unknownpeer!"), "Mainline 4-3-6");
        assert_eq!(
            parse_peer_client(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]),
            "01020304..."
        );
        assert_eq!(parse_peer_client(b"short"), "Unknown");
    }
}
