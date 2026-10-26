//! uTorrent Transport Protocol (uTP — BEP 29) & LEDBAT Congestion Control (RFC 6817)
//!
//! Provides uTP packet framing, 20-byte header encode/decode, one-way delay tracking,
//! dynamic congestion window (`cwnd`) scaling via LEDBAT, and a [`UtpSocketAdapter`]
//! that wraps a UDP socket as an [`AsyncSocket`]-compatible peer transport.
//!
//! ## Protocol Overview
//!
//! - **Framing**: Fixed 20-byte header (big-endian) carrying sequence numbers, ACKs,
//!   receive window advertisement, and microsecond timestamps.
//! - **Delay Measurement**: Measures one-way propagation delay via `timestamp_us` and
//!   `timestamp_difference_us` headers without requiring synchronized clocks.
//! - **LEDBAT Congestion Control**: Target queuing delay threshold (100 ms), linear
//!   `cwnd` adjustment `(TARGET - queuing_delay) / TARGET`, and fast retransmit /
//!   multiplicative decrease (`cwnd = max(cwnd / 2, MIN_CWND)`) on packet loss or
//!   3 duplicate ACKs.
//! - **Connection State Machine**: `ST_SYN` (initiate) → `ST_DATA` / `ST_STATE` (transfer)
//!   → `ST_RESET` (abort) / `ST_FIN` (graceful close).

use crate::error::BitTorrentError;
use crate::io_traits::AsyncSocket;
use std::collections::{BTreeMap, VecDeque};
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

/// The uTP extension types defined by BEP 29.
pub const UTP_EXT_SACK: u8 = 1;
pub const UTP_EXT_EXTENSION_BITS: u8 = 2;

/// A uTP extension header (BEP 29).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UtpExtension {
    /// Selective Acknowledgement (BEP 29).
    ///
    /// Contains a bitmask indicating received out-of-order packets starting at `ack_nr + 2`.
    /// The bitmask length is always a multiple of 4 bytes (minimum 4 bytes).
    SelectiveAck(Vec<u8>),
    /// Other extension header with type and raw data.
    Unknown { extension_type: u8, data: Vec<u8> },
}

impl UtpExtension {
    /// Returns the type identifier byte for this extension.
    pub fn extension_type(&self) -> u8 {
        match self {
            UtpExtension::SelectiveAck(_) => UTP_EXT_SACK,
            UtpExtension::Unknown { extension_type, .. } => *extension_type,
        }
    }

    /// Returns the extension payload data bytes.
    pub fn data(&self) -> &[u8] {
        match self {
            UtpExtension::SelectiveAck(data) => data,
            UtpExtension::Unknown { data, .. } => data,
        }
    }
}

/// Constructs a SACK bitmask from a set or slice of out-of-order received sequence numbers.
///
/// In BEP 29, the bitmask has reverse byte order:
/// - Byte 0: bits 0..7 represent `[ack_nr + 2 .. ack_nr + 9]` (bit 0 = `ack_nr + 2`).
/// - Byte 1: bits 0..7 represent `[ack_nr + 10 .. ack_nr + 17]`, and so on.
/// - `ack_nr + 1` is omitted because it represents the missing packet.
/// - The length must be at least 4 bytes and in multiples of 4.
pub fn build_sack_bitmask(ack_nr: u16, received_seqs: &[u16]) -> Vec<u8> {
    let mut max_bit_offset: Option<usize> = None;
    for &seq in received_seqs {
        let diff = seq.wrapping_sub(ack_nr);
        if diff >= 2 && diff < 32768 {
            let bit_offset = (diff - 2) as usize;
            max_bit_offset = Some(max_bit_offset.map_or(bit_offset, |m| m.max(bit_offset)));
        }
    }

    let Some(max_offset) = max_bit_offset else {
        return Vec::new();
    };

    let min_bytes = (max_offset / 8) + 1;
    // SACK length must be at least 4 bytes and in multiples of 4
    let num_bytes = ((min_bytes + 3) / 4).max(1) * 4;

    if num_bytes <= 32 {
        let mut stack_mask = [0u8; 32];
        for &seq in received_seqs {
            let diff = seq.wrapping_sub(ack_nr);
            if diff >= 2 && diff < 32768 {
                let bit_offset = (diff - 2) as usize;
                let byte_idx = bit_offset / 8;
                let bit_idx = bit_offset % 8;
                if byte_idx < num_bytes {
                    stack_mask[byte_idx] |= 1 << bit_idx;
                }
            }
        }
        return stack_mask[..num_bytes].to_vec();
    }

    let mut bitmask = vec![0u8; num_bytes];
    for &seq in received_seqs {
        let diff = seq.wrapping_sub(ack_nr);
        if diff >= 2 && diff < 32768 {
            let bit_offset = (diff - 2) as usize;
            let byte_idx = bit_offset / 8;
            let bit_idx = bit_offset % 8;
            if byte_idx < bitmask.len() {
                bitmask[byte_idx] |= 1 << bit_idx;
            }
        }
    }

    bitmask
}

/// Returns true if the sequence number `seq` is indicated as received in the SACK bitmask.
pub fn is_seq_sacked(ack_nr: u16, bitmask: &[u8], seq: u16) -> bool {
    let diff = seq.wrapping_sub(ack_nr);
    if diff < 2 || diff >= 32768 {
        return false;
    }
    let bit_offset = (diff - 2) as usize;
    let byte_idx = bit_offset / 8;
    let bit_idx = bit_offset % 8;
    if byte_idx < bitmask.len() {
        (bitmask[byte_idx] & (1 << bit_idx)) != 0
    } else {
        false
    }
}

/// Parses a SACK bitmask and returns all selectively acknowledged sequence numbers in order.
pub fn parse_sacked_seqs(ack_nr: u16, bitmask: &[u8]) -> Vec<u16> {
    let mut seqs = Vec::new();
    for (byte_idx, &byte) in bitmask.iter().enumerate() {
        if byte == 0 {
            continue;
        }
        for bit_idx in 0..8 {
            if (byte & (1 << bit_idx)) != 0 {
                let bit_offset = byte_idx * 8 + bit_idx;
                let seq = ack_nr.wrapping_add((bit_offset + 2) as u16);
                seqs.push(seq);
            }
        }
    }
    seqs
}

/// Identifies missing sequence number ranges between `ack_nr + 1` and the highest out-of-order packet.
///
/// Returns a list of `(start_seq, end_seq)` inclusive tuples representing missing holes.
pub fn detect_receive_gaps(ack_nr: u16, ooo_seqs: &[u16]) -> Vec<(u16, u16)> {
    if ooo_seqs.is_empty() {
        return Vec::new();
    }
    let mut sorted = ooo_seqs.to_vec();
    sorted.sort_by(|&a, &b| a.wrapping_sub(ack_nr).cmp(&b.wrapping_sub(ack_nr)));

    let mut gaps = Vec::new();
    let mut expected = ack_nr.wrapping_add(1);

    for &seq in &sorted {
        let diff = seq.wrapping_sub(expected);
        if diff > 0 && diff < 32768 {
            // Gap from expected to seq - 1 inclusive
            gaps.push((expected, seq.wrapping_sub(1)));
        }
        expected = seq.wrapping_add(1);
    }

    gaps
}

