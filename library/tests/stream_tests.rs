mod common;

use bittorrent_rs::internals::TorrentContext;
use bittorrent_rs::selector::{PieceSelector, StreamingPieceSelector};
use bittorrent_rs::storage::stream::{StorageBlockReader, TorrentStream};
use bittorrent_rs::{BlockWriter, MemStorage, MetaInfoFile, Peer, SessionConfig};
use common::sample_file;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn create_test_context_and_storage(
    torrent_name: &str,
) -> (Arc<Mutex<TorrentContext>>, Arc<MemStorage>) {
    let mut meta = MetaInfoFile::new(sample_file(torrent_name)).expect("Failed to load torrent");
    meta.parse().expect("Failed to parse torrent");

    let piece_length = meta.get_piece_length().unwrap();
    let download_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("stream_test");
    let (total_bytes, _) = meta.local_files_to_download_list(&download_path).unwrap();

    let storage = Arc::new(MemStorage::new(total_bytes as usize));
    let selector = Arc::new(bittorrent_rs::selector::SequentialSelector);

    let context = Arc::new(Mutex::new(
        TorrentContext::new(
            &meta,
            selector,
            storage.clone(),
            &download_path,
            false,
            SessionConfig::default(),
        )
        .expect("Failed to create context"),
    ));

    // Populate piece lengths and clear missing status
    {
        let mut ctx = context.lock().unwrap();
        let num_pieces = ctx.number_of_pieces as u32;
        let mut bytes_left = total_bytes as i64;
        for i in 0..num_pieces {
            let len = if bytes_left > piece_length as i64 {
                piece_length
            } else {
                bytes_left as u32
            };
            ctx.set_piece_length(i, len);
            bytes_left -= len as i64;
        }
    }

    (context, storage)
}

#[test]
fn test_torrent_stream_seek_and_read() {
    let (context, storage) = create_test_context_and_storage("singlefile.torrent");
    let total_bytes = { context.lock().unwrap().total_bytes_to_download };
    let num_pieces = { context.lock().unwrap().number_of_pieces as u32 };

    // Fill storage with deterministic pattern
    let mut data = Vec::with_capacity(total_bytes as usize);
    for i in 0..total_bytes {
        data.push((i % 251) as u8);
    }
    storage.write_block(0, &data).unwrap();

    // Mark all pieces as locally available
    {
        let mut ctx = context.lock().unwrap();
        for p in 0..num_pieces {
            let len = ctx.get_piece_length(p);
            ctx.update_bitfield_status(p, true, len);
        }
    }

    let cursor = Arc::new(AtomicU32::new(0));
    let mut stream = TorrentStream::new(
        context.clone(),
        Arc::new(StorageBlockReader(storage)),
        0,
        total_bytes,
        Some(cursor.clone()),
    );

    assert_eq!(stream.total_len(), total_bytes);
    assert_eq!(stream.position(), 0);

    // 1. Read first 100 bytes
    let mut buf = [0u8; 100];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[0..100]);
    assert_eq!(stream.position(), 100);

    // 2. Seek from start
    let new_pos = stream.seek(SeekFrom::Start(500)).unwrap();
    assert_eq!(new_pos, 500);
    assert_eq!(stream.position(), 500);

    let mut buf = [0u8; 50];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[500..550]);
    assert_eq!(stream.position(), 550);

    // 3. Seek from current
    let new_pos = stream.seek(SeekFrom::Current(100)).unwrap();
    assert_eq!(new_pos, 650);
    let mut buf = [0u8; 20];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[650..670]);

    // 4. Seek backward from current
    let new_pos = stream.seek(SeekFrom::Current(-70)).unwrap();
    assert_eq!(new_pos, 600);
    let mut buf = [0u8; 10];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[600..610]);

    // 5. Seek from end
    let new_pos = stream.seek(SeekFrom::End(-100)).unwrap();
    assert_eq!(new_pos, total_bytes - 100);
    let mut buf = [0u8; 100];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[(total_bytes as usize - 100)..]);

    // 6. At EOF, read returns 0
    let mut buf = [0u8; 10];
    let n = stream.read(&mut buf).unwrap();
    assert_eq!(n, 0);

    // 7. Negative seek produces InvalidInput error
    assert!(stream.seek(SeekFrom::Start(u64::MAX)).is_ok()); // Seeking past end is allowed
    assert!(
        stream
            .seek(SeekFrom::Current(-((u64::MAX / 2) as i64)))
            .is_ok()
    );
    assert!(stream.seek(SeekFrom::Start(0)).is_ok());
    assert!(stream.seek(SeekFrom::Current(-1)).is_err());
}

