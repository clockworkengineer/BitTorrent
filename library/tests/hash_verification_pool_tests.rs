use bittorrent_rs::{
    internals::TorrentContext,
    selector::RarestFirstSelector,
    storage::disk_io::DiskIO,
    storage::verification_pool::{
        HashVerificationPool, PieceVerificationJob,
    },
    BNode, Bencode, MetaInfoFile, MockSocket, Peer, SessionConfig, Socket, TorrentSessionBuilder,
};
use sha1::Digest;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

struct MockBlockStorage {
    data: Mutex<Vec<u8>>,
}

impl MockBlockStorage {
    fn new(size: usize, fill_byte: u8) -> Self {
        Self {
            data: Mutex::new(vec![fill_byte; size]),
        }
    }
}

impl bittorrent_rs::io_traits::BlockReader for MockBlockStorage {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, bittorrent_rs::BitTorrentError> {
        let data = self.data.lock().unwrap();
        let start = offset as usize;
        if start >= data.len() {
            return Ok(0);
        }
        let end = (start + buffer.len()).min(data.len());
        let to_copy = end - start;
        buffer[..to_copy].copy_from_slice(&data[start..end]);
        Ok(to_copy)
    }
}

impl bittorrent_rs::io_traits::BlockWriter for MockBlockStorage {
    fn write_block(&self, offset: u64, data_slice: &[u8]) -> Result<(), bittorrent_rs::BitTorrentError> {
        let mut data = self.data.lock().unwrap();
        let start = offset as usize;
        let end = start + data_slice.len();
        if end > data.len() {
            data.resize(end, 0);
        }
        data[start..end].copy_from_slice(data_slice);
        Ok(())
    }
}

fn unique_test_path(name: &str) -> PathBuf {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    std::env::temp_dir().join(format!("bt_test_verify_{}_{}", name, millis))
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
        (b"length" as &[u8], BNode::Number(file_length_bytes)),
        (b"name" as &[u8], BNode::String(file_name_bytes)),
        (b"piece length" as &[u8], BNode::Number(piece_length_bytes)),
        (b"pieces" as &[u8], BNode::String(pieces_ref)),
    ]);
    let root = BNode::Dictionary(vec![
        (b"announce" as &[u8], BNode::String(announce_url_bytes)),
        (b"info" as &[u8], info),
    ]);
    fs::write(torrent_path, Bencode::encode(&root)).unwrap();
}

