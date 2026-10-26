use bittorrent_rs::{
    FilePriority, MagnetLink, MagnetUriBuilder, MetadataRangeFetcher, PartialMetadataManifest,
    RarestFirstSelector, SessionConfig, TorrentSession, format_bep53_select_only,
    parse_bep53_select_only,
};
use std::time::Duration;

#[test]
fn test_bep53_magnet_link_parsing() {
    // 1. Single index
    let uri1 = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&so=2";
    let magnet1 = MagnetLink::parse(uri1).expect("Parse single index");
    assert!(magnet1.has_file_selection());
    assert_eq!(magnet1.selected_indices(), &[2]);
    assert!(!magnet1.is_file_selected(0));
    assert!(magnet1.is_file_selected(2));

    // 2. Inclusive range
    let uri2 = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&so=6-8";
    let magnet2 = MagnetLink::parse(uri2).expect("Parse range");
    assert_eq!(magnet2.selected_indices(), &[6, 7, 8]);
    assert!(magnet2.is_file_selected(6));
    assert!(magnet2.is_file_selected(7));
    assert!(magnet2.is_file_selected(8));
    assert!(!magnet2.is_file_selected(5));
    assert!(!magnet2.is_file_selected(9));

    // 3. Mixed comma-separated tokens
    let uri3 = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&so=0,2,4,6-8";
    let magnet3 = MagnetLink::parse(uri3).expect("Parse mixed so=");
    assert_eq!(magnet3.selected_indices(), &[0, 2, 4, 6, 7, 8]);

    // 4. Multiple so= parameters in query string
    let uri4 = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&so=1,3&so=5-7&so=9";
    let magnet4 = MagnetLink::parse(uri4).expect("Parse multiple so=");
    assert_eq!(magnet4.selected_indices(), &[1, 3, 5, 6, 7, 9]);

    // 5. Unsorted input with duplicates and whitespace
    let mut indices = Vec::new();
    parse_bep53_select_only(" 8, 4 , 2, 6-7, 4 , 0, bad-token , 10-9 ", &mut indices);
    assert_eq!(indices, vec![0, 2, 4, 6, 7, 8]);

    // 6. No so= parameter -> all files selected by default
    let uri_no_so = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c";
    let magnet_no_so = MagnetLink::parse(uri_no_so).expect("Parse no so=");
    assert!(!magnet_no_so.has_file_selection());
    assert!(magnet_no_so.selected_indices().is_empty());
    assert!(magnet_no_so.is_file_selected(0));
    assert!(magnet_no_so.is_file_selected(42));
    assert!(!magnet_no_so.is_explicitly_selected(0));
}

#[test]
fn test_bep53_magnet_uri_canonical_serialization() {
    // Range compression
    assert_eq!(format_bep53_select_only(&[0, 2, 4, 6, 7, 8]), "0,2,4,6-8");
    assert_eq!(format_bep53_select_only(&[1, 2, 3, 4, 5]), "1-5");
    assert_eq!(format_bep53_select_only(&[3]), "3");
    assert_eq!(format_bep53_select_only(&[]), "");

    let raw_hash = [0xabu8; 20];
    let magnet = MagnetUriBuilder::new(&raw_hash)
        .with_display_name("Multi-File Test")
        .with_selected_files(&[0, 2, 4])
        .with_selected_range(6, 8)
        .build()
        .expect("Build magnet");

    assert_eq!(magnet.selected_indices(), &[0, 2, 4, 6, 7, 8]);

    let uri = magnet.to_uri();
    assert!(
        uri.contains("so=0,2,4,6-8"),
        "URI should contain compressed so parameter: {}",
        uri
    );

    // Roundtrip parse
    let parsed = MagnetLink::parse(&uri).expect("Reparse URI");
    assert_eq!(parsed.selected_indices(), &[0, 2, 4, 6, 7, 8]);
    assert_eq!(parsed.display_name.as_deref(), Some("Multi-File Test"));
}

#[test]
fn test_bep53_magnet_uri_builder_methods() {
    let raw_hash = [0x55u8; 20];
    let magnet = MagnetUriBuilder::new(&raw_hash)
        .select_file(1)
        .select_file(3)
        .with_selected_range(10, 12)
        .build()
        .expect("Build magnet");

    assert_eq!(magnet.selected_indices(), &[1, 3, 10, 11, 12]);

    let mut cloned = magnet.clone();
    cloned.add_selected_file(7);
    cloned.add_selected_range(20, 22);
    assert_eq!(
        cloned.selected_indices(),
        &[1, 3, 7, 10, 11, 12, 20, 21, 22]
    );

    cloned.clear_selection();
    assert!(!cloned.has_file_selection());
    assert!(cloned.selected_indices().is_empty());
}

