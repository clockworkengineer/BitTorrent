# uTorrent Transport Protocol (uTP — BEP 29)

The library provides a high-performance **uTP transport adapter** in [`library/src/network/utp.rs`](file:///c:/Projects/BitTorrent/library/src/network/utp.rs), wrapping UDP sockets as `AsyncSocket`-compatible transports capable of driving BitTorrent peer wire connections.

---

## 1. Overview

uTP (μTP) is a UDP-based transport protocol standardized in **BEP 29**. It is designed to transfer data while being **scrupulously friendly to other network traffic on the local LAN**. Unlike standard TCP Reno or CUBIC, uTP uses the **LEDBAT** (Low Extra Delay Background Transport — RFC 6817) congestion control algorithm, which actively measures one-way queuing delay and throttles back aggressively when it detects router queue buildup. This prevents large torrent downloads from degrading latency-sensitive traffic such as web browsing, DNS queries, and voice/video calls.

### Protocol Comparison

| Property | Standard TCP | uTP (BEP 29) |
| :--- | :--- | :--- |
| **Transport** | TCP | UDP |
| **Congestion Control** | Loss-based (Reno / CUBIC) | Delay-based (RFC 6817 LEDBAT) |
| **NAT Traversal** | Difficult without UPnP | Easier via UDP holepunching (BEP 55) |
| **Queue Impact** | Saturates router buffers | Self-throttles based on queue delay |
| **Loss Recovery** | SACK / DupACK | SACK Extension + RFC 6298 RTO |

---

## 2. Packet Structure & Header Framing

Every uTP packet begins with a fixed **20-byte header** (`UtpHeader`), followed by optional chained extension headers and payload data:

```
 0               1               2               3
 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| type  |  ver  |   extension   |         connection_id         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          timestamp_us                         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     timestamp_difference_us                   |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                           wnd_size                            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|              seq_nr           |             ack_nr            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### Field Descriptions

| Field | Size | Description |
| :--- | :--- | :--- |
| `type` | 4 bits | Packet type: `ST_DATA` (0), `ST_FIN` (1), `ST_STATE` (2), `ST_RESET` (3), `ST_SYN` (4). |
| `version` | 4 bits | Protocol version (always `1`). |
| `extension` | 1 byte | First extension header type (`0` = none, `1` = Selective Acknowledgment). |
| `connection_id` | 2 bytes | Shared connection identifier (initiator uses send ID; responder uses receive ID). |
| `timestamp_us` | 4 bytes | Sender's local microsecond clock at packet dispatch time. |
| `timestamp_difference_us` | 4 bytes | Measured one-way delay (`remote_timestamp - local_timestamp`). |
| `wnd_size` | 4 bytes | Advertised receive window in bytes. |
| `seq_nr` | 2 bytes | 16-bit sequence number (increments monotonically per payload packet). |
| `ack_nr` | 2 bytes | 16-bit sequence number of the last sequentially received packet. |

---

## 3. LEDBAT Congestion Control (RFC 6817)

The `LedbatCongestionController` enforces delay-based congestion window scaling:

### 3.1 Delay Tracking & Baseline Calculation
- **`DelayTracker`**: Maintains 4 circular time buckets of 60 seconds each (4 minutes total), tracking the minimum one-way delay observed over the network.
- **Obsolete Baseline Expiry**: Automatically purges stale delay baselines caused by route shifts or system clock drift.
- **Queuing Delay Estimation**:
  $$\text{queuing\_delay} = \text{current\_delay} - \text{base\_delay}$$

### 3.2 Linear Window Scaling
On every successfully acknowledged packet, the congestion window (`cwnd`) scales according to its offset from the target queuing delay threshold ($\text{TARGET} = 100\text{ ms}$):

$$\Delta = \frac{\text{TARGET} - \text{queuing\_delay}}{\text{TARGET}} \times \frac{\text{bytes\_acked} \times \text{MSS}}{\text{cwnd}}$$

- If $\text{queuing\_delay} < \text{TARGET}$, `cwnd` grows linearly to consume idle bandwidth.
- If $\text{queuing\_delay} > \text{TARGET}$, `cwnd` backs off smoothly to yield bandwidth to other network flows.

---

## 4. Selective Acknowledgment (SACK Extension — BEP 29)

The library implements full BEP 29 chained extension headers with selective acknowledgments:

### 4.1 SACK Extension Header Wire Format
```
 0               1               2               3
 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7 0 1 2 3 4 5 6 7
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| next_ext = 0  |  len = 4 * N  |        bitmask (32 bits)      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```
- Bit 0 in the bitmask represents packet sequence number `ack_nr + 2`.
- Subsequent bits correspond to sequentially higher sequence numbers.
- A value of `1` indicates that the packet has been successfully received out of order.

### 4.2 Out-of-Order Reassembly Buffer
- Incoming packets received out of sequence are buffered in a sorted map (`BTreeMap<u16, Vec<u8>>`).
- The receiver immediately replies with an `ST_STATE` packet carrying a SACK bitmask.
- As missing sequence gaps arrive, contiguous chunks are assembled sequentially into the user stream buffer.
- Buffer capacity is bounded (`max_out_of_order_packets = 128`) to prevent memory exhaustion from packet floods.

---

## 5. SACK-Driven Fast Loss Recovery & RFC 6298 RTO

- **Sequence Gap Detection (`detect_receive_gaps`)**: Proactively evaluates circular 16-bit sequence differences to identify lost packets.
- **Fast Retransmission**: When a SACK bitmask indicates $\ge 3$ higher sequence numbers have arrived safely while an intermediate packet remains missing, the lost packet is retransmitted immediately without waiting for the RTO timer.
- **Burst Loss Window Scaling (`LossRecoveryState`)**: Multi-gap loss epochs trigger multiplicative decrease (`cwnd * 0.65`) while preventing redundant window collapses during a single loss event.
- **RFC 6298 Microsecond Precision RTT/RTO**:
  - Smoothed round-trip time (`srtt_us`) and RTT variance (`rttvar_us`) calculated on microsecond samples.
  - Exponential backoff doubles the RTO timer upon timeout up to a 30s maximum.

---

## 6. Telemetry & Metrics

Real-time pipeline metrics are exposed via `UtpSocketAdapter::pipeline_metrics()`:

```rust
pub struct UtpPipelineMetrics {
    pub fast_retransmits: u64,
    pub rto_timeouts: u64,
    pub burst_loss_events: u64,
    pub sacked_packets: u64,
    pub out_of_order_buffered: usize,
    pub srtt_us: u64,
    pub rttvar_us: u64,
    pub rto_us: u64,
}
```

---

## 7. References

- [BEP 29 — uTorrent Transport Protocol](https://www.bittorrent.org/beps/bep_0029.html)
- [RFC 6817 — Low Extra Delay Background Transport (LEDBAT)](https://www.rfc-editor.org/rfc/rfc6817)
- [RFC 6298 — Computing TCP's Retransmission Timer](https://www.rfc-editor.org/rfc/rfc6298)
- Source: [`library/src/network/utp.rs`](file:///c:/Projects/BitTorrent/library/src/network/utp.rs)
