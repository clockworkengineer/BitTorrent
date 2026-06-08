use bittorrent_rs::disk_io::DiskIO;
use bittorrent_rs::{BNode, Bencode, MetaInfoFile, Selector, TorrentContext};
use sha1::Digest;
use std::fs;
use std::path::{Path, PathBuf};

fn unique_test_path(name: &str) -> PathBuf {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("bittorrent_rs_{}_{}", name, millis))
}

fn cleanup_path(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

fn write_single_file_torrent(
    torrent_path: &Path,
    announce_url: &str,
    file_name: &str,
    file_length: u64,
    piece_length: u32,
    pieces: Vec<u8>,
) {
    let file_length_str = file_length.to_string();
    let piece_length_str = piece_length.to_string();
    let file_length_bytes = file_length_str.as_bytes();
    let piece_length_bytes = piece_length_str.as_bytes();
    let file_name_bytes = file_name.as_bytes();
    let announce_url_bytes = announce_url.as_bytes();
    let pieces_ref = &pieces;

    let info = BNode::Dictionary(vec![
        (
            b"length" as &[u8],
            BNode::Number(file_length_bytes),
        ),
        (
            b"name" as &[u8],
            BNode::String(file_name_bytes),
        ),
        (
            b"piece length" as &[u8],
            BNode::Number(piece_length_bytes),
        ),
        (b"pieces" as &[u8], BNode::String(pieces_ref)),
    ]);
    let root = BNode::Dictionary(vec![
        (
            b"announce" as &[u8],
            BNode::String(announce_url_bytes),
        ),
        (b"info" as &[u8], info),
    ]);
    fs::write(torrent_path, Bencode::encode(&root)).unwrap();
}

#[test]
fn test_process_piece_block_assembly_writes_complete_piece() {
    let download_path = unique_test_path("piece_block_assembly");
    cleanup_path(&download_path);
    fs::create_dir_all(&download_path).unwrap();

    let torrent_file = download_path.join("single_file.torrent");
    let piece_length: u32 = 16384;
    let file_length: u64 = 20000;
    let file_name = "data.bin";
    let file_data: Vec<u8> = vec![42u8; file_length as usize];
    let pieces: Vec<u8> = file_data
        .chunks(piece_length as usize)
        .flat_map(|chunk| sha1::Sha1::digest(chunk).to_vec())
        .collect();

    write_single_file_torrent(
        &torrent_file,
        "http://tracker.example.com/announce",
        file_name,
        file_length,
        piece_length,
        pieces,
    );

    let disk_io = DiskIO::new();
    let mut meta_info = MetaInfoFile::new(&torrent_file).unwrap();
    meta_info.parse().unwrap();
    meta_info.validate().unwrap();
    let mut context =
        TorrentContext::new(&meta_info, Selector::new(), &disk_io, &download_path, false).unwrap();

    let second_piece_block = &file_data[piece_length as usize..];

    assert!(
        context
            .process_piece_block(&disk_io, 1, 0, second_piece_block)
            .unwrap()
    );
    assert!(context.is_piece_local(1));

    let downloaded = fs::read(download_path.join(file_name)).unwrap();
    assert_eq!(&downloaded[piece_length as usize..], second_piece_block);

    cleanup_path(&download_path);
}