/// Telemetry metrics for the uTP packet pipeline, out-of-order reassembly, and SACK loss recovery.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UtpPipelineMetrics {
    /// Total data and control packets transmitted.
    pub packets_sent: u64,
    /// Total data and control packets received.
    pub packets_recv: u64,
    /// Total payload bytes successfully sent.
    pub bytes_sent: u64,
    /// Total payload bytes received and assembled.
    pub bytes_recv: u64,
    /// Number of fast retransmissions triggered by SACK or 3 duplicate ACKs.
    pub retransmits_fast: u64,
    /// Number of retransmissions triggered by RTO timeout.
    pub retransmits_timeout: u64,
    /// Total duplicate ACKs received.
    pub duplicate_acks_count: u64,
    /// SACK packets sent by the receiver.
    pub sack_packets_sent: u64,
    /// SACK packets received by the sender.
    pub sack_packets_recv: u64,
    /// Number of distinct packet loss recovery events entered.
    pub loss_recovery_events: u64,
    /// Number of burst loss events detected (>= 2 concurrent gaps).
    pub burst_loss_events: u64,
    /// Current number of out-of-order packets buffered awaiting reassembly.
    pub out_of_order_buffered: usize,
    /// Highest number of out-of-order packets held in the reassembly buffer concurrently.
    pub max_out_of_order_depth: usize,
    /// Current Retransmission Timeout in microseconds.
    pub rto_us: u32,
    /// Smoothed Round-Trip Time in microseconds.
    pub srtt_us: u32,
    /// Round-Trip Time variation in microseconds.
    pub rttvar_us: u32,
    /// Current congestion window (`cwnd`) in bytes.
    pub cwnd_bytes: u32,
}

/// The five packet types defined by the uTP protocol (BEP 29).
///
/// The type is encoded in the upper 4 bits of the first header byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtpPacketType {
    /// `ST_DATA (0)` — carries payload bytes.
    Data = 0,
    /// `ST_FIN (1)` — graceful connection close (no payload).
    Ack = 1,
    /// `ST_SYN (2)` — initiates a new connection.
    Syn = 2,
    /// `ST_RESET (3)` — immediately aborts the connection.
    Reset = 3,
    /// `ST_STATE (4)` — pure acknowledgement packet (no payload).
    State = 4,
}

impl UtpPacketType {
    /// Converts a raw 4-bit value into a `UtpPacketType`, or `None` if the value is unknown.
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(UtpPacketType::Data),
            1 => Some(UtpPacketType::Ack),
            2 => Some(UtpPacketType::Syn),
            3 => Some(UtpPacketType::Reset),
            4 => Some(UtpPacketType::State),
            _ => None,
        }
    }
}

/// The uTP packet header with optional BEP 29 extension headers (e.g. SACK).
///
/// All multi-byte fields are big-endian. The `packet_type` and `version` share
/// the first byte (type in the upper 4 bits, version in the lower 4 bits).
///
/// | Field                    | Size | Description |
/// |--------------------------|------|-------------|
/// | `packet_type` + `version`| 1 B  | Upper nibble = type, lower nibble = version |
/// | `extension`              | 1 B  | Extension header type (`0` = none) |
/// | `connection_id`          | 2 B  | Connection identifier shared by both peers |
/// | `timestamp_us`           | 4 B  | Sender's wall clock in microseconds |
/// | `timestamp_difference_us`| 4 B  | One-way delay: `remote_ts - local_ts` |
/// | `wnd_size`               | 4 B  | Receive window advertised by the sender |
/// | `seq_nr`                 | 2 B  | Sequence number of this packet |
/// | `ack_nr`                 | 2 B  | Last sequence number acknowledged |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtpHeader {
    /// Packet type (upper 4 bits of byte 0).
    pub packet_type: UtpPacketType,
    /// Protocol version (lower 4 bits of byte 0). Always `1`.
    pub version: u8,
    /// Extension header indicator. `0` means no extension headers follow.
    pub extension: u8,
    /// Identifies the logical connection; shared by both sender and receiver.
    pub connection_id: u16,
    /// Sender's current timestamp in microseconds (used for delay measurement).
    pub timestamp_us: u32,
    /// Estimated one-way propagation delay reflected back to remote peer.
    pub timestamp_difference_us: u32,
    /// Number of bytes the sender is willing to buffer (receive window).
    pub wnd_size: u32,
    /// Sequence number of this packet.
    pub seq_nr: u16,
    /// The last sequence number the sender has acknowledged from the remote peer.
    pub ack_nr: u16,
    /// Any extension headers (e.g. SACK) appended to the 20-byte base header.
    pub extensions: Vec<UtpExtension>,
}

impl Default for UtpHeader {
    fn default() -> Self {
        Self {
            packet_type: UtpPacketType::Data,
            version: 1,
            extension: 0,
            connection_id: 0,
            timestamp_us: 0,
            timestamp_difference_us: 0,
            wnd_size: 1_048_576,
            seq_nr: 0,
            ack_nr: 0,
            extensions: Vec::new(),
        }
    }
}

impl UtpHeader {
    /// Decodes a uTP header and any attached extension headers from a buffer.
    ///
    /// Returns an error if `buf` is shorter than 20 bytes or if any extension header
    /// is truncated or malformed.
    pub fn decode(buf: &[u8]) -> Result<Self, BitTorrentError> {
        if buf.len() < 20 {
            return Err(BitTorrentError::Parse("uTP header too short".into()));
        }
        let type_ver = buf[0];
        let packet_type = UtpPacketType::from_u8(type_ver >> 4)
            .ok_or_else(|| BitTorrentError::Parse("Invalid uTP packet type".into()))?;
        let version = type_ver & 0x0F;
        let extension = buf[1];
        let connection_id = u16::from_be_bytes([buf[2], buf[3]]);
        let timestamp_us = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let timestamp_difference_us = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let wnd_size = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);
        let seq_nr = u16::from_be_bytes([buf[16], buf[17]]);
        let ack_nr = u16::from_be_bytes([buf[18], buf[19]]);

        let mut extensions = Vec::new();
        let mut ext_type = extension;
        let mut offset = 20;

        while ext_type != 0 {
            if buf.len() < offset + 2 {
                return Err(BitTorrentError::Parse(
                    "uTP extension header truncated".into(),
                ));
            }
            let next_ext = buf[offset];
            let ext_len = buf[offset + 1] as usize;
            offset += 2;
            if buf.len() < offset + ext_len {
                return Err(BitTorrentError::Parse(
                    "uTP extension data truncated".into(),
                ));
            }
            let ext_data = &buf[offset..offset + ext_len];
            match ext_type {
                UTP_EXT_SACK => {
                    extensions.push(UtpExtension::SelectiveAck(ext_data.to_vec()));
                }
                _ => {
                    extensions.push(UtpExtension::Unknown {
                        extension_type: ext_type,
                        data: ext_data.to_vec(),
                    });
                }
            }
            offset += ext_len;
            ext_type = next_ext;
        }

