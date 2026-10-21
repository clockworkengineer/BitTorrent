//! Generic behavioral contract test harness for `BlockStorage` implementors (Liskov Substitution Principle).
//!
//! Enforces uniform behavioral invariants across `MemStorage`, `DiskBlockStorage`,
//! and `CachedBlockStorage`:
//! 1. Zero-length reads and writes are safe no-ops.
//! 2. In-bounds writes can be faithfully read back.
//! 3. Out-of-bounds reads and writes return errors without panicking (either immediately or on flush for write-back caches).
//! 4. Integer overflow in offset + length is safely detected and rejected.
//! 5. Flushing is idempotent across repeated calls.
//! 6. Discarding arbitrary ranges is safe and non-panicking.

use bittorrent_rs::BlockStorage;

/// Verifies that any `BlockStorage` implementation satisfies the complete LSP behavioral contract.
#[allow(dead_code)]
pub fn verify_block_storage_contract<S: BlockStorage>(storage: &S, capacity: u64) {
    // 1. Zero-length I/O safety
    let mut empty_buf = [0u8; 0];
    assert_eq!(
        storage
            .read_block(0, &mut empty_buf)
            .expect("0-byte read succeeds"),
        0
    );
    storage.write_block(0, &[]).expect("0-byte write succeeds");

    // 2. In-bounds read / write roundtrip
    if capacity >= 16 {
        let test_pattern: [u8; 16] = [
            0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08,
        ];
        storage
            .write_block(0, &test_pattern)
            .expect("writing at offset 0 succeeds");
        storage.flush().expect("flush after write succeeds");

        let mut read_back = [0u8; 16];
        let bytes_read = storage
            .read_block(0, &mut read_back)
            .expect("reading back offset 0 succeeds");
        assert_eq!(bytes_read, 16);
        assert_eq!(read_back, test_pattern);

        // Interior slice write
        let interior_pattern: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];
        storage
            .write_block(4, &interior_pattern)
            .expect("interior write succeeds");
        storage.flush().expect("flush succeeds");

        let mut interior_read = [0u8; 4];
        let bytes_read = storage
            .read_block(4, &mut interior_read)
            .expect("interior read succeeds");
        assert_eq!(bytes_read, 4);
        assert_eq!(interior_read, interior_pattern);
    }

    // 3. Out-of-bounds rejection
    let oob_offset = capacity + 1;
    let mut oob_read_buf = [0u8; 8];
    assert!(
        storage.read_block(oob_offset, &mut oob_read_buf).is_err(),
        "read beyond capacity must fail"
    );
    let write_res = storage.write_block(oob_offset, &[1, 2, 3]);
    let flush_res = storage.flush();
    assert!(
        write_res.is_err() || flush_res.is_err(),
        "write beyond capacity must fail either immediately or upon flush"
    );

    // Straddling capacity boundary
    if capacity > 0 {
        let straddle_offset = capacity - 1;
        let mut straddle_buf = [0u8; 2];
        assert!(
            storage
                .read_block(straddle_offset, &mut straddle_buf)
                .is_err(),
            "read straddling capacity boundary must fail"
        );
        let write_res = storage.write_block(straddle_offset, &[0xFF, 0xEE]);
        let flush_res = storage.flush();
        assert!(
            write_res.is_err() || flush_res.is_err(),
            "write straddling capacity boundary must fail either immediately or upon flush"
        );
    }

    // 4. Overflow arithmetic protection
    let overflow_offset = u64::MAX - 5;
    let mut overflow_buf = [0u8; 10];
    assert!(
        storage
            .read_block(overflow_offset, &mut overflow_buf)
            .is_err(),
        "overflowing read offset must fail safely without panicking"
    );
    let write_res = storage.write_block(overflow_offset, &[1; 10]);
    let flush_res = storage.flush();
    assert!(
        write_res.is_err() || flush_res.is_err(),
        "overflowing write offset must fail safely without panicking"
    );

    // 5. Idempotent flushing
    // Discard any lingering dirty out-of-bounds blocks so clean flushing can be tested
    storage.discard_range(0, u64::MAX);
    for _ in 0..3 {
        assert!(
            storage.flush().is_ok(),
            "multiple flush calls must be idempotent"
        );
    }

    // 6. Safe discard range
    storage.discard_range(0, capacity);
    storage.discard_range(capacity + 100, 500);
}