#[test]
fn test_torrent_stream_cross_piece_boundary() {
    let (context, storage) = create_test_context_and_storage("singlefile.torrent");
    let total_bytes = { context.lock().unwrap().total_bytes_to_download };
    let piece_length = { context.lock().unwrap().piece_length };
    let num_pieces = { context.lock().unwrap().number_of_pieces as u32 };

    let mut data = Vec::with_capacity(total_bytes as usize);
    for i in 0..total_bytes {
        data.push((i % 255) as u8);
    }
    storage.write_block(0, &data).unwrap();

    // Mark all pieces as locally available
    {
        let mut ctx = context.lock().unwrap();
        for p in 0..num_pieces {
            let len = ctx.get_piece_length(p);
            ctx.update_bitfield_status(p, true, len);
        }
    }

    let mut stream = TorrentStream::new(
        context.clone(),
        Arc::new(StorageBlockReader(storage)),
        0,
        total_bytes,
        None,
    );

    // Read across piece boundary: start 100 bytes before piece 0 end and read 300 bytes into piece 1
    let seek_pos = (piece_length - 100) as u64;
    stream.seek(SeekFrom::Start(seek_pos)).unwrap();

    let mut buf = vec![0u8; 400];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &data[seek_pos as usize..seek_pos as usize + 400]);
}

#[test]
fn test_torrent_stream_for_file() {
    let (context, storage) = create_test_context_and_storage("multifile.torrent");
    let (num_files, files) = {
        let ctx = context.lock().unwrap();
        (ctx.files_to_download.len(), ctx.files_to_download.clone())
    };
    assert!(
        num_files >= 2,
        "multifile.torrent must have at least 2 files"
    );

    let total_bytes = { context.lock().unwrap().total_bytes_to_download };
    let num_pieces = { context.lock().unwrap().number_of_pieces as u32 };

    let mut data = Vec::with_capacity(total_bytes as usize);
    for i in 0..total_bytes {
        data.push(((i + 7) % 256) as u8);
    }
    storage.write_block(0, &data).unwrap();

    // Mark all pieces local
    {
        let mut ctx = context.lock().unwrap();
        for p in 0..num_pieces {
            let len = ctx.get_piece_length(p);
            ctx.update_bitfield_status(p, true, len);
        }
    }

    // Test file 0 stream
    let mut file0_stream = TorrentStream::for_file(context.clone(), 0, None).unwrap();
    assert_eq!(file0_stream.total_len(), files[0].length);

    let mut file0_data = vec![0u8; files[0].length as usize];
    file0_stream.read_exact(&mut file0_data).unwrap();
    assert_eq!(
        &file0_data[..],
        &data[files[0].offset as usize..files[0].offset as usize + files[0].length as usize]
    );

    // Test file 1 stream
    let mut file1_stream = TorrentStream::for_file(context.clone(), 1, None).unwrap();
    assert_eq!(file1_stream.total_len(), files[1].length);

    let mut file1_data = vec![0u8; files[1].length as usize];
    file1_stream.read_exact(&mut file1_data).unwrap();
    assert_eq!(
        &file1_data[..],
        &data[files[1].offset as usize..files[1].offset as usize + files[1].length as usize]
    );

    // Out of bounds file index error
    assert!(TorrentStream::for_file(context.clone(), 9999, None).is_err());
}

