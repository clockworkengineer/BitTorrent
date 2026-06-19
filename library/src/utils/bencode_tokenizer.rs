use crate::error::{BitTorrentError, BencodeError};
use alloc::string::ToString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BencodeToken<'a> {
    DictStart,
    ListStart,
    End,
    Integer(&'a [u8]),
    String(&'a [u8]),
}

#[derive(Debug, Clone)]
pub struct BencodeTokenizer<'a> {
    buffer: &'a [u8],
    position: usize,
}

impl<'a> BencodeTokenizer<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self { buffer, position: 0 }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn set_position(&mut self, pos: usize) {
        self.position = pos;
    }

    fn current_byte(&self) -> Option<&u8> {
        self.buffer.get(self.position)
    }

    /// Read next token
    pub fn next_token(&mut self) -> Option<Result<BencodeToken<'a>, BitTorrentError>> {
        let byte = match self.current_byte() {
            Some(&b) => b,
            None => return None,
        };

        match byte {
            b'd' => {
                self.position += 1;
                Some(Ok(BencodeToken::DictStart))
            }
            b'l' => {
                self.position += 1;
                Some(Ok(BencodeToken::ListStart))
            }
            b'e' => {
                self.position += 1;
                Some(Ok(BencodeToken::End))
            }
            b'i' => {
                self.position += 1;
                let start = self.position;
                while let Some(&b) = self.current_byte() {
                    if b == b'e' {
                        break;
                    }
                    if !(b'0'..=b'9').contains(&b) && b != b'-' {
                        return Some(Err(BitTorrentError::Bencode(BencodeError::InvalidDigit)));
                    }
                    self.position += 1;
                }
                let end = self.position;
                if self.current_byte() != Some(&b'e') {
                    return Some(Err(BitTorrentError::Bencode(BencodeError::UnterminatedInteger)));
                }
                let number_bytes = &self.buffer[start..end];
                if number_bytes.len() > 20 {
                    return Some(Err(BitTorrentError::Bencode(BencodeError::IntegerTooLong)));
                }
                if let Ok(number_str) = core::str::from_utf8(number_bytes) {
                    if number_str.is_empty()
                        || (number_str.starts_with('0') && number_str.len() > 1)
                        || number_str == "-0"
                    {
                        return Some(Err(BitTorrentError::Bencode(BencodeError::InvalidIntegerFormat)));
                    }
                } else {
                    return Some(Err(BitTorrentError::Bencode(BencodeError::Custom("Invalid UTF-8 integer".into()))));
                }
                self.position += 1; // consume 'e'
                Some(Ok(BencodeToken::Integer(number_bytes)))
            }
            b'0'..=b'9' => {
                let start = self.position;
                while let Some(&b) = self.current_byte() {
                    if b == b':' {
                        break;
                    }
                    if !b.is_ascii_digit() {
                        return Some(Err(BitTorrentError::Bencode(BencodeError::InvalidStringLength)));
                    }
                    self.position += 1;
                }
                let length_bytes_res = core::str::from_utf8(&self.buffer[start..self.position]);
                let length_bytes = match length_bytes_res {
                    Ok(s) => s,
                    Err(e) => return Some(Err(BitTorrentError::Bencode(BencodeError::Custom(e.to_string())))),
                };
                if length_bytes.is_empty() || (length_bytes.starts_with('0') && length_bytes != "0") {
                    return Some(Err(BitTorrentError::Bencode(BencodeError::InvalidStringLength)));
                }
                if length_bytes.len() > 10 {
                    return Some(Err(BitTorrentError::Bencode(BencodeError::StringTooLong)));
                }
                let length = match length_bytes.parse::<usize>() {
                    Ok(l) => l,
                    Err(e) => return Some(Err(BitTorrentError::from(e))),
                };
                if length > 16_777_216 { // 16 MB limit
                    return Some(Err(BitTorrentError::Bencode(BencodeError::StringTooLong)));
                }
                self.position += 1; // consume ':'
                let end = self.position + length;
                if end > self.buffer.len() {
                    return Some(Err(BitTorrentError::Bencode(BencodeError::UnexpectedEnd)));
                }
                let string_bytes = &self.buffer[self.position..end];
                self.position = end;
                Some(Ok(BencodeToken::String(string_bytes)))
            }
            b => Some(Err(BitTorrentError::Bencode(BencodeError::InvalidByte(b)))),
        }
    }
}

impl<'a> Iterator for BencodeTokenizer<'a> {
    type Item = Result<BencodeToken<'a>, BitTorrentError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_token()
    }
}
