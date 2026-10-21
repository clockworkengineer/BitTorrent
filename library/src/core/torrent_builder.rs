//! BitTorrent Torrent File Generator / Builder (v1, v2, and Hybrid / BEP 52)
//!
//! Provides a programmatic builder to generate standards-compliant `.torrent` files
//! for BitTorrent v1, pure BitTorrent v2 (BEP 52), and dual-stack Hybrid (v1 + v2) torrents.

use crate::bencode::{BNode, Bencode};
use crate::core::merkle::{BLOCK_SIZE, MerkleTree};
use crate::error::BitTorrentError;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sha1::{Digest as Sha1Digest, Sha1};

#[cfg(feature = "std")]
use std::path::Path;

fn leak_bytes(s: String) -> &'static [u8] {
    Box::leak(s.into_boxed_str()).as_bytes()
}

/// Builder for generating BitTorrent metadata `.torrent` files (v1, v2, and Hybrid).
#[derive(Debug, Clone)]
pub struct TorrentBuilder {
    name: String,
    piece_length: usize,
    trackers: Vec<String>,
    web_seeds: Vec<String>,
    comment: Option<String>,
    is_private: bool,
    is_multi_file: bool,
    files: Vec<(String, Vec<u8>)>,
}

impl TorrentBuilder {
    /// Creates a new `TorrentBuilder` with the specified torrent name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            piece_length: 64 * 1024, // 64 KiB default
            trackers: Vec::new(),
            web_seeds: Vec::new(),
            comment: None,
            is_private: false,
            is_multi_file: false,
            files: Vec::new(),
        }
    }

    /// Sets the piece length in bytes.
    ///
    /// For BitTorrent v2 and Hybrid torrents, `piece_length` must be a power of two
    /// and at least 16 KiB (16,384 bytes).
    pub fn piece_length(mut self, piece_length: usize) -> Self {
        self.piece_length = piece_length;
        self
    }

    /// Adds a tracker URL to the announcement list.
    pub fn tracker(mut self, url: impl Into<String>) -> Self {
        self.trackers.push(url.into());
        self
    }

    /// Sets multiple tracker URLs.
    pub fn trackers(mut self, urls: Vec<String>) -> Self {
        self.trackers = urls;
        self
    }

    /// Adds a web seed URL (BEP 19).
    pub fn web_seed(mut self, url: impl Into<String>) -> Self {
        self.web_seeds.push(url.into());
        self
    }

    /// Sets an optional comment for the torrent.
    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Marks the torrent as private (BEP 27).
    pub fn private(mut self, is_private: bool) -> Self {
        self.is_private = is_private;
        self
    }

    /// Adds a file with in-memory byte contents.
    ///
    /// `path` is the relative file path within the torrent (e.g. `"file.txt"` or `"docs/readme.md"`).
    pub fn add_file(mut self, path: impl Into<String>, data: Vec<u8>) -> Self {
        self.files.push((path.into().replace('\\', "/"), data));
        self
    }

    #[cfg(feature = "std")]
    /// Adds a file by reading its contents from the local filesystem.
    pub fn add_file_from_path(
        mut self,
        relative_path: impl Into<String>,
        disk_path: impl AsRef<Path>,
    ) -> Result<Self, std::io::Error> {
        let data = std::fs::read(disk_path.as_ref())?;
        self.files
            .push((relative_path.into().replace('\\', "/"), data));
        Ok(self)
    }

    /// Explicitly sets whether this torrent is formatted as a multi-file directory torrent.
    pub fn multi_file(mut self, is_multi_file: bool) -> Self {
        self.is_multi_file = is_multi_file;
        self
    }

    #[cfg(feature = "std")]
    /// Recursively adds all files from a directory on the local filesystem.
    ///
    /// Files are sorted lexicographically by relative path to ensure deterministic layout.
    pub fn add_dir(mut self, root_dir: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let root = root_dir.as_ref();
        let mut file_paths = Vec::new();
        let mut dir_stack = vec![root.to_path_buf()];

        while let Some(current_dir) = dir_stack.pop() {
            for entry in std::fs::read_dir(&current_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    dir_stack.push(path);
                } else if path.is_file() {
                    file_paths.push(path);
                }
            }
        }

        // Sort files to ensure deterministic layout
        file_paths.sort();

        for path in file_paths {
            let rel_path = path
                .strip_prefix(root)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
                .to_string_lossy()
                .replace('\\', "/");
            let data = std::fs::read(&path)?;
            self = self.add_file(rel_path, data);
        }

        self.is_multi_file = true;
        Ok(self)
    }

    /// Validates piece length constraints for v2/hybrid (must be power of two >= 16 KiB).
    fn validate_piece_length(&self) -> Result<(), BitTorrentError> {
        if self.piece_length < BLOCK_SIZE || (self.piece_length & (self.piece_length - 1)) != 0 {
            return Err(BitTorrentError::Parse(
                "Piece length for BitTorrent v2 must be a power of two and at least 16 KiB".into(),
            ));
        }
        if self.files.is_empty() {
            return Err(BitTorrentError::Parse(
                "Torrent must contain at least one file".into(),
            ));
        }
        Ok(())
    }

    /// Builds the v2 `file tree` Bencode dictionary and piece layers mapping.
    fn build_v2_file_tree_and_layers(
        &self,
    ) -> Result<(FileTreeNode, BTreeMap<[u8; 32], Vec<u8>>), BitTorrentError> {
        let mut root_node = FileTreeNode::Directory(BTreeMap::new());
        let mut piece_layers = BTreeMap::new();

        for (rel_path, data) in &self.files {
            let tree = MerkleTree::from_bytes(data);
            let pieces_root = tree.root();

            if data.len() > self.piece_length {
                let layer_hashes = tree.piece_layer(self.piece_length)?;
                let mut layer_bytes = Vec::with_capacity(layer_hashes.len() * 32);
                for hash in layer_hashes {
                    layer_bytes.extend_from_slice(&hash);
                }
                piece_layers.insert(pieces_root, layer_bytes);
            }

            let parts: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
            if parts.is_empty() {
                continue;
            }

            root_node.insert_file(&parts, data.len() as u64, pieces_root);
        }

        Ok((root_node, piece_layers))
    }

    /// Builds a pure BitTorrent v2 `.torrent` file (BEP 52).
    pub fn build_v2(&self) -> Result<Vec<u8>, BitTorrentError> {
        self.validate_piece_length()?;
        let (file_tree_node, piece_layers_map) = self.build_v2_file_tree_and_layers()?;

        let piece_length_bytes = leak_bytes(self.piece_length.to_string());
        let mut info_entries: Vec<(&[u8], BNode)> = Vec::new();

        let file_tree_bnode = file_tree_node.to_bnode();
        info_entries.push((b"file tree", file_tree_bnode));
        info_entries.push((b"meta version", BNode::Number(b"2")));
        info_entries.push((b"name", BNode::String(self.name.as_bytes())));
        info_entries.push((b"piece length", BNode::Number(piece_length_bytes)));

        if self.is_private {
            info_entries.push((b"private", BNode::Number(b"1")));
        }

        // Sort info dictionary keys lexicographically
        info_entries.sort_by(|a, b| a.0.cmp(b.0));
        let info_dict = BNode::Dictionary(info_entries);

        let mut root_entries: Vec<(&[u8], BNode)> = Vec::new();
        let primary_announce = self
            .trackers
            .first()
            .map(|s| s.as_str())
            .unwrap_or("http://tracker.example.com/announce");
        root_entries.push((b"announce", BNode::String(primary_announce.as_bytes())));

        let tracker_nodes: Vec<BNode>;
        let tier_nodes: Vec<BNode>;
        if self.trackers.len() > 1 {
            tracker_nodes = self
                .trackers
                .iter()
                .map(|t| BNode::List(vec![BNode::String(t.as_bytes())]))
                .collect();
            root_entries.push((b"announce-list", BNode::List(tracker_nodes)));
        }

        if !self.web_seeds.is_empty() {
            tier_nodes = self
                .web_seeds
                .iter()
                .map(|u| BNode::String(u.as_bytes()))
                .collect();
            root_entries.push((b"url-list", BNode::List(tier_nodes)));
        }

        if let Some(ref comment) = self.comment {
            root_entries.push((b"comment", BNode::String(comment.as_bytes())));
        }

        root_entries.push((b"info", info_dict));

        let piece_layers_entries: Vec<(&[u8], BNode)>;
        if !piece_layers_map.is_empty() {
            piece_layers_entries = piece_layers_map
                .iter()
                .map(|(k, v)| (k.as_slice(), BNode::String(v.as_slice())))
                .collect();
            root_entries.push((b"piece layers", BNode::Dictionary(piece_layers_entries)));
        }

        root_entries.sort_by(|a, b| a.0.cmp(b.0));
        let root_dict = BNode::Dictionary(root_entries);
        Ok(Bencode::encode(&root_dict))
    }

    /// Builds a BitTorrent v2 Hybrid `.torrent` file (BEP 52).
    ///
    /// The hybrid format contains both v1 `pieces` (with piece-aligned files and pad files)
    /// and v2 `file tree` + `piece layers`, allowing simultaneous compatibility with
    /// both v1-only and v2-capable swarms.
    pub fn build_hybrid(&self) -> Result<Vec<u8>, BitTorrentError> {
        self.validate_piece_length()?;
        let (file_tree_node, piece_layers_map) = self.build_v2_file_tree_and_layers()?;

        // For hybrid torrents, align files to piece boundaries by inserting pad files
        let mut aligned_stream = Vec::new();
        let mut v1_files_entries: Vec<BNode> = Vec::new();
        let mut pad_counter = 0;

        let is_single_file =
            self.files.len() == 1 && !self.is_multi_file && !self.files[0].0.contains('/');

        for (rel_path, data) in &self.files {
            let file_len_bytes = leak_bytes(data.len().to_string());
            let path_segments: Vec<BNode> = rel_path
                .split('/')
                .filter(|s| !s.is_empty())
                .map(|s| BNode::String(s.as_bytes()))
                .collect();

            let tree = MerkleTree::from_bytes(data);
            let root_slice = Box::leak(Box::new(tree.root())).as_slice();

            let mut file_dict_entries = vec![
                (b"length" as &[u8], BNode::Number(file_len_bytes)),
                (b"path" as &[u8], BNode::List(path_segments)),
                (b"pieces root" as &[u8], BNode::String(root_slice)),
            ];
            file_dict_entries.sort_by(|a, b| a.0.cmp(b.0));
            let file_dict = BNode::Dictionary(file_dict_entries);
            v1_files_entries.push(file_dict);
            aligned_stream.extend_from_slice(data);

            // Compute padding required to reach piece boundary
            let remainder = data.len() % self.piece_length;
            if remainder > 0 && !is_single_file {
                let pad_len = self.piece_length - remainder;
                let pad_len_bytes = leak_bytes(pad_len.to_string());

                let pad_dict = BNode::Dictionary(vec![
                    (b"attr" as &[u8], BNode::String(b"p")),
                    (b"length" as &[u8], BNode::Number(pad_len_bytes)),
                    (
                        b"path" as &[u8],
                        BNode::List(vec![BNode::String(b".pad"), BNode::String(pad_len_bytes)]),
                    ),
                ]);
                v1_files_entries.push(pad_dict);
                aligned_stream.extend(vec![0u8; pad_len]);
                pad_counter += 1;
            }
        }
        let _ = pad_counter;

        // Compute v1 SHA-1 piece hashes
        let mut v1_pieces = Vec::new();
        for chunk in aligned_stream.chunks(self.piece_length) {
            let hash = Sha1::digest(chunk);
            v1_pieces.extend_from_slice(&hash);
        }

        let piece_length_bytes = leak_bytes(self.piece_length.to_string());
        let mut info_entries: Vec<(&[u8], BNode)> = Vec::new();

        let file_tree_bnode = file_tree_node.to_bnode();
        info_entries.push((b"file tree", file_tree_bnode));
        info_entries.push((b"meta version", BNode::Number(b"2")));
        info_entries.push((b"name", BNode::String(self.name.as_bytes())));
        info_entries.push((b"piece length", BNode::Number(piece_length_bytes)));
        info_entries.push((b"pieces", BNode::String(&v1_pieces)));

        if is_single_file {
            let len_bytes = leak_bytes(self.files[0].1.len().to_string());
            info_entries.push((b"length", BNode::Number(len_bytes)));
        } else {
            info_entries.push((b"files", BNode::List(v1_files_entries)));
        }

        if self.is_private {
            info_entries.push((b"private", BNode::Number(b"1")));
        }

        info_entries.sort_by(|a, b| a.0.cmp(b.0));
        let info_dict = BNode::Dictionary(info_entries);

        let mut root_entries: Vec<(&[u8], BNode)> = Vec::new();
        let primary_announce = self
            .trackers
            .first()
            .map(|s| s.as_str())
            .unwrap_or("http://tracker.example.com/announce");
        root_entries.push((b"announce", BNode::String(primary_announce.as_bytes())));

        let tracker_nodes: Vec<BNode>;
        if self.trackers.len() > 1 {
            tracker_nodes = self
                .trackers
                .iter()
                .map(|t| BNode::List(vec![BNode::String(t.as_bytes())]))
                .collect();
            root_entries.push((b"announce-list", BNode::List(tracker_nodes)));
        }

        if let Some(ref comment) = self.comment {
            root_entries.push((b"comment", BNode::String(comment.as_bytes())));
        }

        root_entries.push((b"info", info_dict));

        let piece_layers_entries: Vec<(&[u8], BNode)>;
        if !piece_layers_map.is_empty() {
            piece_layers_entries = piece_layers_map
                .iter()
                .map(|(k, v)| (k.as_slice(), BNode::String(v.as_slice())))
                .collect();
            root_entries.push((b"piece layers", BNode::Dictionary(piece_layers_entries)));
        }

        root_entries.sort_by(|a, b| a.0.cmp(b.0));
        let root_dict = BNode::Dictionary(root_entries);
        Ok(Bencode::encode(&root_dict))
    }

    /// Builds a standard BitTorrent v1 `.torrent` file.
    pub fn build_v1(&self) -> Result<Vec<u8>, BitTorrentError> {
        if self.files.is_empty() {
            return Err(BitTorrentError::Parse(
                "Torrent must contain at least one file".into(),
            ));
        }

        let mut total_stream = Vec::new();
        let is_single_file =
            self.files.len() == 1 && !self.is_multi_file && !self.files[0].0.contains('/');
        let mut v1_files = Vec::new();

        for (rel_path, data) in &self.files {
            total_stream.extend_from_slice(data);
            if !is_single_file {
                let len_bytes = leak_bytes(data.len().to_string());
                let path_segments: Vec<BNode> = rel_path
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .map(|s| BNode::String(s.as_bytes()))
                    .collect();
                v1_files.push(BNode::Dictionary(vec![
                    (b"length" as &[u8], BNode::Number(len_bytes)),
                    (b"path" as &[u8], BNode::List(path_segments)),
                ]));
            }
        }

        let mut pieces = Vec::new();
        for chunk in total_stream.chunks(self.piece_length) {
            let hash = Sha1::digest(chunk);
            pieces.extend_from_slice(&hash);
        }

        let piece_length_bytes = leak_bytes(self.piece_length.to_string());
        let mut info_entries: Vec<(&[u8], BNode)> = Vec::new();
        info_entries.push((b"name", BNode::String(self.name.as_bytes())));
        info_entries.push((b"piece length", BNode::Number(piece_length_bytes)));
        info_entries.push((b"pieces", BNode::String(&pieces)));

        if is_single_file {
            let len_bytes = leak_bytes(self.files[0].1.len().to_string());
            info_entries.push((b"length", BNode::Number(len_bytes)));
        } else {
            info_entries.push((b"files", BNode::List(v1_files)));
        }

        if self.is_private {
            info_entries.push((b"private", BNode::Number(b"1")));
        }

        info_entries.sort_by(|a, b| a.0.cmp(b.0));
        let info_dict = BNode::Dictionary(info_entries);

        let mut root_entries: Vec<(&[u8], BNode)> = Vec::new();
        let primary_announce = self
            .trackers
            .first()
            .map(|s| s.as_str())
            .unwrap_or("http://tracker.example.com/announce");
        root_entries.push((b"announce", BNode::String(primary_announce.as_bytes())));

        if let Some(ref comment) = self.comment {
            root_entries.push((b"comment", BNode::String(comment.as_bytes())));
        }

        root_entries.push((b"info", info_dict));
        root_entries.sort_by(|a, b| a.0.cmp(b.0));
        let root_dict = BNode::Dictionary(root_entries);
        Ok(Bencode::encode(&root_dict))
    }
}

