use bittorrent_rs::bencode::{BNode, Bencode};
use bittorrent_rs::disk_io::DiskIO;
use bittorrent_rs::{MetaInfoFile, Selector, TorrentContext};
use sha1::Digest;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_test_path(name: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
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

fn write_multi_file_torrent(
    torrent_path: &Path,
    announce_url: &str,
    root_name: &str,
    files: Vec<(&str, u64)>,
    piece_length: u32,
    pieces: Vec<u8>,
) {
    let piece_length_str = piece_length.to_string();
    let piece_length_bytes = piece_length_str.as_bytes();
    let root_name_bytes = root_name.as_bytes();
    let announce_url_bytes = announce_url.as_bytes();
    let pieces_ref = &pieces;

    let files_owned: Vec<(Vec<u8>, Vec<u8>)> = files
        .iter()
        .map(|(path, length)| (path.as_bytes().to_vec(), length.to_string().into_bytes()))
        .collect();

    let file_entries: Vec<BNode<'_>> = files_owned
        .iter()
        .map(|(path_bytes, length_bytes)| {
            let path_list = vec![BNode::String(path_bytes.as_slice())];
            BNode::Dictionary(vec![
                (b"length" as &[u8], BNode::String(length_bytes.as_slice())),
                (b"path" as &[u8], BNode::List(path_list)),
            ])
        })
        .collect();

    let info = BNode::Dictionary(vec![
        (b"files" as &[u8], BNode::List(file_entries)),
        (
            b"name" as &[u8],
            BNode::String(root_name_bytes),
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
fn test_write_piece_across_multiple_files() {
    let download_path = unique_test_path("multi_file_write");
    cleanup_path(&download_path);
    fs::create_dir_all(&download_path).unwrap();

    let torrent_file = download_path.join("multi_file.torrent");
    let piece_data: Vec<u8> = (0..20).collect();
    let piece_hash = sha1::Sha1::digest(&piece_data).to_vec();
    write_multi_file_torrent(
        &torrent_file,
        "http://tracker.example.com/announce",
        "downloads",
        vec![("part1.bin", 10), ("part2.bin", 10)],
        20,
        piece_hash.clone(),
    );

    let mut meta_info = MetaInfoFile::new(&torrent_file).unwrap();
    meta_info.parse().unwrap();
    meta_info.validate().unwrap();
    let piece_length = meta_info.get_piece_length().unwrap();
    let (_, files_to_download) = meta_info.local_files_to_download_list(&download_path).unwrap();
    let disk_io = std::sync::Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().unwrap();

    let mut _context =
        TorrentContext::new(&meta_info, Selector::new(), disk_io.clone(), &download_path, false).unwrap();
    disk_io.create_torrent_bitfield(&mut _context).unwrap();
    let file_one = download_path.join("downloads").join("part1.bin");
    let file_two = download_path.join("downloads").join("part2.bin");

    let piece_bytes = piece_data;
    use bittorrent_rs::BlockStorage;
    disk_io.write_block(0, &piece_bytes).unwrap();

    let mut first = Vec::new();
    fs::File::open(&file_one)
        .unwrap()
        .read_to_end(&mut first)
        .unwrap();
    let mut second = Vec::new();
    fs::File::open(&file_two)
        .unwrap()
        .read_to_end(&mut second)
        .unwrap();

    assert_eq!(first, (0..10).collect::<Vec<u8>>());
    assert_eq!(second, (10..20).collect::<Vec<u8>>());
    cleanup_path(&download_path);
}

#[test]
fn test_process_piece_block_writes_complete_piece_to_disk() {
    let download_path = unique_test_path("piece_block_write");
    cleanup_path(&download_path);
    fs::create_dir_all(&download_path).unwrap();

    let torrent_file = download_path.join("single_file.torrent");
    let piece_data = vec![42u8; 16384];
    let piece_hash = sha1::Sha1::digest(&piece_data).to_vec();
    write_single_file_torrent(
        &torrent_file,
        "http://tracker.example.com/announce",
        "data.bin",
        16384,
        16384,
        piece_hash.clone(),
    );

    let mut meta_info = MetaInfoFile::new(&torrent_file).unwrap();
    meta_info.parse().unwrap();
    meta_info.validate().unwrap();
    let piece_length = meta_info.get_piece_length().unwrap();
    let (_, files_to_download) = meta_info.local_files_to_download_list(&download_path).unwrap();
    let disk_io = std::sync::Arc::new(DiskIO::new(
        &download_path,
        files_to_download,
        piece_length,
    ));
    disk_io.create_local_torrent_structure().unwrap();
    let mut context =
        TorrentContext::new(&meta_info, Selector::new(), disk_io.clone(), &download_path, false).unwrap();
    disk_io.create_torrent_bitfield(&mut context).unwrap();

    let completed = context
        .process_piece_block(&*disk_io, 0, 0, &piece_data)
        .unwrap();

    assert!(completed);
    assert!(context.is_piece_local(0));
    assert_eq!(context.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed), 16384);

    let file_path = download_path.join("data.bin");
    let mut file_contents = Vec::new();
    fs::File::open(&file_path)
        .unwrap()
        .read_to_end(&mut file_contents)
        .unwrap();
    assert_eq!(file_contents, piece_data);

    cleanup_path(&download_path);
}
