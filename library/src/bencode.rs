//! Bencode encoder and decoder
//!
//! Provides structures and functions for decoding and encoding data in the
//! Bencode format, which is the standard serialization format used by BitTorrent.

use crate::error::BitTorrentError;

/// Represents a node in a parsed Bencode structure.
#[derive(Debug, Clone, PartialEq)]
pub enum BNode {
    Dictionary(Vec<(Vec<u8>, BNode)>),
    List(Vec<BNode>),
    Number(Vec<u8>),
    String(Vec<u8>),
}

impl BNode {
    /// Attempts to retrieve a value from a dictionary node using the specified byte slice key.
    /// Returns `None` if the node is not a dictionary or if the key does not exist.
    pub fn dict_get<'a>(&'a self, key: &[u8]) -> Option<&'a BNode> {
        match self {
            BNode::Dictionary(entries) => entries
                .iter()
                .find(|(k, _)| k.as_slice() == key)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    /// Returns the byte slice representation if the node is a `BNode::String`.
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            BNode::String(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Returns the raw byte representation if the node is a `BNode::Number`.
    pub fn as_number_bytes(&self) -> Option<&[u8]> {
        match self {
            BNode::Number(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// Helper struct providing entry points for decoding and encoding Bencode data.
pub struct Bencode;

impl Bencode {
    /// Decodes a Bencode byte slice into a `BNode`.
    /// Returns an error if the input contains invalid Bencode or trailing bytes.
    pub fn decode(buffer: &[u8]) -> Result<BNode, BitTorrentError> {
        let mut parser = Parser {
            buffer,
            position: 0,
        };
        let node = parser.decode_bnode()?;
        if parser.position != parser.buffer.len() {
            return Err(BitTorrentError::InvalidBencode(
                "Trailing bytes after parsing".to_string(),
            ));
        }
        Ok(node)
    }

    /// Encodes a `BNode` structure into a Bencode-compliant byte vector.
    pub fn encode(bnode: &BNode) -> Vec<u8> {
        let mut output = Vec::new();
        encode_node(bnode, &mut output);
        output
    }

    /// Recursively searches the `bnode` dictionary and nested dictionaries for a matching key.
    pub fn get_dictionary_entry<'a>(bnode: &'a BNode, key: &[u8]) -> Option<&'a BNode> {
        match bnode {
            BNode::Dictionary(entries) => {
                for (entry_key, entry_value) in entries {
                    if entry_key.as_slice() == key {
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
    pub fn get_dictionary_entry_string(bnode: &BNode, key: &str) -> Option<String> {
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

/// A Bencode parser state tracker.
struct Parser<'a> {
    buffer: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
    /// Decodes the next `BNode` from the stream based on the leading byte indicator.
    fn decode_bnode(&mut self) -> Result<BNode, BitTorrentError> {
        let byte = self
            .current_byte()
            .ok_or_else(|| BitTorrentError::InvalidBencode("Unexpected end of input".into()))?;
        match byte {
            b'd' => self.decode_dictionary(),
            b'l' => self.decode_list(),
            b'i' => self.decode_integer(),
            b'0'..=b'9' => self.decode_string().map(BNode::String),
            _ => Err(BitTorrentError::InvalidBencode(format!(
                "Unexpected byte: {}",
                byte
            ))),
        }
    }

    /// Decodes a Bencode dictionary (starts with 'd', ends with 'e').
    fn decode_dictionary(&mut self) -> Result<BNode, BitTorrentError> {
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
    fn decode_list(&mut self) -> Result<BNode, BitTorrentError> {
        self.position += 1;
        let mut list = Vec::new();
        while self.current_byte() != Some(&b'e') {
            list.push(self.decode_bnode()?);
        }
        self.position += 1;
        Ok(BNode::List(list))
    }

    /// Decodes a Bencode integer (starts with 'i', ends with 'e').
    fn decode_integer(&mut self) -> Result<BNode, BitTorrentError> {
        self.position += 1;
        let start = self.position;
        while let Some(&b) = self.current_byte() {
            if b == b'e' {
                break;
            }
            if !(b'0'..=b'9').contains(&b) && b != b'-' {
                return Err(BitTorrentError::InvalidBencode(
                    "Invalid integer digit".to_string(),
                ));
            }
            self.position += 1;
        }
        let end = self.position;
        if self.current_byte() != Some(&b'e') {
            return Err(BitTorrentError::InvalidBencode(
                "Unterminated integer".into(),
            ));
        }
        let number_bytes = &self.buffer[start..end];
        let number_str = std::str::from_utf8(number_bytes)
            .map_err(|e| BitTorrentError::InvalidBencode(e.to_string()))?;
        if number_str.is_empty()
            || (number_str.starts_with('0') && number_str.len() > 1)
            || number_str == "-0"
        {
            return Err(BitTorrentError::InvalidBencode(
                "Invalid integer format".into(),
            ));
        }
        self.position += 1;
        Ok(BNode::Number(number_bytes.to_vec()))
    }

    /// Decodes a Bencode string (format: <length>:<data>).
    fn decode_string(&mut self) -> Result<Vec<u8>, BitTorrentError> {
        let start = self.position;
        while let Some(&b) = self.current_byte() {
            if b == b':' {
                break;
            }
            if !b.is_ascii_digit() {
                return Err(BitTorrentError::InvalidBencode(
                    "Invalid string length".into(),
                ));
            }
            self.position += 1;
        }
        let length_bytes = std::str::from_utf8(&self.buffer[start..self.position])
            .map_err(|e| BitTorrentError::InvalidBencode(e.to_string()))?;
        if length_bytes.is_empty() || (length_bytes.starts_with('0') && length_bytes != "0") {
            return Err(BitTorrentError::InvalidBencode(
                "Invalid string length".into(),
            ));
        }
        let length = length_bytes
            .parse::<usize>()
            .map_err(BitTorrentError::from)?;
        self.position += 1;
        let end = self.position + length;
        if end > self.buffer.len() {
            return Err(BitTorrentError::InvalidBencode(
                "String extends past end of buffer".into(),
            ));
        }
        let string_bytes = self.buffer[self.position..end].to_vec();
        self.position = end;
        Ok(string_bytes)
    }

    /// Returns a reference to the byte at the current parser position, if in bounds.
    fn current_byte(&self) -> Option<&u8> {
        self.buffer.get(self.position)
    }
}

/// Helper function to serialize a `BNode` recursively into Bencode format.
fn encode_node(node: &BNode, output: &mut Vec<u8>) {
    match node {
        BNode::Dictionary(dict) => {
            output.push(b'd');
            let mut entries: Vec<&(Vec<u8>, BNode)> = dict.iter().collect();
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