#[test]
fn test_metadata_range_fetcher_pipelining_and_timeout() {
    let mut fetcher = MetadataRangeFetcher::new()
        .with_timeout(Duration::from_millis(50))
        .with_pipeline_depth(2);

    // 48 KiB total = exactly three 16 KiB pieces
    fetcher.set_metadata_size(49152);
    assert_eq!(fetcher.total_pieces(), Some(3));
    assert_eq!(fetcher.pieces_count(), 0);
    assert!(!fetcher.is_complete());

    // Peer 1 requests up to limit 2
    let reqs_peer1 = fetcher.next_requests_for_peer("peer_1", 2);
    assert_eq!(reqs_peer1, vec![0, 1]); // Piece 0 prioritized, then piece 1

    // Peer 1 already has 2 in flight (at max depth), requesting again returns empty
    let reqs_peer1_again = fetcher.next_requests_for_peer("peer_1", 2);
    assert!(reqs_peer1_again.is_empty());

    // Peer 2 requests: piece 0 and 1 are in flight for Peer 1, so Peer 2 gets piece 2
    let reqs_peer2 = fetcher.next_requests_for_peer("peer_2", 2);
    assert_eq!(reqs_peer2, vec![2]);

    // Peer 1 receives reject for piece 1
    fetcher.handle_reject(1, "peer_1");

    // Peer 2 can now pick up rejected piece 1
    let reqs_peer2_retry = fetcher.next_requests_for_peer("peer_2", 1);
    assert_eq!(reqs_peer2_retry, vec![1]);

    // Peer 1 supplies piece 0
    let piece0_data = vec![0x11u8; 16384];
    let complete = fetcher
        .handle_data_piece(0, Some(49152), &piece0_data, "peer_1")
        .unwrap();
    assert!(!complete);
    assert_eq!(fetcher.pieces_count(), 1);

    // Peer 2 supplies piece 1 and piece 2
    let piece1_data = vec![0x22u8; 16384];
    let piece2_data = vec![0x33u8; 16384];
    assert!(
        !fetcher
            .handle_data_piece(1, None, &piece1_data, "peer_2")
            .unwrap()
    );
    assert!(
        fetcher
            .handle_data_piece(2, None, &piece2_data, "peer_2")
            .unwrap()
    );

    assert!(fetcher.is_complete());
    let assembled = fetcher.assemble().expect("Assemble complete metadata");
    assert_eq!(assembled.len(), 49152);
    assert_eq!(&assembled[0..16384], &piece0_data[..]);
    assert_eq!(&assembled[16384..32768], &piece1_data[..]);
    assert_eq!(&assembled[32768..49152], &piece2_data[..]);

    // Verify SHA-1 hash matching
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(&assembled);
    let expected_hash = hasher.finalize().to_vec();

    let verified = fetcher
        .verify_and_assemble(&expected_hash)
        .expect("Verification succeeds");
    assert_eq!(verified.len(), 49152);

    let metrics = fetcher.metrics();
    assert_eq!(metrics.total_pieces, 3);
    assert_eq!(metrics.pieces_received, 3);
    assert_eq!(metrics.pieces_in_flight, 0);
    assert_eq!(metrics.active_peers, 2);
    assert!(metrics.is_complete);
}

