use bittorrent_rs::{BencodeToken, BencodeTokenizer};

#[test]
fn test_bencode_tokenizer_simple() {
    let data = b"d3:bar4:spam3:fooi42ee";
    let mut tokenizer = BencodeTokenizer::new(data);

    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::DictStart);
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::String(b"bar"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::String(b"spam"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::String(b"foo"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::Integer(b"42"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::End);
    assert!(tokenizer.next().is_none());
}

#[test]
fn test_bencode_tokenizer_list() {
    let data = b"li1ei2e3:strd3:keyi3eee";
    let mut tokenizer = BencodeTokenizer::new(data);

    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::ListStart);
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::Integer(b"1"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::Integer(b"2"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::String(b"str"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::DictStart);
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::String(b"key"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::Integer(b"3"));
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::End); // end of dict
    assert_eq!(tokenizer.next().unwrap().unwrap(), BencodeToken::End); // end of list
    assert!(tokenizer.next().is_none());
}
