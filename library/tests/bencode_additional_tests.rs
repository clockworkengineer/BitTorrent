use bittorrent_rs::{BNode, Bencode};

#[test]
fn test_encode_dictionary_sorts_keys() {
    let node = BNode::Dictionary(vec![
        (b"spam" as &[u8], BNode::Number(b"42")),
        (b"bar" as &[u8], BNode::String(b"spam")),
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

#[test]
fn test_decode_nesting_depth_limit() {
    // 49 nested lists is allowed (leaf is at depth 50)
    let mut ok_input = vec![b'l'; 49];
    ok_input.push(b'i');
    ok_input.push(b'1');
    ok_input.push(b'e');
    ok_input.extend(std::iter::repeat(b'e').take(49));
    assert!(Bencode::decode(&ok_input).is_ok());

    // 50 nested lists is rejected (leaf is at depth 51)
    let mut bad_input = vec![b'l'; 50];
    bad_input.push(b'i');
    bad_input.push(b'1');
    bad_input.push(b'e');
    bad_input.extend(std::iter::repeat(b'e').take(50));
    let err = Bencode::decode(&bad_input);
    assert!(err.is_err());
    assert!(format!("{}", err.unwrap_err()).contains("Nesting depth limit exceeded"));
}

#[test]
fn test_decode_large_string_limit() {
    // Length value too large (more than 10 digits)
    assert!(Bencode::decode(b"10000000000:abc").is_err());
    // Safe limit check (16MB+)
    assert!(Bencode::decode(b"20000000:abc").is_err());
}

#[test]
fn test_decode_large_integer_limit() {
    // Too many digits (more than 20)
    assert!(Bencode::decode(b"i123456789012345678901e").is_err());
}

#[test]
fn test_get_dictionary_entry_non_dict() {
    let list_node = BNode::List(vec![BNode::Number(b"42")]);
    assert_eq!(Bencode::get_dictionary_entry(&list_node, b"key"), None);
    assert_eq!(Bencode::get_dictionary_entry_string(&list_node, "key"), None);

    let num_node = BNode::Number(b"123");
    assert_eq!(Bencode::get_dictionary_entry(&num_node, b"key"), None);
    assert_eq!(Bencode::get_dictionary_entry_string(&num_node, "key"), None);

    let str_node = BNode::String(b"value");
    assert_eq!(Bencode::get_dictionary_entry(&str_node, b"key"), None);
    assert_eq!(Bencode::get_dictionary_entry_string(&str_node, "key"), None);
}

