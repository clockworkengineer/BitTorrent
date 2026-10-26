use bittorrent_rs::{BNode, Bencode, MagnetLink, PeerMessage};

#[test]
fn test_magnet_link_parse_hex() {
    let uri = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&dn=Ubuntu+Desktop&tr=http%3A%2F%2Ftracker.ubuntu.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969";
    let magnet = MagnetLink::parse(uri).unwrap();

    let expected_hash = vec![
        0x3f, 0x06, 0x91, 0x23, 0xcd, 0x20, 0x52, 0xc9, 0x35, 0x40, 0xc4, 0x90, 0x8e, 0xf4, 0x8d,
        0xf8, 0xf4, 0x80, 0x39, 0x3c,
    ];

    assert_eq!(magnet.info_hash, expected_hash);
    assert_eq!(magnet.display_name.as_deref(), Some("Ubuntu Desktop"));
    assert_eq!(magnet.trackers.len(), 2);
    assert_eq!(
        magnet.trackers[0],
        "http://tracker.ubuntu.com:6969/announce"
    );
    assert_eq!(magnet.trackers[1], "udp://tracker.coppersurfer.tk:6969");
}

#[test]
fn test_magnet_link_parse_base32() {
    // base32: 3f069123cd2052c93540c4908ef48df8f480393c -> H4DJCI6NEBJM5NKAOSAIP5EM7D2IBWJ4 in base32
    // Let's test standard base32
    let uri = "magnet:?xt=urn:btih:h4djci6nebjm5nkaosaip5em7d2ibwj4&dn=Ubuntu+Desktop";
    let magnet = MagnetLink::parse(uri).unwrap();

    let expected_hash = vec![
        0x3f, 0x06, 0x91, 0x23, 0xcd, 0x20, 0x52, 0xce, 0xb5, 0x40, 0x74, 0x80, 0x87, 0xf4, 0x8c,
        0xf8, 0xf4, 0x80, 0xd9, 0x3c,
    ];

    assert_eq!(magnet.info_hash, expected_hash);
    assert_eq!(magnet.display_name.as_deref(), Some("Ubuntu Desktop"));
}

#[test]
fn test_bencode_decode_partial() {
    // Bencode dictionary {"msg_type": 1, "piece": 0} followed by raw binary data: [0xDE, 0xAD, 0xBE, 0xEF]
    // Bencoded: d8:msg_typei1e5:piecei0ee
    let payload = b"d8:msg_typei1e5:piecei0ee\xDE\xAD\xBE\xEF";
    let (node, consumed) = Bencode::decode_partial(payload).unwrap();

    assert_eq!(consumed, 25); // "d8:msg_typei1e5:piecei0ee" has 25 bytes
    assert!(matches!(node, BNode::Dictionary(_)));
    assert_eq!(&payload[consumed..], b"\xDE\xAD\xBE\xEF");
}

#[test]
fn test_peer_message_extended_encode_decode() {
    let payload = b"d1:md11:ut_metadatai1eee";
    let msg = PeerMessage::Extended { ext_id: 0, payload };
    let bytes = msg.encode();

    // length prefix (4 bytes) + message_id (20) (1 byte) + ext_id (1 byte) + payload
    assert_eq!(bytes.len(), 4 + 1 + 1 + payload.len());
    assert_eq!(bytes[4], 20); // ID 20 for Extended
    assert_eq!(bytes[5], 0); // ID 0 for handshake

    let decoded = PeerMessage::decode(&bytes[4..]).unwrap();
    assert_eq!(decoded, msg);
}

#[test]
fn test_magnet_link_parse_invalid_prefix() {
    let uri = "http://tracker.ubuntu.com:6969/announce";
    match MagnetLink::parse(uri) {
        Err(e) => assert!(format!("{}", e).contains("Invalid magnet link prefix")),
        Ok(_) => panic!("Expected error"),
    }
}

#[test]
fn test_magnet_link_parse_missing_xt() {
    let uri = "magnet:?dn=Ubuntu+Desktop&tr=http%3A%2F%2Ftracker.ubuntu.com%3A6969%2Fannounce";
    match MagnetLink::parse(uri) {
        Err(e) => assert!(format!("{}", e).contains("Missing info hash (xt)")),
        Ok(_) => panic!("Expected error"),
    }
}

#[test]
fn test_magnet_link_parse_invalid_infohash() {
    // Info hash too short (hex should be 40 chars)
    let uri_too_short =
        "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f48039&dn=Ubuntu+Desktop";
    match MagnetLink::parse(uri_too_short) {
        Err(e) => assert!(format!("{}", e).contains("Invalid info hash format")),
        Ok(_) => panic!("Expected error"),
    }

    // Base32 too short (should be 32 chars)
    let uri_b32_too_short = "magnet:?xt=urn:btih:h4djci6nebjm5nkaosaip5em7d2ibw&dn=Ubuntu+Desktop";
    assert!(MagnetLink::parse(uri_b32_too_short).is_err());

    // Invalid hex character
    let uri_invalid_hex =
        "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393g&dn=Ubuntu+Desktop";
    assert!(MagnetLink::parse(uri_invalid_hex).is_err());
}

