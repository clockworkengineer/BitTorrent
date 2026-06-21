//! uTorrent Transport Protocol (uTP — BEP 29)
//!
//! Provides uTP packet framing, 20-byte header encode/decode, and a
//! [`UtpSocketAdapter`] that wraps a UDP socket as an [`AsyncSocket`]-compatible
//! peer transport.
//!
//! ## Current Scope
//!
//! This module implements the **framing and connection state machine** (SYN → DATA → ACK → RESET)
//! but does **not** implement the full LEDBAT congestion control algorithm.
//! The receive window (`wnd_size`) is fixed at 1 MiB and is not dynamically adjusted.
//!
//! See [docs/utp.md](https://github.com/clockworkengineer/BitTorrent/blob/main/docs/utp.md)
//! for the complete header layout and protocol state machine diagram.

use crate::error::BitTorrentError;
use crate::io_traits::AsyncSocket;
use std::sync::{Arc, Mutex};
use std::net::UdpSocket;

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
#[derive(Debug, Clone)]
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
    /// Estimated one-way propagation delay: `sender_ts - last_received_ts`.
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

/// Adapts a UDP socket to the [`AsyncSocket`] trait using the uTP framing protocol.
///
/// `UtpSocketAdapter` implements the uTP connection state machine:
/// - **`connect()`** — sends a `ST_SYN` packet to initiate the handshake.
/// - **`write()`** — wraps the payload in a `ST_DATA` packet with an incrementing sequence number.
/// - **`read()`** — receives UDP datagrams, parses uTP headers, sends `ST_STATE` ACKs for data packets.
/// - **`close()`** — sends `ST_RESET` and marks the socket as closed.
///
/// > **Note**: The receive window (`wnd_size`) is currently fixed at 1 MiB.
/// > LEDBAT-based dynamic window sizing is a planned future enhancement.
#[derive(Debug)]
pub struct UtpSocketAdapter {
    udp: Arc<UdpSocket>,
    connection_id: u16,
    seq_nr: Mutex<u16>,
    ack_nr: Mutex<u16>,
    closed: std::sync::atomic::AtomicBool,
    // LEDBAT states
    cwnd: Mutex<u32>,
    base_delay: Mutex<u32>,
    sent_packets: Mutex<std::collections::VecDeque<(u16, std::time::Instant, Vec<u8>)>>,
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
        let adapter = UtpSocketAdapter {
            udp: Arc::new(udp),
            connection_id,
            seq_nr: Mutex::new(1),
            ack_nr: Mutex::new(0),
            closed: std::sync::atomic::AtomicBool::new(false),
            cwnd: Mutex::new(1460),
            base_delay: Mutex::new(u32::MAX),
            sent_packets: Mutex::new(std::collections::VecDeque::new()),
        };

        // Send ST_SYN handshake packet
        adapter.send_packet(UtpPacketType::Syn, &[])?;

        Ok(adapter)
    }

    /// Builds and sends a uTP packet of the given type with the supplied payload.
    ///
    /// Acquires the sequence and acknowledgement number locks, constructs the
    /// 20-byte header, appends the payload, and sends the combined packet over UDP.
    /// Increments the sequence number after a successful send.
    fn send_packet(&self, packet_type: UtpPacketType, payload: &[u8]) -> Result<(), BitTorrentError> {
        let mut seq = self.seq_nr.lock().unwrap();
        let ack = self.ack_nr.lock().unwrap();

        let timestamp_us = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() & 0xFFFFFFFF) as u32;

        let header = UtpHeader {
            packet_type,
            version: 1,
            extension: 0,
            connection_id: self.connection_id,
            timestamp_us,
            timestamp_difference_us: 0,
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
            if sent.len() > 128 {
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
            let n = self.udp.recv(&mut packet_buf).map_err(BitTorrentError::Io)?;
            if n < 20 {
                continue;
            }

            if let Ok(header) = UtpHeader::decode(&packet_buf[..20]) {
                // Track the remote peer's sequence number for our ACK field
                {
                    let mut ack = self.ack_nr.lock().unwrap();
                    *ack = header.seq_nr;
                }

                // Compute dynamic delay for LEDBAT
                let now_us = (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros() & 0xFFFFFFFF) as u32;

                if header.timestamp_us > 0 {
                    let current_delay = now_us.saturating_sub(header.timestamp_us);
                    let mut base = self.base_delay.lock().unwrap();
                    if current_delay < *base {
                        *base = current_delay;
                    }
                    let queuing_delay = current_delay.saturating_sub(*base);

                    // LEDBAT adjustment: target delay is 100ms (100_000 microseconds)
                    let target_delay = 100_000;
                    let mut cwnd_val = self.cwnd.lock().unwrap();
                    if queuing_delay < target_delay {
                        *cwnd_val = cwnd_val.saturating_add(146);
                    } else {
                        *cwnd_val = (*cwnd_val).saturating_sub(146).max(1460);
                    }
                }

                // Acknowledge sent packets
                {
                    let mut sent = self.sent_packets.lock().unwrap();
                    sent.retain(|&(seq, _, _)| {
                        let diff = header.ack_nr.wrapping_sub(seq);
                        diff > 32768
                    });
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

    /// Wraps `buf` in a `ST_DATA` uTP packet and sends it over UDP.
    ///
    /// Returns an error if the socket is already closed.
    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "Socket closed",
            )));
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
