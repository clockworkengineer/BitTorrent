//! Torrent metainfo parser
//!
//! Handles parsing of `.torrent` files, extracting tracker URLs, files to download,
//! piece size information, and computing the info hash of the torrent file's `info` section.


use crate::utils::bencode_tokenizer::{BencodeToken, BencodeTokenizer};
use crate::error::BitTorrentError;
use sha1::{Digest, Sha1};

#[cfg(feature = "std")]
use std::collections::HashMap;
#[cfg(feature = "std")]
use std::fs;
#[cfg(feature = "std")]
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

#[cfg(not(feature = "std"))]
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use alloc::string::String;
use alloc::string::ToString;
use alloc::format;

#[cfg(not(feature = "std"))]
const MAIN_SEPARATOR: char = '/';

#[cfg(feature = "std")]
type MetaInfoDict = HashMap<String, Vec<u8>>;
#[cfg(not(feature = "std"))]
type MetaInfoDict = BTreeMap<String, Vec<u8>>;

#[cfg(feature = "std")]
pub type PieceLayersMap = HashMap<[u8; 32], Vec<[u8; 32]>>;
#[cfg(not(feature = "std"))]
pub type PieceLayersMap = BTreeMap<[u8; 32], Vec<[u8; 32]>>;

/// Priority level for downloading a file within a torrent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum FilePriority {
    /// Do not download pieces exclusive to this file.
    Skip = 0,
    /// Low priority.
    Low = 1,
    /// Normal priority (default).
    #[default]
    Normal = 4,
    /// High priority.
    High = 7,
}

impl FilePriority {
    /// Returns the priority as a lowercase string slice.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    /// Parses a string loosely into a `FilePriority`.
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_ascii_lowercase().trim() {
            "skip" | "0" | "none" | "off" => Self::Skip,
            "low" | "1" => Self::Low,
            "high" | "7" => Self::High,
            _ => Self::Normal,
        }
    }
}

/// Detailed information about a single file in a multi-file torrent or the single file itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDetails {
    /// Absolute path on the local filesystem (may be a UNC path on Windows).
    pub name: String,
    /// Path relative to the torrent root, using forward slashes (e.g. "Sintel/Sintel.mp4").
    /// This is the correct string to append to a BEP 19 web-seed URL.
    pub torrent_path: String,
    pub length: u64,
    pub md5sum: Option<String>,
    pub offset: u64,
    /// Root hash of the file's SHA-256 Merkle tree (BitTorrent v2 / BEP 52).
    pub pieces_root: Option<[u8; 32]>,
    /// Download priority for this file.
    pub priority: FilePriority,
}

impl FileDetails {
    /// Returns the root hash of the file's Merkle tree, if present.
    pub fn pieces_root(&self) -> Option<&[u8; 32]> {
        self.pieces_root.as_ref()
    }

    /// Returns the root hash of the file's Merkle tree formatted as hexadecimal.
    pub fn pieces_root_hex(&self) -> Option<String> {
        self.pieces_root.map(|r| {
            r.iter().map(|b| format!("{:02x}", b)).collect()
        })
    }

    /// Returns true if this file is a padding file inserted for v1 piece alignment in hybrid torrents.
    pub fn is_pad_file(&self) -> bool {
        self.torrent_path.contains("/.pad/") || self.torrent_path.starts_with(".pad/")
    }
}

/// Represents a parsed `.torrent` metadata file.
#[derive(Debug)]
pub struct MetaInfoFile {
    #[cfg(feature = "std")]
    pub torrent_file_name: PathBuf,
    meta_info_data: Vec<u8>,
    meta_info_dict: MetaInfoDict,
    #[cfg(feature = "v2")]
    pub piece_layers: PieceLayersMap,
    parsed: bool,
}

