use crate::error::BitTorrentError;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::format;

/// Parsed BitTorrent Magnet URI representation supporting BEP 9, BEP 19 WebSeeds,
/// BEP 52 BitTorrent v2/Hybrid dual-hashes, and WebTorrent WebSocket trackers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagnetLink {
    /// 20-byte SHA-1 info-hash for v1/hybrid swarms, or 20-byte truncated v2 prefix if pure v2.
    pub info_hash: Vec<u8>,
    /// Optional 32-byte SHA-256 info-hash for BEP 52 v2/hybrid swarms (`urn:btmh:1220...`).
    pub v2_info_hash: Option<Vec<u8>>,
    /// Optional torrent display name (`dn`).
    pub display_name: Option<String>,
    /// List of announce tracker URLs (`tr`), including HTTP, UDP, and WebSocket (`wss://`/`ws://`).
    pub trackers: Vec<String>,
    /// List of BEP 19 WebSeed mirror HTTP/HTTPS URLs (`ws`).
    pub web_seeds: Vec<String>,
    /// List of BEP 19 exact source URLs (`xs`).
    pub exact_sources: Vec<String>,
    /// List of BEP 19 acceptable source URLs (`as`).
    pub acceptable_sources: Vec<String>,
}

impl MagnetLink {
    /// Parses a magnet URI string supporting v1 (`urn:btih:`), v2 (`urn:btmh:1220`),
    /// hybrid dual-hashes, WebSeeds (`ws=`), and WebSocket trackers (`tr=wss://...`).
    ///
    /// # Errors
    /// Returns [`BitTorrentError::Parse`] if the URI does not start with `magnet:?`,
    /// or if no valid info-hash (`xt`) is specified.
    pub fn parse(uri: &str) -> Result<Self, BitTorrentError> {
        if !uri.starts_with("magnet:?") {
            return Err(BitTorrentError::Parse("Invalid magnet link prefix".into()));
        }
        let query = &uri["magnet:?".len()..];
        let mut v1_info_hash = None;
        let mut v2_info_hash = None;
        let mut display_name = None;
        let mut trackers = Vec::new();
        let mut web_seeds = Vec::new();
        let mut exact_sources = Vec::new();
        let mut acceptable_sources = Vec::new();

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
                        if let Some(bytes) = parse_hex_hash_exact(hash_str, 20) {
                            v1_info_hash = Some(bytes);
                        } else if let Some(bytes) = parse_base32_hash(hash_str) {
                            v1_info_hash = Some(bytes);
                        } else {
                            return Err(BitTorrentError::Parse(format!("Invalid info hash format: {}", hash_str)));
                        }
                    } else if val.starts_with("urn:btmh:1220") {
                        let hash_str = &val["urn:btmh:1220".len()..];
                        if let Some(bytes) = parse_hex_hash_exact(hash_str, 32) {
                            v2_info_hash = Some(bytes);
                        } else {
                            return Err(BitTorrentError::Parse(format!("Invalid v2 info hash format: {}", hash_str)));
                        }
                    }
                }
                "dn" => {
                    display_name = Some(percent_decode(val));
                }
                "tr" => {
                    let tracker = percent_decode(val);
                    if !trackers.contains(&tracker) {
                        trackers.push(tracker);
                    }
                }
                "ws" => {
                    let seed = percent_decode(val);
                    if !web_seeds.contains(&seed) {
                        web_seeds.push(seed);
                    }
                }
                "xs" => {
                    let xs = percent_decode(val);
                    if !exact_sources.contains(&xs) {
                        exact_sources.push(xs);
                    }
                }
                "as" => {
                    let as_url = percent_decode(val);
                    if !acceptable_sources.contains(&as_url) {
                        acceptable_sources.push(as_url);
                    }
                }
                _ => {}
            }
        }

        // Determine effective info-hash
        let info_hash = match (v1_info_hash, v2_info_hash.as_ref()) {
            (Some(v1), _) => v1,
            (None, Some(v2)) => v2[..20].to_vec(),
            (None, None) => {
                return Err(BitTorrentError::Parse("Missing info hash (xt) in magnet link".into()));
            }
        };

        Ok(MagnetLink {
            info_hash,
            v2_info_hash,
            display_name,
            trackers,
            web_seeds,
            exact_sources,
            acceptable_sources,
        })
    }

    /// Returns `true` if this magnet link contains both v1 (SHA-1) and v2 (SHA-256) info-hashes.
    pub fn is_hybrid(&self) -> bool {
        if let Some(ref v2) = self.v2_info_hash {
            self.info_hash.len() == 20 && &v2[..20] != &self.info_hash[..]
        } else {
            false
        }
    }

    /// Returns `true` if this magnet link contains a BEP 52 v2 SHA-256 info-hash.
    pub fn is_v2(&self) -> bool {
        self.v2_info_hash.is_some()
    }

    /// Returns `true` if this magnet link is pure v2 (no independent v1 SHA-1 hash provided).
    pub fn is_v2_only(&self) -> bool {
        self.v2_info_hash.is_some() && !self.is_hybrid()
    }

    /// Returns `true` if this magnet link contains only a v1 SHA-1 info-hash.
    pub fn is_v1_only(&self) -> bool {
        self.v2_info_hash.is_none()
    }

    /// Returns `true` if this magnet link specifies at least one BEP 19 WebSeed mirror URL.
    pub fn has_web_seeds(&self) -> bool {
        !self.web_seeds.is_empty()
    }

    /// Returns `true` if this magnet link contains at least one WebTorrent WebSocket tracker (`wss://` or `ws://`).
    pub fn is_webtorrent(&self) -> bool {
        self.trackers.iter().any(|t| t.starts_with("wss://") || t.starts_with("ws://"))
    }

    /// Returns all registered WebSocket tracker URLs (`wss://` or `ws://`).
    pub fn websocket_trackers(&self) -> Vec<String> {
        self.trackers
            .iter()
            .filter(|t| t.starts_with("wss://") || t.starts_with("ws://"))
            .cloned()
            .collect()
    }

    /// Returns all registered UDP tracker URLs (`udp://`).
    pub fn udp_trackers(&self) -> Vec<String> {
        self.trackers
            .iter()
            .filter(|t| t.starts_with("udp://"))
            .cloned()
            .collect()
    }

    /// Returns all registered HTTP/HTTPS tracker URLs (`http://` or `https://`).
    pub fn http_trackers(&self) -> Vec<String> {
        self.trackers
            .iter()
            .filter(|t| t.starts_with("http://") || t.starts_with("https://"))
            .cloned()
            .collect()
    }

    /// Appends a tracker URL to the magnet link if not already present.
    pub fn add_tracker(&mut self, url: impl Into<String>) {
        let u = url.into();
        if !self.trackers.contains(&u) {
            self.trackers.push(u);
        }
    }

    /// Appends a WebSeed URL to the magnet link if not already present.
    pub fn add_web_seed(&mut self, url: impl Into<String>) {
        let u = url.into();
        if !self.web_seeds.contains(&u) {
            self.web_seeds.push(u);
        }
    }

    /// Generates a canonical, percent-encoded BitTorrent magnet URI representing this `MagnetLink`.
    pub fn to_uri(&self) -> String {
        let mut uri = String::from("magnet:?");
        let mut params = Vec::new();

        // If hybrid or v1, output v1 xt
        if self.is_v1_only() || self.is_hybrid() {
            let mut hex_v1 = String::with_capacity(40);
            for b in &self.info_hash {
                hex_v1.push_str(&format!("{:02x}", b));
            }
            params.push(format!("xt=urn:btih:{}", hex_v1));
        }

        // If v2, output v2 xt
        if let Some(ref v2) = self.v2_info_hash {
            let mut hex_v2 = String::with_capacity(64);
            for b in v2 {
                hex_v2.push_str(&format!("{:02x}", b));
            }
            params.push(format!("xt=urn:btmh:1220{}", hex_v2));
        } else if !self.is_v1_only() && !self.is_hybrid() {
            let mut hex_v1 = String::with_capacity(40);
            for b in &self.info_hash {
                hex_v1.push_str(&format!("{:02x}", b));
            }
            params.push(format!("xt=urn:btih:{}", hex_v1));
        }

        if let Some(ref dn) = self.display_name {
            params.push(format!("dn={}", percent_encode(dn)));
        }

        for tr in &self.trackers {
            params.push(format!("tr={}", percent_encode(tr)));
        }

        for ws in &self.web_seeds {
            params.push(format!("ws={}", percent_encode(ws)));
        }

        for xs in &self.exact_sources {
            params.push(format!("xs={}", percent_encode(xs)));
        }

        for as_url in &self.acceptable_sources {
            params.push(format!("as={}", percent_encode(as_url)));
        }

        uri.push_str(&params.join("&"));
        uri
    }
}

