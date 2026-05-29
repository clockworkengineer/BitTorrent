use bittorrent_rs::average::Average;
use bittorrent_rs::util::{info_hash_to_string, pack_u32, pack_u64, unpack_u32, unpack_u64};

#[test]
fn test_pack_unpack_u32() {
    let value = 0x12345678u32;
    let packed = pack_u32(value);
    assert_eq!(unpack_u32(&packed, 0), value);
}

#[test]
fn test_pack_unpack_u64() {
    let value = 0x0123456789ABCDEFu64;
    let packed = pack_u64(value);
    assert_eq!(unpack_u64(&packed, 0), value);
}

#[test]
fn test_info_hash_to_string() {
    let bytes = [0x00u8, 0xAB, 0xCD, 0xEF];
    assert_eq!(info_hash_to_string(&bytes), "00abcdef");
}

#[test]
fn test_average_add_and_get() {
    let mut average = Average::default();
    assert_eq!(average.get(), 0);

    average.add(10);
    average.add(20);
    average.add(30);
    assert_eq!(average.get(), 20);
}