        Ok(UtpHeader {
            packet_type,
            version,
            extension,
            connection_id,
            timestamp_us,
            timestamp_difference_us,
            wnd_size,
            seq_nr,
            ack_nr,
            extensions,
        })
    }

    /// Serializes the header and all attached extension headers into bytes (big-endian).
    ///
    /// If extensions are present, byte 1 is set to the first extension's type ID,
    /// and each extension header is serialized in sequence with `next_ext` linking.
    pub fn encode(&self) -> Vec<u8> {
        let first_ext = self
            .extensions
            .first()
            .map(|e| e.extension_type())
            .unwrap_or(self.extension);
        let mut buf = Vec::with_capacity(
            20 + self
                .extensions
                .iter()
                .map(|e| 2 + e.data().len())
                .sum::<usize>(),
        );
        buf.push(((self.packet_type as u8) << 4) | (self.version & 0x0F));
        buf.push(first_ext);
        buf.extend_from_slice(&self.connection_id.to_be_bytes());
        buf.extend_from_slice(&self.timestamp_us.to_be_bytes());
        buf.extend_from_slice(&self.timestamp_difference_us.to_be_bytes());
        buf.extend_from_slice(&self.wnd_size.to_be_bytes());
        buf.extend_from_slice(&self.seq_nr.to_be_bytes());
        buf.extend_from_slice(&self.ack_nr.to_be_bytes());

        for (i, ext) in self.extensions.iter().enumerate() {
            let next_ext = if i + 1 < self.extensions.len() {
                self.extensions[i + 1].extension_type()
            } else {
                0
            };
            buf.push(next_ext);
            buf.push(ext.data().len() as u8);
            buf.extend_from_slice(ext.data());
        }

        buf
    }

    /// Returns the total serialized size in bytes of this header including all extension headers.
    pub fn total_header_len(&self) -> usize {
        20 + self
            .extensions
            .iter()
            .map(|e| 2 + e.data().len())
            .sum::<usize>()
    }

    /// Returns the SACK bitmask slice if a SelectiveAck extension is present in the header.
    pub fn sack_bitmask(&self) -> Option<&[u8]> {
        self.extensions.iter().find_map(|ext| match ext {
            UtpExtension::SelectiveAck(mask) => Some(mask.as_slice()),
            _ => None,
        })
    }

    /// Returns true if the sequence number `seq` is selectively acknowledged by this header.
    pub fn is_seq_sacked(&self, seq: u16) -> bool {
        self.sack_bitmask()
            .map(|mask| is_seq_sacked(self.ack_nr, mask, seq))
            .unwrap_or(false)
    }

    /// Adds a SACK extension header with the given bitmask.
    pub fn with_sack(mut self, bitmask: Vec<u8>) -> Self {
        self.extension = UTP_EXT_SACK;
        self.extensions.push(UtpExtension::SelectiveAck(bitmask));
        self
    }
}

/// Helper function to determine if sequence number `seq` is acknowledged by `ack`
/// in 16-bit circular sequence space (`seq <= ack`).
#[inline]
pub fn seq_acked(seq: u16, ack: u16) -> bool {
    ack.wrapping_sub(seq) < 32768
}

/// A single delay measurement bucket for tracking historical minimum propagation delays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelayBucket {
    /// Minimum one-way delay observed during this bucket's time window in microseconds.
    pub min_delay: u32,
    /// Start timestamp of this bucket in microseconds.
    pub start_time_us: u64,
}

impl DelayBucket {
    /// Creates a new empty bucket starting at `start_time_us`.
    pub fn new(start_time_us: u64) -> Self {
        Self {
            min_delay: u32::MAX,
            start_time_us,
        }
    }
}

/// Tracks minimum base delays across a circular sliding window of time buckets.
///
/// Implements the base delay filter recommended by RFC 6817 (LEDBAT) Section 4.2.
/// Defaults to 4 buckets of 60 seconds each (4 minutes total history), ensuring
/// obsolete minimums caused by route changes or clock drift expire automatically.
#[derive(Debug, Clone)]
pub struct DelayTracker {
    buckets: [DelayBucket; 4],
    current_idx: usize,
    bucket_duration_us: u64,
    last_sample_us: u32,
}

impl Default for DelayTracker {
    fn default() -> Self {
        Self::new(60_000_000) // 60 seconds per bucket = 4 minutes history
    }
}

impl DelayTracker {
    /// Creates a new `DelayTracker` with the specified bucket duration in microseconds.
    pub fn new(bucket_duration_us: u64) -> Self {
        Self {
            buckets: [
                DelayBucket::new(0),
                DelayBucket::new(0),
                DelayBucket::new(0),
                DelayBucket::new(0),
            ],
            current_idx: 0,
            bucket_duration_us,
            last_sample_us: 0,
        }
    }

    /// Records a new one-way delay sample at timestamp `now_us`.
    pub fn add_sample(&mut self, delay_us: u32, now_us: u64) {
        if delay_us == 0 {
            return;
        }
        self.last_sample_us = delay_us;

        let cur = &mut self.buckets[self.current_idx];
        if cur.start_time_us == 0 {
            cur.start_time_us = now_us;
            cur.min_delay = delay_us;
        } else if now_us.saturating_sub(cur.start_time_us) >= self.bucket_duration_us {
            // Advance to the next bucket in the circular buffer
            self.current_idx = (self.current_idx + 1) % self.buckets.len();
            self.buckets[self.current_idx] = DelayBucket {
                min_delay: delay_us,
                start_time_us: now_us,
            };
        } else {
            cur.min_delay = cur.min_delay.min(delay_us);
        }
    }

    /// Computes the estimated base delay (minimum observed delay across non-expired buckets).
    ///
    /// If no valid samples have been recorded yet, returns `u32::MAX`.
    pub fn base_delay(&self) -> u32 {
        let mut min = u32::MAX;
        for b in &self.buckets {
            if b.start_time_us > 0 && b.min_delay < min {
                min = b.min_delay;
            }
        }
        min
    }

    /// Returns the latest delay sample received, or 0 if none has been received.
    pub fn last_sample(&self) -> u32 {
        self.last_sample_us
    }

    /// Returns the configured duration of a single bucket in microseconds.
    pub fn bucket_duration_us(&self) -> u64 {
        self.bucket_duration_us
    }
}

/// The loss recovery state of the congestion controller (RFC 6582 NewReno / RFC 6817).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LossRecoveryState {
    /// Normal operation: LEDBAT adjusts window linearly based on queuing delay.
    #[default]
    Normal,
    /// Fast recovery epoch: window has been scaled down; subsequent duplicate ACKs or SACKs
    /// covering sequence numbers at or below `recovery_seq` do not repeatedly halve the window.
    Recovery {
        /// Sequence number that must be cumulatively acknowledged to exit recovery.
        recovery_seq: u16,
        /// The reduced congestion window for this recovery period.
        loss_window: u32,
    },
}

