use bittorrent_rs::{MagnetLink, Bencode, BNode, PeerMessage};

#[test]
fn test_magnet_link_parse_hex() {
    let uri = "magnet:?xt=urn:btih:3f069123cd2052c93540c4908ef48df8f480393c&dn=Ubuntu+Desktop&tr=http%3A%2F%2Ftracker.ubuntu.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969";
    let magnet = MagnetLink::parse(uri).unwrap();
    
    let expected_hash = vec![
        0x3f, 0x06, 0x91, 0x23, 0xcd, 0x20, 0x52, 0xc9, 0x35, 0x40,
        0xc4, 0x90, 0x8e, 0xf4, 0x8d, 0xf8, 0xf4, 0x80, 0x39, 0x3c,
    ];
    
    assert_eq!(magnet.info_hash, expected_hash);
    assert_eq!(magnet.display_name.as_deref(), Some("Ubuntu Desktop"));
    assert_eq!(magnet.trackers.len(), 2);
    assert_eq!(magnet.trackers[0], "http://tracker.ubuntu.com:6969/announce");
    assert_eq!(magnet.trackers[1], "udp://tracker.coppersurfer.tk:6969");
}

#[test]
fn test_magnet_link_parse_base32() {
    // base32: 3f069123cd2052c93540c4908ef48df8f480393c -> H4DJCI6NEBJM5NKAOSAIP5EM7D2IBWJ4 in base32
    // Let's test standard base32
    let uri = "magnet:?xt=urn:btih:h4djci6nebjm5nkaosaip5em7d2ibwj4&dn=Ubuntu+Desktop";
    let magnet = MagnetLink::parse(uri).unwrap();
    
    let expected_hash = vec![
        0x3f, 0x06, 0x91, 0x23, 0xcd, 0x20, 0x52, 0xce, 0xb5, 0x40,
        0x74, 0x80, 0x87, 0xf4, 0x8c, 0xf8, 0xf4, 0x80, 0xd9, 0x3c,
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
    assert_eq!(bytes[5], 0);  // ID 0 for handshake
    
    let decoded = PeerMessage::decode(&bytes[4..]).unwrap();
    assert_eq!(decoded, msg);
}
