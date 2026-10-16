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
use std::collections::VecDeque;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

/// The five packet types defined by the uTP protocol (BEP 29).
///
/// The type is encoded in the upper 4 bits of the first header byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtpPacketType {
    /// `ST_DATA (0)` — carries payload bytes.
    Data  = 0,
    /// `ST_FIN (1)` — graceful connection close (no payload).
    Ack   = 1,
    /// `ST_SYN (2)` — initiates a new connection.
    Syn   = 2,
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

/// The fixed 20-byte uTP packet header.
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
}

impl UtpHeader {
    /// Decodes a 20-byte uTP header from the start of a buffer.
    ///
    /// Returns an error if `buf` is shorter than 20 bytes or the packet type
    /// field contains an unrecognized value.
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
        })
    }

    /// Serializes the header into exactly 20 bytes (big-endian).
    ///
    /// The `packet_type` occupies the upper 4 bits of byte 0 and `version` the lower 4 bits.
    pub fn encode(&self) -> [u8; 20] {
        let mut buf = [0u8; 20];
        buf[0] = ((self.packet_type as u8) << 4) | (self.version & 0x0F);
        buf[1] = self.extension;
        buf[2..4].copy_from_slice(&self.connection_id.to_be_bytes());
        buf[4..8].copy_from_slice(&self.timestamp_us.to_be_bytes());
        buf[8..12].copy_from_slice(&self.timestamp_difference_us.to_be_bytes());
        buf[12..16].copy_from_slice(&self.wnd_size.to_be_bytes());
        buf[16..18].copy_from_slice(&self.seq_nr.to_be_bytes());
        buf[18..20].copy_from_slice(&self.ack_nr.to_be_bytes());
        buf
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

/// LEDBAT (Low Extra Delay Background Transport — RFC 6817) Congestion Controller.
///
/// Implements delay-based congestion control:
/// - Target queuing delay: 100 ms (100,000 µs).
/// - Dynamic `cwnd` scaling proportional to `(TARGET - queuing_delay) / TARGET`.
/// - Fast Retransmit / Multiplicative Decrease on 3 duplicate ACKs or packet loss (`cwnd = max(cwnd / 2, MIN_CWND)`).
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
        }
    }

    /// Updates the remote peer's advertised receive window (`wnd_size`).
    pub fn set_remote_window(&mut self, wnd_size: u32) {
        if wnd_size > 0 {
            self.remote_wnd_size = wnd_size;
        }
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
            let off_target = (self.target_delay_us as f64 - queuing_delay as f64) / (self.target_delay_us as f64);
            // Bound off_target to [-1.0, 1.0] to prevent extreme fluctuations
            let off_target = off_target.clamp(-1.0, 1.0);

            let delta = (off_target * (bytes_acked as f64) * (self.mss as f64)) / self.cwnd_float;
            self.cwnd_float = (self.cwnd_float + delta).clamp(self.min_cwnd as f64, self.max_cwnd as f64);
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

    /// Alias for [`on_loss`].
    pub fn on_packet_loss(&mut self) -> u32 {
        self.on_loss()
    }

    /// Handles an incoming acknowledgement number, checking for duplicate ACKs.
    ///
    /// Returns `true` if Fast Retransmit was triggered (3 duplicate ACKs reached).
    pub fn handle_ack(&mut self, ack_nr: u16, bytes_acked: u32, delay_sample_us: u32, now_us: u64) -> bool {
        if bytes_acked > 0 {
            self.duplicate_acks = 0;
            self.last_ack_nr = ack_nr;
            self.on_ack(bytes_acked, delay_sample_us, now_us);
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
    sent_packets: Mutex<VecDeque<(u16, std::time::Instant, Vec<u8>)>>,
}

impl UtpSocketAdapter {
    /// Connects to a remote peer and returns a ready `UtpSocketAdapter`.
    ///
    /// Binds a local UDP socket on an ephemeral port, connects it to `ip:port`,
    /// and sends an initial `ST_SYN` packet to begin the uTP handshake.
    ///
    /// Returns an error if the UDP socket cannot be bound or the `ST_SYN` cannot be sent.
    pub fn connect(ip: &str, port: u16) -> Result<Self, BitTorrentError> {
        let udp = UdpSocket::bind("0.0.0.0:0").map_err(BitTorrentError::Io)?;
        udp.connect(format!("{}:{}", ip, port)).map_err(BitTorrentError::Io)?;

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
        }
    }

    /// Returns the current congestion window (`cwnd`) of the adapter in bytes.
    pub fn cwnd(&self) -> u32 {
        self.controller.lock().unwrap().cwnd()
    }

    /// Returns the estimated base delay in microseconds.
    pub fn base_delay(&self) -> u32 {
        self.controller.lock().unwrap().base_delay()
    }

    /// Returns the latest estimated queuing delay in microseconds.
    pub fn queuing_delay(&self) -> u32 {
        self.controller.lock().unwrap().queuing_delay()
    }

    /// Returns the number of bytes currently in flight.
    pub fn bytes_in_flight(&self) -> u32 {
        self.sent_packets.lock().unwrap().iter().map(|(_, _, p)| p.len() as u32).sum()
    }

    /// Builds and sends a uTP packet of the given type with the supplied payload.
    ///
    /// Acquires the sequence and acknowledgement number locks, constructs the
    /// 20-byte header with current timestamps and reflected delay, appends the payload,
    /// and sends the combined packet over UDP.
    pub fn send_packet(&self, packet_type: UtpPacketType, payload: &[u8]) -> Result<(), BitTorrentError> {
        let mut seq = self.seq_nr.lock().unwrap();
        let ack = self.ack_nr.lock().unwrap();

        let now_us = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() & 0xFFFFFFFF) as u32;

        let last_ts = *self.last_remote_timestamp_us.lock().unwrap();
        let last_recv = *self.last_remote_recv_us.lock().unwrap();
        let timestamp_difference_us = if last_ts > 0 {
            last_recv.wrapping_sub(last_ts)
        } else {
            0
        };

        let header = UtpHeader {
            packet_type,
            version: 1,
            extension: 0,
            connection_id: self.connection_id,
            timestamp_us: now_us,
            timestamp_difference_us,
            wnd_size: 1_048_576,
            seq_nr: *seq,
            ack_nr: *ack,
        };

        let mut packet = header.encode().to_vec();
        packet.extend_from_slice(payload);

        self.udp.send(&packet).map_err(BitTorrentError::Io)?;

        if packet_type == UtpPacketType::Data {
            let mut sent = self.sent_packets.lock().unwrap();
            sent.push_back((*seq, std::time::Instant::now(), payload.to_vec()));
            if sent.len() > 256 {
                sent.pop_front();
            }
        }

        *seq = seq.wrapping_add(1);
        Ok(())
    }
}

