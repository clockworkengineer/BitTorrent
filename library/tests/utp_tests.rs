use bittorrent_rs::io_traits::AsyncSocket;
use bittorrent_rs::network::utp::{
    DelayTracker, LedbatCongestionController, UTP_EXT_SACK, UtpExtension, UtpHeader, UtpPacketType,
    UtpSocketAdapter, build_sack_bitmask, is_seq_sacked, parse_sacked_seqs, seq_acked,
};
use std::net::UdpSocket;
use std::sync::Arc;

#[test]
fn test_utp_header_encode_decode_roundtrip() {
    let original = UtpHeader {
        packet_type: UtpPacketType::Data,
        version: 1,
        extension: 0,
        connection_id: 0xABCD,
        timestamp_us: 123_456_789,
        timestamp_difference_us: 42_000,
        wnd_size: 1_048_576,
        seq_nr: 101,
        ack_nr: 100,
        extensions: Vec::new(),
    };

    let encoded = original.encode();
    assert_eq!(encoded.len(), 20);

    let decoded = UtpHeader::decode(&encoded).expect("Failed to decode valid header");
    assert_eq!(decoded.packet_type, UtpPacketType::Data);
    assert_eq!(decoded.version, 1);
    assert_eq!(decoded.extension, 0);
    assert_eq!(decoded.connection_id, 0xABCD);
    assert_eq!(decoded.timestamp_us, 123_456_789);
    assert_eq!(decoded.timestamp_difference_us, 42_000);
    assert_eq!(decoded.wnd_size, 1_048_576);
    assert_eq!(decoded.seq_nr, 101);
    assert_eq!(decoded.ack_nr, 100);
    assert!(decoded.extensions.is_empty());
}

#[test]
fn test_utp_header_packet_types_and_error_handling() {
    let types = [
        (UtpPacketType::Data, 0u8),
        (UtpPacketType::Ack, 1u8),
        (UtpPacketType::Syn, 2u8),
        (UtpPacketType::Reset, 3u8),
        (UtpPacketType::State, 4u8),
    ];

    for (pt, expected_nibble) in types {
        assert_eq!(UtpPacketType::from_u8(expected_nibble), Some(pt));
        let hdr = UtpHeader {
            packet_type: pt,
            version: 1,
            extension: 0,
            connection_id: 1,
            timestamp_us: 100,
            timestamp_difference_us: 50,
            wnd_size: 65536,
            seq_nr: 1,
            ack_nr: 0,
            extensions: Vec::new(),
        };
        let encoded = hdr.encode();
        assert_eq!(encoded[0] >> 4, expected_nibble);
        let decoded = UtpHeader::decode(&encoded).expect("Decode failed");
        assert_eq!(decoded.packet_type, pt);
    }

    // Invalid packet type (e.g. 5)
    assert_eq!(UtpPacketType::from_u8(5), None);
    let mut invalid_buf = [0u8; 20];
    invalid_buf[0] = 0x51; // type 5, version 1
    assert!(UtpHeader::decode(&invalid_buf).is_err());

    // Truncated header
    assert!(UtpHeader::decode(&invalid_buf[..19]).is_err());
}

#[test]
fn test_seq_acked_circular_arithmetic() {
    // Normal sequential progression
    assert!(seq_acked(1, 1));
    assert!(seq_acked(1, 2));
    assert!(seq_acked(1, 100));
    assert!(!seq_acked(2, 1));
    assert!(!seq_acked(100, 1));

    // Circular 16-bit wrap-around
    assert!(seq_acked(65535, 0));
    assert!(seq_acked(65535, 1));
    assert!(seq_acked(65534, 1));
    assert!(!seq_acked(1, 65535));
    assert!(!seq_acked(2, 65535));
}

