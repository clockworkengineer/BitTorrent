use bittorrent_rs::{BNode, Bencode};

#[test]
fn test_encode_dictionary_sorts_keys() {
    let node = BNode::Dictionary(vec![
        (b"spam".to_vec(), BNode::Number(b"42".to_vec())),
        (b"bar".to_vec(), BNode::String(b"spam".to_vec())),
    ]);

    let encoded = Bencode::encode(&node);
    assert_eq!(encoded, b"d3:bar4:spam4:spami42ee".to_vec());
}

#[test]
fn test_decode_invalid_integer_format() {
    assert!(Bencode::decode(b"i01e").is_err());
    assert!(Bencode::decode(b"i-0e").is_err());
}

#[test]
fn test_decode_invalid_string_length() {
    assert!(Bencode::decode(b"02:ab").is_err());
    assert!(Bencode::decode(b"3:ab").is_err());
}

#[test]
fn test_get_dictionary_entry_string_nested() {
    let node =
        Bencode::decode(b"d4:infod4:name4:testee").expect("Failed to decode nested dictionary");
    assert_eq!(
        Bencode::get_dictionary_entry_string(&node, "name"),
        Some("test".to_string())
    );
}