impl MetaInfoFile {
    /// Loads a torrent file directly from memory raw bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        MetaInfoFile {
            #[cfg(feature = "std")]
            torrent_file_name: PathBuf::new(),
            meta_info_data: bytes.to_vec(),
            #[cfg(feature = "std")]
            meta_info_dict: HashMap::new(),
            #[cfg(not(feature = "std"))]
            meta_info_dict: BTreeMap::new(),
            #[cfg(all(feature = "std", feature = "v2"))]
            piece_layers: HashMap::new(),
            #[cfg(all(not(feature = "std"), feature = "v2"))]
            piece_layers: BTreeMap::new(),
            parsed: false,
        }
    }

    /// Loads a torrent file from the given path but does not parse it yet.
    #[cfg(feature = "std")]
    pub fn new(path: impl AsRef<Path>) -> Result<Self, BitTorrentError> {
        let torrent_file_name = path.as_ref().to_path_buf();
        let mut meta_info = MetaInfoFile {
            torrent_file_name,
            meta_info_data: Vec::new(),
            meta_info_dict: HashMap::new(),
            #[cfg(feature = "v2")]
            piece_layers: HashMap::new(),
            parsed: false,
        };
        meta_info.load()?;
        Ok(meta_info)
    }

    /// Reads the raw file contents into memory.
    #[cfg(feature = "std")]
    fn load(&mut self) -> Result<(), BitTorrentError> {
        self.meta_info_data = fs::read(&self.torrent_file_name)?;
        Ok(())
    }

    /// Parses the bencoded raw metadata into dictionary keys and validates fields.
    pub fn parse(&mut self) -> Result<(), BitTorrentError> {
        let mut tokenizer = BencodeTokenizer::new(&self.meta_info_data);
        let root_token = tokenizer.next_token()
            .ok_or_else(|| BitTorrentError::Bencode(
                crate::error::BencodeError::Custom("Torrent file root is empty".into()),
            ))??;
        if root_token != BencodeToken::DictStart {
            return Err(BitTorrentError::Bencode(
                crate::error::BencodeError::Custom("Torrent file root is not a dictionary".into()),
            ));
        }

        while let Some(token_res) = tokenizer.next_token() {
            let token = token_res?;
            if token == BencodeToken::End {
                break;
            }
            let key = match token {
                BencodeToken::String(k) => k,
                _ => return Err(BitTorrentError::Bencode(
                    crate::error::BencodeError::Custom("Torrent root key is not a string".into()),
                )),
            };

            match key {
                b"announce" | b"comment" | b"created by" | b"creation date" => {
                    let val_token = tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    let val_bytes = match val_token {
                        BencodeToken::String(s) => s.to_vec(),
                        BencodeToken::Integer(i) => i.to_vec(),
                        _ => return Err(BitTorrentError::Bencode(
                            crate::error::BencodeError::Custom("Expected string or integer value".into()),
                        )),
                    };
                    self.meta_info_dict.insert(String::from_utf8_lossy(key).to_string(), val_bytes);
                }
                b"announce-list" => {
                    let list_token = tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    if list_token != BencodeToken::ListStart {
                        return Err(BitTorrentError::Bencode(
                            crate::error::BencodeError::Custom("announce-list is not a list".into()),
                        ));
                    }
                    let mut urls = Vec::new();
                    loop {
                        let next_tok = tokenizer.next_token()
                            .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                        if next_tok == BencodeToken::End {
                            break;
                        }
                        if next_tok != BencodeToken::ListStart {
                            return Err(BitTorrentError::Bencode(
                                crate::error::BencodeError::Custom("announce-list inner element is not a list".into()),
                            ));
                        }
                        loop {
                            let inner_tok = tokenizer.next_token()
                                .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                            if inner_tok == BencodeToken::End {
                                break;
                            }
                            if let BencodeToken::String(s) = inner_tok {
                                urls.push(String::from_utf8_lossy(s).to_string());
                            } else {
                                return Err(BitTorrentError::Bencode(
                                    crate::error::BencodeError::Custom("announce-list url is not a string".into()),
                                ));
                            }
                        }
                    }
                    self.meta_info_dict.insert("announce-list".to_string(), urls.join(",").into_bytes());
                }
                b"url-list" => {
                    let val_token = tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    match val_token {
                        BencodeToken::String(s) => {
                            self.meta_info_dict.insert("url-list".to_string(), s.to_vec());
                        }
                        BencodeToken::ListStart => {
                            let mut urls = Vec::new();
                            loop {
                                let next_tok = tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                if next_tok == BencodeToken::End {
                                    break;
                                }
                                if let BencodeToken::String(s) = next_tok {
                                    urls.push(String::from_utf8_lossy(s).to_string());
                                }
                            }
                            self.meta_info_dict.insert("url-list".to_string(), urls.join(",").into_bytes());
                        }
                        _ => return Err(BitTorrentError::Bencode(
                            crate::error::BencodeError::Custom("url-list must be a string or list".into()),
                        )),
                    }
                }
                b"piece layers" => {
                    let val_token = tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    if val_token == BencodeToken::DictStart {
                        while let Some(k_tok) = tokenizer.next_token() {
                            let k = k_tok?;
                            if k == BencodeToken::End {
                                break;
                            }
                            let k_bytes = match k {
                                BencodeToken::String(s) => s,
                                _ => break,
                            };
                            let val_tok = tokenizer.next_token()
                                .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                            let val_bytes = match val_tok {
                                BencodeToken::String(s) => s,
                                _ => break,
                            };
                            #[cfg(feature = "v2")]
                            if k_bytes.len() == 32 {
                                let mut root = [0u8; 32];
                                root.copy_from_slice(k_bytes);
                                let mut layer = Vec::with_capacity(val_bytes.len() / 32);
                                for chunk in val_bytes.chunks_exact(32) {
                                    let mut h = [0u8; 32];
                                    h.copy_from_slice(chunk);
                                    layer.push(h);
                                }
                                self.piece_layers.insert(root, layer);
                            }
                        }
                    }
                }
                b"info" => {
                    let start_pos = tokenizer.position();
                    let val_token = tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    
                    let mut depth = 0;
                    match val_token {
                        BencodeToken::DictStart | BencodeToken::ListStart => {
                            depth += 1;
                            while depth > 0 {
                                let tok = tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                if tok == BencodeToken::DictStart || tok == BencodeToken::ListStart {
                                    depth += 1;
                                } else if tok == BencodeToken::End {
                                    depth -= 1;
                                }
                            }
                        }
                        _ => {}
                    }
                    let end_pos = tokenizer.position();
                    let raw_info_bytes = &self.meta_info_data[start_pos..end_pos];
                    
                    // Parse fields inside 'info' dictionary
                    let mut info_tokenizer = BencodeTokenizer::new(raw_info_bytes);
                    let info_root = info_tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    if info_root != BencodeToken::DictStart {
                        return Err(BitTorrentError::Bencode(
                            crate::error::BencodeError::Custom("info must be a dictionary".into()),
                        ));
                    }
                    
                    while let Some(info_tok_res) = info_tokenizer.next_token() {
                        let info_tok = info_tok_res?;
                        if info_tok == BencodeToken::End {
                            break;
                        }
                        let info_key = match info_tok {
                            BencodeToken::String(k) => k,
                            _ => return Err(BitTorrentError::Bencode(
                                crate::error::BencodeError::Custom("info key is not a string".into()),
                            )),
                        };
                        
                        match info_key {
                            b"name" | b"piece length" | b"meta version" | b"private" | b"pieces" | b"length" | b"md5sum" => {
                                let val = info_tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                let val_bytes = match val {
                                    BencodeToken::String(s) => s.to_vec(),
                                    BencodeToken::Integer(i) => i.to_vec(),
                                    _ => return Err(BitTorrentError::Bencode(
                                        crate::error::BencodeError::Custom("Expected length/md5sum as string or integer".into()),
                                    )),
                                };
                                self.meta_info_dict.insert(String::from_utf8_lossy(info_key).to_string(), val_bytes);
                            }
                            b"files" => {
                                let list_tok = info_tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                if list_tok != BencodeToken::ListStart {
                                    return Err(BitTorrentError::Bencode(
                                        crate::error::BencodeError::Custom("files must be a list".into()),
                                    ));
                                }
                                let mut file_no = 0;
                                loop {
                                    let item_tok = info_tokenizer.next_token()
                                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                    if item_tok == BencodeToken::End {
                                        break;
                                    }
                                    if item_tok != BencodeToken::DictStart {
                                        return Err(BitTorrentError::Bencode(
                                            crate::error::BencodeError::Custom("files list item must be a dictionary".into()),
                                        ));
                                    }
                                    
                                    let mut length = String::new();
                                    let mut md5sum = String::new();
                                    let mut path_segments = Vec::new();
                                    let mut file_pieces_root: Option<Vec<u8>> = None;
                                    
                                    loop {
                                        let f_key_tok = info_tokenizer.next_token()
                                            .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                        if f_key_tok == BencodeToken::End {
                                            break;
                                        }
                                        let f_key = match f_key_tok {
                                            BencodeToken::String(k) => k,
                                            _ => return Err(BitTorrentError::Bencode(
                                                crate::error::BencodeError::Custom("File dict key is not a string".into()),
                                            )),
                                        };
                                        let f_val = info_tokenizer.next_token()
                                            .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                        match f_key {
                                            b"length" => {
                                                match f_val {
                                                    BencodeToken::Integer(i) | BencodeToken::String(i) => {
                                                        length = String::from_utf8_lossy(i).to_string();
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            b"pieces root" => {
                                                if let BencodeToken::String(s) = f_val {
                                                    if s.len() == 32 {
                                                        file_pieces_root = Some(s.to_vec());
                                                    }
                                                }
                                            }
                                            b"md5sum" => {
                                                match f_val {
                                                    BencodeToken::Integer(s) | BencodeToken::String(s) => {
                                                        md5sum = String::from_utf8_lossy(s).to_string();
                                                    }
                                                    _ => {}
                                                }
                                            }
                                            b"path" => {
                                                if f_val != BencodeToken::ListStart {
                                                    return Err(BitTorrentError::Bencode(
                                                        crate::error::BencodeError::Custom("path must be a list".into()),
                                                    ));
                                                }
                                                loop {
                                                    let path_tok = info_tokenizer.next_token()
                                                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                                    if path_tok == BencodeToken::End {
                                                        break;
                                                    }
                                                    if let BencodeToken::String(s) = path_tok {
                                                        path_segments.push(String::from_utf8_lossy(s).to_string());
                                                    }
                                                }
                                            }
                                            _ => {
                                                // Skip
                                                let mut depth = 0;
                                                match f_val {
                                                    BencodeToken::DictStart | BencodeToken::ListStart => {
                                                        depth += 1;
                                                        while depth > 0 {
                                                            let t = info_tokenizer.next_token()
                                                                .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                                            if t == BencodeToken::DictStart || t == BencodeToken::ListStart {
                                                                depth += 1;
                                                            } else if t == BencodeToken::End {
                                                                depth -= 1;
                                                            }
                                                        }
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
                                    
                                    let mut file_entry = path_segments
                                        .iter()
                                        .map(|seg| format!("{}{}", MAIN_SEPARATOR, seg))
                                        .collect::<String>();
                                    file_entry.push('\0');
                                    file_entry.push_str(&length);
                                    file_entry.push('\0');
                                    file_entry.push_str(&md5sum);

                                    let file_rel_path = path_segments.join("/");
                                    if let Some(pr) = file_pieces_root {
                                        let norm_p = file_rel_path.replace('\\', "/").trim_start_matches('/').to_string();
                                        self.meta_info_dict.insert(format!("pieces_root_path:{}", norm_p), pr.clone());
                                        self.meta_info_dict.insert(format!("pieces_root_{}", file_no), pr);
                                    }
                                    
                                    self.meta_info_dict.insert(file_no.to_string(), file_entry.into_bytes());
                                    file_no += 1;
                                }
                            }
                            b"file tree" => {
                                let tree_tok = info_tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                if tree_tok != BencodeToken::DictStart {
                                    return Err(BitTorrentError::Bencode(
                                        crate::error::BencodeError::Custom("file tree must be a dictionary".into()),
                                    ));
                                }
                                let mut current_path = Vec::new();
                                let mut files = Vec::new();
                                parse_file_tree_tokenized(&mut info_tokenizer, &mut current_path, &mut files)?;
                                
                                for (i, (path, length, pieces_root)) in files.into_iter().enumerate() {
                                    let file_entry = format!("{}{}\0{}", MAIN_SEPARATOR, path.replace("/", &MAIN_SEPARATOR.to_string()), length);
                                    let norm_path = path.replace('\\', "/").trim_start_matches('/').to_string();
                                    self.meta_info_dict.insert(format!("pieces_root_path:{}", norm_path), pieces_root.clone());
                                    self.meta_info_dict.insert(format!("pieces_root_{}", i), pieces_root);
                                    self.meta_info_dict.insert(i.to_string(), file_entry.into_bytes());
                                }
                            }
                            _ => {
                                // Skip
                                let val = info_tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                let mut depth = 0;
                                match val {
                                    BencodeToken::DictStart | BencodeToken::ListStart => {
                                        depth += 1;
                                        while depth > 0 {
                                            let t = info_tokenizer.next_token()
                                                .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                            if t == BencodeToken::DictStart || t == BencodeToken::ListStart {
                                                depth += 1;
                                            } else if t == BencodeToken::End {
                                                depth -= 1;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    
                    // Compute info hash
                    let is_v2 = if let Some(ver) = self.meta_info_dict.get("meta version") {
                        String::from_utf8_lossy(ver).trim() == "2"
                    } else {
                        false
                    };
                    let has_pieces = self.meta_info_dict.contains_key("pieces");
                    let is_hybrid = is_v2 && has_pieces;

                    let v1_hash: Option<[u8; 20]> = if !is_v2 || is_hybrid {
                        let mut v1_hasher = Sha1::new();
                        v1_hasher.update(raw_info_bytes);
                        let hash: [u8; 20] = v1_hasher.finalize().into();
                        self.meta_info_dict.insert("v1 info hash".to_string(), hash.to_vec());
                        Some(hash)
                    } else {
                        None
                    };

                    if is_v2 {
                        #[cfg(feature = "v2")]
                        {
                            use sha2::Digest;
                            let mut v2_hasher = sha2::Sha256::new();
                            v2_hasher.update(raw_info_bytes);
                            let v2_hash: [u8; 32] = v2_hasher.finalize().into();
                            self.meta_info_dict.insert("v2 info hash".to_string(), v2_hash.to_vec());

                            if is_hybrid {
                                // Hybrid mode: v1 info-hash is 20-byte SHA-1 for backwards compatibility with v1 swarms
                                if let Some(h) = v1_hash {
                                    self.meta_info_dict.insert("info hash".to_string(), h.to_vec());
                                }
                            } else {
                                // Pure v2 mode: 32-byte SHA-256 info-hash
                                self.meta_info_dict.insert("info hash".to_string(), v2_hash.to_vec());
                            }
                        }
                        #[cfg(not(feature = "v2"))]
                        {
                            return Err(BitTorrentError::Parse("BitTorrent v2 is not compiled in this build".into()));
                        }
                    } else if let Some(h) = v1_hash {
                        self.meta_info_dict.insert("info hash".to_string(), h.to_vec());
                    }
                }
                _ => {
                    // Skip
                    let val = tokenizer.next_token()
                        .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                    let mut depth = 0;
                    match val {
                        BencodeToken::DictStart | BencodeToken::ListStart => {
                            depth += 1;
                            while depth > 0 {
                                let t = tokenizer.next_token()
                                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                if t == BencodeToken::DictStart || t == BencodeToken::ListStart {
                                    depth += 1;
                                } else if t == BencodeToken::End {
                                    depth -= 1;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Validate required fields
        if !self.meta_info_dict.contains_key("announce") {
            return Err(BitTorrentError::MissingField("announce".into()));
        }
        if !self.meta_info_dict.contains_key("name") {
            return Err(BitTorrentError::MissingField("name".into()));
        }
        if !self.meta_info_dict.contains_key("piece length") {
            return Err(BitTorrentError::MissingField("piece length".into()));
        }
        
        let is_v2 = if let Some(ver) = self.meta_info_dict.get("meta version") {
            String::from_utf8_lossy(ver).trim() == "2"
        } else {
            false
        };

        if !is_v2 && !self.meta_info_dict.contains_key("pieces") {
            return Err(BitTorrentError::MissingField("pieces".into()));
        }
        if !self.meta_info_dict.contains_key("info hash") {
            return Err(BitTorrentError::MissingField("info".into()));
        }

        self.parsed = true;
        Ok(())
    }

    /// Returns the primary tracker URL announced in the torrent metadata.
    pub fn get_tracker(&self) -> Result<String, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }
        let announce = self
            .meta_info_dict
            .get("announce")
            .ok_or_else(|| BitTorrentError::MissingField("announce".into()))?;
        Ok(String::from_utf8_lossy(announce).to_string())
    }

    /// Returns the torrent name from the metadata.
    pub fn get_name(&self) -> Result<String, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }
        let name = self
            .meta_info_dict
            .get("name")
            .ok_or_else(|| BitTorrentError::MissingField("name".into()))?;
        Ok(String::from_utf8_lossy(name).to_string())
    }

    /// Returns the total size in bytes of all files in the torrent.
    pub fn get_total_size(&self) -> Result<u64, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }
        if !self.meta_info_dict.contains_key("0") {
            let length_bytes = self
                .meta_info_dict
                .get("length")
                .ok_or_else(|| BitTorrentError::MissingField("length".into()))?;
            let length = String::from_utf8_lossy(length_bytes).parse::<u64>()?;
            Ok(length)
        } else {
            let mut total = 0u64;
            let mut file_no = 0;
            loop {
                let key = file_no.to_string();
                if let Some(entry) = self.meta_info_dict.get(&key) {
                    let text = String::from_utf8_lossy(entry);
                    let parts: Vec<&str> = text.split('\0').collect();
                    if let Some(len_str) = parts.get(1) {
                        total += len_str.parse::<u64>().unwrap_or(0);
                    }
                    file_no += 1;
                } else {
                    break;
                }
            }
            Ok(total)
        }
    }

    /// Returns the optional comment associated with the torrent.
    pub fn get_comment(&self) -> Option<String> {
        self.meta_info_dict
            .get("comment")
            .map(|b| String::from_utf8_lossy(b).to_string())
    }

    /// Retrieves all tracker URLs from `announce` and `announce-list`.
    pub fn get_tracker_urls(&self) -> Result<Vec<String>, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }

        let mut urls = Vec::new();
        let announce = self
            .meta_info_dict
            .get("announce")
            .ok_or_else(|| BitTorrentError::MissingField("announce".into()))?;
        urls.push(String::from_utf8_lossy(announce).to_string());

        if let Some(announce_list) = self.meta_info_dict.get("announce-list") {
            let list = String::from_utf8_lossy(announce_list);
            for entry in list.split(',') {
                let tracker = entry.trim();
                if tracker.is_empty() || urls.contains(&tracker.to_string()) {
                    continue;
                }
                urls.push(tracker.to_string());
            }
        }

        Ok(urls)
    }

    /// Retrieves the list of WebSeed URLs from the "url-list" field.
    pub fn get_web_seeds(&self) -> Vec<String> {
        let mut urls = Vec::new();
        if let Some(url_list_bytes) = self.meta_info_dict.get("url-list") {
            let list_str = String::from_utf8_lossy(url_list_bytes);
            for entry in list_str.split(',') {
                let url = entry.trim();
                if !url.is_empty() {
                    urls.push(url.to_string());
                }
            }
        }
        urls
    }

    /// Returns whether the torrent is marked as private (BEP 27).
    pub fn is_private(&self) -> bool {
        if let Some(private_bytes) = self.meta_info_dict.get("private") {
            let val = String::from_utf8_lossy(private_bytes);
            val.trim() == "1"
        } else {
            false
        }
    }

    /// Returns whether the torrent is a BitTorrent V2 torrent (BEP 52).
    pub fn is_v2(&self) -> bool {
        if let Some(ver) = self.meta_info_dict.get("meta version") {
            String::from_utf8_lossy(ver).trim() == "2"
        } else {
            false
        }
    }

    /// Returns whether this is a BitTorrent v2 hybrid torrent (has both v1 pieces and v2 file tree).
    pub fn is_hybrid(&self) -> bool {
        self.is_v2() && self.meta_info_dict.contains_key("pieces")
    }

    /// Returns the 20-byte SHA-1 info-hash of the torrent's `info` dictionary (v1 or hybrid).
    pub fn get_v1_info_hash(&self) -> Result<[u8; 20], BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed("File has not been parsed.".into()));
        }
        let bytes = self.meta_info_dict.get("v1 info hash")
            .ok_or_else(|| BitTorrentError::MissingField("v1 info hash".into()))?;
        if bytes.len() == 20 {
            let mut arr = [0u8; 20];
            arr.copy_from_slice(bytes);
            Ok(arr)
        } else {
            Err(BitTorrentError::Parse("Invalid v1 info hash length".into()))
        }
    }

    /// Returns the 32-byte SHA-256 info-hash of the torrent's `info` dictionary (v2 or hybrid).
    pub fn get_v2_info_hash(&self) -> Result<[u8; 32], BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed("File has not been parsed.".into()));
        }
        let bytes = self.meta_info_dict.get("v2 info hash")
            .ok_or_else(|| BitTorrentError::MissingField("v2 info hash".into()))?;
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(bytes);
            Ok(arr)
        } else {
            Err(BitTorrentError::Parse("Invalid v2 info hash length".into()))
        }
    }

    /// Returns the map of piece layers parsed from the torrent file.
    #[cfg(feature = "v2")]
    pub fn get_piece_layers(&self) -> &PieceLayersMap {
        &self.piece_layers
    }

    /// Returns the primary info hash of the `info` dictionary of the torrent.
    pub fn get_info_hash(&self) -> Result<Vec<u8>, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }
        self.meta_info_dict
            .get("info hash")
            .map(|bytes| bytes.clone())
            .ok_or_else(|| BitTorrentError::MissingField("info hash".into()))
    }

    /// Returns the length of each standard piece in bytes.
    pub fn get_piece_length(&self) -> Result<u32, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }
        let piece_length = self
            .meta_info_dict
            .get("piece length")
            .ok_or_else(|| BitTorrentError::MissingField("piece length".into()))?;
        let text = String::from_utf8_lossy(piece_length);
        Ok(text.parse::<u32>()?)
    }

    /// Returns the raw concatenation of 20-byte SHA-1 hash values for all pieces.
    pub fn get_pieces_info_hash(&self) -> Result<Vec<u8>, BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }
        self.meta_info_dict
            .get("pieces")
            .map(|bytes| bytes.clone())
            .ok_or_else(|| BitTorrentError::MissingField("pieces".into()))
    }

    /// Validates the structure and content of parsed metainfo fields.
    pub fn validate(&self) -> Result<(), BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }

        let piece_length = self.get_piece_length()?;
        if piece_length == 0 {
            return Err(BitTorrentError::Parse(
                "Torrent piece length must be greater than zero.".into(),
            ));
        }

        if !self.is_v2() {
            let pieces = self.get_pieces_info_hash()?;
            if pieces.is_empty() || pieces.len() % crate::constants::HASH_LENGTH != 0 {
                return Err(BitTorrentError::Parse(
                    "Invalid torrent pieces hash length.".into(),
                ));
            }
        }

        #[cfg(feature = "std")]
        {
            let tracker = self.get_tracker()?;
            url::Url::parse(&tracker).map_err(|err| BitTorrentError::Parse(err.to_string()))?;

            let files = self.local_files_to_download_list(Path::new("."))?.1;
            if files.is_empty() {
                return Err(BitTorrentError::Parse(
                    "Torrent contains no file entries.".into(),
                ));
            }
        }

        #[cfg(not(feature = "std"))]
        {
            let tracker = self.get_tracker()?;
            if !tracker.starts_with("http://") && !tracker.starts_with("https://") && !tracker.starts_with("udp://") {
                return Err(BitTorrentError::Parse(
                    "Tracker URL has an invalid scheme.".into(),
                ));
            }
        }

        Ok(())
    }

    /// Resolves target files to be downloaded from the metainfo, returning their total bytes and details.
    #[cfg(feature = "std")]
    pub fn local_files_to_download_list(
        &self,
        download_path: impl AsRef<Path>,
    ) -> Result<(u64, Vec<FileDetails>), BitTorrentError> {
        if !self.parsed {
            return Err(BitTorrentError::NotParsed(
                "File has not been parsed.".into(),
            ));
        }

        let download_path = download_path.as_ref();
        let canonical_download = if download_path.exists() {
            download_path.canonicalize().map_err(|e| BitTorrentError::Parse(e.to_string()))?
        } else {
            download_path.to_path_buf()
        };
        let mut files_to_download = Vec::new();
        let mut total_bytes = 0u64;

        if !self.meta_info_dict.contains_key("0") {
            let name_bytes = self
                .meta_info_dict
                .get("name")
                .ok_or_else(|| BitTorrentError::MissingField("name".into()))?;
            let length_bytes = self
                .meta_info_dict
                .get("length")
                .ok_or_else(|| BitTorrentError::MissingField("length".into()))?;
            let name = String::from_utf8_lossy(name_bytes);
            Self::validate_relative_path(&name)?;
            let length = String::from_utf8_lossy(length_bytes).parse::<u64>()?;
            let file_path = canonical_download.join(name.as_ref());
            if !file_path.starts_with(&canonical_download) {
                return Err(BitTorrentError::Parse(
                    "Path traversal detected: file path is outside the download directory.".into(),
                ));
            }
            let pieces_root = self.meta_info_dict.get("pieces root")
                .or_else(|| self.meta_info_dict.get(&format!("pieces_root_path:{}", name.as_ref().replace('\\', "/").trim_start_matches('/'))))
                .or_else(|| self.meta_info_dict.get("pieces_root_0"))
                .and_then(|bytes| {
                    if bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(bytes);
                        Some(arr)
                    } else {
                        None
                    }
                });
            files_to_download.push(FileDetails {
                name: file_path.to_string_lossy().into_owned(),
                // Single-file torrent: torrent_path is just the file name.
                torrent_path: name.as_ref().replace('\\', "/"),
                length,
                md5sum: self
                    .meta_info_dict
                    .get("md5sum")
                    .map(|bytes| String::from_utf8_lossy(bytes).to_string()),
                offset: 0,
                pieces_root,
                priority: FilePriority::Normal,
            });
            total_bytes = length;
        } else {
            let name_bytes = self
                .meta_info_dict
                .get("name")
                .ok_or_else(|| BitTorrentError::MissingField("name".into()))?;
            let root_name = String::from_utf8_lossy(name_bytes);
            Self::validate_relative_path(&root_name)?;
            let directory = canonical_download.join(root_name.as_ref());
            if !directory.starts_with(&canonical_download) {
                return Err(BitTorrentError::Parse(
                    "Path traversal detected: root directory is outside the download directory.".into(),
                ));
            }
            let mut file_no = 0;
            loop {
                let key = file_no.to_string();
                let entry = if let Some(entry) = self.meta_info_dict.get(&key) {
                    String::from_utf8_lossy(entry).to_string()
                } else {
                    break;
                };
                let parts: Vec<&str> = entry.split('\0').collect();
                let path_value = parts.get(0).copied().unwrap_or("");
                let length_value = parts.get(1).copied().unwrap_or("0");
                let md5sum_value = parts.get(2).copied().unwrap_or("");
                let trimmed_path = path_value.trim_start_matches(['/', '\\'].as_ref());
                Self::validate_relative_path(trimmed_path)?;
                let file_path = directory.join(trimmed_path);
                if !file_path.starts_with(&directory) {
                    return Err(BitTorrentError::Parse(
                        "Path traversal detected: file path is outside the torrent root directory.".into(),
                    ));
                }
                let length = length_value.parse::<u64>()?;
                // torrent_path = "<root_name>/<file_sub_path>" with forward slashes.
                let torrent_path = format!(
                    "{}/{}",
                    root_name.as_ref().replace('\\', "/"),
                    trimmed_path.replace('\\', "/")
                );
                let norm_path = trimmed_path.replace('\\', "/").trim_start_matches('/').to_string();
                let pieces_root = self.meta_info_dict.get(&format!("pieces_root_path:{}", norm_path))
                    .or_else(|| self.meta_info_dict.get(&format!("pieces_root_{}", file_no)))
                    .and_then(|bytes| {
                        if bytes.len() == 32 {
                            let mut arr = [0u8; 32];
                            arr.copy_from_slice(bytes);
                            Some(arr)
                        } else {
                            None
                        }
                    });
                files_to_download.push(FileDetails {
                    name: file_path.to_string_lossy().into_owned(),
                    torrent_path,
                    length,
                    md5sum: if md5sum_value.is_empty() {
                        None
                    } else {
                        Some(md5sum_value.to_string())
                    },
                    offset: total_bytes,
                    pieces_root,
                    priority: FilePriority::Normal,
                });
                total_bytes += length;
                file_no += 1;
            }
        }

        Ok((total_bytes, files_to_download))
    }


    /// Validates that a parsed relative path is secure and contains no directory traversal elements.
    #[cfg(feature = "std")]
    fn validate_relative_path(path: &str) -> Result<(), BitTorrentError> {
        if path.is_empty() {
            return Err(BitTorrentError::Parse(
                "Torrent file path cannot be empty.".into(),
            ));
        }
        if path.contains('\0') {
            return Err(BitTorrentError::Parse(
                "Torrent file path cannot contain null bytes.".into(),
            ));
        }
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(BitTorrentError::Parse(
                "Torrent file paths must be relative.".into(),
            ));
        }
        if path.contains(':') {
            return Err(BitTorrentError::Parse(
                "Torrent file path contains invalid colon characters.".into(),
            ));
        }
        for segment in path.split(|c| c == '/' || c == '\\') {
            if segment.is_empty() {
                return Err(BitTorrentError::Parse(
                    "Torrent file path contains an empty path segment.".into(),
                ));
            }
            if segment == "." || segment == ".." {
                return Err(BitTorrentError::Parse(
                    "Torrent file path contains invalid relative segments.".into(),
                ));
            }
            
            // Check Windows reserved names case-insensitively
            let mut stem = segment.to_uppercase();
            if let Some(pos) = stem.find('.') {
                stem.truncate(pos);
            }
            match stem.as_str() {
                "CON" | "PRN" | "AUX" | "NUL" => {
                    return Err(BitTorrentError::Parse(format!(
                        "Path contains reserved Windows device name: {}",
                        segment
                    )));
                }
                _ => {}
            }
            if (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 {
                if let Some(digit) = stem.chars().nth(3) {
                    if digit.is_ascii_digit() && digit != '0' {
                        return Err(BitTorrentError::Parse(format!(
                            "Path contains reserved Windows device name: {}",
                            segment
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn parse_file_tree_tokenized(
    tokenizer: &mut BencodeTokenizer<'_>,
    current_path: &mut Vec<String>,
    files: &mut Vec<(String, u64, Vec<u8>)>,
) -> Result<(), BitTorrentError> {
    loop {
        let token = tokenizer.next_token()
            .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
        if token == BencodeToken::End {
            break;
        }
        let key = match token {
            BencodeToken::String(k) => k,
            _ => return Err(BitTorrentError::Bencode(crate::error::BencodeError::Custom("File tree key must be a string".into()))),
        };
        
        let val_token = tokenizer.next_token()
            .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
        if val_token != BencodeToken::DictStart {
            return Err(BitTorrentError::Bencode(crate::error::BencodeError::Custom("File tree value must be a dictionary".into())));
        }
        
        if key.is_empty() {
            // This dictionary contains file properties: length, pieces root, etc.
            let mut length: u64 = 0;
            let mut pieces_root = Vec::new();
            loop {
                let prop_key_tok = tokenizer.next_token()
                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                if prop_key_tok == BencodeToken::End {
                    break;
                }
                let prop_key = match prop_key_tok {
                    BencodeToken::String(k) => k,
                    _ => return Err(BitTorrentError::Bencode(crate::error::BencodeError::Custom("File property key must be a string".into()))),
                };
                let prop_val = tokenizer.next_token()
                    .ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                match prop_key {
                    b"length" => {
                        if let BencodeToken::Integer(i) = prop_val {
                            if let Ok(s) = core::str::from_utf8(i) {
                                if let Ok(l) = s.parse::<u64>() {
                                    length = l;
                                }
                            }
                        }
                    }
                    b"pieces root" => {
                        if let BencodeToken::String(s) = prop_val {
                            pieces_root = s.to_vec();
                        }
                    }
                    _ => {
                        // Skip prop_val
                        let mut depth = 0;
                        match prop_val {
                            BencodeToken::DictStart | BencodeToken::ListStart => {
                                depth += 1;
                                while depth > 0 {
                                    let t = tokenizer.next_token().ok_or_else(|| BitTorrentError::Bencode(crate::error::BencodeError::UnexpectedEnd))??;
                                    if t == BencodeToken::DictStart || t == BencodeToken::ListStart {
                                        depth += 1;
                                    } else if t == BencodeToken::End {
                                        depth -= 1;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            let file_path = current_path.join("/");
            files.push((file_path, length, pieces_root));
        } else {
            // This is a directory/file name segment. Recurse!
            if let Ok(segment) = core::str::from_utf8(key) {
                current_path.push(segment.to_string());
                parse_file_tree_tokenized(tokenizer, current_path, files)?;
                current_path.pop();
            } else {
                return Err(BitTorrentError::Bencode(crate::error::BencodeError::Custom("Invalid UTF-8 directory segment".into())));
            }
        }
    }
    Ok(())
}