/// LEDBAT (Low Extra Delay Background Transport — RFC 6817) Congestion Controller.
///
/// Implements delay-based congestion control:
/// - Target queuing delay: 100 ms (100,000 µs).
/// - Dynamic `cwnd` scaling proportional to `(TARGET - queuing_delay) / TARGET`.
/// - Fast Retransmit / Multiplicative Decrease on 3 duplicate ACKs or packet loss (`cwnd = max(cwnd / 2, MIN_CWND)`).
/// - SACK-driven Fast Recovery with bounded window reduction and burst loss scaling.
#[derive(Debug, Clone)]
pub struct LedbatCongestionController {
    target_delay_us: u32,
    cwnd: u32,
    cwnd_float: f64,
    min_cwnd: u32,
    max_cwnd: u32,
    mss: u32,
    delay_tracker: DelayTracker,
    last_queuing_delay_us: u32,
    last_ack_nr: u16,
    duplicate_acks: u32,
    remote_wnd_size: u32,
    loss_recovery: LossRecoveryState,
    srtt_us: Option<u32>,
    rttvar_us: u32,
    rto_us: u32,
}

impl Default for LedbatCongestionController {
    fn default() -> Self {
        Self::new()
    }
}

impl LedbatCongestionController {
    /// Default target queuing delay of 100 ms (100,000 microseconds) per RFC 6817 / BEP 29.
    pub const DEFAULT_TARGET_DELAY_US: u32 = 100_000;
    /// Maximum Segment Size (MSS) in bytes.
    pub const DEFAULT_MSS: u32 = 1460;
    /// Minimum congestion window (2 * MSS).
    pub const DEFAULT_MIN_CWND: u32 = 2 * 1460;
    /// Maximum allowable congestion window (1 MiB).
    pub const DEFAULT_MAX_CWND: u32 = 1_048_576;
    /// Default initial Retransmission Timeout (1,000,000 µs = 1 second) per RFC 6298.
    pub const DEFAULT_INITIAL_RTO_US: u32 = 1_000_000;
    /// Minimum Retransmission Timeout (500,000 µs = 500 ms).
    pub const MIN_RTO_US: u32 = 500_000;
    /// Maximum Retransmission Timeout (3,000,000 µs = 3 seconds).
    pub const MAX_RTO_US: u32 = 3_000_000;

    /// Creates a new `LedbatCongestionController` with default RFC 6817 parameters.
    pub fn new() -> Self {
        Self::with_target(Self::DEFAULT_TARGET_DELAY_US)
    }

    /// Creates a new `LedbatCongestionController` with a custom target queuing delay in microseconds.
    pub fn with_target(target_delay_us: u32) -> Self {
        Self {
            target_delay_us,
            cwnd: Self::DEFAULT_MIN_CWND,
            cwnd_float: Self::DEFAULT_MIN_CWND as f64,
            min_cwnd: Self::DEFAULT_MIN_CWND,
            max_cwnd: Self::DEFAULT_MAX_CWND,
            mss: Self::DEFAULT_MSS,
            delay_tracker: DelayTracker::default(),
            last_queuing_delay_us: 0,
            last_ack_nr: 0,
            duplicate_acks: 0,
            remote_wnd_size: Self::DEFAULT_MAX_CWND,
            loss_recovery: LossRecoveryState::Normal,
            srtt_us: None,
            rttvar_us: 250_000,
            rto_us: Self::DEFAULT_INITIAL_RTO_US,
        }
    }

    /// Updates the remote peer's advertised receive window (`wnd_size`).
    pub fn set_remote_window(&mut self, wnd_size: u32) {
        if wnd_size > 0 {
            self.remote_wnd_size = wnd_size;
        }
    }

    /// Returns the current loss recovery state.
    pub fn loss_recovery_state(&self) -> LossRecoveryState {
        self.loss_recovery
    }

    /// Returns true if currently in a fast recovery epoch.
    pub fn is_in_recovery(&self) -> bool {
        matches!(self.loss_recovery, LossRecoveryState::Recovery { .. })
    }

    /// Enters fast loss recovery for packet loss ending at `recovery_seq`.
    ///
    /// If `is_burst_loss` is true (e.g. multiple discontinuous dropped packets in a window),
    /// scales the window proportionally (`cwnd * 0.65`) instead of severe halving (`cwnd * 0.50`),
    /// preserving throughput in high-latency network conditions.
    ///
    /// Returns `true` if a new recovery epoch was initiated, or `false` if the loss
    /// was already covered by an active recovery epoch.
    pub fn enter_recovery(&mut self, recovery_seq: u16, is_burst_loss: bool) -> bool {
        if let LossRecoveryState::Recovery {
            recovery_seq: curr_seq,
            ..
        } = self.loss_recovery
        {
            if seq_acked(recovery_seq, curr_seq) {
                return false;
            }
        }

        let new_cwnd = if is_burst_loss {
            ((self.cwnd_float * 0.65).round() as u32).max(self.min_cwnd)
        } else {
            ((self.cwnd_float * 0.50).round() as u32).max(self.min_cwnd)
        };

        self.cwnd = new_cwnd;
        self.cwnd_float = new_cwnd as f64;
        self.loss_recovery = LossRecoveryState::Recovery {
            recovery_seq,
            loss_window: new_cwnd,
        };
        true
    }

    /// Exits fast loss recovery back to normal linear mode.
    pub fn exit_recovery(&mut self) {
        self.loss_recovery = LossRecoveryState::Normal;
    }

    /// Updates smoothed RTT and variance per RFC 6298 with microsecond precision.
    pub fn update_rtt(&mut self, rtt_sample_us: u32) {
        if rtt_sample_us == 0 {
            return;
        }
        match self.srtt_us {
            None => {
                self.srtt_us = Some(rtt_sample_us);
                self.rttvar_us = rtt_sample_us / 2;
                self.rto_us =
                    (rtt_sample_us + 4 * self.rttvar_us).clamp(Self::MIN_RTO_US, Self::MAX_RTO_US);
            }
            Some(srtt) => {
                let diff = (srtt as i64 - rtt_sample_us as i64).unsigned_abs() as u32;
                self.rttvar_us = (3 * self.rttvar_us + diff) / 4;
                let new_srtt = (7 * srtt + rtt_sample_us) / 8;
                self.srtt_us = Some(new_srtt);
                self.rto_us =
                    (new_srtt + 4 * self.rttvar_us).clamp(Self::MIN_RTO_US, Self::MAX_RTO_US);
            }
        }
    }

    /// Backs off RTO timer exponentially upon retransmission timeout (RFC 6298).
    pub fn on_rto_timeout(&mut self) {
        self.rto_us = self.rto_us.saturating_mul(2).min(Self::MAX_RTO_US);
        self.on_loss();
    }

    /// Returns the current Retransmission Timeout (RTO) in microseconds.
    pub fn rto_us(&self) -> u32 {
        self.rto_us
    }

    /// Returns the current Retransmission Timeout as a standard Duration.
    pub fn current_rto_duration(&self) -> std::time::Duration {
        std::time::Duration::from_micros(self.rto_us as u64)
    }

    /// Returns the smoothed RTT in microseconds, if estimated.
    pub fn srtt_us(&self) -> Option<u32> {
        self.srtt_us
    }

    /// Returns the RTT variance in microseconds.
    pub fn rttvar_us(&self) -> u32 {
        self.rttvar_us
    }