/// Programmatic builder for constructing [`MagnetLink`] instances and canonical magnet URIs.
#[derive(Debug, Default, Clone)]
pub struct MagnetUriBuilder {
    info_hash: Option<Vec<u8>>,
    v2_info_hash: Option<Vec<u8>>,
    display_name: Option<String>,
    trackers: Vec<String>,
    web_seeds: Vec<String>,
    exact_sources: Vec<String>,
    acceptable_sources: Vec<String>,
}

impl MagnetUriBuilder {
    /// Creates a new `MagnetUriBuilder` initialized with a 20-byte SHA-1 info-hash.
    pub fn new(info_hash: &[u8]) -> Self {
        Self {
            info_hash: Some(info_hash.to_vec()),
            ..Default::default()
        }
    }

    /// Creates a new `MagnetUriBuilder` initialized with a 32-byte BEP 52 SHA-256 info-hash.
    pub fn from_v2_hash(v2_hash: &[u8]) -> Self {
        let truncated = if v2_hash.len() >= 20 {
            v2_hash[..20].to_vec()
        } else {
            v2_hash.to_vec()
        };
        Self {
            info_hash: Some(truncated),
            v2_info_hash: Some(v2_hash.to_vec()),
            ..Default::default()
        }
    }

    /// Adds a 32-byte BEP 52 SHA-256 info-hash for hybrid torrents.
    pub fn with_v2_hash(mut self, v2_hash: &[u8]) -> Self {
        self.v2_info_hash = Some(v2_hash.to_vec());
        self
    }

