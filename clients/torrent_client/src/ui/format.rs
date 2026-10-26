//! Formatters and file system UI helpers.

/// Returns an appropriate emoji icon based on filename extension.
pub fn get_file_icon(path: &str) -> &'static str {
    let Some(dot_idx) = path.rfind('.') else {
        return "📄";
    };
    let ext = &path[dot_idx + 1..];
    if ext.eq_ignore_ascii_case("mp4")
        || ext.eq_ignore_ascii_case("mkv")
        || ext.eq_ignore_ascii_case("avi")
        || ext.eq_ignore_ascii_case("mov")
        || ext.eq_ignore_ascii_case("webm")
        || ext.eq_ignore_ascii_case("flv")
    {
        "🎬"
    } else if ext.eq_ignore_ascii_case("mp3")
        || ext.eq_ignore_ascii_case("flac")
        || ext.eq_ignore_ascii_case("wav")
        || ext.eq_ignore_ascii_case("ogg")
        || ext.eq_ignore_ascii_case("m4a")
        || ext.eq_ignore_ascii_case("aac")
    {
        "🎵"
    } else if ext.eq_ignore_ascii_case("iso")
        || ext.eq_ignore_ascii_case("img")
        || ext.eq_ignore_ascii_case("dmg")
    {
        "💿"
    } else if ext.eq_ignore_ascii_case("zip")
        || ext.eq_ignore_ascii_case("tar")
        || ext.eq_ignore_ascii_case("gz")
        || ext.eq_ignore_ascii_case("7z")
        || ext.eq_ignore_ascii_case("rar")
        || ext.eq_ignore_ascii_case("bz2")
    {
        "📦"
    } else if ext.eq_ignore_ascii_case("pdf")
        || ext.eq_ignore_ascii_case("epub")
        || ext.eq_ignore_ascii_case("txt")
        || ext.eq_ignore_ascii_case("doc")
        || ext.eq_ignore_ascii_case("docx")
    {
        "📄"
    } else if ext.eq_ignore_ascii_case("exe")
        || ext.eq_ignore_ascii_case("msi")
        || ext.eq_ignore_ascii_case("deb")
        || ext.eq_ignore_ascii_case("rpm")
    {
        "⚙"
    } else {
        "📄"
    }
}

/// Formats remaining duration into a concise, human-readable ETA string.
pub fn fmt_eta(remaining_bytes: u64, bps: u64) -> String {
    if bps == 0 {
        "∞".to_string()
    } else {
        let seconds = remaining_bytes / bps;
        if seconds == 0 {
            "Done".to_string()
        } else if seconds < 60 {
            format!("{}s", seconds)
        } else if seconds < 3600 {
            format!("{}m {}s", seconds / 60, seconds % 60)
        } else if seconds < 86400 {
            format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
        } else if seconds < 86400 * 30 {
            format!("{}d {}h", seconds / 86400, (seconds % 86400) / 3600)
        } else {
            "∞".to_string()
        }
    }
}

/// Opens the containing folder of a downloaded torrent in the native system file manager.
#[cfg(windows)]
pub fn open_containing_folder(path: &std::path::Path) {
    if path.is_file() {
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    } else {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
}

#[cfg(not(windows))]
pub fn open_containing_folder(_path: &std::path::Path) {}

/// Simple helper to URL encode a string for magnet display name without allocating formatting strings.
pub fn urlencoding_simple(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xF) as usize] as char);
        }
    }
    out
}