    /// Processes an incoming ACK and delay sample, updating the congestion window.
    ///
    /// Implements RFC 6817 linear window controller:
    /// `off_target = (TARGET - queuing_delay) / TARGET`
    /// `cwnd += (off_target * bytes_acked * MSS) / cwnd`
    pub fn on_ack(&mut self, bytes_acked: u32, delay_sample_us: u32, now_us: u64) -> u32 {
        if delay_sample_us > 0 {
            self.delay_tracker.add_sample(delay_sample_us, now_us);
            let base = self.delay_tracker.base_delay();
            let queuing_delay = delay_sample_us.saturating_sub(base);
            self.last_queuing_delay_us = queuing_delay;

            // RFC 6817 linear controller
            let off_target = (self.target_delay_us as f64 - queuing_delay as f64)
                / (self.target_delay_us as f64);
            // Bound off_target to [-1.0, 1.0] to prevent extreme fluctuations
            let off_target = off_target.clamp(-1.0, 1.0);

            let delta = (off_target * (bytes_acked as f64) * (self.mss as f64)) / self.cwnd_float;
            self.cwnd_float =
                (self.cwnd_float + delta).clamp(self.min_cwnd as f64, self.max_cwnd as f64);
            self.cwnd = self.cwnd_float.round() as u32;
        }

        self.cwnd
    }

    /// Multiplicative decrease upon packet loss or timeout.
    ///
    /// Halves `cwnd`, bounded from below by `MIN_CWND`.
    pub fn on_loss(&mut self) -> u32 {
        self.cwnd_float = (self.cwnd_float / 2.0).max(self.min_cwnd as f64);
        self.cwnd = self.cwnd_float.round() as u32;
        self.cwnd
    }

    /// Alias for [`Self::on_loss`].
    pub fn on_packet_loss(&mut self) -> u32 {
        self.on_loss()
    }

    /// Handles an incoming acknowledgement number, checking for duplicate ACKs.
    ///
    /// Returns `true` if Fast Retransmit was triggered (3 duplicate ACKs reached).
    pub fn handle_ack(
        &mut self,
        ack_nr: u16,
        bytes_acked: u32,
        delay_sample_us: u32,
        now_us: u64,
    ) -> bool {
        // If in recovery and cumulative ACK covers recovery_seq, exit back to normal
        if let LossRecoveryState::Recovery { recovery_seq, .. } = self.loss_recovery {
            if seq_acked(recovery_seq, ack_nr) {
                self.loss_recovery = LossRecoveryState::Normal;
            }
        }

        if bytes_acked > 0 {
            self.duplicate_acks = 0;
            self.last_ack_nr = ack_nr;
            if self.loss_recovery == LossRecoveryState::Normal {
                self.on_ack(bytes_acked, delay_sample_us, now_us);
            } else if delay_sample_us > 0 {
                self.delay_tracker.add_sample(delay_sample_us, now_us);
                let base = self.delay_tracker.base_delay();
                self.last_queuing_delay_us = delay_sample_us.saturating_sub(base);
            }
            false
        } else if ack_nr == self.last_ack_nr {
            self.duplicate_acks += 1;
            if self.duplicate_acks == 3 {
                self.on_loss();
                return true;
            }
            false
        } else {
            self.duplicate_acks = 0;
            self.last_ack_nr = ack_nr;
            if delay_sample_us > 0 {
                self.delay_tracker.add_sample(delay_sample_us, now_us);
            }
            false
        }
    }

    /// Returns the current congestion window (`cwnd`) in bytes.
    pub fn cwnd(&self) -> u32 {
        self.cwnd
    }

    /// Returns the effective transmission window in bytes: `min(cwnd, remote_wnd_size)`.
    pub fn effective_window(&self) -> u32 {
        self.cwnd.min(self.remote_wnd_size)
    }

    /// Returns whether `bytes_in_flight` is within the effective congestion window.
    pub fn can_send(&self, bytes_in_flight: u32) -> bool {
        bytes_in_flight < self.effective_window()
    }

    /// Returns available window capacity in bytes.
    pub fn available_window(&self, bytes_in_flight: u32) -> u32 {
        self.effective_window().saturating_sub(bytes_in_flight)
    }

    /// Returns the base delay estimate in microseconds.
    pub fn base_delay(&self) -> u32 {
        self.delay_tracker.base_delay()
    }

    /// Returns the latest computed queuing delay in microseconds.
    pub fn queuing_delay(&self) -> u32 {
        self.last_queuing_delay_us
    }

    /// Returns the configured target queuing delay in microseconds.
    pub fn target_delay(&self) -> u32 {
        self.target_delay_us
    }

    /// Returns a reference to the internal [`DelayTracker`].
    pub fn delay_tracker(&self) -> &DelayTracker {
        &self.delay_tracker
    }

    /// Sets a manual congestion window for testing and initialization.
    pub fn set_cwnd(&mut self, cwnd: u32) {
        self.cwnd = cwnd.clamp(self.min_cwnd, self.max_cwnd);
        self.cwnd_float = self.cwnd as f64;
    }
}

/// Adapts a UDP socket to the [`AsyncSocket`] trait using the uTP framing protocol and LEDBAT.
///
/// `UtpSocketAdapter` implements the uTP connection state machine:
/// - **`connect()`** — sends a `ST_SYN` packet to initiate the handshake.
/// - **`write()`** — wraps the payload in a `ST_DATA` packet with sequence tracking and LEDBAT window checks.
/// - **`read()`** — receives UDP datagrams, measures one-way delay, updates LEDBAT `cwnd`, and sends `ST_STATE` ACKs.
/// - **`close()`** — sends `ST_RESET` and marks the socket as closed.
/// A packet sent by the adapter that is tracked until acknowledged.
#[derive(Debug, Clone)]
pub struct SentPacket {
    /// Sequence number of the packet.
    pub seq: u16,
    /// Time when the packet was first transmitted.
    pub timestamp: std::time::Instant,
    /// Payload bytes.
    pub payload: Vec<u8>,
    /// Whether this packet has been selectively acknowledged via a SACK extension.
    pub sacked: bool,
    /// Number of times this packet has been retransmitted.
    pub retransmit_count: u32,
    /// Timestamp when this packet was most recently sent or retransmitted.
    pub last_sent_time: std::time::Instant,
}

/// Adapts a UDP socket to the [`AsyncSocket`] trait using the uTP framing protocol, LEDBAT,
/// dynamic SACK out-of-order reassembly, and fast loss recovery pipeline.
///
/// `UtpSocketAdapter` implements the uTP connection state machine and pipeline:
/// - **`connect()`** — sends a `ST_SYN` packet to initiate the handshake.
/// - **`write()`** — wraps the payload in a `ST_DATA` packet with sequence tracking and LEDBAT window checks.
/// - **`read()`** — receives UDP datagrams, measures one-way delay, updates LEDBAT `cwnd`, and sends `ST_STATE` ACKs.
/// - **`close()`** — sends `ST_RESET` and marks the socket as closed.
/// - **SACK (BEP 29)** — generates SACK extension headers when out-of-order packets arrive, stitches contiguous packets,
///   and detects gaps in the sender pipeline to trigger SACK-driven Fast Retransmit without waiting for RTO.
/// - **Burst Loss Recovery** — scales congestion window gracefully during burst drop events and avoids redundant window halving.
#[derive(Debug)]
pub struct UtpSocketAdapter {
    udp: Arc<UdpSocket>,
    connection_id: u16,
    seq_nr: Mutex<u16>,
    ack_nr: Mutex<u16>,
    closed: std::sync::atomic::AtomicBool,
    // LEDBAT Congestion Controller & delay tracking
    controller: Mutex<LedbatCongestionController>,
    last_remote_timestamp_us: Mutex<u32>,
    last_remote_recv_us: Mutex<u32>,
    sent_packets: Mutex<VecDeque<SentPacket>>,
    // Out-of-order packet reassembly & buffered incoming bytes
    out_of_order_received: Mutex<BTreeMap<u16, Vec<u8>>>,
    recv_buffer: Mutex<VecDeque<u8>>,
    metrics: Mutex<UtpPipelineMetrics>,
    max_out_of_order_packets: usize,
}

