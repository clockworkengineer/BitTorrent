//! Bencode encoder and decoder
//!
//! Provides structures and functions for decoding and encoding data in the
//! Bencode format, which is the standard serialization format used by BitTorrent.

use crate::error::{BitTorrentError, BencodeError};
use alloc::vec::Vec;
use alloc::string::String;
use alloc::string::ToString;

/// Represents a node in a parsed Bencode structure.
#[derive(Debug, Clone, PartialEq)]
pub enum BNode<'a> {
    Dictionary(Vec<(&'a [u8], BNode<'a>)>),
    List(Vec<BNode<'a>>),
    Number(&'a [u8]),
    String(&'a [u8]),
}

impl<'a> BNode<'a> {
    /// Attempts to retrieve a value from a dictionary node using the specified byte slice key.
    /// Returns `None` if the node is not a dictionary or if the key does not exist.
    pub fn dict_get(&self, key: &[u8]) -> Option<&BNode<'a>> {
        match self {
            BNode::Dictionary(entries) => entries
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Returns the byte slice representation if the node is a `BNode::String`.
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            BNode::String(bytes) => Some(*bytes),
            _ => None,
        }
    }

    /// Returns the raw byte representation if the node is a `BNode::Number`.
    pub fn as_number_bytes(&self) -> Option<&[u8]> {
        match self {
            BNode::Number(bytes) => Some(*bytes),
            _ => None,
        }
    }
}

/// Helper struct providing entry points for decoding and encoding Bencode data.
pub struct Bencode;

impl Bencode {
    /// Decodes a Bencode byte slice into a `BNode`.
    /// Returns an error if the input contains invalid Bencode or trailing bytes.
    pub fn decode(buffer: &[u8]) -> Result<BNode<'_>, BitTorrentError> {
        let mut parser = Parser {
            buffer,
            position: 0,
            depth: 0,
        };
        let node = parser.decode_bnode()?;
        if parser.position != parser.buffer.len() {
            return Err(BitTorrentError::Bencode(BencodeError::TrailingBytes));
        }
        Ok(node)
    }

    /// Decodes a Bencode byte slice and returns the BNode alongside the number of bytes consumed.
    /// This is useful when raw payload bytes follow the Bencode message.
    pub fn decode_partial(buffer: &[u8]) -> Result<(BNode<'_>, usize), BitTorrentError> {
        let mut parser = Parser {
            buffer,
            position: 0,
            depth: 0,
        };
        let node = parser.decode_bnode()?;
        Ok((node, parser.position))
    }

    /// Encodes a `BNode` structure into a Bencode-compliant byte vector.
    pub fn encode(bnode: &BNode<'_>) -> Vec<u8> {
        let mut output = Vec::new();
        encode_node(bnode, &mut output);
        output
    }

    /// Recursively searches the `bnode` dictionary and nested dictionaries for a matching key.
    pub fn get_dictionary_entry<'a, 'b>(bnode: &'b BNode<'a>, key: &[u8]) -> Option<&'b BNode<'a>> {
        match bnode {
            BNode::Dictionary(entries) => {
                for (entry_key, entry_value) in entries {
                    if *entry_key == key {
                        return Some(entry_value);
                    }
                }
                for (_, entry_value) in entries {
                    if let BNode::Dictionary(_) = entry_value {
                        if let Some(found) = Bencode::get_dictionary_entry(entry_value, key) {
                            return Some(found);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Recursively retrieves a dictionary entry by key and formats its value as a UTF-8 string.
    pub fn get_dictionary_entry_string(bnode: &BNode<'_>, key: &str) -> Option<String> {
        Bencode::get_dictionary_entry(bnode, key.as_bytes()).and_then(|node| {
            if let Some(bytes) = node.as_string() {
                return Some(String::from_utf8_lossy(bytes).to_string());
            }
            if let Some(bytes) = node.as_number_bytes() {
                return Some(String::from_utf8_lossy(bytes).to_string());
            }
            None
        })
    }
}

const MAX_DEPTH: usize = 50;
const MAX_STRING_LEN: usize = 16_777_216; // 16 MB limit

/// A Bencode parser state tracker.
struct Parser<'a> {
    buffer: &'a [u8],
    position: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    /// Decodes the next `BNode` from the stream based on the leading byte indicator.
    fn decode_bnode(&mut self) -> Result<BNode<'a>, BitTorrentError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(BitTorrentError::Bencode(BencodeError::NestingDepthExceeded));
        }
        let byte = self
            .current_byte()
            .ok_or_else(|| BitTorrentError::Bencode(BencodeError::UnexpectedEnd))?;
        let node = match byte {
            b'd' => self.decode_dictionary(),
            b'l' => self.decode_list(),
            b'i' => self.decode_integer(),
            b'0'..=b'9' => self.decode_string().map(BNode::String),
            _ => Err(BitTorrentError::Bencode(BencodeError::InvalidByte(*byte))),
        };
        self.depth -= 1;
        node
    }

    /// Decodes a Bencode dictionary (starts with 'd', ends with 'e').
    fn decode_dictionary(&mut self) -> Result<BNode<'a>, BitTorrentError> {
        self.position += 1;
        let mut dict = Vec::new();
        while self.current_byte() != Some(&b'e') {
            let key = self.decode_string()?;
            let value = self.decode_bnode()?;
            dict.push((key, value));
        }
        self.position += 1;
        Ok(BNode::Dictionary(dict))
    }

    /// Decodes a Bencode list (starts with 'l', ends with 'e').
    fn decode_list(&mut self) -> Result<BNode<'a>, BitTorrentError> {
        self.position += 1;
        let mut list = Vec::new();
        while self.current_byte() != Some(&b'e') {
            list.push(self.decode_bnode()?);
        }
        self.position += 1;
        Ok(BNode::List(list))
    }

    /// Decodes a Bencode integer (starts with 'i', ends with 'e').
    fn decode_integer(&mut self) -> Result<BNode<'a>, BitTorrentError> {
        self.position += 1;
        let start = self.position;
        while let Some(&b) = self.current_byte() {
            if b == b'e' {
                break;
            }
            if !(b'0'..=b'9').contains(&b) && b != b'-' {
                return Err(BitTorrentError::Bencode(BencodeError::InvalidDigit));
            }
            self.position += 1;
        }
        let end = self.position;
        if self.current_byte() != Some(&b'e') {
            return Err(BitTorrentError::Bencode(BencodeError::UnterminatedInteger));
        }
        let number_bytes = &self.buffer[start..end];
        if number_bytes.len() > 20 {
            return Err(BitTorrentError::Bencode(BencodeError::IntegerTooLong));
        }
        let number_str = core::str::from_utf8(number_bytes)
            .map_err(|e| BitTorrentError::Bencode(BencodeError::Custom(e.to_string())))?;
        if number_str.is_empty()
            || (number_str.starts_with('0') && number_str.len() > 1)
            || number_str == "-0"
        {
            return Err(BitTorrentError::Bencode(BencodeError::InvalidIntegerFormat));
        }
        self.position += 1;
        Ok(BNode::Number(number_bytes))
    }

    /// Decodes a Bencode string (format: <length>:<data>).
    fn decode_string(&mut self) -> Result<&'a [u8], BitTorrentError> {
        let start = self.position;
        while let Some(&b) = self.current_byte() {
            if b == b':' {
                break;
            }
            if !b.is_ascii_digit() {
                return Err(BitTorrentError::Bencode(BencodeError::InvalidStringLength));
            }
            self.position += 1;
        }
        let length_bytes = core::str::from_utf8(&self.buffer[start..self.position])
            .map_err(|e| BitTorrentError::Bencode(BencodeError::Custom(e.to_string())))?;
        if length_bytes.is_empty() || (length_bytes.starts_with('0') && length_bytes != "0") {
            return Err(BitTorrentError::Bencode(BencodeError::InvalidStringLength));
        }
        if length_bytes.len() > 10 {
            return Err(BitTorrentError::Bencode(BencodeError::StringTooLong));
        }
        let length = length_bytes
            .parse::<usize>()
            .map_err(BitTorrentError::from)?;
        if length > MAX_STRING_LEN {
            return Err(BitTorrentError::Bencode(BencodeError::StringTooLong));
        }
        self.position += 1;
        let end = self.position + length;
        if end > self.buffer.len() {
            return Err(BitTorrentError::Bencode(BencodeError::UnexpectedEnd));
        }
        let string_bytes = &self.buffer[self.position..end];
        self.position = end;
        Ok(string_bytes)
    }

    /// Returns a reference to the byte at the current parser position, if in bounds.
    fn current_byte(&self) -> Option<&u8> {
        self.buffer.get(self.position)
    }
}

/// Helper function to serialize a `BNode` recursively into Bencode format.
fn encode_node(node: &BNode<'_>, output: &mut Vec<u8>) {
    match node {
        BNode::Dictionary(dict) => {
            output.push(b'd');
            let mut entries: Vec<&(&[u8], BNode<'_>)> = dict.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (key, value) in entries {
                output.extend_from_slice(key.len().to_string().as_bytes());
                output.push(b':');
                output.extend_from_slice(key);
                encode_node(value, output);
            }
            output.push(b'e');
        }
        BNode::List(list) => {
            output.push(b'l');
            for item in list {
                encode_node(item, output);
            }
            output.push(b'e');
        }
        BNode::Number(number) => {
            output.push(b'i');
            output.extend_from_slice(number);
            output.push(b'e');
        }
        BNode::String(value) => {
            output.extend_from_slice(value.len().to_string().as_bytes());
            output.push(b':');
            output.extend_from_slice(value);
        }
    }
}
