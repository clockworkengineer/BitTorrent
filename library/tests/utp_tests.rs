use bittorrent_rs::io_traits::AsyncSocket;
use bittorrent_rs::network::utp::{
    seq_acked, DelayTracker, LedbatCongestionController, UtpHeader, UtpPacketType,
    UtpSocketAdapter,
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
    assert!(fast_retransmit, "Expected 3rd duplicate ACK to trigger fast retransmit");
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