#[test]
fn test_partial_metadata_manifest_expedited_resolution() {
    // Construct a realistic bencoded multi-file info dictionary
    // Files:
    //   Index 0: "video.mp4" (131072 bytes = 128 KiB)
    //   Index 1: "subtitles.srt" (16384 bytes = 16 KiB)
    //   Index 2: "cover.jpg" (32768 bytes = 32 KiB)
    //   Index 3: "readme.txt" (8192 bytes = 8 KiB)
    // Total bytes = 188416 bytes (~184 KiB)
    // Piece length = 32768 (32 KiB) -> 6 pieces total
    let info_dict_str = concat!(
        "d",
        "5:files",
        "l",
        "d6:lengthi131072e4:pathl9:video.mp4ee",
        "d6:lengthi16384e4:pathl13:subtitles.srtee",
        "d6:lengthi32768e4:pathl9:cover.jpgee",
        "d6:lengthi8192e4:pathl10:readme.txtee",
        "e",
        "4:name12:AwesomeMovie",
        "12:piece lengthi32768e",
        "6:pieces120:012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789",
        "e"
    );
    let full_info_bytes = info_dict_str.as_bytes();

    // 1. Test parsing complete info dictionary
    let manifest = PartialMetadataManifest::try_parse_partial(full_info_bytes)
        .expect("Parse complete metadata manifest");
    assert_eq!(manifest.name, "AwesomeMovie");
    assert_eq!(manifest.piece_length, 32768);
    assert_eq!(manifest.total_bytes, 188416);
    assert_eq!(manifest.files.len(), 4);

    assert_eq!(manifest.files[0].name, "video.mp4");
    assert_eq!(manifest.files[0].length, 131072);
    assert_eq!(manifest.files[0].piece_range, (0, 3)); // 128 KiB / 32 KiB = 4 pieces (0..=3)

    assert_eq!(manifest.files[1].name, "subtitles.srt");
    assert_eq!(manifest.files[1].length, 16384);
    assert_eq!(manifest.files[1].piece_range, (4, 4)); // bytes 131072..147456 -> piece 4

    assert_eq!(manifest.files[2].name, "cover.jpg");
    assert_eq!(manifest.files[2].length, 32768);
    assert_eq!(manifest.files[2].piece_range, (4, 5)); // bytes 147456..180224 -> pieces 4 and 5

    assert_eq!(manifest.files[3].name, "readme.txt");
    assert_eq!(manifest.files[3].length, 8192);
    assert_eq!(manifest.files[3].piece_range, (5, 5)); // bytes 180224..188416 -> piece 5

    // 2. Test expedited parsing from truncated prefix (before pieces string is finished)
    // Cut off right after piece length key, before pieces finishes
    let cut_pos = info_dict_str.find("6:pieces").unwrap() + 15;
    let truncated_bytes = &full_info_bytes[..cut_pos];
    let expedited = PartialMetadataManifest::try_parse_partial(truncated_bytes)
        .expect("Expedited scan of partial dictionary should succeed");
    assert_eq!(expedited.name, "AwesomeMovie");
    assert_eq!(expedited.files.len(), 4);
    assert_eq!(expedited.total_bytes, 188416);

    // 3. Evaluate BEP 53 selection: select only files 1 and 3 (subtitles + readme)
    let eval = manifest.evaluate_bep53_selection(&[1, 3]);
    assert_eq!(eval.selected_indices, vec![1, 3]);
    assert_eq!(eval.selected_files.len(), 2);
    assert_eq!(eval.unselected_files.len(), 2);

    let expected_selected_bytes = 16384 + 8192; // 24576 bytes
    assert_eq!(eval.selected_bytes, expected_selected_bytes);
    assert_eq!(eval.total_bytes, 188416);
    assert!(eval.byte_savings_percent > 85.0); // > 85% bandwidth saved by skipping video

    // Subtitles is on piece 4; readme is on piece 5 -> pieces 4 and 5
    assert_eq!(eval.overlapping_pieces, vec![4, 5]);
    assert_eq!(eval.selected_piece_count, 2);
    assert_eq!(eval.total_piece_count, 6);
    assert!(eval.piece_savings_percent > 65.0);
}