impl UtpSocketAdapter {
    /// Connects to a remote peer and returns a ready `UtpSocketAdapter`.
    ///
    /// Binds a local UDP socket on an ephemeral port, connects it to `ip:port`,
    /// and sends an initial `ST_SYN` packet to begin the uTP handshake.
    ///
    /// Returns an error if the UDP socket cannot be bound or the `ST_SYN` cannot be sent.
    pub fn connect(ip: &str, port: u16) -> Result<Self, BitTorrentError> {
        let is_ipv6 = ip.contains(':');
        let bind_addr = if is_ipv6 { "[::]:0" } else { "0.0.0.0:0" };
        let udp = UdpSocket::bind(bind_addr).map_err(BitTorrentError::Io)?;
        let remote_addr = crate::util::format_socket_addr(ip, port);
        udp.connect(remote_addr).map_err(BitTorrentError::Io)?;

        let connection_id = rand::random::<u16>();
        let adapter = Self::from_raw_socket(udp, connection_id);

        // Send ST_SYN handshake packet
        adapter.send_packet(UtpPacketType::Syn, &[])?;

        Ok(adapter)
    }

    /// Wraps an already bound and connected `UdpSocket` into a `UtpSocketAdapter`.
    pub fn from_raw_socket(udp: UdpSocket, connection_id: u16) -> Self {
        let _ = udp.set_read_timeout(Some(std::time::Duration::from_millis(200)));
        UtpSocketAdapter {
            udp: Arc::new(udp),
            connection_id,
            seq_nr: Mutex::new(1),
            ack_nr: Mutex::new(0),
            closed: std::sync::atomic::AtomicBool::new(false),
            controller: Mutex::new(LedbatCongestionController::new()),
            last_remote_timestamp_us: Mutex::new(0),
            last_remote_recv_us: Mutex::new(0),
            sent_packets: Mutex::new(VecDeque::new()),
            out_of_order_received: Mutex::new(BTreeMap::new()),
            recv_buffer: Mutex::new(VecDeque::new()),
            metrics: Mutex::new(UtpPipelineMetrics::default()),
            max_out_of_order_packets: 512,
        }
    }

    /// Returns a snapshot of the pipeline telemetry metrics.
    pub fn pipeline_metrics(&self) -> UtpPipelineMetrics {
        let mut m = self.metrics.lock().unwrap().clone();
        let ctrl = self.controller.lock().unwrap();
        m.cwnd_bytes = ctrl.cwnd();
        m.rto_us = ctrl.rto_us();
        m.srtt_us = ctrl.srtt_us().unwrap_or(0);
        m.rttvar_us = ctrl.rttvar_us();
        m.out_of_order_buffered = self.out_of_order_received.lock().unwrap().len();
        m
    }

    /// Returns the current loss recovery state of the congestion controller.
    pub fn loss_recovery_state(&self) -> LossRecoveryState {
        self.controller.lock().unwrap().loss_recovery_state()
    }

    /// Returns true if the adapter is currently inside an active fast recovery epoch.
    pub fn is_in_loss_recovery(&self) -> bool {
        self.controller.lock().unwrap().is_in_recovery()
    }

    /// Returns all sequence numbers currently buffered out-of-order on the receiver.
    pub fn out_of_order_sequences(&self) -> Vec<u16> {
        self.out_of_order_received
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// Detects missing sequence ranges currently unreceived between `ack_nr + 1` and buffered packets.
    pub fn detect_receive_gaps(&self) -> Vec<(u16, u16)> {
        let ooo = self.out_of_order_sequences();
        let ack = *self.ack_nr.lock().unwrap();
        detect_receive_gaps(ack, &ooo)
    }

    /// Sets the maximum capacity of the out-of-order reassembly buffer.
    pub fn set_max_out_of_order_packets(&mut self, max: usize) {
        self.max_out_of_order_packets = max;
    }

    /// Returns the current congestion window (`cwnd`) of the adapter in bytes.
    pub fn cwnd(&self) -> u32 {
        self.controller.lock().unwrap().cwnd()
    }

    /// Sets the next outgoing sequence number (primarily for testing out-of-order delivery).
    pub fn set_seq_nr(&self, seq: u16) {
        *self.seq_nr.lock().unwrap() = seq;
    }

    /// Returns the estimated base delay in microseconds.
    pub fn base_delay(&self) -> u32 {
        self.controller.lock().unwrap().base_delay()
    }

    /// Returns the latest estimated queuing delay in microseconds.
    pub fn queuing_delay(&self) -> u32 {
        self.controller.lock().unwrap().queuing_delay()
    }

    /// Returns the number of bytes currently in flight (excluding SACKed packets).
    pub fn bytes_in_flight(&self) -> u32 {
        self.sent_packets
            .lock()
            .unwrap()
            .iter()
            .filter(|p| !p.sacked)
            .map(|p| p.payload.len() as u32)
            .sum()
    }

    /// Returns the number of sent packets currently marked as SACKed by the peer.
    pub fn sacked_packets_count(&self) -> usize {
        self.sent_packets
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.sacked)
            .count()
    }

    /// Returns the number of out-of-order packets currently buffered awaiting reassembly.
    pub fn out_of_order_count(&self) -> usize {
        self.out_of_order_received.lock().unwrap().len()
    }

    /// Manually records a sent packet into the unacknowledged queue (for testing loss and pipeline simulation).
    pub fn record_sent_packet(&self, seq: u16, payload: &[u8]) {
        let mut sent = self.sent_packets.lock().unwrap();
        sent.push_back(SentPacket {
            seq,
            timestamp: std::time::Instant::now(),
            payload: payload.to_vec(),
            sacked: false,
            retransmit_count: 0,
            last_sent_time: std::time::Instant::now(),
        });
    }

    /// Retransmits an unacknowledged data packet without advancing `seq_nr`.
    pub fn retransmit_data_packet(&self, seq: u16, payload: &[u8]) -> Result<(), BitTorrentError> {
        let ack = *self.ack_nr.lock().unwrap();

        let now_us = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros()
            & 0xFFFFFFFF) as u32;

        let last_ts = *self.last_remote_timestamp_us.lock().unwrap();
        let last_recv = *self.last_remote_recv_us.lock().unwrap();
        let timestamp_difference_us = if last_ts > 0 {
            last_recv.wrapping_sub(last_ts)
        } else {
            0
        };

        let header = UtpHeader {
            packet_type: UtpPacketType::Data,
            version: 1,
            extension: 0,
            connection_id: self.connection_id,
            timestamp_us: now_us,
            timestamp_difference_us,
            wnd_size: 1_048_576,
            seq_nr: seq,
            ack_nr: ack,
            extensions: Vec::new(),
        };

