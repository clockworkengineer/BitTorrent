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

use alloc::string::String;
use alloc::format;

/// Packs a 32-bit unsigned integer into a 4-byte big-endian array.
pub fn pack_u32(value: u32) -> [u8; 4] {
    [
        (value >> 24) as u8,
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ]
}

/// Unpacks a 32-bit unsigned integer from a big-endian byte slice starting at `offset`.
pub fn unpack_u32(buffer: &[u8], offset: usize) -> u32 {
    ((buffer[offset] as u32) << 24)
        | ((buffer[offset + 1] as u32) << 16)
        | ((buffer[offset + 2] as u32) << 8)
        | (buffer[offset + 3] as u32)
}

/// Packs a 64-bit unsigned integer into an 8-byte big-endian array.
pub fn pack_u64(value: u64) -> [u8; 8] {
    [
        (value >> 56) as u8,
        (value >> 48) as u8,
        (value >> 40) as u8,
        (value >> 32) as u8,
        (value >> 24) as u8,
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ]
}

/// Unpacks a 64-bit unsigned integer from a big-endian byte slice starting at `offset`.
pub fn unpack_u64(buffer: &[u8], offset: usize) -> u64 {
    ((buffer[offset] as u64) << 56)
        | ((buffer[offset + 1] as u64) << 48)
        | ((buffer[offset + 2] as u64) << 40)
        | ((buffer[offset + 3] as u64) << 32)
        | ((buffer[offset + 4] as u64) << 24)
        | ((buffer[offset + 5] as u64) << 16)
        | ((buffer[offset + 6] as u64) << 8)
        | (buffer[offset + 7] as u64)
}

/// Formats a 20-byte info hash byte slice into its hexadecimal string representation.
pub fn info_hash_to_string(info_hash: &[u8]) -> String {
    info_hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Calculates the byte index and bit mask for a piece in a standard BitTorrent bitfield.
pub fn get_bitfield_index_and_mask(piece_number: u32) -> (usize, u8) {
    let byte_index = (piece_number >> 3) as usize;
    let bit_mask = 0x80 >> (piece_number & 0x7);
    (byte_index, bit_mask)
}

#[cfg(feature = "std")]
static DEBUG_LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
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
                .expect("cannot open debug.log");
            Mutex::new(f)
        });
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{}", msg);
            let _ = f.flush();
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

pub struct YieldNow {
    yielded: bool,
}

impl core::future::Future for YieldNow {
    type Output = ();

    fn poll(mut self: core::pin::Pin<&mut Self>, cx: &mut core::task::Context<'_>) -> core::task::Poll<Self::Output> {
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
