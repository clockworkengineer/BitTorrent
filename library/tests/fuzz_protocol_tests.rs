//! Adversarial Fuzz and Security Boundary Tests
//!
//! Validates Attribute 7 and Phase 5 Deliverables:
//! Ensures Bencode parser and BitTorrent wire protocol decoder never panic on
//! corrupted, malicious, deeply nested, truncated, or random fuzz payloads.

use bittorrent_rs::bencode::Bencode;
use bittorrent_rs::peer_message::PeerMessage;

/// Pseudo-random number generator (xorshift64) for deterministic, fast property fuzzing.
struct FuzzRng {
    state: u64,
}

impl FuzzRng {
    fn new(seed: u64) -> Self {
        FuzzRng {
            state: if seed == 0 { 0xDEADBEEFCAFE1234 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_u8(&mut self) -> u8 {
        (self.next_u64() & 0xFF) as u8
    }

    fn gen_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(len);
        for _ in 0..len {
            v.push(self.next_u8());
        }
        v
    }
}

#[test]
fn test_fuzz_bencode_random_bytes_no_panic() {
    let mut rng = FuzzRng::new(0x1337);

    for _ in 0..10_000 {
        let len = (rng.next_u64() % 256) as usize;
        let payload = rng.gen_bytes(len);

        // Parser must cleanly succeed or return an Err, never panic
        let _ = Bencode::decode(&payload);
    }
}

#[test]
fn test_fuzz_bencode_deep_nesting_depth_limit() {
    // Construct deeply nested list structure: "l"*2000 + "i42e" + "e"*2000
    let mut malicious = vec![b'l'; 2000];
    malicious.extend_from_slice(b"i42e");
    malicious.resize(malicious.len() + 2000, b'e');

    // Must return NestingDepthExceeded or Parse error without stack overflow
    let res = Bencode::decode(&malicious);
    assert!(res.is_err(), "Deep nesting payload must fail safely");
}

#[test]
fn test_fuzz_bencode_malformed_integers() {
    let adversarial_cases: &[&[u8]] = &[
        b"i",
        b"ie",
        b"i-e",
        b"i--5e",
        b"i00e",
        b"i-0e",
        b"i123456789012345678901234567890e", // Overflow i64
        b"i999999999999999999999999999999e",
        b"i+42e",
        b"i 42e",
        b"i42 e",
        b"i42\0e",
    ];

    for &case in adversarial_cases {
        let res = Bencode::decode(case);
        assert!(res.is_err(), "Malformed integer {:?} must be rejected", String::from_utf8_lossy(case));
    }
}

#[test]
fn test_fuzz_bencode_string_length_overflow_and_truncation() {
    let malicious_strings: &[&[u8]] = &[
        b"999999999999999999999999:abc", // Exceeds usize
        b"1000000:short",                 // Declares 1MB, only 5 bytes present
        b"-5:hello",                      // Negative length
        b"00:hello",                      // Leading zero
        b"3:",                            // Missing payload entirely
        b":",                             // Missing length and payload
    ];

    for &case in malicious_strings {
        let res = Bencode::decode(case);
        assert!(res.is_err(), "Malformed string {:?} must be rejected", String::from_utf8_lossy(case));
    }
}

#[test]
fn test_fuzz_wire_message_random_payloads_no_panic() {
    let mut rng = FuzzRng::new(0xBEEF);

    for _ in 0..10_000 {
        let len = (rng.next_u64() % 128) as usize;
        let payload = rng.gen_bytes(len);

        // Decoder must cleanly decode or return Err, never panic
        let _ = PeerMessage::decode(&payload);
    }
}

#[test]
fn test_fuzz_wire_message_boundary_edge_cases() {
    // 1. Empty buffer returns KeepAlive
    assert_eq!(PeerMessage::decode(&[]).unwrap(), PeerMessage::KeepAlive);

    // 2. Choke, Unchoke, Interested, NotInterested
    assert_eq!(PeerMessage::decode(&[0]).unwrap(), PeerMessage::Choke);
    assert_eq!(PeerMessage::decode(&[1]).unwrap(), PeerMessage::Unchoke);
    assert_eq!(PeerMessage::decode(&[2]).unwrap(), PeerMessage::Interested);
    assert_eq!(PeerMessage::decode(&[3]).unwrap(), PeerMessage::NotInterested);

    // 3. Have with truncated index payload
    assert!(PeerMessage::decode(&[4, 0, 1]).is_err()); // Only 2 bytes, needs 4

    // 4. Request with truncated payload
    assert!(PeerMessage::decode(&[6, 0, 0, 0, 1, 0, 0]).is_err());

    // 5. Piece with truncated payload (<8 bytes)
    assert!(PeerMessage::decode(&[7, 0, 0, 0, 1]).is_err());

    // 6. Unknown message IDs
    assert!(PeerMessage::decode(&[99, 1, 2, 3]).is_err());
    assert!(PeerMessage::decode(&[255]).is_err());
}

#[test]
fn test_fuzz_handshake_malformed_and_truncated() {
    let malformed_handshakes: &[&[u8]] = &[
        &[],
        &[19],
        b"\x13BitTorrent protocol", // 20 bytes, missing 48 trailing bytes
        b"\x12WrongProtocolName000000000000000000000000000000000000000000000000",
        &[0; 68], // Zero length prefix
    ];

    for &case in malformed_handshakes {
        let res = PeerMessage::handshake_decode(case);
        assert!(res.is_err(), "Malformed handshake must be rejected safely");
    }
}
