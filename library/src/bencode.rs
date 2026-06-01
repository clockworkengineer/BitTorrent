use crate::error::BitTorrentError;

#[derive(Debug, Clone, PartialEq)]
pub enum BNode {
    Dictionary(Vec<(Vec<u8>, BNode)>),
    List(Vec<BNode>),
    Number(Vec<u8>),
    String(Vec<u8>),
}

impl BNode {
    pub fn dict_get<'a>(&'a self, key: &[u8]) -> Option<&'a BNode> {
        match self {
            BNode::Dictionary(entries) => entries
                .iter()
                .find(|(k, _)| k.as_slice() == key)
                .map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            BNode::String(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub fn as_number_bytes(&self) -> Option<&[u8]> {
        match self {
            BNode::Number(bytes) => Some(bytes),
            _ => None,
        }
    }
}

pub struct Bencode;

impl Bencode {
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

    pub fn encode(bnode: &BNode) -> Vec<u8> {
        let mut output = Vec::new();
        encode_node(bnode, &mut output);
        output
    }

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

struct Parser<'a> {
    buffer: &'a [u8],
    position: usize,
}

impl<'a> Parser<'a> {
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

    fn decode_list(&mut self) -> Result<BNode, BitTorrentError> {
        self.position += 1;
        let mut list = Vec::new();
        while self.current_byte() != Some(&b'e') {
            list.push(self.decode_bnode()?);
        }
        self.position += 1;
        Ok(BNode::List(list))
    }

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

    fn current_byte(&self) -> Option<&u8> {
        self.buffer.get(self.position)
    }
}

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