#[test]
fn test_delay_tracker_sliding_window_and_base_delay() {
    // 4 buckets of 60 seconds (60_000_000 us) each
    let mut tracker = DelayTracker::new(60_000_000);
    assert_eq!(tracker.base_delay(), u32::MAX);

    // Initial samples at t=0
    tracker.add_sample(50_000, 0); // 50ms
    tracker.add_sample(40_000, 1_000_000); // 40ms minimum
    tracker.add_sample(60_000, 2_000_000); // 60ms
    assert_eq!(tracker.base_delay(), 40_000);
    assert_eq!(tracker.last_sample(), 60_000);

    // Advance to bucket 1 at t=65s
    tracker.add_sample(45_000, 65_000_000);
    assert_eq!(tracker.base_delay(), 40_000); // Still 40ms from bucket 0

    // Advance to bucket 2 at t=130s with new lowest delay
    tracker.add_sample(30_000, 130_000_000);
    assert_eq!(tracker.base_delay(), 30_000); // New global base delay 30ms

    // Advance to bucket 3 at t=190s
    tracker.add_sample(35_000, 190_000_000);
    assert_eq!(tracker.base_delay(), 30_000);

    // Advance to bucket 0 (overwrites original bucket 0) at t=250s with high delay (e.g. route switch)
    tracker.add_sample(80_000, 250_000_000);
    // Base delay is still 30_000 from bucket 2
    assert_eq!(tracker.base_delay(), 30_000);

    // After further advances, overwriting bucket 2:
    tracker.add_sample(75_000, 310_000_000); // overwrites bucket 1
    tracker.add_sample(70_000, 375_000_000); // overwrites bucket 2 (which had 30_000)
    // The old 30ms base delay has now expired! Minimum is now 35ms (bucket 3) or 70ms
    assert_eq!(tracker.base_delay(), 35_000);
}

#[test]
fn test_ledbat_congestion_controller_window_scaling() {
    let mut ctrl = LedbatCongestionController::new();
    let initial_cwnd = ctrl.cwnd();
    assert_eq!(initial_cwnd, 2 * 1460); // 2920 bytes

    // 1. Establish base delay of 20ms (20_000 µs)
    let t0 = 1_000_000u64;
    ctrl.on_ack(1460, 20_000, t0);
    assert_eq!(ctrl.base_delay(), 20_000);

    // 2. Low queuing delay: current delay is 40ms -> queuing delay is 20ms.
    // Target delay is 100ms. Since 20ms < 100ms, cwnd should INCREASE.
    let prev_cwnd = ctrl.cwnd();
    ctrl.on_ack(1460, 40_000, t0 + 10_000);
    assert!(
        ctrl.cwnd() > prev_cwnd,
        "Expected cwnd to grow: prev={}, new={}",
        prev_cwnd,
        ctrl.cwnd()
    );
    assert_eq!(ctrl.queuing_delay(), 20_000);

    // 3. High queuing delay: current delay is 180ms -> queuing delay is 160ms.
    // Since 160ms > 100ms target, cwnd should DECREASE.
    let high_cwnd = ctrl.cwnd();
    ctrl.on_ack(1460, 180_000, t0 + 20_000);
    assert!(
        ctrl.cwnd() < high_cwnd,
        "Expected cwnd to shrink: high={}, new={}",
        high_cwnd,
        ctrl.cwnd()
    );
    assert_eq!(ctrl.queuing_delay(), 160_000);
}

#[test]
fn test_ledbat_loss_backoff_and_fast_retransmit() {
    let mut ctrl = LedbatCongestionController::new();
    ctrl.set_cwnd(20_000);
    assert_eq!(ctrl.cwnd(), 20_000);

    // Multiplicative decrease cuts cwnd in half
    ctrl.on_loss();
    assert_eq!(ctrl.cwnd(), 10_000);

    ctrl.on_loss();
    assert_eq!(ctrl.cwnd(), 5_000);

    ctrl.on_loss();
    // 5000 / 2 = 2500, but MIN_CWND is 2920 bytes (2 * MSS)
    assert_eq!(ctrl.cwnd(), 2920);

    // Fast Retransmit on 3 duplicate ACKs
    ctrl.set_cwnd(16_000);
    // Establish initial ACK for seq 50
    assert!(!ctrl.handle_ack(50, 1460, 20_000, 100));
    ctrl.set_cwnd(16_000); // reset to 16_000 after growth

    // Duplicate ACK 1
    assert!(!ctrl.handle_ack(50, 0, 20_000, 101));
    assert_eq!(ctrl.cwnd(), 16_000);
    // Duplicate ACK 2
    assert!(!ctrl.handle_ack(50, 0, 20_000, 102));
    assert_eq!(ctrl.cwnd(), 16_000);
    // Duplicate ACK 3 -> Fast Retransmit triggers loss backoff!
    let fast_retransmit = ctrl.handle_ack(50, 0, 20_000, 103);
    assert!(
        fast_retransmit,
        "Expected 3rd duplicate ACK to trigger fast retransmit"
    );
    assert_eq!(ctrl.cwnd(), 8_000);
}

