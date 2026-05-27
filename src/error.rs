use std::error::Error as StdError;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum BitTorrentError {
    Io(io::Error),
    InvalidBencode(String),
    MissingField(String),
    Parse(String),
    NotParsed(String),
}

impl fmt::Display for BitTorrentError {
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
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            BitTorrentError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for BitTorrentError {
    fn from(err: io::Error) -> Self {
        BitTorrentError::Io(err)
    }
}

impl From<std::num::ParseIntError> for BitTorrentError {
    fn from(err: std::num::ParseIntError) -> Self {
        BitTorrentError::Parse(err.to_string())
    }
}

impl From<std::string::FromUtf8Error> for BitTorrentError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        BitTorrentError::Parse(err.to_string())
    }
}
