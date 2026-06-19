use std::fs;
use std::path::Path;
use bittorrent_rs::metainfo::MetaInfoFile;
use sync_hub::torrent_gen::generate_torrent_bytes;

#[test]
fn test_torrent_generation_and_parsing() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create a mock directory with some files
    let test_dir = Path::new("downloads/test_sync_dir");
    fs::create_dir_all(test_dir)?;
    
    fs::write(test_dir.join("file1.txt"), b"Hello World from Sync Hub!")?;
    fs::write(test_dir.join("file2.txt"), b"BitTorrent library makes sync hub easy.")?;
    fs::create_dir_all(test_dir.join("subdir"))?;
    fs::write(test_dir.join("subdir/file3.txt"), b"Deep nested file sync test.")?;

    // 2. Generate the torrent bytes
    let torrent_bytes = generate_torrent_bytes(test_dir, "test_sync_dir", 1024)?;

    // 3. Parse and validate using the core library's MetaInfoFile
    let mut metainfo = MetaInfoFile::from_bytes(&torrent_bytes);
    metainfo.parse()?;
    metainfo.validate()?;

    // Assertions
    assert!(metainfo.is_private(), "Sync torrent should be marked as private");
    
    let (_, files) = metainfo.local_files_to_download_list(".")?;
    assert_eq!(files.len(), 3, "Should detect exactly 3 files");

    assert!(files.iter().any(|f| f.torrent_path == "test_sync_dir/file1.txt"));
    assert!(files.iter().any(|f| f.torrent_path == "test_sync_dir/file2.txt"));
    assert!(files.iter().any(|f| f.torrent_path == "test_sync_dir/subdir/file3.txt"));

    // Cleanup
    let _ = fs::remove_dir_all(test_dir);

    Ok(())
}
