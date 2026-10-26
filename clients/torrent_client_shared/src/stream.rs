//! Reusable HTTP Range Streaming Engine
//!
//! Provides parsing and response generation for HTTP 1.1 streaming clients
//! (e.g., VLC, mpv, web browsers). Supports:
//! - Method handling (`GET`, `HEAD`, `OPTIONS`)
//! - Range header extraction (`Range: bytes=start-end`, `bytes=start-`)
//! - CORS headers (`Access-Control-Allow-Origin: *`, preflight handling)
//! - HTTP 206 Partial Content and 416 Range Not Satisfiable
//! - Chunked streaming over any `Read + Seek` stream to any `Write` sink.

use crate::media::detect_mime_type;
use std::io::{BufRead, Read, Seek, SeekFrom, Write};

/// An incoming HTTP request line and headers relevant to streaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub range_header: Option<String>,
}

/// Reads and parses an HTTP request from a buffered reader.
/// Returns `Ok(None)` if the stream closed before a full request line was received.
pub fn read_http_request<R: BufRead>(reader: &mut R) -> std::io::Result<Option<HttpRequest>> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 || request_line.trim().is_empty() {
        return Ok(None);
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(None);
    }

    let method = parts[0].to_uppercase();
    let path = parts[1].to_string();

    let mut range_header = None;
    let mut header_line = String::new();
    loop {
        header_line.clear();
        if reader.read_line(&mut header_line)? == 0 {
            break;
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break;
        }
        if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("range:") {
            range_header = Some(trimmed["range:".len()..].trim().to_string());
        }
    }

    Ok(Some(HttpRequest {
        method,
        path,
        range_header,
    }))
}

/// Writes an HTTP 204 No Content preflight CORS response.
pub fn send_cors_preflight_response<W: Write>(writer: &mut W) -> std::io::Result<()> {
    let response = "HTTP/1.1 204 No Content\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
Access-Control-Allow-Headers: Range, Content-Type, Accept\r\n\
Access-Control-Max-Age: 86400\r\n\r\n";
    writer.write_all(response.as_bytes())?;
    writer.flush()
}

/// Writes a simple HTTP error response.
pub fn send_http_error<W: Write>(
    writer: &mut W,
    status_code: u16,
    status_text: &str,
    message: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {} {}\r\n\
Access-Control-Allow-Origin: *\r\n\
Content-Type: text/plain\r\n\
Content-Length: {}\r\n\r\n{}",
        status_code,
        status_text,
        message.len(),
        message
    );
    writer.write_all(response.as_bytes())?;
    writer.flush()
}

/// Parsed byte range request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
    pub is_range_request: bool,
}

/// Parses a `Range: bytes=start-end` or `bytes=start-` header value against total content length.
pub fn parse_range_spec(range_header: Option<&str>, total_len: u64) -> ByteRange {
    let mut start = 0u64;
    let mut end = total_len.saturating_sub(1);
    let mut is_range = false;

    if let Some(r) = range_header {
        if let Some(spec) = r.strip_prefix("bytes=") {
            let parts: Vec<&str> = spec.split('-').collect();
            if parts.len() == 2 {
                if let Ok(s) = parts[0].parse::<u64>() {
                    start = s;
                }
                if !parts[1].is_empty()
                    && let Ok(e) = parts[1].parse::<u64>()
                {
                    end = e.min(total_len.saturating_sub(1));
                }
                is_range = true;
            }
        }
    }

    ByteRange {
        start,
        end,
        is_range_request: is_range,
    }
}

