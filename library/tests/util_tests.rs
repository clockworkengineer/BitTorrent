use bittorrent_rs::average::Average;
use bittorrent_rs::util::{info_hash_to_string, pack_u32, pack_u64, unpack_u32, unpack_u64, get_bitfield_index_and_mask, acquire_buffer};

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

#[test]
fn test_pack_unpack_u32_with_offset() {
    let mut buffer = vec![0u8; 10];
    let value = 0x12345678u32;
    let packed = pack_u32(value);
    buffer[4..8].copy_from_slice(&packed);
    assert_eq!(unpack_u32(&buffer, 4), value);
}

#[test]
fn test_pack_unpack_u64_with_offset() {
    let mut buffer = vec![0u8; 20];
    let value = 0x0123456789ABCDEFu64;
    let packed = pack_u64(value);
    buffer[6..14].copy_from_slice(&packed);
    assert_eq!(unpack_u64(&buffer, 6), value);
}

#[test]
fn test_get_bitfield_index_and_mask() {
    assert_eq!(get_bitfield_index_and_mask(0), (0, 0x80));
    assert_eq!(get_bitfield_index_and_mask(4), (0, 0x08));
    assert_eq!(get_bitfield_index_and_mask(7), (0, 0x01));
    assert_eq!(get_bitfield_index_and_mask(8), (1, 0x80));
    assert_eq!(get_bitfield_index_and_mask(13), (1, 0x04));
}

#[test]
fn test_static_buffer_pool() {
    let mut buffers = Vec::new();
    
    // Acquire all 8 buffers
    for _ in 0..8 {
        let buf = acquire_buffer();
        assert!(buf.is_some());
        buffers.push(buf.unwrap());
    }
    
    // The 9th acquire must fail (return None)
    let extra_buf = acquire_buffer();
    assert!(extra_buf.is_none());
    
    // Mutate one of the buffers to verify it is accessible
    let active_slice = buffers[0].as_mut();
    active_slice[0] = 99;
    assert_eq!(active_slice[0], 99);
    
    // Drop one buffer and reclaim it
    drop(buffers.pop());
    
    let reclaimed = acquire_buffer();
    assert!(reclaimed.is_some());
}

