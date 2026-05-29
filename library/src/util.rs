pub fn pack_u32(value: u32) -> [u8; 4] {
    [
        (value >> 24) as u8,
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ]
}

pub fn unpack_u32(buffer: &[u8], offset: usize) -> u32 {
    ((buffer[offset] as u32) << 24)
        | ((buffer[offset + 1] as u32) << 16)
        | ((buffer[offset + 2] as u32) << 8)
        | (buffer[offset + 3] as u32)
}

pub fn pack_u64(value: u64) -> [u8; 8] {
    [
        (value >> 56) as u8,
        (value >> 48) as u8,
        (value >> 40) as u8,
        (value >> 32) as u8,
        (value >> 24) as u8,
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ]
}

pub fn unpack_u64(buffer: &[u8], offset: usize) -> u64 {
    ((buffer[offset] as u64) << 56)
        | ((buffer[offset + 1] as u64) << 48)
        | ((buffer[offset + 2] as u64) << 40)
        | ((buffer[offset + 3] as u64) << 32)
        | ((buffer[offset + 4] as u64) << 24)
        | ((buffer[offset + 5] as u64) << 16)
        | ((buffer[offset + 6] as u64) << 8)
        | (buffer[offset + 7] as u64)
}

pub fn info_hash_to_string(info_hash: &[u8]) -> String {
    info_hash.iter().map(|b| format!("{:02x}", b)).collect()
}
