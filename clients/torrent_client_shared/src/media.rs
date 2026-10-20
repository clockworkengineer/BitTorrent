//! Media file detection, MIME type mapping, and primary media file resolution.

/// Returns true if the file name has a common video or audio extension.
pub fn is_media_filename(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".mp4")
        || lower.ends_with(".mkv")
        || lower.ends_with(".webm")
        || lower.ends_with(".avi")
        || lower.ends_with(".mov")
        || lower.ends_with(".m4v")
        || lower.ends_with(".flv")
        || lower.ends_with(".wmv")
        || lower.ends_with(".mp3")
        || lower.ends_with(".flac")
        || lower.ends_with(".ogg")
        || lower.ends_with(".wav")
        || lower.ends_with(".aac")
        || lower.ends_with(".m4a")
}

/// Detects the MIME content type based on the file extension.
pub fn detect_mime_type(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".mp4") || lower.ends_with(".m4v") {
        "video/mp4"
    } else if lower.ends_with(".mkv") {
        "video/x-matroska"
    } else if lower.ends_with(".webm") {
        "video/webm"
    } else if lower.ends_with(".avi") {
        "video/x-msvideo"
    } else if lower.ends_with(".mov") {
        "video/quicktime"
    } else if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".flac") {
        "audio/flac"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else {
        "video/mp4"
    }
}

/// Finds the best (largest) media file index from an iterator of files and lengths.
///
/// Falls back to the largest file overall if no recognized media file exists.
pub fn find_primary_media_file<I, S>(files: I) -> Option<usize>
where
    I: IntoIterator<Item = (S, u64)>,
    S: AsRef<str>,
{
    let mut best_media = None;
    let mut max_media_len = 0;
    let mut best_any = None;
    let mut max_any_len = 0;

    for (idx, (path, len)) in files.into_iter().enumerate() {
        if len > max_any_len {
            max_any_len = len;
            best_any = Some(idx);
        }
        if is_media_filename(path.as_ref()) && len > max_media_len {
            max_media_len = len;
            best_media = Some(idx);
        }
    }
    best_media.or(best_any)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_media_filename() {
        assert!(is_media_filename("movie.mp4"));
        assert!(is_media_filename("SONG.FLAC"));
        assert!(is_media_filename("path/to/video.mkv"));
        assert!(is_media_filename("stream.webm"));
        assert!(is_media_filename("clip.avi"));
        assert!(is_media_filename("clip.mov"));
        assert!(is_media_filename("clip.m4v"));
        assert!(is_media_filename("track.mp3"));
        assert!(is_media_filename("track.ogg"));
        assert!(is_media_filename("track.wav"));
        assert!(is_media_filename("track.aac"));
        assert!(is_media_filename("track.m4a"));
        assert!(!is_media_filename("document.pdf"));
        assert!(!is_media_filename("archive.tar.gz"));
        assert!(!is_media_filename("notes.txt"));
    }

    #[test]
    fn test_detect_mime_type() {
        assert_eq!(detect_mime_type("test.mp4"), "video/mp4");
        assert_eq!(detect_mime_type("test.m4v"), "video/mp4");
        assert_eq!(detect_mime_type("test.mkv"), "video/x-matroska");
        assert_eq!(detect_mime_type("test.webm"), "video/webm");
        assert_eq!(detect_mime_type("test.avi"), "video/x-msvideo");
        assert_eq!(detect_mime_type("test.mov"), "video/quicktime");
        assert_eq!(detect_mime_type("test.mp3"), "audio/mpeg");
        assert_eq!(detect_mime_type("test.flac"), "audio/flac");
        assert_eq!(detect_mime_type("test.ogg"), "audio/ogg");
        assert_eq!(detect_mime_type("test.wav"), "audio/wav");
        assert_eq!(detect_mime_type("test.unknown"), "video/mp4");
    }

    #[test]
    fn test_find_primary_media_file() {
        let files = vec![
            ("sample.nfo", 1_000u64),
            ("video.mp4", 500_000_000u64),
            ("trailer.mp4", 50_000_000u64),
            ("huge_archive.zip", 1_000_000_000u64),
        ];

        // Should prefer the largest media file even if non-media is larger
        assert_eq!(
            find_primary_media_file(files.iter().map(|(p, l)| (*p, *l))),
            Some(1)
        );

        // If no media file, should select the largest overall
        let non_media = vec![
            ("data1.bin", 100u64),
            ("data2.bin", 500u64),
            ("data3.bin", 300u64),
        ];
        assert_eq!(
            find_primary_media_file(non_media.iter().map(|(p, l)| (*p, *l))),
            Some(1)
        );

        // Empty list
        let empty: Vec<(&str, u64)> = vec![];
        assert_eq!(find_primary_media_file(empty), None);
    }
}