impl AsyncSocket for UtpSocketAdapter {
    /// Receives one uTP DATA packet and copies its payload into `buf`.
    ///
    /// Loops until a `ST_DATA` packet is received (skipping non-data packets).
    /// Sends a `ST_STATE` ACK for each data packet received.
    /// Returns `Ok(0)` if a `ST_RESET` packet is received or the socket is closed.
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(0);
        }

        let mut packet_buf = vec![0u8; 2048];
        loop {
            if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(0);
            }
            let n = match self.udp.recv(&mut packet_buf) {
                Ok(n) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                    continue;
                }
                Err(e) => return Err(BitTorrentError::Io(e)),
            };
            if n < 20 {
                continue;
            }

            if let Ok(header) = UtpHeader::decode(&packet_buf[..20]) {
                let now_us = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() & 0xFFFFFFFF) as u32;

                // Track the remote peer's sequence number for our ACK field
                {
                    let mut ack = self.ack_nr.lock().unwrap();
                    *ack = header.seq_nr;
                }

                // Track remote timestamps for our reflected timestamp_difference_us
                if header.timestamp_us > 0 {
                    *self.last_remote_timestamp_us.lock().unwrap() = header.timestamp_us;
                    *self.last_remote_recv_us.lock().unwrap() = now_us;
                }

                // Acknowledge sent packets and calculate newly acked bytes
                let mut bytes_acked: u32 = 0;
                {
                    let mut sent = self.sent_packets.lock().unwrap();
                    let mut unacked = VecDeque::new();
                    while let Some((seq, ts, payload)) = sent.pop_front() {
                        if seq_acked(seq, header.ack_nr) {
                            bytes_acked = bytes_acked.saturating_add(payload.len() as u32);
                        } else {
                            unacked.push_back((seq, ts, payload));
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
                    ctrl.handle_ack(header.ack_nr, bytes_acked, header.timestamp_difference_us, now_us_u64);
                }

                if header.packet_type == UtpPacketType::Reset {
                    return Ok(0); // Connection aborted by remote
                }

                if header.packet_type == UtpPacketType::Data {
                    let payload_len = n - 20;
                    let to_read = buf.len().min(payload_len);
                    buf[..to_read].copy_from_slice(&packet_buf[20..20 + to_read]);

                    // Acknowledge receipt with ST_STATE
                    let _ = self.send_packet(UtpPacketType::State, &[]);

                    return Ok(to_read);
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

        // Check congestion window before sending
        let start_wait = std::time::Instant::now();
        loop {
            let in_flight: u32 = self.sent_packets.lock().unwrap().iter().map(|(_, _, p)| p.len() as u32).sum();
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
