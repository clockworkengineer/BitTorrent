use bittorrent_rs::TorrentSession;
use std::env;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn print_usage() {
    eprintln!(
        "Usage: torrent_stream_example <torrent-file-or-magnet> [download-dir] [port] [file-index]"
    );
    eprintln!();
    eprintln!("Arguments:");
    eprintln!("  <torrent-file-or-magnet> Path to a .torrent file or a magnet URI string");
    eprintln!(
        "  [download-dir]           Directory to store incoming pieces (default: ./downloads)"
    );
    eprintln!("  [port]                   Local HTTP streaming port (default: 8080)");
    eprintln!(
        "  [file-index]             0-based index of the file to stream (default: largest file)"
    );
    eprintln!();
    eprintln!("Example:");
    eprintln!("  torrent_stream_example BigBuckBunny.torrent ./downloads 8080");
    eprintln!("  Then open in VLC: Media -> Open Network Stream -> http://127.0.0.1:8080/stream");
}

fn handle_client(
    mut stream_sock: TcpStream,
    session: &Arc<TorrentSession>,
    file_index: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = BufReader::new(&stream_sock);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    if request_line.is_empty() {
        return Ok(());
    }

    let mut range_header = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if trimmed.to_lowercase().starts_with("range:") {
            range_header = Some(trimmed.to_string());
        }
    }

    // Create a seekable TorrentStream for the selected file
    let mut torrent_stream = session.create_file_stream(file_index)?;
    let total_len = torrent_stream.seek(SeekFrom::End(0))?;
    torrent_stream.seek(SeekFrom::Start(0))?;

    let (start_byte, end_byte) = if let Some(ref range) = range_header {
        // e.g. "Range: bytes=1048576-" or "Range: bytes=0-499"
        let parts: Vec<&str> = range.split('=').collect();
        if parts.len() == 2 {
            let spec: Vec<&str> = parts[1].split('-').collect();
            let start: u64 = spec[0].trim().parse().unwrap_or(0);
            let end: u64 = if spec.len() > 1 && !spec[1].trim().is_empty() {
                spec[1].trim().parse().unwrap_or(total_len - 1)
            } else {
                total_len - 1
            };
            (start, end.min(total_len - 1))
        } else {
            (0, total_len - 1)
        }
    } else {
        (0, total_len - 1)
    };

    let content_len = end_byte.saturating_sub(start_byte) + 1;
    torrent_stream.seek(SeekFrom::Start(start_byte))?;

    println!(
        "  -> Streaming range: bytes {}-{}/{} ({} bytes requested)",
        start_byte, end_byte, total_len, content_len
    );

    let headers = if range_header.is_some() {
        format!(
            "HTTP/1.1 206 Partial Content\r\n\
             Accept-Ranges: bytes\r\n\
             Content-Range: bytes {}-{}/{}\r\n\
             Content-Length: {}\r\n\
             Content-Type: video/mp4\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\r\n",
            start_byte, end_byte, total_len, content_len
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Accept-Ranges: bytes\r\n\
             Content-Length: {}\r\n\
             Content-Type: video/mp4\r\n\
             Access-Control-Allow-Origin: *\r\n\
             Connection: close\r\n\r\n",
            total_len
        )
    };

    stream_sock.write_all(headers.as_bytes())?;

    // Pipe bytes from TorrentStream to TCP socket in 32 KiB chunks
    let mut buffer = [0u8; 32768];
    let mut sent = 0u64;

    while sent < content_len {
        let to_read = (content_len - sent).min(buffer.len() as u64) as usize;
        match torrent_stream.read(&mut buffer[..to_read]) {
            Ok(0) => break, // EOF or stream end
            Ok(n) => {
                if stream_sock.write_all(&buffer[..n]).is_err() {
                    // Client disconnected (e.g. paused or sought elsewhere)
                    break;
                }
                sent += n as u64;
            }
            Err(e) => {
                eprintln!("  ! Stream read error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        std::process::exit(1);
    }

    let source = &args[0];
    let download_dir = PathBuf::from(args.get(1).map(|s| s.as_str()).unwrap_or("./downloads"));
    let port: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8080);
    let requested_file_index: Option<usize> = args.get(3).and_then(|s| s.parse().ok());

    std::fs::create_dir_all(&download_dir)?;

    println!("=== BitTorrent Sequential Media Streaming Server ===");
    println!("Source:        {}", source);
    println!("Download Dir:  {}", download_dir.display());
    println!("HTTP Port:     {}", port);

    // Initialize session from .torrent or Magnet link
    let mut session = if source.starts_with("magnet:") {
        println!("Bootstrapping session from Magnet URI...");
        TorrentSession::from_magnet(source, &download_dir).build()?
    } else {
        println!("Loading session from torrent file: {}", source);
        let path = Path::new(source);
        TorrentSession::builder(path, &download_dir).build()?
    };

    session.start_download()?;
    println!("Download engine started.");

    let session_arc = Arc::new(session);

    // Determine target file index to stream (default to largest file)
    let (target_file_index, file_name, file_len) = {
        let ctx = session_arc.context();
        let guard = ctx.lock().unwrap();
        println!(
            "\nAvailable Files in Torrent (Total: {}):",
            guard.files_to_download.len()
        );
        for (i, f) in guard.files_to_download.iter().enumerate() {
            println!("  [{}] {} ({} bytes)", i, f.name, f.length);
        }

        if let Some(idx) = requested_file_index {
            let f = guard
                .files_to_download
                .get(idx)
                .expect("Specified file-index is out of range");
            (idx, f.name.clone(), f.length)
        } else {
            let (max_idx, max_f) = guard
                .files_to_download
                .iter()
                .enumerate()
                .max_by_key(|(_, f)| f.length)
                .expect("Torrent has no files");
            (max_idx, max_f.name.clone(), max_f.length)
        }
    };

    println!(
        "\nSelected file for streaming: [{}] {} ({} bytes)",
        target_file_index, file_name, file_len
    );

    let bind_addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&bind_addr)?;
    println!(
        "\nStreaming HTTP server listening at http://{}/stream",
        bind_addr
    );
    println!("Paste this URL into VLC, mpv, or any web browser to stream!");
    println!("Press Ctrl+C to stop.\n");

    for stream in listener.incoming() {
        match stream {
            Ok(sock) => {
                let sess_clone = session_arc.clone();
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(sock, &sess_clone, target_file_index) {
                        eprintln!("Client handler error: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("TCP accept error: {}", e);
            }
        }
    }

    Ok(())
}