#[test]
fn test_torrent_stream_wait_and_timeout() {
    let (context, storage) = create_test_context_and_storage("singlefile.torrent");
    let total_bytes = { context.lock().unwrap().total_bytes_to_download };

    // Mark pieces missing
    {
        let mut ctx = context.lock().unwrap();
        ctx.mark_piece_missing(0, true);
        ctx.mark_piece_local(0, false);
    }

    let mut stream = TorrentStream::new(
        context.clone(),
        Arc::new(StorageBlockReader(storage)),
        0,
        total_bytes,
        None,
    )
    .with_read_timeout(Duration::from_millis(50))
    .with_poll_interval(Duration::from_millis(10));

    let mut buf = [0u8; 100];
    let err = stream.read(&mut buf).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);

    // ZERO timeout produces WouldBlock
    let mut stream_zero = stream.with_read_timeout(Duration::ZERO);
    let err_zb = stream_zero.read(&mut buf).unwrap_err();
    assert_eq!(err_zb.kind(), std::io::ErrorKind::WouldBlock);
}

#[test]
fn test_torrent_stream_dynamic_piece_arrival() {
    let (context, storage) = create_test_context_and_storage("singlefile.torrent");
    let total_bytes = { context.lock().unwrap().total_bytes_to_download };
    let piece_length = { context.lock().unwrap().piece_length };

    // Piece 0 is local, Piece 1 is missing initially
    {
        let mut ctx = context.lock().unwrap();
        ctx.update_bitfield_status(0, true, piece_length);
        ctx.mark_piece_missing(1, true);
        ctx.mark_piece_local(1, false);
    }

    let piece1_data = vec![0xABu8; piece_length as usize];

    let mut stream = TorrentStream::new(
        context.clone(),
        Arc::new(StorageBlockReader(storage.clone())),
        0,
        total_bytes,
        None,
    )
    .with_read_timeout(Duration::from_secs(3))
    .with_poll_interval(Duration::from_millis(20));

    // Seek to piece 1 start
    stream.seek(SeekFrom::Start(piece_length as u64)).unwrap();

    // Spawn background thread to simulate peer completing piece 1 download after 50ms
    let context_clone = context.clone();
    let storage_clone = storage.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let offset = piece_length as u64;
        storage_clone.write_block(offset, &piece1_data).unwrap();
        let mut ctx = context_clone.lock().unwrap();
        ctx.update_bitfield_status(1, true, piece_length);
    });

    let mut buf = [0u8; 100];
    stream.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[..], &[0xABu8; 100]);
}

#[test]
fn test_streaming_piece_selector_prioritization() {
    let (context, _) = create_test_context_and_storage("singlefile.torrent");
    let num_pieces = { context.lock().unwrap().number_of_pieces as u32 };

    let cursor = Arc::new(AtomicU32::new(2));
    let selector = StreamingPieceSelector::new(cursor.clone(), 3); // Lookahead window [2..5)

    let dummy_stream = std::net::TcpStream::connect("127.0.0.1:1").unwrap_or_else(|_| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let s = std::net::TcpStream::connect(addr).unwrap();
        drop(listener);
        s
    });

    let mut peer = Peer::new("127.0.0.1".into(), 6881, dummy_stream);
    peer.set_torrent_context(context.clone());
    // Peer has all pieces
    peer.remote_piece_bitfield = vec![0xFF; (num_pieces as usize + 7) / 8];

    // All pieces are missing locally
    {
        let mut ctx = context.lock().unwrap();
        for p in 0..num_pieces {
            ctx.mark_piece_missing(p, true);
            ctx.mark_piece_local(p, false);
        }
    }

    let ctx_guard = context.lock().unwrap();

    // 1. Selector should pick piece 2 first (the cursor piece)
    let selected = selector.select_piece(&ctx_guard, &peer);
    assert_eq!(selected, Some(2));
    drop(ctx_guard);

    // 2. Mark piece 2 as local, now it should pick piece 3 (within lookahead [2..5))
    {
        let mut ctx = context.lock().unwrap();
        ctx.mark_piece_local(2, true);
    }
    let ctx_guard = context.lock().unwrap();
    assert_eq!(selector.select_piece(&ctx_guard, &peer), Some(3));
    drop(ctx_guard);

    // 3. Update cursor to piece 6 (simulating seeking in media player)
    selector.set_cursor_piece(6);
    let ctx_guard = context.lock().unwrap();
    assert_eq!(selector.select_piece(&ctx_guard, &peer), Some(6));
}

