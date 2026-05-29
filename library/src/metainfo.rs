use crate::bencode::{BNode, Bencode};
use crate::error::BitTorrentError;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FileDetails {
    pub name: String,
    pub length: u64,
    pub md5sum: Option<String>,
    pub offset: u64,
}

#[derive(Debug)]
pub struct MetaInfoFile {
    pub torrent_file_name: PathBuf,
    meta_info_data: Vec<u8>,
    meta_info_dict: HashMap<String, Vec<u8>>,
    parsed: bool,
}

impl MetaInfoFile {
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

    fn load(&mut self) -> Result<(), BitTorrentError> {
        self.meta_info_data = fs::read(&self.torrent_file_name)?;
        Ok(())
    }

    pub fn parse(&mut self) -> Result<(), BitTorrentError> {
        let root = Bencode::decode(&self.meta_info_data)?;
        if !matches!(root, BNode::Dictionary(_)) {
            return Err(BitTorrentError::InvalidBencode(
                "Torrent file root is not a dictionary".into(),
            ));
        }

        self.require_string_or_numeric(&root, "announce")?;
        self.get_list_of_strings(&root, "announce-list")?;
        self.get_string_or_numeric(&root, "comment")?;
        self.get_string_or_numeric(&root, "created by")?;
        self.get_string_or_numeric(&root, "creation date")?;
        self.require_string_or_numeric(&root, "name")?;
        self.require_string_or_numeric(&root, "piece length")?;
        self.require_string_or_numeric(&root, "pieces")?;
        self.get_string_or_numeric(&root, "private")?;
        self.get_string_or_numeric(&root, "url-list")?;

        if Bencode::get_dictionary_entry(&root, b"files").is_none() {
            self.require_string_or_numeric(&root, "length")?;
            self.get_string_or_numeric(&root, "md5sum")?;
        } else {
            self.get_list_of_dictionarys(&root, "files")?;
        }

        self.calculate_info_hash(&root)?;
        self.parsed = true;
        Ok(())
    }

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

        let pieces = self.get_pieces_info_hash()?;
        if pieces.is_empty() || pieces.len() % crate::constants::HASH_LENGTH != 0 {
            return Err(BitTorrentError::Parse(
                "Invalid torrent pieces hash length.".into(),
            ));
        }

        let tracker = self.get_tracker()?;
        url::Url::parse(&tracker)
            .map_err(|err| BitTorrentError::Parse(err.to_string()))?;

        let files = self.local_files_to_download_list(Path::new("."))?.1;
        if files.is_empty() {
            return Err(BitTorrentError::Parse(
                "Torrent contains no file entries.".into(),
            ));
        }

        Ok(())
    }

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
            let directory = download_path.join(root_name.as_ref());
            let mut file_no = 0;
            loop {
                let key = file_no.to_string();
                let entry = if let Some(entry) = self.meta_info_dict.get(&key) {
                    String::from_utf8_lossy(entry).to_string()
                } else {
                    break;
                };
                let parts: Vec<&str> = entry.split(',').collect();
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

    fn get_string_or_numeric(&mut self, root: &BNode, field: &str) -> Result<(), BitTorrentError> {
        if let Some(entry) = Bencode::get_dictionary_entry(root, field.as_bytes()) {
            if let Some(bytes) = entry.as_string() {
                self.meta_info_dict
                    .insert(field.to_string(), bytes.to_vec());
            } else if let Some(bytes) = entry.as_number_bytes() {
                self.meta_info_dict
                    .insert(field.to_string(), bytes.to_vec());
            }
        }
        Ok(())
    }

    fn require_string_or_numeric(&mut self, root: &BNode, field: &str) -> Result<(), BitTorrentError> {
        if Bencode::get_dictionary_entry(root, field.as_bytes()).is_none() {
            return Err(BitTorrentError::MissingField(field.to_string()));
        }
        self.get_string_or_numeric(root, field)
    }

    fn get_list_of_strings(&mut self, root: &BNode, field: &str) -> Result<(), BitTorrentError> {
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
                self.meta_info_dict
                    .insert(field.to_string(), values.join(",").into_bytes());
            }
        }
        Ok(())
    }

    fn get_list_of_dictionarys(
        &mut self,
        root: &BNode,
        field: &str,
    ) -> Result<(), BitTorrentError> {
        if let Some(entry) = Bencode::get_dictionary_entry(root, field.as_bytes()) {
            if let BNode::List(list) = entry {
                let mut file_no = 0;
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
                                        .map(|seg| format!("{}{}", std::path::MAIN_SEPARATOR, seg))
                                        .collect::<String>(),
                                );
                            }
                        }
                        file_entry.push(',');
                        file_entry.push_str(
                            &Bencode::get_dictionary_entry_string(item, "length")
                                .unwrap_or_default(),
                        );
                        file_entry.push(',');
                        file_entry.push_str(
                            &Bencode::get_dictionary_entry_string(item, "md5sum")
                                .unwrap_or_default(),
                        );

                        self.meta_info_dict
                            .insert(file_no.to_string(), file_entry.into_bytes());
                        file_no += 1;
                    }
                }
            }
        }
        Ok(())
    }

    fn calculate_info_hash(&mut self, root: &BNode) -> Result<(), BitTorrentError> {
        let info = Bencode::get_dictionary_entry(root, b"info")
            .ok_or_else(|| BitTorrentError::MissingField("info".into()))?;
        let encoded = Bencode::encode(info);
        let mut hasher = Sha1::new();
        hasher.update(&encoded);
        let digest = hasher.finalize();
        self.meta_info_dict
            .insert("info hash".to_string(), digest.to_vec());
        Ok(())
    }

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
