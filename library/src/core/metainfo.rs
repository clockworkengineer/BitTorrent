//! Torrent metainfo parser
//!
//! Handles parsing of `.torrent` files, extracting tracker URLs, files to download,
//! piece size information, and computing the info hash of the torrent file's `info` section.

use crate::bencode::{BNode, Bencode};
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

/// Detailed information about a single file in a multi-file torrent or the single file itself.
#[derive(Debug, Clone)]
pub struct FileDetails {
    pub name: String,
    pub length: u64,
    pub md5sum: Option<String>,
    pub offset: u64,
}

/// Represents a parsed `.torrent` metadata file.
#[derive(Debug)]
pub struct MetaInfoFile {
    #[cfg(feature = "std")]
    pub torrent_file_name: PathBuf,
    meta_info_data: Vec<u8>,
    meta_info_dict: MetaInfoDict,
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
        let root = Bencode::decode(&self.meta_info_data)?;
        if !matches!(root, BNode::Dictionary(_)) {
            return Err(BitTorrentError::Bencode(
                crate::error::BencodeError::Custom("Torrent file root is not a dictionary".into()),
            ));
        }

        Self::require_string_or_numeric(&mut self.meta_info_dict, &root, "announce")?;
        Self::get_list_of_strings(&mut self.meta_info_dict, &root, "announce-list")?;
        Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "comment")?;
        Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "created by")?;
        Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "creation date")?;
        Self::require_string_or_numeric(&mut self.meta_info_dict, &root, "name")?;
        Self::require_string_or_numeric(&mut self.meta_info_dict, &root, "piece length")?;
        Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "meta version")?;
        Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "private")?;

        let is_v2 = if let Some(ver) = self.meta_info_dict.get("meta version") {
            String::from_utf8_lossy(ver).trim() == "2"
        } else {
            false
        };

        if is_v2 {
            Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "pieces")?;
        } else {
            Self::require_string_or_numeric(&mut self.meta_info_dict, &root, "pieces")?;
        }

        if let Some(entry) = Bencode::get_dictionary_entry(&root, b"url-list") {
            match entry {
                BNode::String(_) => {
                    Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "url-list")?;
                }
                BNode::List(_) => {
                    Self::get_list_of_strings(&mut self.meta_info_dict, &root, "url-list")?;
                }
                _ => {}
            }
        }

        if is_v2 {
            if let Some(file_tree) = Bencode::get_dictionary_entry(&root, b"file tree") {
                let mut current_path = Vec::new();
                let mut files = Vec::new();
                traverse_file_tree(file_tree, &mut current_path, &mut files);
                for (i, (path, length, pieces_root)) in files.into_iter().enumerate() {
                    let file_entry = format!("{}{}\0{}", MAIN_SEPARATOR, path.replace("/", &MAIN_SEPARATOR.to_string()), length);
                    self.meta_info_dict.insert(format!("pieces_root_{}", i), pieces_root);
                    self.meta_info_dict.insert(i.to_string(), file_entry.into_bytes());
                }
            }
        } else {
            if Bencode::get_dictionary_entry(&root, b"files").is_none() {
                Self::require_string_or_numeric(&mut self.meta_info_dict, &root, "length")?;
                Self::get_string_or_numeric(&mut self.meta_info_dict, &root, "md5sum")?;
            } else {
                Self::get_list_of_dictionarys(&mut self.meta_info_dict, &root, "files")?;
            }
        }

        Self::calculate_info_hash(&mut self.meta_info_dict, &root)?;
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

    /// Returns the SHA-1 info hash of the `info` dictionary of the torrent.
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
            let file_path = download_path.join(name.as_ref());
            files_to_download.push(FileDetails {
                name: file_path.to_string_lossy().into_owned(),
                length,
                md5sum: self
                    .meta_info_dict
                    .get("md5sum")
                    .map(|bytes| String::from_utf8_lossy(bytes).to_string()),
                offset: 0,
            });
            total_bytes = length;
        } else {
            let name_bytes = self
                .meta_info_dict
                .get("name")
                .ok_or_else(|| BitTorrentError::MissingField("name".into()))?;
            let root_name = String::from_utf8_lossy(name_bytes);
            Self::validate_relative_path(&root_name)?;
            let directory = download_path.join(root_name.as_ref());
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
                let length = length_value.parse::<u64>()?;
                files_to_download.push(FileDetails {
                    name: file_path.to_string_lossy().into_owned(),
                    length,
                    md5sum: if md5sum_value.is_empty() {
                        None
                    } else {
                        Some(md5sum_value.to_string())
                    },
                    offset: total_bytes,
                });
                total_bytes += length;
                file_no += 1;
            }
        }

        Ok((total_bytes, files_to_download))
    }

    /// Extracts a string or numeric value from a `BNode` and caches it in `meta_info_dict`.
    fn get_string_or_numeric(
        meta_info_dict: &mut MetaInfoDict,
        root: &BNode<'_>,
        field: &str,
    ) -> Result<(), BitTorrentError> {
        if let Some(entry) = Bencode::get_dictionary_entry(root, field.as_bytes()) {
            if let Some(bytes) = entry.as_string() {
                meta_info_dict.insert(field.to_string(), bytes.to_vec());
            } else if let Some(bytes) = entry.as_number_bytes() {
                meta_info_dict.insert(field.to_string(), bytes.to_vec());
            }
        }
        Ok(())
    }

    /// Asserts that a field exists and extracts it as string or numeric, returning an error otherwise.
    fn require_string_or_numeric(
        meta_info_dict: &mut MetaInfoDict,
        root: &BNode<'_>,
        field: &str,
    ) -> Result<(), BitTorrentError> {
        if Bencode::get_dictionary_entry(root, field.as_bytes()).is_none() {
            return Err(BitTorrentError::MissingField(field.to_string()));
        }
        Self::get_string_or_numeric(meta_info_dict, root, field)
    }

    /// Extracts a list of strings from a `BNode` and saves them in `meta_info_dict` as a comma-separated string.
    fn get_list_of_strings(
        meta_info_dict: &mut MetaInfoDict,
        root: &BNode<'_>,
        field: &str,
    ) -> Result<(), BitTorrentError> {
        if let Some(entry) = Bencode::get_dictionary_entry(root, field.as_bytes()) {
            if let BNode::List(list) = entry {
                let mut values = Vec::new();
                for item in list {
                    match item {
                        BNode::String(bytes) => {
                            values.push(String::from_utf8_lossy(bytes).to_string());
                        }
                        BNode::List(inner) => {
                            if let Some(BNode::String(bytes)) = inner.get(0) {
                                values.push(String::from_utf8_lossy(bytes).to_string());
                            }
                        }
                        _ => {}
                    }
                }
                meta_info_dict.insert(field.to_string(), values.join(",").into_bytes());
            }
        }
        Ok(())
    }

    /// Extracts file dict arrays under the "files" list, caching them in `meta_info_dict` as numbered entries.
    fn get_list_of_dictionarys(
        meta_info_dict: &mut MetaInfoDict,
        root: &BNode<'_>,
        field: &str,
    ) -> Result<(), BitTorrentError> {
        if let Some(entry) = Bencode::get_dictionary_entry(root, field.as_bytes()) {
            if let BNode::List(list) = entry {
                let mut file_no: i32 = 0;
                for item in list {
                    if let BNode::Dictionary(_) = item {
                        let mut file_entry = String::new();
                        if let Some(path_node) = Bencode::get_dictionary_entry(item, b"path") {
                            if let BNode::List(path_list) = path_node {
                                let mut path_segments = Vec::new();
                                for segment in path_list {
                                    if let BNode::String(bytes) = segment {
                                        path_segments
                                            .push(String::from_utf8_lossy(bytes).to_string());
                                    }
                                }
                                file_entry.push_str(
                                    &path_segments
                                        .iter()
                                        .map(|seg| format!("{}{}", MAIN_SEPARATOR, seg))
                                        .collect::<String>(),
                                );
                            }
                        }
                        file_entry.push('\0');
                        file_entry.push_str(
                            &Bencode::get_dictionary_entry_string(item, "length")
                                .unwrap_or_default(),
                        );
                        file_entry.push('\0');
                        file_entry.push_str(
                            &Bencode::get_dictionary_entry_string(item, "md5sum")
                                .unwrap_or_default(),
                        );

                        meta_info_dict.insert(file_no.to_string(), file_entry.into_bytes());
                        file_no += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// Computes the SHA-1 info hash of the `info` sub-dictionary.
    fn calculate_info_hash(
        meta_info_dict: &mut MetaInfoDict,
        root: &BNode<'_>,
    ) -> Result<(), BitTorrentError> {
        let info = Bencode::get_dictionary_entry(root, b"info")
            .ok_or_else(|| BitTorrentError::MissingField("info".into()))?;
        let encoded = Bencode::encode(info);
        
        let is_v2 = if let Some(ver) = meta_info_dict.get("meta version") {
            String::from_utf8_lossy(ver).trim() == "2"
        } else {
            false
        };
        
        if is_v2 {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(&encoded);
            let digest = hasher.finalize();
            meta_info_dict.insert("info hash".to_string(), digest.to_vec());
        } else {
            let mut hasher = Sha1::new();
            hasher.update(&encoded);
            let digest = hasher.finalize();
            meta_info_dict.insert("info hash".to_string(), digest.to_vec());
        }
        Ok(())
    }

    /// Validates that a parsed relative path is secure and contains no directory traversal elements.
    #[cfg(feature = "std")]
    fn validate_relative_path(path: &str) -> Result<(), BitTorrentError> {
        if path.is_empty() {
            return Err(BitTorrentError::Parse(
                "Torrent file path cannot be empty.".into(),
            ));
        }
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(BitTorrentError::Parse(
                "Torrent file paths must be relative.".into(),
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
        }
        Ok(())
    }
}

fn traverse_file_tree(
    node: &BNode<'_>,
    current_path: &mut Vec<String>,
    files: &mut Vec<(String, u64, Vec<u8>)>,
) {
    if let BNode::Dictionary(entries) = node {
        if let Some(leaf_props) = entries.iter().find(|(k, _)| k.is_empty()).map(|(_, v)| v) {
            let length = leaf_props.dict_get(b"length")
                .and_then(|n| n.as_number_bytes())
                .and_then(|b| core::str::from_utf8(b).ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let pieces_root = leaf_props.dict_get(b"pieces root")
                .and_then(|s| s.as_string())
                .unwrap_or(&[])
                .to_vec();
            let file_path = current_path.join("/");
            files.push((file_path, length, pieces_root));
        } else {
            for (key, val) in entries {
                if let Ok(segment) = core::str::from_utf8(key) {
                    current_path.push(segment.to_string());
                    traverse_file_tree(val, current_path, files);
                    current_path.pop();
                }
            }
        }
    }
}