        let mut packet = header.encode();
        packet.extend_from_slice(payload);

        self.udp.send(&packet).map_err(BitTorrentError::Io)?;
        let mut m = self.metrics.lock().unwrap();
        m.packets_sent += 1;
        m.bytes_sent += payload.len() as u64;
        Ok(())
    }

    /// Checks inflight packets for Retransmission Timeouts (RTO) and retransmits the earliest expired packet.
    pub fn check_timeouts(&self) {
        let rto = { self.controller.lock().unwrap().current_rto_duration() };
        let mut timeout_pkt = None;
        {
            let mut sent = self.sent_packets.lock().unwrap();
            for pkt in sent.iter_mut() {
                if !pkt.sacked && pkt.last_sent_time.elapsed() >= rto {
                    timeout_pkt = Some((pkt.seq, pkt.payload.clone()));
                    pkt.last_sent_time = std::time::Instant::now();
                    pkt.retransmit_count += 1;
                    break; // Retransmit earliest timed-out packet per RFC 6298
                }
            }
        }

        if let Some((seq, payload)) = timeout_pkt {
            let _ = self.retransmit_data_packet(seq, &payload);
            let mut ctrl = self.controller.lock().unwrap();
            ctrl.on_rto_timeout();
            let mut m = self.metrics.lock().unwrap();
            m.retransmits_timeout += 1;
            m.loss_recovery_events += 1;
        }
    }

    /// Builds and sends a uTP packet of the given type with optional extension headers and payload.
    pub fn send_packet_with_extensions(
        &self,
        packet_type: UtpPacketType,
        payload: &[u8],
        extensions: Vec<UtpExtension>,
    ) -> Result<(), BitTorrentError> {
        let mut seq = self.seq_nr.lock().unwrap();
        let ack = self.ack_nr.lock().unwrap();

        let now_us = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros()
            & 0xFFFFFFFF) as u32;

        let last_ts = *self.last_remote_timestamp_us.lock().unwrap();
        let last_recv = *self.last_remote_recv_us.lock().unwrap();
        let timestamp_difference_us = if last_ts > 0 {
            last_recv.wrapping_sub(last_ts)
        } else {
            0
        };

        let first_ext = extensions.first().map(|e| e.extension_type()).unwrap_or(0);
        let header = UtpHeader {
            packet_type,
            version: 1,
            extension: first_ext,
            connection_id: self.connection_id,
            timestamp_us: now_us,
            timestamp_difference_us,
            wnd_size: 1_048_576,
            seq_nr: *seq,
            ack_nr: *ack,
            extensions: extensions.clone(),
        };

        let mut packet = header.encode();
        packet.extend_from_slice(payload);

        self.udp.send(&packet).map_err(BitTorrentError::Io)?;

        {
            let mut m = self.metrics.lock().unwrap();
            m.packets_sent += 1;
            if packet_type == UtpPacketType::Data {
                m.bytes_sent += payload.len() as u64;
            }
            if extensions
                .iter()
                .any(|e| e.extension_type() == UTP_EXT_SACK)
            {
                m.sack_packets_sent += 1;
            }
        }

        if packet_type == UtpPacketType::Data {
            let mut sent = self.sent_packets.lock().unwrap();
            sent.push_back(SentPacket {
                seq: *seq,
                timestamp: std::time::Instant::now(),
                payload: payload.to_vec(),
                sacked: false,
                retransmit_count: 0,
                last_sent_time: std::time::Instant::now(),
            });
            if sent.len() > 256 {
                sent.pop_front();
            }
        }

        *seq = seq.wrapping_add(1);
        Ok(())
    }

    /// Builds and sends a uTP packet of the given type with the supplied payload.
    pub fn send_packet(
        &self,
        packet_type: UtpPacketType,
        payload: &[u8],
    ) -> Result<(), BitTorrentError> {
        self.send_packet_with_extensions(packet_type, payload, Vec::new())
    }
}