/// Serves an HTTP Range media stream from any `Read + Seek` source to any `Write` destination.
///
/// Invokes `on_progress(current_offset, bytes_in_chunk)` when seeking and when each chunk is written.
pub fn serve_http_range_stream<R: Read + Seek, W: Write>(
    writer: &mut W,
    reader: &mut R,
    file_name: &str,
    total_len: u64,
    is_head: bool,
    range_header: Option<&str>,
    mut on_progress: impl FnMut(u64, usize),
) -> std::io::Result<()> {
    if total_len == 0 {
        writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")?;
        writer.flush()?;
        return Ok(());
    }

    let range = parse_range_spec(range_header, total_len);

    if range.start >= total_len {
        let response = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\r\n",
            total_len
        );
        writer.write_all(response.as_bytes())?;
        writer.flush()?;
        return Ok(());
    }

    let mime_type = detect_mime_type(file_name);
    let content_len = range.end.saturating_sub(range.start) + 1;

    let headers = if range.is_range_request {
        format!(
            "HTTP/1.1 206 Partial Content\r\n\
Content-Type: {}\r\n\
Content-Range: bytes {}-{}/{}\r\n\
Content-Length: {}\r\n\
Accept-Ranges: bytes\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length\r\n\
Connection: keep-alive\r\n\r\n",
            mime_type, range.start, range.end, total_len, content_len
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\n\
Content-Type: {}\r\n\
Content-Length: {}\r\n\
Accept-Ranges: bytes\r\n\
Access-Control-Allow-Origin: *\r\n\
Access-Control-Expose-Headers: Content-Range, Accept-Ranges, Content-Length\r\n\
Connection: keep-alive\r\n\r\n",
            mime_type, total_len
        )
    };

    writer.write_all(headers.as_bytes())?;
    writer.flush()?;

    if is_head {
        return Ok(());
    }

    reader.seek(SeekFrom::Start(range.start))?;
    on_progress(range.start, 0);

    let mut remaining = content_len;
    let mut buf = [0u8; 65536];

    while remaining > 0 {
        let to_read = (buf.len() as u64).min(remaining) as usize;
        match reader.read(&mut buf[..to_read]) {
            Ok(0) => break,
            Ok(bytes_read) => {
                if writer.write_all(&buf[..bytes_read]).is_err() {
                    break;
                }
                let _ = writer.flush();
                remaining -= bytes_read as u64;
                let current_offset = range.end.saturating_sub(remaining);
                on_progress(current_offset, bytes_read);
            }
            Err(_) => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_range_spec() {
        let r1 = parse_range_spec(Some("bytes=0-499"), 1000);
        assert_eq!(r1.start, 0);
        assert_eq!(r1.end, 499);
        assert!(r1.is_range_request);

        let r2 = parse_range_spec(Some("bytes=500-"), 1000);
        assert_eq!(r2.start, 500);
        assert_eq!(r2.end, 999);
        assert!(r2.is_range_request);

        let r3 = parse_range_spec(Some("bytes=500-5000"), 1000);
        assert_eq!(r3.start, 500);
        assert_eq!(r3.end, 999);
        assert!(r3.is_range_request);

        let r4 = parse_range_spec(None, 1000);
        assert_eq!(r4.start, 0);
        assert_eq!(r4.end, 999);
        assert!(!r4.is_range_request);
    }

    #[test]
    fn test_read_http_request() {
        let raw =
            b"GET /stream/test.mp4 HTTP/1.1\r\nHost: localhost\r\nRange: bytes=100-200\r\n\r\n";
        let mut cursor = Cursor::new(&raw[..]);
        let req = read_http_request(&mut cursor).unwrap().unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/stream/test.mp4");
        assert_eq!(req.range_header.as_deref(), Some("bytes=100-200"));
    }

    #[test]
    fn test_serve_http_range_stream_full() {
        let data = vec![b'x'; 200];
        let mut reader = Cursor::new(data);
        let mut out = Vec::new();

        let mut progress_calls = Vec::new();
        serve_http_range_stream(
            &mut out,
            &mut reader,
            "sample.mp4",
            200,
            false,
            None,
            |off, chunk| progress_calls.push((off, chunk)),
        )
        .unwrap();

        let out_str = String::from_utf8_lossy(&out);
        assert!(out_str.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(out_str.contains("Content-Length: 200\r\n"));
        assert!(out_str.contains("Content-Type: video/mp4\r\n"));
        assert!(!progress_calls.is_empty());
    }

    #[test]
    fn test_serve_http_range_stream_partial() {
        let data: Vec<u8> = (0..100).collect();
        let mut reader = Cursor::new(data);
        let mut out = Vec::new();

        serve_http_range_stream(
            &mut out,
            &mut reader,
            "sample.mp4",
            100,
            false,
            Some("bytes=10-19"),
            |_, _| {},
        )
        .unwrap();

        let out_str = String::from_utf8_lossy(&out);
        assert!(out_str.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(out_str.contains("Content-Range: bytes 10-19/100\r\n"));
        assert!(out_str.contains("Content-Length: 10\r\n"));

        // Body should be bytes 10..=19
        let header_end = out_str.find("\r\n\r\n").unwrap() + 4;
        let body = &out[header_end..];
        let expected: Vec<u8> = (10..=19).collect();
        assert_eq!(body, &expected[..]);
    }

    #[test]
    fn test_serve_http_range_stream_head() {
        let data = vec![b'a'; 100];
        let mut reader = Cursor::new(data);
        let mut out = Vec::new();

        serve_http_range_stream(
            &mut out,
            &mut reader,
            "sample.mp4",
            100,
            true, // HEAD
            None,
            |_, _| {},
        )
        .unwrap();

        let out_str = String::from_utf8_lossy(&out);
        assert!(out_str.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(out_str.ends_with("\r\n\r\n"));
    }

    #[test]
    fn test_serve_http_range_stream_out_of_range() {
        let data = vec![b'b'; 50];
        let mut reader = Cursor::new(data);
        let mut out = Vec::new();

        serve_http_range_stream(
            &mut out,
            &mut reader,
            "sample.mp4",
            50,
            false,
            Some("bytes=100-200"),
            |_, _| {},
        )
        .unwrap();

        let out_str = String::from_utf8_lossy(&out);
        assert!(out_str.starts_with("HTTP/1.1 416 Range Not Satisfiable\r\n"));
    }
}
