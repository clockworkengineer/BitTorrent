//! Utility functions
//!
//! Provides binary serialization/deserialization helper functions (such as packing
//! and unpacking integers to/from network byte order) and formatting utilities.

#[cfg(feature = "std")]
use std::fs::OpenOptions;
#[cfg(feature = "std")]
use std::io::Write;
#[cfg(feature = "std")]
use std::sync::{Mutex, OnceLock};

use alloc::format;
use alloc::string::String;

/// Packs a 32-bit unsigned integer into a 4-byte big-endian array.
pub fn pack_u32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

/// Unpacks a 32-bit unsigned integer from a big-endian byte slice starting at `offset`.
pub fn unpack_u32(buffer: &[u8], offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&buffer[offset..offset + 4]);
    u32::from_be_bytes(bytes)
}

/// Packs a 64-bit unsigned integer into an 8-byte big-endian array.
pub fn pack_u64(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

/// Unpacks a 64-bit unsigned integer from a big-endian byte slice starting at `offset`.
pub fn unpack_u64(buffer: &[u8], offset: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&buffer[offset..offset + 8]);
    u64::from_be_bytes(bytes)
}

/// Decodes a compact IPv4 peer list (6 bytes per peer: 4 bytes IP, 2 bytes port) starting from a given offset.
pub fn decode_compact_ipv4_peers(peers: &[u8], offset: usize) -> alloc::vec::Vec<(String, u16)> {
    let mut list = alloc::vec::Vec::new();
    let mut num = offset;
    while num + 6 <= peers.len() {
        let ip = format!(
            "{}.{}.{}.{}",
            peers[num],
            peers[num + 1],
            peers[num + 2],
            peers[num + 3]
        );
        let port = ((peers[num + 4] as u16) << 8) | peers[num + 5] as u16;
        list.push((ip, port));
        num += 6;
    }
    list
}

/// Encodes an IPv4 peer list into compact 6-byte representation (4 bytes IPv4, 2 bytes port).
pub fn encode_compact_ipv4_peers(peers: &[(String, u16)]) -> alloc::vec::Vec<u8> {
    let mut bytes = alloc::vec::Vec::new();
    for (ip, port) in peers {
        let parts: alloc::vec::Vec<&str> = ip.split('.').collect();
        if parts.len() == 4 {
            if let (Ok(b0), Ok(b1), Ok(b2), Ok(b3)) = (
                parts[0].parse::<u8>(),
                parts[1].parse::<u8>(),
                parts[2].parse::<u8>(),
                parts[3].parse::<u8>(),
            ) {
                bytes.push(b0);
                bytes.push(b1);
                bytes.push(b2);
                bytes.push(b3);
                bytes.extend_from_slice(&port.to_be_bytes());
            }
        }
    }
    bytes
}

/// Decodes a compact IPv6 peer list (18 bytes per peer: 16 bytes IPv6, 2 bytes port) starting from a given offset (BEP 7).
pub fn decode_compact_ipv6_peers(peers: &[u8], offset: usize) -> alloc::vec::Vec<(String, u16)> {
    let mut list = alloc::vec::Vec::new();
    let mut num = offset;
    while num + 18 <= peers.len() {
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&peers[num..num + 16]);
        #[cfg(feature = "std")]
        let ip = std::net::Ipv6Addr::from(octets).to_string();
        #[cfg(not(feature = "std"))]
        let ip = format!(
            "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
            u16::from_be_bytes([octets[0], octets[1]]),
            u16::from_be_bytes([octets[2], octets[3]]),
            u16::from_be_bytes([octets[4], octets[5]]),
            u16::from_be_bytes([octets[6], octets[7]]),
            u16::from_be_bytes([octets[8], octets[9]]),
            u16::from_be_bytes([octets[10], octets[11]]),
            u16::from_be_bytes([octets[12], octets[13]]),
            u16::from_be_bytes([octets[14], octets[15]])
        );
        let port = ((peers[num + 16] as u16) << 8) | peers[num + 17] as u16;
        list.push((ip, port));
        num += 18;
    }
    list
}

/// Encodes an IPv6 peer list into compact 18-byte representation (16 bytes IPv6, 2 bytes port) (BEP 7).
pub fn encode_compact_ipv6_peers(peers: &[(String, u16)]) -> alloc::vec::Vec<u8> {
    #[allow(unused_mut)]
    let mut bytes = alloc::vec::Vec::new();
    for (ip, port) in peers {
        let clean_ip = ip.trim_start_matches('[').trim_end_matches(']');
        #[cfg(feature = "std")]
        {
            if let Ok(addr) = clean_ip.parse::<std::net::Ipv6Addr>() {
                bytes.extend_from_slice(&addr.octets());
                bytes.extend_from_slice(&port.to_be_bytes());
            }
        }
        #[cfg(not(feature = "std"))]
        {
            let _ = (clean_ip, port);
        }
    }
    bytes
}