#[test]
fn test_context_bep53_file_selection_priority_and_selector_filtering() {
    use bittorrent_rs::FileDetails;
    use std::sync::Arc;

    let temp_dir = std::env::temp_dir().join(format!("bt_bep53_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    let raw_hash = vec![0x99u8; 20];
    let config = SessionConfig::default();
    let selector = Arc::new(RarestFirstSelector);

    let mut context = bittorrent_rs::internals::TorrentContext::new_magnet_bootstrap(
        raw_hash,
        vec!["udp://tracker.opentrackr.org:1337".into()],
        selector,
        &temp_dir,
        config,
    )
    .expect("Bootstrap context");

    // Manually simulate a 3-file torrent structure
    context.piece_length = 32768; // 32 KiB
    context.number_of_pieces = 4;
    context.bitfield = vec![0u8; 1];
    context.pieces_missing = vec![0b00001111; 1];
    context.total_bytes_to_download = 131072; // 128 KiB
    context.files_to_download = vec![
        FileDetails {
            name: "file0.bin".into(),
            torrent_path: "file0.bin".into(),
            length: 32768,
            offset: 0,
            priority: FilePriority::Normal,
            md5sum: None,
            pieces_root: None,
        },
        FileDetails {
            name: "file1.bin".into(),
            torrent_path: "file1.bin".into(),
            length: 65536,
            offset: 32768,
            priority: FilePriority::Normal,
            md5sum: None,
            pieces_root: None,
        },
        FileDetails {
            name: "file2.bin".into(),
            torrent_path: "file2.bin".into(),
            length: 32768,
            offset: 98304,
            priority: FilePriority::Normal,
            md5sum: None,
            pieces_root: None,
        },
    ];

    // File 0: bytes 0..32768 -> piece 0
    // File 1: bytes 32768..98304 -> pieces 1, 2
    // File 2: bytes 98304..131072 -> piece 3

    // Apply BEP 53 selection: select ONLY file 0 and file 2 (skipping file 1)
    context.set_selected_files(&[0, 2]);
    assert_eq!(context.selected_files(), Some(&[0, 2][..]));

    // Check file priorities
    assert_eq!(context.files_to_download[0].priority, FilePriority::Normal);
    assert_eq!(context.files_to_download[1].priority, FilePriority::Skip);
    assert_eq!(context.files_to_download[2].priority, FilePriority::Normal);

    // Check piece wantedness
    assert!(
        context.is_piece_wanted(0),
        "Piece 0 belongs to file 0, must be wanted"
    );
    assert!(
        !context.is_piece_wanted(1),
        "Piece 1 belongs to skipped file 1, must NOT be wanted"
    );
    assert!(
        !context.is_piece_wanted(2),
        "Piece 2 belongs to skipped file 1, must NOT be wanted"
    );
    assert!(
        context.is_piece_wanted(3),
        "Piece 3 belongs to file 2, must be wanted"
    );

    // Check Piece Priority Queue contains only pieces 0 and 3
    let pq_pieces: Vec<u32> = {
        let pq = context.piece_priority_queue.lock().unwrap();
        pq.iter().map(|p| p.piece_index).collect()
    };
    assert!(pq_pieces.contains(&0));
    assert!(pq_pieces.contains(&3));
    assert!(!pq_pieces.contains(&1));
    assert!(!pq_pieces.contains(&2));

    // Evaluate BEP 53 savings report
    let eval = context
        .bep53_evaluation()
        .expect("Evaluation report available");
    assert_eq!(eval.selected_indices, vec![0, 2]);
    assert_eq!(eval.selected_bytes, 65536);
    assert_eq!(eval.total_bytes, 131072);
    assert_eq!(eval.byte_savings_percent, 50.0);
    assert_eq!(eval.overlapping_pieces, vec![0, 3]);
    assert_eq!(eval.piece_savings_percent, 50.0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_session_builder_and_runtime_bep53_selection() {
    let temp_dir = std::env::temp_dir().join(format!("bt_session_bep53_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // Build session from magnet URI containing so=0,2
    let magnet_uri =
        "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&dn=SelectedTorrent&so=0,2";
    let mut session = TorrentSession::from_magnet(magnet_uri, &temp_dir)
        .metadata_pipeline_depth(8)
        .metadata_request_timeout(3)
        .build()
        .expect("Build magnet session with BEP 53");

    assert_eq!(session.selected_files(), Some(vec![0, 2]));

    // Dynamically update selection during runtime to files 1 and 3
    session
        .select_files(&[1, 3])
        .expect("Update file selection at runtime");
    assert_eq!(session.selected_files(), Some(vec![1, 3]));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_magnet_metadata_transition_end_to_end() {
    use bittorrent_rs::bencode::{BNode, Bencode};
    use bittorrent_rs::core::torrent_context::TorrentContext;
    use sha1::{Digest, Sha1};

    let temp_dir = std::env::temp_dir().join(format!("bt_meta_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    // Create a mock info dictionary
    let mut info_entries = Vec::new();
    let piece_length_str = "16384".to_string();
    info_entries.push((
        b"piece length".as_slice(),
        BNode::Number(piece_length_str.as_bytes()),
    ));
    let dummy_hash = [0xabu8; 20];
    info_entries.push((b"pieces".as_slice(), BNode::String(&dummy_hash)));
    info_entries.push((b"name".as_slice(), BNode::String(b"test_payload.bin")));
    let length_str = "16384".to_string();
    info_entries.push((b"length".as_slice(), BNode::Number(length_str.as_bytes())));

    let info_node = BNode::Dictionary(info_entries);
    let info_dict_bytes = Bencode::encode(&info_node);

    let mut hasher = Sha1::new();
    hasher.update(&info_dict_bytes);
    let info_hash = hasher.finalize().to_vec();

    let mut ctx = TorrentContext::new_magnet_bootstrap(
        info_hash.clone(),
        vec!["http://tracker.example.com:80/announce".to_string()],
        std::sync::Arc::new(RarestFirstSelector),
        &temp_dir,
        SessionConfig::default(),
    )
    .expect("Create magnet bootstrap context");

    assert!(ctx.pieces_info_hash.is_empty());
    assert_eq!(ctx.number_of_pieces, 0);

    // Call transition_from_metadata
    ctx.transition_from_metadata(&info_dict_bytes, &temp_dir)
        .expect("Transition from metadata");

    assert!(!ctx.pieces_info_hash.is_empty());
    assert_eq!(ctx.number_of_pieces, 1);
    assert_eq!(ctx.piece_length, 16384);
    assert_eq!(ctx.total_bytes_to_download, 16384);
    assert_eq!(ctx.files_to_download.len(), 1);
    assert_eq!(ctx.files_to_download[0].length, 16384);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