#[test]
fn test_ledbat_effective_window_and_can_send() {
    let mut ctrl = LedbatCongestionController::new();
    ctrl.set_cwnd(10_000);
    ctrl.set_remote_window(8_000);

    // Effective window is min(cwnd, remote_wnd_size)
    assert_eq!(ctrl.effective_window(), 8_000);
    assert_eq!(ctrl.available_window(5_000), 3_000);
    assert!(ctrl.can_send(7_999));
    assert!(!ctrl.can_send(8_000));
    assert!(!ctrl.can_send(9_000));
}

#[test]
fn test_utp_socket_adapter_loopback_transfer() {
    // Create two bound UDP sockets connected to each other on loopback
    let udp_a = UdpSocket::bind("127.0.0.1:0").expect("Bind A failed");
    let udp_b = UdpSocket::bind("127.0.0.1:0").expect("Bind B failed");

    let addr_a = udp_a.local_addr().unwrap();
    let addr_b = udp_b.local_addr().unwrap();

    udp_a.connect(addr_b).unwrap();
    udp_b.connect(addr_a).unwrap();

    let adapter_a = Arc::new(UtpSocketAdapter::from_raw_socket(udp_a, 100));
    let adapter_b = Arc::new(UtpSocketAdapter::from_raw_socket(udp_b, 100));

    let test_data = b"Hello, uTP LEDBAT Congestion Control!";

    let a_clone = adapter_a.clone();
    let handle_write = std::thread::spawn(move || {
        futures::executor::block_on(async {
            a_clone.write(test_data).await.expect("Write failed");
        });
    });

    let b_clone = adapter_b.clone();
    let handle_read = std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut read_buf = vec![0u8; 128];
            let n = b_clone.read(&mut read_buf).await.expect("Read failed");
            assert_eq!(&read_buf[..n], test_data);
        });
    });

    handle_write.join().unwrap();
    handle_read.join().unwrap();

    // Verify adapter status
    assert!(adapter_a.cwnd() >= 2920);
    assert!(adapter_b.cwnd() >= 2920);

    // Clean close
    adapter_a.close();
    adapter_b.close();
}

#[test]
fn test_sack_bitmask_encode_and_is_sacked() {
    let ack_nr = 100u16;
    // Packets received out-of-order: ack_nr + 2 (102), ack_nr + 5 (105), ack_nr + 10 (110)
    let received = [102u16, 105, 110];
    let bitmask = build_sack_bitmask(ack_nr, &received);

    // BEP 29 requires SACK bitmask length to be at least 4 bytes and a multiple of 4
    assert!(bitmask.len() >= 4);
    assert_eq!(bitmask.len() % 4, 0);

    // Byte 0:
    // bit 0 (1 << 0) represents 102 (100 + 2) -> set
    // bit 3 (1 << 3) represents 105 (100 + 5) -> set
    assert_ne!(bitmask[0] & (1 << 0), 0);
    assert_ne!(bitmask[0] & (1 << 3), 0);

    // Byte 1:
    // bit 0 (1 << 0) represents 110 (100 + 10) -> set
    assert_ne!(bitmask[1] & (1 << 0), 0);

    // Verification via is_seq_sacked
    assert!(is_seq_sacked(ack_nr, &bitmask, 102));
    assert!(is_seq_sacked(ack_nr, &bitmask, 105));
    assert!(is_seq_sacked(ack_nr, &bitmask, 110));

    // Non-sacked sequence numbers
    assert!(!is_seq_sacked(ack_nr, &bitmask, 100)); // ack_nr itself is cumulatively acked, not sacked
    assert!(!is_seq_sacked(ack_nr, &bitmask, 101)); // ack_nr + 1 is the missing packet
    assert!(!is_seq_sacked(ack_nr, &bitmask, 103));
    assert!(!is_seq_sacked(ack_nr, &bitmask, 104));
    assert!(!is_seq_sacked(ack_nr, &bitmask, 106));
    assert!(!is_seq_sacked(ack_nr, &bitmask, 111));

    // parse_sacked_seqs reconstructs exactly the received seq numbers
    let parsed = parse_sacked_seqs(ack_nr, &bitmask);
    assert_eq!(parsed, vec![102, 105, 110]);
}

#[test]
fn test_sack_sequence_wraparound() {
    let ack_nr = 65534u16;
    // In 16-bit space, 65534 + 2 wraps to 0, 65534 + 3 wraps to 1, 65534 + 5 wraps to 3
    let received = [0u16, 1, 3];
    let bitmask = build_sack_bitmask(ack_nr, &received);

    assert_eq!(bitmask.len(), 4);
    assert!(is_seq_sacked(ack_nr, &bitmask, 0));
    assert!(is_seq_sacked(ack_nr, &bitmask, 1));
    assert!(is_seq_sacked(ack_nr, &bitmask, 3));

    assert!(!is_seq_sacked(ack_nr, &bitmask, 65534)); // ack_nr
    assert!(!is_seq_sacked(ack_nr, &bitmask, 65535)); // missing packet ack_nr + 1
    assert!(!is_seq_sacked(ack_nr, &bitmask, 2));

    let parsed = parse_sacked_seqs(ack_nr, &bitmask);
    assert_eq!(parsed, vec![0, 1, 3]);
}