/// Formats an IP address and port into a standard socket address string.
///
/// If `ip` is an IPv6 address containing `:` and not already enclosed in square brackets,
/// it is formatted as `[ip]:port`. Otherwise, it is formatted as `ip:port`.
pub fn format_socket_addr(ip: &str, port: u16) -> String {
    if ip.contains(':') && !ip.starts_with('[') {
        format!("[{}]:{}", ip, port)
    } else {
        format!("{}:{}", ip, port)
    }
}

/// Encodes an arbitrary byte slice into a lowercase hexadecimal string.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use core::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Encodes an arbitrary byte slice into an uppercase hexadecimal string.
pub fn to_hex_upper(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use core::fmt::Write;
        let _ = write!(s, "{:02X}", b);
    }
    s
}

/// Decodes a hexadecimal string slice into raw bytes.
pub fn from_hex(hex: &str) -> Result<alloc::vec::Vec<u8>, crate::error::BitTorrentError> {
    if hex.len() % 2 != 0 {
        return Err(crate::error::BitTorrentError::Parse(
            "Hex string must have an even length".into(),
        ));
    }
    let mut bytes = alloc::vec::Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|e| crate::error::BitTorrentError::Parse(alloc::string::ToString::to_string(&e)))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

/// Formats a 20-byte info hash byte slice into its hexadecimal string representation.
pub fn info_hash_to_string(info_hash: &[u8]) -> String {
    to_hex(info_hash)
}

/// Calculates the byte index and bit mask for a piece in a standard BitTorrent bitfield.
pub fn get_bitfield_index_and_mask(piece_number: u32) -> (usize, u8) {
    let byte_index = (piece_number >> 3) as usize;
    let bit_mask = 0x80 >> (piece_number & 0x7);
    (byte_index, bit_mask)
}

#[cfg(feature = "std")]
static DEBUG_LOG: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
#[cfg(feature = "std")]
static LOG_SENDER: OnceLock<Mutex<Option<std::sync::mpsc::Sender<String>>>> = OnceLock::new();

/// Sets the global log sender channel to forward logs to.
#[cfg(feature = "std")]
pub fn set_log_sender(sender: std::sync::mpsc::Sender<String>) {
    let mutex = LOG_SENDER.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = mutex.lock() {
        *guard = Some(sender);
    }
}

/// Appends a debug message to `debug.log`.
pub fn log_debug(msg: &str) {
    #[cfg(feature = "std")]
    {
        let file = DEBUG_LOG.get_or_init(|| {
            let f = OpenOptions::new()
                .create(true)
                .append(true)
                .open("debug.log")
                .ok();
            Mutex::new(f)
        });
        if let Ok(mut guard) = file.lock() {
            if let Some(ref mut f) = *guard {
                let _ = writeln!(f, "{}", msg);
                let _ = f.flush();
            }
        }

        if let Some(mutex) = LOG_SENDER.get() {
            if let Ok(guard) = mutex.lock() {
                if let Some(ref sender) = *guard {
                    let _ = sender.send(msg.to_string());
                }
            }
        }
    }
    #[cfg(not(feature = "std"))]
    {
        // No-op or custom light print in embedded systems
        let _ = msg;
    }
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        #[cfg(feature = "std")]
        {
            $crate::util::log_debug(&alloc::format!($($arg)*));
        }
    };
}

pub struct YieldNow {
    yielded: bool,
}

impl core::future::Future for YieldNow {
    type Output = ();

    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        if self.yielded {
            core::task::Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            core::task::Poll::Pending
        }
    }
}

pub fn yield_now() -> YieldNow {
    YieldNow { yielded: false }
}

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

pub const BUFFER_SIZE: usize = crate::constants::BLOCK_SIZE + 9;
const MAX_BUFFERS: usize = 8;

pub struct StaticBuffer {
    index: usize,
}

impl StaticBuffer {
    /// Returns a mutable slice reference to the underlying pooled memory buffer.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { &mut *POOL.buffers[self.index].get() }
    }
}

impl AsMut<[u8]> for StaticBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

impl Drop for StaticBuffer {
    fn drop(&mut self) {
        POOL.used[self.index].store(false, Ordering::Release);
    }
}

struct Pool {
    buffers: [UnsafeCell<[u8; BUFFER_SIZE]>; MAX_BUFFERS],
    used: [AtomicBool; MAX_BUFFERS],
}

unsafe impl Sync for Pool {}