impl AsyncSocket for UtpSocketAdapter {
    /// Receives one uTP DATA packet and copies its payload into `buf`.
    ///
    /// Loops until data is available, handling out-of-order reassembly and SACK generation.
    /// Returns `Ok(0)` if a `ST_RESET` packet is received or the socket is closed.
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(0);
        }

        // Drain any contiguous buffered payload from out-of-order reassembly first
        {
            let mut recv = self.recv_buffer.lock().unwrap();
            if !recv.is_empty() {
                let to_read = buf.len().min(recv.len());
                for (i, byte) in recv.drain(..to_read).enumerate() {
                    buf[i] = byte;
                }
                return Ok(to_read);
            }
        }

        let mut packet_buf = vec![0u8; 2048];
        loop {
            if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(0);
            }
            let n = match self.udp.recv(&mut packet_buf) {
                Ok(n) => n,
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    self.check_timeouts();
                    continue;
                }
                Err(ref e)
                    if e.raw_os_error() == Some(10054)
                        || e.kind() == std::io::ErrorKind::ConnectionReset =>
                {
                    // Windows WSAECONNRESET (10054) delivered when ICMP port unreachable is received
                    continue;
                }
                Err(e) => return Err(BitTorrentError::Io(e)),
            };
            if n < 20 {
                continue;
            }

            {
                let mut m = self.metrics.lock().unwrap();
                m.packets_recv += 1;
            }

            if let Ok(header) = UtpHeader::decode(&packet_buf[..n]) {
                let now_us = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros()
                    & 0xFFFFFFFF) as u32;

                // Track remote timestamps for our reflected timestamp_difference_us
                if header.timestamp_us > 0 {
                    *self.last_remote_timestamp_us.lock().unwrap() = header.timestamp_us;
                    *self.last_remote_recv_us.lock().unwrap() = now_us;
                }

                if header.sack_bitmask().is_some() {
                    let mut m = self.metrics.lock().unwrap();
                    m.sack_packets_recv += 1;
                }

                // Acknowledge sent packets, updating RTT and selective SACK bitmask
                let mut bytes_acked: u32 = 0;
                let mut packets_to_fast_retransmit = Vec::new();
                let mut highest_sacked_seq: Option<u16> = None;

                {
                    let mut sent = self.sent_packets.lock().unwrap();
                    let mut unacked = VecDeque::new();
                    while let Some(mut pkt) = sent.pop_front() {
                        if seq_acked(pkt.seq, header.ack_nr) {
                            if !pkt.sacked {
                                bytes_acked = bytes_acked.saturating_add(pkt.payload.len() as u32);
                                if pkt.retransmit_count == 0 {
                                    let sample_rtt_us = pkt.timestamp.elapsed().as_micros() as u32;
                                    self.controller.lock().unwrap().update_rtt(sample_rtt_us);
                                }
                            }
                        } else {
                            if let Some(mask) = header.sack_bitmask() {
                                if is_seq_sacked(header.ack_nr, mask, pkt.seq) && !pkt.sacked {
                                    pkt.sacked = true;
                                    bytes_acked =
                                        bytes_acked.saturating_add(pkt.payload.len() as u32);
                                    highest_sacked_seq =
                                        Some(highest_sacked_seq.map_or(pkt.seq, |m| {
                                            if seq_acked(m, pkt.seq) { pkt.seq } else { m }
                                        }));
                                }
                            }
                            unacked.push_back(pkt);
                        }
                    }

                    // SACK Fast Loss Detection (RFC 6675 / BEP 29):
                    // An unacknowledged packet is considered lost if at least 3 SACKed packets
                    // have arrived with sequence numbers ahead of it.
                    if let Some(highest_sacked) = highest_sacked_seq {
                        let sacked_seqs: Vec<u16> =
                            unacked.iter().filter(|p| p.sacked).map(|p| p.seq).collect();

                        for pkt in unacked.iter_mut() {
                            if !pkt.sacked
                                && seq_acked(pkt.seq, highest_sacked)
                                && pkt.seq != highest_sacked
                            {
                                let sacked_after = sacked_seqs
                                    .iter()
                                    .filter(|&&s| seq_acked(pkt.seq, s) && s != pkt.seq)
                                    .count();
                                if sacked_after >= 3
                                    && pkt.last_sent_time.elapsed()
                                        >= std::time::Duration::from_millis(15)
                                {
                                    packets_to_fast_retransmit.push((pkt.seq, pkt.payload.clone()));
                                    pkt.last_sent_time = std::time::Instant::now();
                                    pkt.retransmit_count += 1;
                                }
                            }
                        }
                    }

                    *sent = unacked;
                }

                // Update LEDBAT controller with ACK and delay sample
                let now_us_u64 = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() as u64;

                {
                    let mut ctrl = self.controller.lock().unwrap();
                    ctrl.set_remote_window(header.wnd_size);
                    let triggered_dup = ctrl.handle_ack(
                        header.ack_nr,
                        bytes_acked,
                        header.timestamp_difference_us,
                        now_us_u64,
                    );
                    if triggered_dup {
                        let mut m = self.metrics.lock().unwrap();
                        m.duplicate_acks_count += 3;
                    }
                }

                // Execute any fast retransmissions triggered by SACK analysis
                if !packets_to_fast_retransmit.is_empty() {
                    let is_burst = packets_to_fast_retransmit.len() >= 2;
                    let highest_sent = *self.seq_nr.lock().unwrap();
                    {
                        let mut ctrl = self.controller.lock().unwrap();
                        if ctrl.enter_recovery(highest_sent, is_burst) {
                            let mut m = self.metrics.lock().unwrap();
                            m.loss_recovery_events += 1;
                            if is_burst {
                                m.burst_loss_events += 1;
                            }
                        }
                    }

                    for (seq, payload) in &packets_to_fast_retransmit {
                        let _ = self.retransmit_data_packet(*seq, payload);
                        let mut m = self.metrics.lock().unwrap();
                        m.retransmits_fast += 1;
                    }
                }

                if header.packet_type == UtpPacketType::Reset {
                    return Ok(0); // Connection aborted by remote
                }

                if header.packet_type == UtpPacketType::Data {
                    let total_hdr = header.total_header_len();
                    let payload = if n >= total_hdr {
                        &packet_buf[total_hdr..n]
                    } else {
                        &[]
                    };

                    {
                        let mut m = self.metrics.lock().unwrap();
                        m.bytes_recv += payload.len() as u64;
                    }

                    let cumulative_ack = {
                        let mut ack_guard = self.ack_nr.lock().unwrap();
                        let current_ack = *ack_guard;

                        if current_ack == 0 {
                            // First data packet received
                            *ack_guard = header.seq_nr;
                            let mut recv = self.recv_buffer.lock().unwrap();
                            recv.extend(payload);
                        } else if header.seq_nr == current_ack.wrapping_add(1) {
                            // In-order arrival
                            *ack_guard = header.seq_nr;
                            let mut recv = self.recv_buffer.lock().unwrap();
                            recv.extend(payload);

                            // Stitch any consecutive out-of-order packets buffered
                            let mut ooo = self.out_of_order_received.lock().unwrap();
                            while let Some(next_p) = ooo.remove(&ack_guard.wrapping_add(1)) {
                                *ack_guard = ack_guard.wrapping_add(1);
                                recv.extend(next_p);
                            }
                            let mut m = self.metrics.lock().unwrap();
                            m.out_of_order_buffered = ooo.len();
                        } else if !seq_acked(header.seq_nr, current_ack) {
                            // Out-of-order packet ahead of expected sequence
                            let mut ooo = self.out_of_order_received.lock().unwrap();
                            let distance = header.seq_nr.wrapping_sub(current_ack);
                            if distance < 32768 && ooo.len() < self.max_out_of_order_packets {
                                ooo.insert(header.seq_nr, payload.to_vec());
                                let mut m = self.metrics.lock().unwrap();
                                m.out_of_order_buffered = ooo.len();
                                m.max_out_of_order_depth = m.max_out_of_order_depth.max(ooo.len());
                            }
                        }

                        *ack_guard
                    }; // ack_guard dropped here to prevent reentrancy deadlock when calling send_packet below

                    // Acknowledge receipt with ST_STATE, attaching SACK if out-of-order packets exist
                    let ooo_keys: Vec<u16> = self
                        .out_of_order_received
                        .lock()
                        .unwrap()
                        .keys()
                        .cloned()
                        .collect();
                    if !ooo_keys.is_empty() {
                        let sack_mask = build_sack_bitmask(cumulative_ack, &ooo_keys);
                        if !sack_mask.is_empty() {
                            let _ = self.send_packet_with_extensions(
                                UtpPacketType::State,
                                &[],
                                vec![UtpExtension::SelectiveAck(sack_mask)],
                            );
                        } else {
                            let _ = self.send_packet(UtpPacketType::State, &[]);
                        }
                    } else {
                        let _ = self.send_packet(UtpPacketType::State, &[]);
                    }

                    let mut recv = self.recv_buffer.lock().unwrap();
                    if !recv.is_empty() {
                        let to_read = buf.len().min(recv.len());
                        for (i, byte) in recv.drain(..to_read).enumerate() {
                            buf[i] = byte;
                        }
                        return Ok(to_read);
                    }
                }
            }
        }
    }

    /// Wraps `buf` in a `ST_DATA` uTP packet and sends it over UDP, respecting LEDBAT congestion window.
    ///
    /// Returns an error if the socket is already closed.
    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "Socket closed",
            )));
        }

        // Check for timeouts and retransmit if expired before writing new data
        self.check_timeouts();

        // Check congestion window before sending
        let start_wait = std::time::Instant::now();
        loop {
            let in_flight: u32 = self.bytes_in_flight();
            let can_send = self.controller.lock().unwrap().can_send(in_flight);
            if can_send || start_wait.elapsed() > std::time::Duration::from_millis(500) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        self.send_packet(UtpPacketType::Data, buf)?;
        Ok(buf.len())
    }

    /// Closes the connection by sending `ST_RESET` and marking the adapter as closed.
    ///
    /// Subsequent calls to `read()` or `write()` will return an error or `Ok(0)`.
    fn close(&self) {
        if !self.closed.swap(true, std::sync::atomic::Ordering::Relaxed) {
            // Notify the remote peer that we are aborting
            let _ = self.send_packet(UtpPacketType::Reset, &[]);
        }
    }
}