#[test]
fn test_magnet_link_percent_decode_edge_cases() {
    // Percent decoding with incomplete escape at the end
    let uri = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&dn=Ubuntu+Desktop%2";
    let magnet = MagnetLink::parse(uri).unwrap();
    assert_eq!(magnet.display_name.as_deref(), Some("Ubuntu Desktop%"));

    // Plus decoded to space
    let uri_plus =
        "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&dn=Ubuntu+Desktop+21.04";
    let magnet_plus = MagnetLink::parse(uri_plus).unwrap();
    assert_eq!(
        magnet_plus.display_name.as_deref(),
        Some("Ubuntu Desktop 21.04")
    );

    // Non-hex characters in escape sequence
    let uri_non_hex =
        "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&dn=Ubuntu%2GDesktop";
    let magnet_non_hex = MagnetLink::parse(uri_non_hex).unwrap();
    assert_eq!(
        magnet_non_hex.display_name.as_deref(),
        Some("Ubuntu%Desktop")
    );
}

#[test]
fn test_magnet_transition_from_metadata_completed_transitions_to_seeding() {
    use sha1::{Digest, Sha1};
    use std::path::PathBuf;

    let temp_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test_magnet_transition_seeding");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Create file content and write it to disk
    let file_content = vec![0x42u8; 16384];
    std::fs::write(temp_dir.join("payload.bin"), &file_content).unwrap();

    let mut hasher = Sha1::new();
    hasher.update(&file_content);
    let piece_hash = hasher.finalize();

    let mut info_entries = Vec::new();
    let piece_length_str = "16384".to_string();
    info_entries.push((
        b"piece length".as_slice(),
        BNode::Number(piece_length_str.as_bytes()),
    ));
    info_entries.push((b"pieces".as_slice(), BNode::String(&piece_hash)));
    info_entries.push((b"name".as_slice(), BNode::String(b"payload.bin")));
    let length_str = "16384".to_string();
    info_entries.push((b"length".as_slice(), BNode::Number(length_str.as_bytes())));

    let info_node = BNode::Dictionary(info_entries);
    let info_dict_bytes = Bencode::encode(&info_node);

    let mut info_hasher = Sha1::new();
    info_hasher.update(&info_dict_bytes);
    let info_hash = info_hasher.finalize().to_vec();

    let mut ctx = bittorrent_rs::internals::TorrentContext::new_magnet_bootstrap(
        info_hash.clone(),
        vec!["http://tracker.example.com:80/announce".to_string()],
        std::sync::Arc::new(bittorrent_rs::RarestFirstSelector),
        &temp_dir,
        bittorrent_rs::SessionConfig::default(),
    )
    .expect("Create magnet bootstrap context");

    ctx.transition_from_metadata(&info_dict_bytes, &temp_dir)
        .expect("Transition from metadata");

    assert_eq!(ctx.status, bittorrent_rs::TorrentStatus::Seeding);
    assert_eq!(ctx.missing_pieces_count, 0);
    assert!(ctx.download_finished.is_set());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_magnet_link_with_cached_torrent_restores_as_seeding() {
    use sha1::{Digest, Sha1};
    use std::path::PathBuf;

    let temp_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test_magnet_cached_seeding");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let file_content = vec![0x77u8; 16384];
    std::fs::write(temp_dir.join("single_payload.bin"), &file_content).unwrap();

    let mut hasher = Sha1::new();
    hasher.update(&file_content);
    let piece_hash = hasher.finalize();

    let mut info_entries = Vec::new();
    let piece_length_str = "16384".to_string();
    info_entries.push((
        b"piece length".as_slice(),
        BNode::Number(piece_length_str.as_bytes()),
    ));
    info_entries.push((b"pieces".as_slice(), BNode::String(&piece_hash)));
    info_entries.push((b"name".as_slice(), BNode::String(b"single_payload.bin")));
    let length_str = "16384".to_string();
    info_entries.push((b"length".as_slice(), BNode::Number(length_str.as_bytes())));

    let info_node = BNode::Dictionary(info_entries);
    let info_dict_bytes = Bencode::encode(&info_node);

    let mut info_hasher = Sha1::new();
    info_hasher.update(&info_dict_bytes);
    let info_hash = info_hasher.finalize().to_vec();
    let hash_hex = bittorrent_rs::util::info_hash_to_string(&info_hash);

    // Create .torrent root dict
    let mut root_entries = Vec::new();
    root_entries.push((
        b"announce".as_slice(),
        BNode::String(b"http://tracker.example.com:80/announce"),
    ));
    root_entries.push((b"info".as_slice(), info_node));
    let torrent_bytes = Bencode::encode(&BNode::Dictionary(root_entries));

    // Save cached .torrent in download_dir
    let cached_torrent_path = temp_dir.join(format!("{}.torrent", hash_hex));
    std::fs::write(&cached_torrent_path, &torrent_bytes).unwrap();

    let magnet_uri = format!("magnet:?xt=urn:btih:{}&dn=single_payload.bin", hash_hex);
    let mut session = bittorrent_rs::TorrentSessionBuilder::from_magnet(&magnet_uri, &temp_dir)
        .seeding(true)
        .build()
        .expect("Build from magnet with cached torrent");

    assert_eq!(session.status(), bittorrent_rs::TorrentStatus::Seeding);
    assert_eq!(session.progress(), 100.0);

    // Verify start_download does not fail when already seeding
    assert!(session.start_download().is_ok());
    assert_eq!(session.status(), bittorrent_rs::TorrentStatus::Seeding);

    let _ = session.stop();
    let _ = std::fs::remove_dir_all(&temp_dir);
}