#[test]
fn test_http_range_stream_server() {
    let (context, storage) = create_test_context_and_storage("singlefile.torrent");
    let total_bytes = { context.lock().unwrap().total_bytes_to_download };
    let num_pieces = { context.lock().unwrap().number_of_pieces as u32 };

    let mut data = Vec::with_capacity(total_bytes as usize);
    for i in 0..total_bytes {
        data.push((i % 251) as u8);
    }
    storage.write_block(0, &data).unwrap();

    {
        let mut ctx = context.lock().unwrap();
        for p in 0..num_pieces {
            let len = ctx.get_piece_length(p);
            ctx.update_bitfield_status(p, true, len);
        }
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let context_clone = context.clone();
    let storage_clone = storage.clone();

    let server_handle = std::thread::spawn(move || {
        for _ in 0..3 {
            let (mut tcp_stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(tcp_stream.try_clone().unwrap());
            let mut req_line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut req_line).unwrap();

            let mut range_header = None;
            loop {
                let mut line = String::new();
                std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if line.to_lowercase().starts_with("range:") {
                    range_header = Some(line.trim()["range:".len()..].trim().to_string());
                }
            }

            let mut torrent_stream = TorrentStream::new(
                context_clone.clone(),
                Arc::new(StorageBlockReader(storage_clone.clone())),
                0,
                total_bytes,
                None,
            );
            let total_len = torrent_stream.total_len();

            let (start, end, status) = if let Some(r) = range_header {
                if r.starts_with("bytes=") {
                    let spec = &r["bytes=".len()..];
                    let parts: Vec<&str> = spec.split('-').collect();
                    let s: u64 = parts[0].parse().unwrap();
                    let e: u64 = if !parts[1].is_empty() {
                        parts[1].parse().unwrap()
                    } else {
                        total_len - 1
                    };
                    (s, e, 206)
                } else {
                    (0, total_len - 1, 200)
                }
            } else {
                (0, total_len - 1, 200)
            };

            let content_len = end.saturating_sub(start) + 1;
            let header = if status == 206 {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\n\r\n",
                    start, end, total_len, content_len
                )
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", total_len)
            };

            if std::io::Write::write_all(&mut tcp_stream, header.as_bytes()).is_err() {
                continue;
            }
            torrent_stream.seek(SeekFrom::Start(start)).unwrap();
            let mut remaining = content_len;
            let mut buf = [0u8; 1024];
            while remaining > 0 {
                let to_read = (buf.len() as u64).min(remaining) as usize;
                let n = torrent_stream.read(&mut buf[..to_read]).unwrap();
                if n == 0 {
                    break;
                }
                if std::io::Write::write_all(&mut tcp_stream, &buf[..n]).is_err() {
                    break;
                }
                remaining -= n as u64;
            }
        }
    });

    // Test 1: Full content GET
    {
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        std::io::Write::write_all(&mut client, b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut reader = std::io::BufReader::new(client);
        let mut status_line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut status_line).unwrap();
        assert!(status_line.contains("200 OK"));

        loop {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
        }

        let mut body = vec![0u8; total_bytes as usize];
        std::io::Read::read_exact(&mut reader, &mut body).unwrap();
        assert_eq!(&body[..], &data[..]);
    }

    // Test 2: Range request bytes=100-199
    {
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        std::io::Write::write_all(
            &mut client,
            b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=100-199\r\n\r\n",
        )
        .unwrap();
        let mut reader = std::io::BufReader::new(client);
        let mut status_line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut status_line).unwrap();
        assert!(status_line.contains("206 Partial Content"));

        loop {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
        }

        let mut body = [0u8; 100];
        std::io::Read::read_exact(&mut reader, &mut body).unwrap();
        assert_eq!(&body[..], &data[100..200]);
    }

    // Test 3: Range request bytes=500-
    {
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        std::io::Write::write_all(
            &mut client,
            b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes=500-\r\n\r\n",
        )
        .unwrap();
        let mut reader = std::io::BufReader::new(client);
        let mut status_line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut status_line).unwrap();
        assert!(status_line.contains("206 Partial Content"));

        loop {
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
        }

        let mut body = [0u8; 50];
        std::io::Read::read_exact(&mut reader, &mut body).unwrap();
        assert_eq!(&body[..], &data[500..550]);
    }

    server_handle.join().unwrap();
}
