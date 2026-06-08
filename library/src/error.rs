//! Error types
//!
//! Defines the custom `BitTorrentError` enum, which represents errors
//! that can occur during Bencode parsing, peer communication, or file I/O,
//! alongside conversions from standard library error types.

#[cfg(feature = "std")]
use std::io;

/// Custom error type representing various errors in the BitTorrent library.
#[derive(Debug)]
pub enum BitTorrentError {
    #[cfg(feature = "std")]
    Io(io::Error),
    InvalidBencode(alloc::string::String),
    MissingField(alloc::string::String),
    Parse(alloc::string::String),
    NotParsed(alloc::string::String),
}

impl core::fmt::Display for BitTorrentError {
    /// Formats the error for user-facing display.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            #[cfg(feature = "std")]
            BitTorrentError::Io(err) => write!(f, "I/O error: {err}"),
            BitTorrentError::InvalidBencode(msg) => write!(f, "Invalid Bencode: {msg}"),
            BitTorrentError::MissingField(field) => write!(f, "Missing field: {field}"),
            BitTorrentError::Parse(msg) => write!(f, "Parse error: {msg}"),
            BitTorrentError::NotParsed(msg) => write!(f, "BitTorrent Error: {msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for BitTorrentError {
    /// Returns the underlying source of the error, if applicable.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            BitTorrentError::Io(err) => Some(err),
            _ => None,
        }
    }
}

#[cfg(feature = "std")]
impl From<io::Error> for BitTorrentError {
    /// Converts a standard `io::Error` into a `BitTorrentError`.
    fn from(err: io::Error) -> Self {
        BitTorrentError::Io(err)
    }
}

impl From<core::num::ParseIntError> for BitTorrentError {
    /// Converts a standard `ParseIntError` into a `BitTorrentError`.
    fn from(err: core::num::ParseIntError) -> Self {
        use alloc::string::ToString;
        BitTorrentError::Parse(err.to_string())
    }
}

impl From<alloc::string::FromUtf8Error> for BitTorrentError {
    /// Converts a standard `FromUtf8Error` into a `BitTorrentError`.
    fn from(err: alloc::string::FromUtf8Error) -> Self {
        use alloc::string::ToString;
        BitTorrentError::Parse(err.to_string())
    }
}
