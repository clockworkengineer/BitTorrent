//! uTorrent Transport Protocol (uTP - BEP 29)
//!
//! Provides packet framing, parsing, and connection wrapper implementing the `AsyncSocket` trait.

use crate::error::BitTorrentError;
use crate::io_traits::AsyncSocket;
use std::sync::{Arc, Mutex};
use std::net::UdpSocket;
use core::pin::Pin;
use core::future::Future;

/// uTP Packet Types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtpPacketType {
    Data = 0,
    Ack = 1,
    Syn = 2,
    Reset = 3,
    State = 4,
}

impl UtpPacketType {
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

/// uTP Packet Header
#[derive(Debug, Clone)]
pub struct UtpHeader {
    pub packet_type: UtpPacketType,
    pub version: u8,
    pub extension: u8,
    pub connection_id: u16,
    pub timestamp_us: u32,
    pub timestamp_difference_us: u32,
    pub wnd_size: u32,
    pub seq_nr: u16,
    pub ack_nr: u16,
}

impl UtpHeader {
    /// Decodes a 20-byte uTP header from the start of a buffer.
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

    /// Encodes the header into a 20-byte vector.
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

/// A lightweight uTP socket wrapper implementing the AsyncSocket trait.
pub struct UtpSocketAdapter {
    udp: Arc<UdpSocket>,
    connection_id: u16,
    seq_nr: Mutex<u16>,
    ack_nr: Mutex<u16>,
    closed: std::sync::atomic::AtomicBool,
}

impl UtpSocketAdapter {
    /// Connects to a remote target and returns a uTP socket adapter wrapper.
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
        };
        
        // Send ST_SYN handshake packet
        adapter.send_packet(UtpPacketType::Syn, &[])?;
        
        Ok(adapter)
    }

    /// Encapsulates payload bytes in a uTP packet and writes to the UDP socket.
    fn send_packet(&self, packet_type: UtpPacketType, payload: &[u8]) -> Result<(), BitTorrentError> {
        let mut seq = self.seq_nr.lock().unwrap();
        let ack = self.ack_nr.lock().unwrap();
        
        let header = UtpHeader {
            packet_type,
            version: 1,
            extension: 0,
            connection_id: self.connection_id,
            timestamp_us: 0,
            timestamp_difference_us: 0,
            wnd_size: 1_048_576, // 1MB standard window
            seq_nr: *seq,
            ack_nr: *ack,
        };
        
        let mut packet = header.encode().to_vec();
        packet.extend_from_slice(payload);
        
        self.udp.send(&packet).map_err(BitTorrentError::Io)?;
        *seq = seq.wrapping_add(1);
        Ok(())
    }
}

impl AsyncSocket for UtpSocketAdapter {
    fn read<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>> {
        Box::pin(async move {
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
                    // Update our ACK number matching the incoming sequence
                    {
                        let mut ack = self.ack_nr.lock().unwrap();
                        *ack = header.seq_nr;
                    }
                    
                    if header.packet_type == UtpPacketType::Reset {
                        return Ok(0); // Reset connection
                    }
                    
                    if header.packet_type == UtpPacketType::Data {
                        let payload_len = n - 20;
                        let to_read = buf.len().min(payload_len);
                        buf[..to_read].copy_from_slice(&packet_buf[20..20 + to_read]);
                        
                        // Send ST_ACK acknowledging receipt
                        let _ = self.send_packet(UtpPacketType::State, &[]);
                        
                        return Ok(to_read);
                    }
                }
            }
        })
    }

    fn write<'a>(
        &'a self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>> {
        Box::pin(async move {
            if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted,
                    "Socket closed",
                )));
            }
            
            self.send_packet(UtpPacketType::Data, buf)?;
            Ok(buf.len())
        })
    }

    fn close(&self) {
        if !self.closed.swap(true, std::sync::atomic::Ordering::Relaxed) {
            // Send ST_RESET to notify peer
            let _ = self.send_packet(UtpPacketType::Reset, &[]);
        }
    }
}
