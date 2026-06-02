//! Error types
//!
//! Defines the custom `BitTorrentError` enum, which represents errors
//! that can occur during Bencode parsing, peer communication, or file I/O,
//! alongside conversions from standard library error types.

use std::error::Error as StdError;
use std::fmt;
use std::io;

/// Custom error type representing various errors in the BitTorrent library.
#[derive(Debug)]
pub enum BitTorrentError {
    Io(io::Error),
    InvalidBencode(String),
    MissingField(String),
    Parse(String),
    NotParsed(String),
}

impl fmt::Display for BitTorrentError {
    /// Formats the error for user-facing display.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitTorrentError::Io(err) => write!(f, "I/O error: {err}"),
            BitTorrentError::InvalidBencode(msg) => write!(f, "Invalid Bencode: {msg}"),
            BitTorrentError::MissingField(field) => write!(f, "Missing field: {field}"),
            BitTorrentError::Parse(msg) => write!(f, "Parse error: {msg}"),
            BitTorrentError::NotParsed(msg) => write!(f, "BitTorrent Error: {msg}"),
        }
    }
}

impl StdError for BitTorrentError {
    /// Returns the underlying source of the error, if applicable.
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            BitTorrentError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for BitTorrentError {
    /// Converts a standard `io::Error` into a `BitTorrentError`.
    fn from(err: io::Error) -> Self {
        BitTorrentError::Io(err)
    }
}

impl From<std::num::ParseIntError> for BitTorrentError {
    /// Converts a standard `ParseIntError` into a `BitTorrentError`.
    fn from(err: std::num::ParseIntError) -> Self {
        BitTorrentError::Parse(err.to_string())
    }
}

impl From<std::string::FromUtf8Error> for BitTorrentError {
    /// Converts a standard `FromUtf8Error` into a `BitTorrentError`.
    fn from(err: std::string::FromUtf8Error) -> Self {
        BitTorrentError::Parse(err.to_string())
    }
}
