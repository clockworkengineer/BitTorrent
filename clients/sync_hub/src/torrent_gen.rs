use std::fs;
use std::path::{Path, PathBuf};
use sha1::{Digest, Sha1};
use bittorrent_rs::bencode::{BNode, Bencode};

pub fn generate_torrent_bytes(
    sync_dir: &Path,
    name: &str,
    piece_length: usize,
) -> Result<Vec<u8>, std::io::Error> {
    // 1. Gather all files recursively
    let mut files = Vec::new();
    fn visit_dirs(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    visit_dirs(&path, files)?;
                } else {
                    files.push(path);
                }
            }
        }
        Ok(())
    }
    visit_dirs(sync_dir, &mut files)?;
    // Sort files to ensure deterministic layout
    files.sort();

    // 2. Compute pieces hashes
    let mut piece_hashes = Vec::new();
    let mut current_piece_buf = Vec::with_capacity(piece_length);
    let mut file_infos = Vec::new();

    for file_path in &files {
        let metadata = fs::metadata(file_path)?;
        let length = metadata.len();
        
        // Relative path segments
        let rel_path = file_path.strip_prefix(sync_dir).unwrap();
        let path_segments: Vec<String> = rel_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();

        file_infos.push((length, path_segments));

        // Read file contents to hash pieces
        let mut file = fs::File::open(file_path)?;
        use std::io::Read;
        let mut chunk = vec![0u8; 16384];
        loop {
            let bytes_read = file.read(&mut chunk)?;
            if bytes_read == 0 {
                break;
            }
            current_piece_buf.extend_from_slice(&chunk[..bytes_read]);
            while current_piece_buf.len() >= piece_length {
                let mut hasher = Sha1::new();
                hasher.update(&current_piece_buf[..piece_length]);
                piece_hashes.extend_from_slice(&hasher.finalize());
                current_piece_buf.drain(..piece_length);
            }
        }
    }

    // Hash the trailing partial piece if any
    if !current_piece_buf.is_empty() {
        let mut hasher = Sha1::new();
        hasher.update(&current_piece_buf);
        piece_hashes.extend_from_slice(&hasher.finalize());
    }

    // 3. Build BNode structure
    // Info dictionary entries
    let mut info_entries = Vec::new();
    info_entries.push((b"name".as_slice(), BNode::String(name.as_bytes())));
    
    let piece_len_str = piece_length.to_string();
    info_entries.push((b"piece length".as_slice(), BNode::Number(piece_len_str.as_bytes())));
    info_entries.push((b"pieces".as_slice(), BNode::String(&piece_hashes)));
    info_entries.push((b"private".as_slice(), BNode::Number(b"1")));

    let mut bencode_files = Vec::new();
    // Keep reference-holding structures alive while constructing BNode
    let mut path_lists = Vec::new();
    let mut lengths_str = Vec::new();

    for (len, _) in &file_infos {
        let len_str = len.to_string();
        lengths_str.push(len_str);
    }

    for (i, (_, path_segs)) in file_infos.iter().enumerate() {
        let mut file_dict = Vec::new();
        file_dict.push((b"length".as_slice(), BNode::Number(lengths_str[i].as_bytes())));
        
        let segs_node: Vec<BNode> = path_segs
            .iter()
            .map(|s| BNode::String(s.as_bytes()))
            .collect();
        path_lists.push(segs_node);
    }

    for i in 0..file_infos.len() {
        let mut file_dict = Vec::new();
        file_dict.push((b"length".as_slice(), BNode::Number(lengths_str[i].as_bytes())));
        file_dict.push((b"path".as_slice(), BNode::List(path_lists[i].clone())));
        bencode_files.push(BNode::Dictionary(file_dict));
    }

    info_entries.push((b"files".as_slice(), BNode::List(bencode_files)));

    let info_node = BNode::Dictionary(info_entries);

    // Root dictionary
    let mut root_entries = Vec::new();
    root_entries.push((b"announce".as_slice(), BNode::String(b"http://127.0.0.1:6881/announce")));
    root_entries.push((b"info".as_slice(), info_node));

    let root_node = BNode::Dictionary(root_entries);
    Ok(Bencode::encode(&root_node))
}