#[test]
fn test_hash_verification_pool_multi_worker_concurrency() {
    let pool = HashVerificationPool::new(4, 128);
    let piece_len = 32768;
    let num_pieces = 16;
    let storage = Arc::new(MockBlockStorage::new(piece_len * num_pieces, 0x42));

    let mut expected = Vec::new();
    for p in 0..num_pieces {
        let offset = p * piece_len;
        let slice = &storage.data.lock().unwrap()[offset..offset + piece_len];
        let hash = sha1::Sha1::digest(slice);
        let mut h = [0u8; 20];
        h.copy_from_slice(&hash);
        expected.push(h);

        let job = PieceVerificationJob {
            piece_number: p as u32,
            piece_length: piece_len as u32,
            piece_offset: offset as u64,
            expected_v1_hash: Some(h),
            expected_v2_root: None,
            storage: Some(storage.clone()),
            direct_buffer: None,
            block_sources: vec![Some("192.168.1.100".into())],
        };
        pool.submit(job).expect("Submit piece verification job");
    }

    let mut completed = 0;
    while completed < num_pieces {
        let results = pool.poll_results();
        for res in results {
            assert!(res.is_valid);
            assert_eq!(res.expected_v1_hash, Some(expected[res.piece_number as usize]));
            completed += 1;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let metrics = pool.metrics();
    assert_eq!(metrics.jobs_submitted, num_pieces as u64);
    assert_eq!(metrics.jobs_completed, num_pieces as u64);
    assert_eq!(metrics.valid_pieces, num_pieces as u64);
    assert_eq!(metrics.corrupted_pieces, 0);
    assert_eq!(metrics.total_bytes_verified, (num_pieces * piece_len) as u64);
    assert!(metrics.throughput_mb_s() >= 0.0);
}

#[test]
fn test_hash_verification_pool_corrupt_detection_and_blame() {
    let pool = HashVerificationPool::new(2, 32);
    let piece_len = 16384;
    let storage = Arc::new(MockBlockStorage::new(piece_len, 0x99));

    let bogus_hash = [0x55; 20];
    let bad_peer = "203.0.113.42";

    let job = PieceVerificationJob {
        piece_number: 3,
        piece_length: piece_len as u32,
        piece_offset: 0,
        expected_v1_hash: Some(bogus_hash),
        expected_v2_root: None,
        storage: Some(storage.clone()),
        direct_buffer: None,
        block_sources: vec![Some(bad_peer.into())],
    };

    pool.submit(job).expect("Submit job");

    let result = pool
        .recv_result_timeout(Duration::from_secs(1))
        .expect("Receive result");
    assert!(!result.is_valid);
    assert_eq!(result.expected_v1_hash, Some(bogus_hash));
    assert_ne!(result.actual_v1_hash, Some(bogus_hash));
    assert_eq!(result.block_sources, vec![Some(bad_peer.into())]);

    let metrics = pool.metrics();
    assert_eq!(metrics.corrupted_pieces, 1);
    assert_eq!(metrics.valid_pieces, 0);
}

#[test]
fn test_torrent_context_async_verification_pipeline_and_selection_guard() {
    let test_dir = unique_test_path("context_pipeline");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir_all(&test_dir).unwrap();

    let torrent_file = test_dir.join("test.torrent");
    let piece_length: u32 = 16384;
    let file_length: u64 = 32768;
    let file_name = "test.bin";
    let file_data: Vec<u8> = vec![0x88; file_length as usize];
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

    let mut meta_info = MetaInfoFile::new(&torrent_file).unwrap();
    meta_info.parse().unwrap();
    let (_, files) = meta_info.local_files_to_download_list(&test_dir).unwrap();
    let disk_io = Arc::new(DiskIO::new(&test_dir, files, piece_length));
    disk_io.create_local_torrent_structure().unwrap();

    let config = SessionConfig::default()
        .with_async_verification(true)
        .with_verification_workers(4);

    let mut ctx = TorrentContext::new(
        &meta_info,
        Arc::new(RarestFirstSelector),
        disk_io.clone(),
        &test_dir,
        false,
        config,
    ).unwrap();

    let piece_number = 0;
    assert!(!ctx.is_piece_verifying(piece_number));
    assert!(!ctx.is_piece_local(piece_number));

    // Submit complete block for piece 0
    let block = &file_data[0..piece_length as usize];
    let complete = ctx
        .process_piece_block(&*disk_io, piece_number, 0, block, "127.0.0.1")
        .expect("Process piece block");
    assert!(complete);

    // Piece is now verifying in background
    assert!(ctx.is_piece_verifying(piece_number));

    // Peer selection guard: Mock peer has all pieces, selector should NOT pick piece 0 while verifying
    let (socket, _in_tx, _out_rx) = MockSocket::new();
    let socket = Arc::new(Socket::Mock(socket));
    let mut peer = Peer::new_with_socket("127.0.0.1".to_string(), 6881, socket);
    peer.remote_piece_bitfield = vec![0xFF]; // Has piece 0 and 1

    let selected = ctx.selector.select_piece(&ctx, &peer);
    assert_ne!(
        selected,
        Some(piece_number),
        "Selector must not pick piece currently undergoing verification"
    );

    // Wait and drain verification result
    thread::sleep(Duration::from_millis(100));
    let drained = ctx.drain_verification_results(&*disk_io);
    assert_eq!(drained, 1);

    // Piece is now local and verified
    assert!(!ctx.is_piece_verifying(piece_number));
    assert!(ctx.is_piece_local(piece_number));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_torrent_context_parallel_recheck_all_pieces() {
    let test_dir = unique_test_path("parallel_recheck");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir_all(&test_dir).unwrap();

    let torrent_file = test_dir.join("test_recheck.torrent");
    let piece_length: u32 = 16384;
    let file_length: u64 = 65536; // 4 pieces
    let file_name = "recheck.bin";
    let file_data: Vec<u8> = vec![0x77; file_length as usize];
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

    let mut meta_info = MetaInfoFile::new(&torrent_file).unwrap();
    meta_info.parse().unwrap();
    let (_, files) = meta_info.local_files_to_download_list(&test_dir).unwrap();
    let disk_io = Arc::new(DiskIO::new(&test_dir, files, piece_length));
    disk_io.create_local_torrent_structure().unwrap();

    // Write file content so data on disk matches
    fs::write(test_dir.join(file_name), &file_data).unwrap();

    let mut ctx = TorrentContext::new(
        &meta_info,
        Arc::new(RarestFirstSelector),
        disk_io.clone(),
        &test_dir,
        false,
        SessionConfig::default(),
    ).unwrap();

    let progress = ctx.recheck_all_pieces().expect("Start parallel recheck");
    assert_eq!(progress.total_pieces, 4);

    let start = std::time::Instant::now();
    while !progress.is_finished() && start.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(10));
    }

    assert!(progress.is_finished());
    assert_eq!(progress.valid_pieces.load(std::sync::atomic::Ordering::Relaxed), 4);
    assert_eq!(progress.corrupt_pieces.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(progress.percent_complete(), 100.0);

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_disk_io_create_torrent_bitfield_parallel() {
    let test_dir = unique_test_path("disk_io_parallel");
    let _ = fs::remove_dir_all(&test_dir);
    fs::create_dir_all(&test_dir).unwrap();

    let torrent_file = test_dir.join("disk_io_test.torrent");
    let piece_length: u32 = 16384;
    let file_length: u64 = 32768; // 2 pieces
    let file_name = "disk_io_payload.bin";
    let file_data: Vec<u8> = vec![0x33; file_length as usize];
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

    let mut meta_info = MetaInfoFile::new(&torrent_file).unwrap();
    meta_info.parse().unwrap();
    let (_, files) = meta_info.local_files_to_download_list(&test_dir).unwrap();
    let disk_io = Arc::new(DiskIO::new(&test_dir, files, piece_length));
    disk_io.create_local_torrent_structure().unwrap();

    // Write file content so data on disk matches
    fs::write(test_dir.join(file_name), &file_data).unwrap();

    let mut ctx = TorrentContext::new(
        &meta_info,
        Arc::new(RarestFirstSelector),
        disk_io.clone(),
        &test_dir,
        false,
        SessionConfig::default(),
    ).unwrap();

    let pool = HashVerificationPool::new(4, 32);
    let progress = disk_io
        .create_torrent_bitfield_parallel(&mut ctx, &pool)
        .expect("Parallel bitfield generation");

    assert!(progress.is_finished());
    assert_eq!(progress.valid_pieces.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(ctx.missing_pieces_count, 0);
    assert!(ctx.is_piece_local(0));
    assert!(ctx.is_piece_local(1));

    let _ = fs::remove_dir_all(&test_dir);
}

#[test]
fn test_session_builder_verification_worker_pool_options() {
    let temp_dir = unique_test_path("builder_opts");
    let _ = fs::remove_dir_all(&temp_dir);

    let session = TorrentSessionBuilder::from_magnet(
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        &temp_dir,
    )
    .enable_async_verification(true)
    .verification_workers(6)
    .verification_queue_capacity(512)
    .build()
    .expect("Build session with verification options");

    let binding = session.context();
    let ctx = binding.lock().unwrap();
    assert!(ctx.config.enable_async_verification);
    assert_eq!(ctx.config.verification_workers, 6);
    assert_eq!(ctx.config.verification_queue_capacity, 512);

    let metrics = ctx.verification_metrics();
    assert_eq!(metrics.active_workers, 6);

    let _ = fs::remove_dir_all(&temp_dir);
}
