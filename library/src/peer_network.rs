//! Peer network stream wrapper
//!
//! Provides the `PeerNetwork` wrapper around a TCP network socket to handle
//! reading and writing of low-level BitTorrent wire messages and handshakes.

use crate::error::BitTorrentError;
use crate::peer_message::PeerMessage;
use std::io::{Read, Result as IoResult, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use alloc::vec::Vec;

/// A socket communication helper for sending and receiving raw BitTorrent peer messages.
#[derive(Debug, Clone)]
pub struct PeerNetwork {
    stream: Arc<Mutex<TcpStream>>,
}

impl PeerNetwork {
    /// Creates a new `PeerNetwork` instance wrapping the given `TcpStream`.
    pub fn new(stream: TcpStream) -> Self {
        let _ = stream.set_nonblocking(true);
        PeerNetwork {
            stream: Arc::new(Mutex::new(stream)),
        }
    }

    /// Writes raw bytes to the underlying TCP stream.
    pub async fn write(&self, buffer: &[u8]) -> IoResult<usize> {
        let mut offset = 0;
        while offset < buffer.len() {
            let write_res = {
                let mut lock = self.stream.lock().unwrap();
                lock.write(&buffer[offset..])
            };
            match write_res {
                Ok(n) => {
                    offset += n;
                    if offset >= buffer.len() {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    crate::util::yield_now().await;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(buffer.len())
    }

    /// Reads up to `length` bytes from the stream into the provided buffer.
    pub async fn read(&self, buffer: &mut [u8], length: usize) -> IoResult<usize> {
        loop {
            let read_res = {
                let mut lock = self.stream.lock().unwrap();
                lock.read(&mut buffer[..length])
            };
            match read_res {
                Ok(n) => return Ok(n),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    crate::util::yield_now().await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Reads exactly enough bytes from the stream to fill the provided buffer.
    pub async fn read_exact(&self, buffer: &mut [u8]) -> IoResult<()> {
        let mut offset = 0;
        while offset < buffer.len() {
            let read_res = {
                let mut lock = self.stream.lock().unwrap();
                lock.read(&mut buffer[offset..])
            };
            match read_res {
                Ok(0) => return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                )),
                Ok(n) => {
                    offset += n;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    crate::util::yield_now().await;
                }
                Err(e) => return Err(e),
            }
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
    pub async fn read_message<'a>(&self, read_buffer: &'a mut Vec<u8>) -> Result<PeerMessage<'a>, BitTorrentError> {
        let mut length_buf = [0u8; 4];
        self.read_exact(&mut length_buf).await?;
        let length = u32::from_be_bytes(length_buf) as usize;
        if length == 0 {
            return Ok(PeerMessage::KeepAlive);
        }
        if length > read_buffer.len() {
            read_buffer.resize(length, 0);
        }
        self.read_exact(&mut read_buffer[..length]).await?;
        PeerMessage::decode(&read_buffer[..length])
    }

    /// Starts asynchronous/on-demand read processing. Currently a no-op placeholder.
    pub fn start_reads(&self) {
        // current implementation reads on demand through read_message()
    }

    /// Closes the connection by shutting down both read and write halves of the stream.
    pub fn close(&self) {
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
        let network = PeerNetwork::new(stream);
        futures::executor::block_on(async {
            let written = network.write(&vec![0xAB; 64]).await.unwrap();
            assert_eq!(written, 64);
        });

        handle.join().unwrap();
    }
}
