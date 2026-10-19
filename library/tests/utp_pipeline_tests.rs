use bittorrent_rs::io_traits::AsyncSocket;
use bittorrent_rs::network::utp::{
    LedbatCongestionController, LossRecoveryState, UtpPacketType, UtpSocketAdapter,
    detect_receive_gaps,
};
use std::net::UdpSocket;
use std::sync::Arc;

#[test]
fn test_detect_receive_gaps_and_sequence_wraparound() {
    // 1. Single missing packet gap: ack = 10, received [12] -> missing 11
    let gaps = detect_receive_gaps(10, &[12]);
    assert_eq!(gaps, vec![(11, 11)]);

    // 2. Multi-packet contiguous gap: ack = 10, received [15] -> missing 11..=14
    let gaps = detect_receive_gaps(10, &[15]);
    assert_eq!(gaps, vec![(11, 14)]);

    // 3. Multiple discontinuous gaps: ack = 10, received [12, 13, 16] -> missing 11 and 14..=15
    let gaps = detect_receive_gaps(10, &[12, 13, 16]);
    assert_eq!(gaps, vec![(11, 11), (14, 15)]);

    // 4. No gaps: empty or contiguous
    assert!(detect_receive_gaps(10, &[]).is_empty());
    assert!(detect_receive_gaps(10, &[11]).is_empty());

    // 5. 16-bit circular sequence wraparound: ack = 65534, received [65535, 1, 3] -> missing 0 and 2
    let gaps = detect_receive_gaps(65534, &[65535, 1, 3]);
    assert_eq!(gaps, vec![(0, 0), (2, 2)]);
}

#[test]
fn test_burst_loss_recovery_and_window_scaling() {
    let mut ctrl = LedbatCongestionController::new();
    ctrl.set_cwnd(30_000);

    // Verify initial normal state
    assert_eq!(ctrl.loss_recovery_state(), LossRecoveryState::Normal);
    assert!(!ctrl.is_in_recovery());

    // 1. Single packet loss entry (cuts to 50%)
    assert!(ctrl.enter_recovery(100, false));
    assert!(ctrl.is_in_recovery());
    assert_eq!(ctrl.cwnd(), 15_000);

    // 2. Redundant loss notifications within the same recovery epoch do NOT repeatedly halve
    assert!(!ctrl.enter_recovery(90, false));
    assert!(!ctrl.enter_recovery(100, false));
    assert_eq!(ctrl.cwnd(), 15_000);

    // 3. Cumulative ACK covering recovery_seq (100) exits recovery
    ctrl.handle_ack(100, 1460, 20_000, 1_000);
    assert_eq!(ctrl.loss_recovery_state(), LossRecoveryState::Normal);
    assert!(!ctrl.is_in_recovery());

    // 4. Burst loss scaling (multi-packet drop preserves 65% of window rather than collapsing to 50%)
    ctrl.set_cwnd(40_000);
    assert!(ctrl.enter_recovery(200, true));
    assert_eq!(ctrl.cwnd(), 26_000); // 40_000 * 0.65 = 26_000

    // Cumulative ACK covering recovery_seq (201 > 200) exits
    ctrl.handle_ack(201, 1460, 20_000, 2_000);
    assert_eq!(ctrl.loss_recovery_state(), LossRecoveryState::Normal);
}

#[test]
fn test_rto_timeout_calculation_and_exponential_backoff() {
    let mut ctrl = LedbatCongestionController::new();

    // Initial RTO defaults to 1,000,000 µs (1 second) per RFC 6298
    assert_eq!(ctrl.rto_us(), 1_000_000);
    assert!(ctrl.srtt_us().is_none());

    // First RTT sample: 80 ms (80,000 µs)
    ctrl.update_rtt(80_000);
    assert_eq!(ctrl.srtt_us(), Some(80_000));
    assert_eq!(ctrl.rttvar_us(), 40_000);
    // rto = srtt + 4 * rttvar = 80_000 + 160_000 = 240_000 clamped to MIN_RTO (500_000)
    assert_eq!(ctrl.rto_us(), 500_000);

    // High latency sample: 600 ms (600_000 µs)
    ctrl.update_rtt(600_000);
    assert!(ctrl.srtt_us().unwrap() > 80_000);
    assert!(ctrl.rto_us() >= 500_000);

    // Exponential timeout backoff
    let prev_rto = ctrl.rto_us();
    ctrl.on_rto_timeout();
    assert_eq!(ctrl.rto_us(), prev_rto * 2);

    // Capped at MAX_RTO (3,000,000 µs = 3 seconds)
    ctrl.on_rto_timeout();
    ctrl.on_rto_timeout();
    ctrl.on_rto_timeout();
    assert_eq!(ctrl.rto_us(), 3_000_000);
}