#[allow(clippy::declare_interior_mutable_const, clippy::large_const_arrays)]
static POOL: Pool = {
    const INIT_BOOL: AtomicBool = AtomicBool::new(false);
    const INIT_BUF: UnsafeCell<[u8; BUFFER_SIZE]> = UnsafeCell::new([0; BUFFER_SIZE]);
    Pool {
        buffers: [INIT_BUF; MAX_BUFFERS],
        used: [INIT_BOOL; MAX_BUFFERS],
    }
};

pub fn acquire_buffer() -> Option<StaticBuffer> {
    for i in 0..MAX_BUFFERS {
        if POOL.used[i]
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return Some(StaticBuffer { index: i });
        }
    }
    None
}

/// Computes the canonical Allowed Fast set for a peer according to BEP 6.
///
/// Returns an ordered list of up to `k` piece indices that a choked peer is
/// permitted to request.
///
/// Canonical algorithm from BEP 6:
/// 1. Mask IPv4 to /24 (top 3 octets, 4th octet = 0) or IPv6 to /48.
/// 2. Prepend masked IP to the 20-byte torrent info-hash.
/// 3. Repeatedly compute SHA-1 hash over the state buffer.
/// 4. Extract big-endian 32-bit integers from the 20-byte digest modulo `num_pieces`.
/// 5. Append unique piece indices until `k` pieces are gathered or all pieces exhausted.
pub fn generate_allowed_fast_set(
    k: usize,
    num_pieces: usize,
    info_hash: &[u8],
    peer_ip: &str,
) -> alloc::vec::Vec<u32> {
    if num_pieces == 0 || k == 0 || info_hash.len() < 20 {
        return alloc::vec::Vec::new();
    }
    let target_k = k.min(num_pieces);
    let mut allowed_fast = alloc::vec::Vec::with_capacity(target_k);

    // Extract IP bytes and apply BEP 6 masking (/24 for IPv4, /48 for IPv6)
    let mut x = alloc::vec::Vec::new();
    #[cfg(feature = "std")]
    {
        use std::net::{IpAddr, SocketAddr};
        let stripped = peer_ip.trim_start_matches("webrtc:");
        if let Ok(addr) = stripped
            .parse::<IpAddr>()
            .or_else(|_| stripped.parse::<SocketAddr>().map(|s| s.ip()))
        {
            match addr {
                IpAddr::V4(v4) => {
                    let oct = v4.octets();
                    x.extend_from_slice(&[oct[0], oct[1], oct[2], 0]);
                }
                IpAddr::V6(v6) => {
                    let mut oct = v6.octets();
                    oct[6..].fill(0);
                    x.extend_from_slice(&oct);
                }
            }
        }
    }
    if x.is_empty() {
        // Fallback manual IPv4 parser
        let clean = peer_ip.trim_start_matches("webrtc:");
        let host = clean.split(':').next().unwrap_or(clean);
        let parts: alloc::vec::Vec<&str> = host.split('.').collect();
        if parts.len() == 4 {
            if let (Ok(b0), Ok(b1), Ok(b2), _) = (
                parts[0].parse::<u8>(),
                parts[1].parse::<u8>(),
                parts[2].parse::<u8>(),
                parts[3].parse::<u8>(),
            ) {
                x.extend_from_slice(&[b0, b1, b2, 0]);
            }
        }
    }
    if x.is_empty() {
        x.extend_from_slice(&[127, 0, 0, 0]);
    }
    x.extend_from_slice(&info_hash[..20]);

    while allowed_fast.len() < target_k {
        use sha1::Digest;
        let mut hasher = sha1::Sha1::new();
        hasher.update(&x);
        let digest = hasher.finalize();
        for i in 0..5 {
            if allowed_fast.len() >= target_k {
                break;
            }
            let j = i * 4;
            let y = u32::from_be_bytes([digest[j], digest[j + 1], digest[j + 2], digest[j + 3]]);
            let index = y % (num_pieces as u32);
            if !allowed_fast.contains(&index) {
                allowed_fast.push(index);
            }
        }
        x = digest.to_vec();
    }

    allowed_fast
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bep6_canonical_fast_set() {
        let info_hash = [0xaa; 20];
        let ip = "80.4.4.200";
        let num_pieces = 1313;

        let fast_7 = generate_allowed_fast_set(7, num_pieces, &info_hash, ip);
        assert_eq!(fast_7, alloc::vec![1059, 431, 808, 1217, 287, 376, 1188]);

        let fast_9 = generate_allowed_fast_set(9, num_pieces, &info_hash, ip);
        assert_eq!(
            fast_9,
            alloc::vec![1059, 431, 808, 1217, 287, 376, 1188, 353, 508]
        );
    }
}
