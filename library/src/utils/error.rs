//! Error types
//!
//! Defines the custom `BitTorrentError` enum, which represents errors
//! that can occur during Bencode parsing, peer communication, or file I/O,
//! alongside conversions from standard library error types.

#[cfg(feature = "std")]
use std::io;

/// Detailed Bencode parsing error reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BencodeError {
    UnexpectedEnd,
    InvalidDigit,
    UnterminatedInteger,
    InvalidIntegerFormat,
    InvalidStringLength,
    StringTooLong,
    IntegerTooLong,
    TrailingBytes,
    NestingDepthExceeded,
    InvalidByte(u8),
    Custom(alloc::string::String),
}

impl core::fmt::Display for BencodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BencodeError::UnexpectedEnd => write!(f, "Unexpected end of input"),
            BencodeError::InvalidDigit => write!(f, "Invalid integer digit"),
            BencodeError::UnterminatedInteger => write!(f, "Unterminated integer"),
            BencodeError::InvalidIntegerFormat => write!(f, "Invalid integer format"),
            BencodeError::InvalidStringLength => write!(f, "Invalid string length"),
            BencodeError::StringTooLong => write!(f, "String length exceeds safe limit"),
            BencodeError::IntegerTooLong => write!(f, "Integer representation is too long"),
            BencodeError::TrailingBytes => write!(f, "Trailing bytes after parsing"),
            BencodeError::NestingDepthExceeded => write!(f, "Nesting depth limit exceeded"),
            BencodeError::InvalidByte(b) => write!(f, "Unexpected byte: {b}"),
            BencodeError::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

/// Custom error type representing various errors in the BitTorrent library.
#[derive(Debug)]
pub enum BitTorrentError {
    #[cfg(feature = "std")]
    Io(io::Error),
    Bencode(BencodeError),
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
            BitTorrentError::Bencode(err) => write!(f, "Invalid Bencode: {err}"),
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

impl From<alloc::string::String> for BitTorrentError {
    fn from(msg: alloc::string::String) -> Self {
        BitTorrentError::Parse(msg)
    }
}

impl From<&str> for BitTorrentError {
    fn from(msg: &str) -> Self {
        BitTorrentError::Parse(alloc::string::String::from(msg))
    }
}
