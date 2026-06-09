//! Peer network stream wrapper
//!
//! Provides the `PeerNetwork` wrapper around a TCP network socket to handle
//! reading and writing of low-level BitTorrent wire messages and handshakes.

use crate::error::BitTorrentError;
use crate::peer_message::PeerMessage;
use crate::io_traits::AsyncSocket;
use std::sync::Arc;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::net::TcpStream;
#[cfg(feature = "std")]
use std::sync::Mutex;
#[cfg(feature = "std")]
use std::io::{Read, Write};
#[cfg(feature = "std")]
use core::pin::Pin;
#[cfg(feature = "std")]
use core::future::Future;

/// A socket communication helper for sending and receiving raw BitTorrent peer messages.
pub struct PeerNetwork {
    socket: Arc<dyn AsyncSocket>,
}

impl core::fmt::Debug for PeerNetwork {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PeerNetwork").finish()
    }
}

impl Clone for PeerNetwork {
    fn clone(&self) -> Self {
        PeerNetwork {
            socket: self.socket.clone(),
        }
    }
}

impl PeerNetwork {
    /// Creates a new `PeerNetwork` instance wrapping the given `AsyncSocket` implementation.
    pub fn new(socket: Arc<dyn AsyncSocket>) -> Self {
        PeerNetwork { socket }
    }

    /// Writes raw bytes to the underlying socket.
    pub async fn write(&self, buffer: &[u8]) -> Result<usize, BitTorrentError> {
        self.socket.write(buffer).await
    }

    /// Reads up to `length` bytes from the socket into the provided buffer.
    pub async fn read(&self, buffer: &mut [u8], length: usize) -> Result<usize, BitTorrentError> {
        self.socket.read(&mut buffer[..length]).await
    }

    /// Reads exactly enough bytes from the socket to fill the provided buffer.
    pub async fn read_exact(&self, buffer: &mut [u8]) -> Result<(), BitTorrentError> {
        let mut offset = 0;
        while offset < buffer.len() {
            let n = self.socket.read(&mut buffer[offset..]).await?;
            if n == 0 {
                return Err(BitTorrentError::Parse("Unexpected EOF reading socket".into()));
            }
            offset += n;
        }
        Ok(())
    }

    /// Encodes and writes a BitTorrent connection handshake to the network stream.
    pub async fn write_handshake(
        &self,
        info_hash: &[u8],
        peer_id: &[u8],
    ) -> Result<usize, BitTorrentError> {
        let buffer = PeerMessage::handshake_encode(info_hash, peer_id)?;
        Ok(self.write(&buffer).await?)
    }

    /// Reads and decodes a BitTorrent connection handshake from the network stream.
    pub async fn read_handshake(&self) -> Result<(Vec<u8>, Vec<u8>), BitTorrentError> {
        let mut buffer = [0u8; crate::constants::INITIAL_HANDSHAKE_LENGTH];
        self.read_exact(&mut buffer).await?;
        PeerMessage::handshake_decode(&buffer)
    }

    /// Encodes and writes a high-level `PeerMessage` to the stream.
    pub async fn write_message(&self, message: PeerMessage<'_>) -> Result<usize, BitTorrentError> {
        let buffer = message.encode();
        Ok(self.write(&buffer).await?)
    }

    /// Reads the next message length prefix and body from the stream, returning the decoded `PeerMessage`.
    pub async fn read_message<'a>(&self, read_buffer: &'a mut [u8]) -> Result<PeerMessage<'a>, BitTorrentError> {
        let mut length_buf = [0u8; 4];
        self.read_exact(&mut length_buf).await?;
        let length = u32::from_be_bytes(length_buf) as usize;
        if length == 0 {
            return Ok(PeerMessage::KeepAlive);
        }
        if length > read_buffer.len() {
            return Err(BitTorrentError::Parse(alloc::format!(
                "Message length {} exceeds read buffer size {}",
                length,
                read_buffer.len()
            )));
        }
        self.read_exact(&mut read_buffer[..length]).await?;
        PeerMessage::decode(&read_buffer[..length])
    }

    /// Starts asynchronous/on-demand read processing. Currently a no-op placeholder.
    pub fn start_reads(&self) {
        // current implementation reads on demand through read_message()
    }

    /// Closes the connection.
    pub fn close(&self) {
        self.socket.close();
    }
}

/// Standard library `TcpStream` wrapper implementing `AsyncSocket`.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct TcpSocket {
    stream: Arc<Mutex<TcpStream>>,
}

#[cfg(feature = "std")]
impl TcpSocket {
    /// Creates a new `TcpSocket` wrapping the standard `TcpStream`.
    pub fn new(stream: TcpStream) -> Self {
        // Ensure read/write timeouts are configured to prevent indefinite blocking
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
        TcpSocket {
            stream: Arc::new(Mutex::new(stream)),
        }
    }
}

#[cfg(feature = "std")]
impl AsyncSocket for TcpSocket {
    fn read<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>> {
        Box::pin(async move {
            let read_res = {
                let mut lock = self.stream.lock().unwrap();
                lock.read(buf)
            };
            match read_res {
                Ok(n) => Ok(n),
                Err(e) => Err(BitTorrentError::Io(e)),
            }
        })
    }

    fn write<'a>(
        &'a self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>> {
        Box::pin(async move {
            let write_res = {
                let mut lock = self.stream.lock().unwrap();
                lock.write_all(buf)
            };
            match write_res {
                Ok(_) => Ok(buf.len()),
                Err(e) => Err(BitTorrentError::Io(e)),
            }
        })
    }

    fn close(&self) {
        let lock = self.stream.lock().unwrap();
        let _ = lock.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    #[test]
    fn test_peer_network_write_all_reads_full_buffer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 64];
            let mut read = 0;
            while read < 64 {
                read += stream.read(&mut buf[read..]).unwrap();
            }
            assert_eq!(buf, vec![0xAB; 64]);
        });

        let stream = TcpStream::connect(addr).unwrap();
        let socket = Arc::new(TcpSocket::new(stream));
        let network = PeerNetwork::new(socket);
        futures::executor::block_on(async {
            let written = network.write(&vec![0xAB; 64]).await.unwrap();
            assert_eq!(written, 64);
        });

        handle.join().unwrap();
    }
}