    /// Sets the torrent display name (`dn`).
    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }

    /// Adds a tracker URL (`tr`).
    pub fn with_tracker(mut self, url: impl Into<String>) -> Self {
        let u = url.into();
        if !self.trackers.contains(&u) {
            self.trackers.push(u);
        }
        self
    }

    /// Adds multiple tracker URLs (`tr`).
    pub fn with_trackers(mut self, urls: impl IntoIterator<Item = String>) -> Self {
        for url in urls {
            if !self.trackers.contains(&url) {
                self.trackers.push(url);
            }
        }
        self
    }

    /// Adds a BEP 19 WebSeed mirror URL (`ws`).
    pub fn with_web_seed(mut self, url: impl Into<String>) -> Self {
        let u = url.into();
        if !self.web_seeds.contains(&u) {
            self.web_seeds.push(u);
        }
        self
    }

    /// Adds multiple BEP 19 WebSeed mirror URLs (`ws`).
    pub fn with_web_seeds(mut self, urls: impl IntoIterator<Item = String>) -> Self {
        for url in urls {
            if !self.web_seeds.contains(&url) {
                self.web_seeds.push(url);
            }
        }
        self
    }

    /// Adds a BEP 19 exact source URL (`xs`).
    pub fn with_exact_source(mut self, url: impl Into<String>) -> Self {
        let u = url.into();
        if !self.exact_sources.contains(&u) {
            self.exact_sources.push(u);
        }
        self
    }

    /// Automatically injects canonical public WebTorrent WebSocket trackers
    /// (`wss://tracker.openwebtorrent.com`, `wss://tracker.btorrent.xyz`, `wss://tracker.fastcast.nz`)
    /// for instant browser swarm bridging.
    pub fn with_webtorrent_defaults(mut self) -> Self {
        let defaults = [
            "wss://tracker.openwebtorrent.com",
            "wss://tracker.btorrent.xyz",
            "wss://tracker.fastcast.nz",
        ];
        for d in defaults {
            let u = d.to_string();
            if !self.trackers.contains(&u) {
                self.trackers.push(u);
            }
        }
        self
    }

    /// Builds the configured [`MagnetLink`].
    pub fn build(self) -> Result<MagnetLink, BitTorrentError> {
        let info_hash = self.info_hash.ok_or_else(|| {
            BitTorrentError::Parse("MagnetUriBuilder requires at least one info hash".into())
        })?;

        Ok(MagnetLink {
            info_hash,
            v2_info_hash: self.v2_info_hash,
            display_name: self.display_name,
            trackers: self.trackers,
            web_seeds: self.web_seeds,
            exact_sources: self.exact_sources,
            acceptable_sources: self.acceptable_sources,
        })
    }

    /// Builds the [`MagnetLink`] and serializes it directly into a canonical magnet URI.
    pub fn to_uri(self) -> Result<String, BitTorrentError> {
        Ok(self.build()?.to_uri())
    }
}

/// Percent-decodes a URL query parameter string.
pub fn percent_decode(s: &str) -> String {
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

/// Percent-encodes a string preserving RFC 3986 unreserved characters (`A-Z`, `a-z`, `0-9`, `-`, `_`, `.`, `~`).
pub fn percent_encode(s: &str) -> String {
    let mut res = String::new();
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                res.push(b as char);
            }
            _ => {
                res.push('%');
                res.push_str(&format!("{:02X}", b));
            }
        }
    }
    res
}

fn hex_chars_to_byte(h1: u8, h2: u8) -> Option<u8> {
    let n1 = char::from(h1).to_digit(16)?;
    let n2 = char::from(h2).to_digit(16)?;
    Some(((n1 << 4) | n2) as u8)
}

fn parse_hex_hash_exact(hex: &str, byte_len: usize) -> Option<Vec<u8>> {
    if hex.len() != byte_len * 2 {
        return None;
    }
    let mut bytes = Vec::with_capacity(byte_len);
    for i in 0..byte_len {
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