#[test]
fn test_utp_header_with_sack_extension_roundtrip() {
    let sack_mask = vec![0x05, 0x01, 0x00, 0x00]; // 4 bytes
    let header = UtpHeader {
        packet_type: UtpPacketType::State,
        version: 1,
        extension: UTP_EXT_SACK,
        connection_id: 0x55AA,
        timestamp_us: 500_000,
        timestamp_difference_us: 10_000,
        wnd_size: 1_048_576,
        seq_nr: 10,
        ack_nr: 5,
        extensions: vec![UtpExtension::SelectiveAck(sack_mask.clone())],
    };

    let encoded = header.encode();
    // 20 bytes base + 2 bytes extension header + 4 bytes mask = 26 bytes
    assert_eq!(encoded.len(), 26);
    assert_eq!(encoded[1], UTP_EXT_SACK); // Header extension field
    assert_eq!(encoded[20], 0); // Next extension = 0 (none)
    assert_eq!(encoded[21], 4); // Extension length = 4
    assert_eq!(&encoded[22..26], &sack_mask[..]);

    let decoded = UtpHeader::decode(&encoded).expect("Decode with SACK failed");
    assert_eq!(decoded.packet_type, UtpPacketType::State);
    assert_eq!(decoded.connection_id, 0x55AA);
    assert_eq!(decoded.extension, UTP_EXT_SACK);
    assert_eq!(decoded.extensions.len(), 1);
    assert_eq!(decoded.sack_bitmask(), Some(sack_mask.as_slice()));
    assert_eq!(decoded.total_header_len(), 26);

    // Sequence 5 + 2 = 7 is bit 0 of byte 0 (0x05 has bit 0 set)
    assert!(decoded.is_seq_sacked(7));
    // Sequence 5 + 4 = 9 is bit 2 of byte 0 (0x05 has bit 2 set)
    assert!(decoded.is_seq_sacked(9));
    // Sequence 5 + 10 = 15 is bit 0 of byte 1 (0x01 has bit 0 set)
    assert!(decoded.is_seq_sacked(15));
    // Sequence 5 + 3 = 8 is not set
    assert!(!decoded.is_seq_sacked(8));
}

#[test]
fn test_utp_multiple_extension_chain() {
    let sack_mask = vec![0x01, 0x00, 0x00, 0x00];
    let custom_data = vec![0xDE, 0xAD];
    let header = UtpHeader {
        packet_type: UtpPacketType::Data,
        version: 1,
        extension: UTP_EXT_SACK,
        connection_id: 42,
        timestamp_us: 100,
        timestamp_difference_us: 50,
        wnd_size: 65536,
        seq_nr: 1,
        ack_nr: 1,
        extensions: vec![
            UtpExtension::SelectiveAck(sack_mask.clone()),
            UtpExtension::Unknown {
                extension_type: 2,
                data: custom_data.clone(),
            },
        ],
    };

    let encoded = header.encode();
    // 20 (base) + 2 + 4 (ext 1) + 2 + 2 (ext 2) = 30 bytes
    assert_eq!(encoded.len(), 30);

    let decoded = UtpHeader::decode(&encoded).expect("Decode chained extensions failed");
    assert_eq!(decoded.extensions.len(), 2);
    assert_eq!(decoded.extensions[0], UtpExtension::SelectiveAck(sack_mask));
    assert_eq!(
        decoded.extensions[1],
        UtpExtension::Unknown {
            extension_type: 2,
            data: custom_data
        }
    );
    assert_eq!(decoded.total_header_len(), 30);
}