#[test]
fn test_bounded_out_of_order_reassembly_capacity() {
    let udp_s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_r = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr_s = udp_s.local_addr().unwrap();
    let addr_r = udp_r.local_addr().unwrap();

    udp_s.connect(addr_r).unwrap();
    udp_r.connect(addr_s).unwrap();

    let adapter_s = Arc::new(UtpSocketAdapter::from_raw_socket(udp_s, 901));
    let mut adapter_r = UtpSocketAdapter::from_raw_socket(udp_r, 901);
    // Restrict receiver capacity to 3 out-of-order packets
    adapter_r.set_max_out_of_order_packets(3);
    let adapter_r = Arc::new(adapter_r);

    // Start background reader
    let r_clone = adapter_r.clone();
    let handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut buf = vec![0u8; 100];
            let _ = r_clone.read(&mut buf).await;
        })
    });

    std::thread::sleep(std::time::Duration::from_millis(20));

    // Send packet 1 (in-order)
    adapter_s.set_seq_nr(1);
    adapter_s.send_packet(UtpPacketType::Data, b"pkt1").unwrap();

    std::thread::sleep(std::time::Duration::from_millis(20));

    // Send out-of-order packets 3, 4, 5, 6, 7 (5 packets, exceeding capacity of 3)
    for seq in [3u16, 4, 5, 6, 7] {
        adapter_s.set_seq_nr(seq);
        adapter_s
            .send_packet(UtpPacketType::Data, format!("pkt{}", seq).as_bytes())
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Verify receiver buffered at most max_out_of_order_packets (3)
    assert!(adapter_r.out_of_order_count() <= 3);
    let metrics = adapter_r.pipeline_metrics();
    assert!(metrics.max_out_of_order_depth <= 3);

    adapter_s.close();
    adapter_r.close();
    let _ = handle.join();
}

#[test]
fn test_sack_driven_fast_loss_detection_and_retransmit() {
    let udp_s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_r = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr_s = udp_s.local_addr().unwrap();
    let addr_r = udp_r.local_addr().unwrap();

    udp_s.connect(addr_r).unwrap();
    udp_r.connect(addr_s).unwrap();

    let adapter_s = Arc::new(UtpSocketAdapter::from_raw_socket(udp_s, 902));
    let adapter_r = Arc::new(UtpSocketAdapter::from_raw_socket(udp_r, 902));

    // Receiver reader thread expects 15 bytes ("AAA", "BBB", "CCC", "DDD", "EEE")
    let r_clone = adapter_r.clone();
    let read_handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut buf = vec![0u8; 15];
            let mut total = 0;
            while total < 15 {
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

    // 1. Sender transmits Packet 1 (seq 1)
    adapter_s.set_seq_nr(1);
    adapter_s.send_packet(UtpPacketType::Data, b"AAA").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(25));

    // 2. Packet 2 (seq 2, b"BBB") is DROPPED in the network!
    // We register packet 2 in sender's sent_packets without delivering it to the receiver
    adapter_s.record_sent_packet(2, b"BBB");

    // 3. Sender transmits Packets 3, 4, 5 (seqs 3, 4, 5) which arrive at receiver out-of-order
    for (seq, data) in [(3u16, b"CCC"), (4, b"DDD"), (5, b"EEE")] {
        adapter_s.set_seq_nr(seq);
        adapter_s.send_packet(UtpPacketType::Data, data).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // Wait for receiver to buffer Packets 3, 4, 5 and emit SACK back to sender
    let start = std::time::Instant::now();
    while adapter_r.out_of_order_count() < 3 && start.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        adapter_r.out_of_order_count(),
        3,
        "Receiver must buffer packets 3, 4, 5"
    );

    // Sender background reader processes the incoming SACK packet
    let s_clone = adapter_s.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(async {
            let mut buf = [0u8; 16];
            let _ = s_clone.read(&mut buf).await;
        });
    });

    // Verify SACK Fast Retransmission automatically triggered for Packet 2
    let start_wait = std::time::Instant::now();
    while adapter_s.pipeline_metrics().retransmits_fast == 0
        && start_wait.elapsed() < std::time::Duration::from_secs(2)
    {
        std::thread::sleep(std::time::Duration::from_millis(15));
    }

    let metrics_s = adapter_s.pipeline_metrics();
    assert!(
        metrics_s.retransmits_fast >= 1,
        "Expected SACK gap to trigger Fast Retransmit without timeout: {:?}",
        metrics_s
    );

    // Receiver should now receive retransmitted Packet 2 and complete assembly
    let (total_read, data) = read_handle.join().unwrap();
    assert_eq!(total_read, 15);
    assert_eq!(&data[..15], b"AAABBBCCCDDDEEE");
    assert_eq!(adapter_r.out_of_order_count(), 0);

    adapter_s.close();
    adapter_r.close();
}