/// Helper node representing a file or directory in the BitTorrent v2 `file tree`.
enum FileTreeNode {
    Directory(BTreeMap<String, FileTreeNode>),
    File { length: u64, pieces_root: [u8; 32] },
}

impl FileTreeNode {
    fn insert_file(&mut self, path_segments: &[&str], length: u64, pieces_root: [u8; 32]) {
        match self {
            FileTreeNode::Directory(entries) => {
                if path_segments.len() == 1 {
                    let file_name = path_segments[0].to_string();
                    entries.insert(
                        file_name,
                        FileTreeNode::File {
                            length,
                            pieces_root,
                        },
                    );
                } else {
                    let dir_name = path_segments[0].to_string();
                    let child_dir = entries
                        .entry(dir_name)
                        .or_insert_with(|| FileTreeNode::Directory(BTreeMap::new()));
                    child_dir.insert_file(&path_segments[1..], length, pieces_root);
                }
            }
            FileTreeNode::File { .. } => unreachable!(),
        }
    }

    fn to_bnode(&self) -> BNode<'static> {
        match self {
            FileTreeNode::Directory(entries) => {
                let mut b_entries = Vec::with_capacity(entries.len());
                for (name, child) in entries {
                    let child_bnode = child.to_bnode();
                    b_entries.push((
                        Box::leak(name.clone().into_boxed_str()).as_bytes(),
                        child_bnode,
                    ));
                }
                b_entries.sort_by(|a, b| a.0.cmp(b.0));
                BNode::Dictionary(b_entries)
            }
            FileTreeNode::File {
                length,
                pieces_root,
            } => {
                let len_str = Box::leak(length.to_string().into_boxed_str());
                let root_slice = Box::leak(Box::new(*pieces_root)).as_slice();

                let file_meta = vec![
                    (
                        b"length" as &'static [u8],
                        BNode::Number(len_str.as_bytes()),
                    ),
                    (b"pieces root" as &'static [u8], BNode::String(root_slice)),
                ];
                let empty_key_dict = vec![(b"" as &'static [u8], BNode::Dictionary(file_meta))];
                BNode::Dictionary(empty_key_dict)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "std")]
    fn test_add_dir_and_build_v1() {
        let temp_dir = std::env::temp_dir().join("torrent_builder_test_dir");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(temp_dir.join("sub")).unwrap();
        std::fs::write(temp_dir.join("a.txt"), b"Hello A").unwrap();
        std::fs::write(temp_dir.join("sub/b.txt"), b"Hello B nested").unwrap();

        let builder = TorrentBuilder::new("test_bundle")
            .piece_length(16 * 1024)
            .add_dir(&temp_dir)
            .unwrap();

        assert_eq!(builder.files.len(), 2);
        assert_eq!(builder.files[0].0, "a.txt");
        assert_eq!(builder.files[1].0, "sub/b.txt");

        let bytes = builder.build_v1().unwrap();
        assert!(!bytes.is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