#[test]
fn test_utp_socket_adapter_sack_processing_avoids_retransmit() {
    let udp_a = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_b = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr_a = udp_a.local_addr().unwrap();
    let addr_b = udp_b.local_addr().unwrap();

    udp_a.connect(addr_b).unwrap();
    udp_b.connect(addr_a).unwrap();

    let adapter_a = Arc::new(UtpSocketAdapter::from_raw_socket(udp_a, 500));

    // Send 3 data packets from A: seq_nr 1, 2, 3
    futures::executor::block_on(async {
        adapter_a.write(b"Packet 1").await.unwrap();
        adapter_a.write(b"Packet 2").await.unwrap();
        adapter_a.write(b"Packet 3").await.unwrap();
    });

    assert_eq!(adapter_a.bytes_in_flight(), 24); // 8 bytes * 3
    assert_eq!(adapter_a.sacked_packets_count(), 0);

    // Simulate B receiving Packet 1 and Packet 3, but missing Packet 2.
    // B sends a ST_STATE acknowledging packet 1, with a SACK bitmask for packet 3 (1 + 2 = 3).
    let sack_mask = build_sack_bitmask(1, &[3]);
    let state_header = UtpHeader {
        packet_type: UtpPacketType::State,
        version: 1,
        extension: UTP_EXT_SACK,
        connection_id: 500,
        timestamp_us: 1000,
        timestamp_difference_us: 100,
        wnd_size: 1_048_576,
        seq_nr: 1,
        ack_nr: 1,
        extensions: vec![UtpExtension::SelectiveAck(sack_mask)],
    };

    udp_b.send(&state_header.encode()).unwrap();

    // Give adapter A a read opportunity to process the incoming ST_STATE ACK
    let a_clone = adapter_a.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut buf = [0u8; 16];
            let _ = a_clone.read(&mut buf).await;
        });
    });

    std::thread::sleep(std::time::Duration::from_millis(50));

    // Packet 1 is cumulatively ACKed (removed).
    // Packet 3 is SACKed (marked sacked = true, excluded from bytes_in_flight).
    // Packet 2 remains un-sacked (in flight / needs retransmission).
    assert_eq!(adapter_a.sacked_packets_count(), 1);
    assert_eq!(adapter_a.bytes_in_flight(), 8); // only Packet 2 is in flight!

    adapter_a.close();
}

#[test]
fn test_utp_socket_adapter_out_of_order_reassembly_and_sack_reply() {
    let udp_sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr_s = udp_sender.local_addr().unwrap();
    let addr_r = udp_receiver.local_addr().unwrap();

    udp_sender.connect(addr_r).unwrap();
    udp_receiver.connect(addr_s).unwrap();

    let adapter_s = Arc::new(UtpSocketAdapter::from_raw_socket(udp_sender, 800));
    let adapter_r = Arc::new(UtpSocketAdapter::from_raw_socket(udp_receiver, 800));

    // Adapter R starts reading in the background
    let r_clone = adapter_r.clone();
    let read_handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut buf = vec![0u8; 9];
            let mut total = 0;
            while total < 9 {
                let n = r_clone.read(&mut buf[total..]).await.unwrap();
                if n == 0 {
                    break;
                }
                total += n;
            }
            (total, buf)
        })
    });

    std::thread::sleep(std::time::Duration::from_millis(20));

    // Sender transmits Packet 1 (seq 1)
    adapter_s.set_seq_nr(1);
    adapter_s.send_packet(UtpPacketType::Data, b"AAA").unwrap();

    std::thread::sleep(std::time::Duration::from_millis(30));

    // Sender transmits Packet 3 (seq 3) OUT OF ORDER (packet 2 delayed)
    adapter_s.set_seq_nr(3);
    adapter_s.send_packet(UtpPacketType::Data, b"CCC").unwrap();

    // Wait for receiver to process Packet 3 into the out-of-order buffer
    let start = std::time::Instant::now();
    while adapter_r.out_of_order_count() == 0 && start.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Verify receiver buffered the out-of-order Packet 3
    assert_eq!(adapter_r.out_of_order_count(), 1);

    // Let sender process incoming ST_STATE ACK with SACK bitmask
    let s_clone = adapter_s.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut buf = [0u8; 16];
            let _ = s_clone.read(&mut buf).await;
        });
    });

    let start = std::time::Instant::now();
    while adapter_s.sacked_packets_count() == 0
        && start.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        adapter_s.sacked_packets_count(),
        1,
        "Packet 3 should be marked as SACKed on sender"
    );

    // Now sender delivers the missing Packet 2 (seq 2)
    adapter_s.set_seq_nr(2);
    adapter_s.send_packet(UtpPacketType::Data, b"BBB").unwrap();

    let (total_read, read_data) = read_handle.join().unwrap();
    assert_eq!(total_read, 9);
    // Verified contiguous reassembly of AAA, BBB, CCC in correct sequential order
    assert_eq!(&read_data[..9], b"AAABBBCCC");
    assert_eq!(adapter_r.out_of_order_count(), 0);

    adapter_s.close();
    adapter_r.close();
}
