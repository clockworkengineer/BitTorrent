//! Peer network stream wrapper
//!
//! Provides the `PeerNetwork` wrapper around a TCP network socket to handle
//! reading and writing of low-level BitTorrent wire messages and handshakes.

use crate::error::BitTorrentError;
use crate::io_traits::{AsyncSocket, Socket};
use crate::peer_message::PeerMessage;
use alloc::vec::Vec;
use std::sync::Arc;

#[cfg(feature = "std")]
use std::io::{Read, Write};
#[cfg(feature = "std")]
use std::net::TcpStream;
#[cfg(feature = "std")]
use std::sync::Mutex;

/// An `AsyncSocket` decorator that transparently encrypts outgoing bytes and decrypts incoming bytes using RC4 (MSE/PE).
#[cfg(all(feature = "std", feature = "mse"))]
pub struct EncryptedSocket {
    inner: Arc<Socket>,
    rc4_encrypt: Mutex<crate::mse::Rc4>,
    rc4_decrypt: Mutex<crate::mse::Rc4>,
}

#[cfg(all(feature = "std", feature = "mse"))]
impl core::fmt::Debug for EncryptedSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EncryptedSocket").finish()
    }
}

#[cfg(all(feature = "std", feature = "mse"))]
impl EncryptedSocket {
    /// Wraps an existing socket with RC4 encryption and decryption streams.
    pub fn new(inner: Arc<Socket>, encrypt: crate::mse::Rc4, decrypt: crate::mse::Rc4) -> Self {
        Self {
            inner,
            rc4_encrypt: Mutex::new(encrypt),
            rc4_decrypt: Mutex::new(decrypt),
        }
    }

    /// Access the underlying socket being decorated.
    pub fn inner(&self) -> &Arc<Socket> {
        &self.inner
    }
}

#[cfg(all(feature = "std", feature = "mse"))]
impl AsyncSocket for EncryptedSocket {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        let n = Box::pin(self.inner.read(buf)).await?;
        if n > 0 {
            let mut dec_guard = self.rc4_decrypt.lock().unwrap();
            dec_guard.encrypt(&mut buf[..n]);
        }
        Ok(n)
    }

    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        let mut encrypted = buf.to_vec();
        {
            let mut enc_guard = self.rc4_encrypt.lock().unwrap();
            enc_guard.encrypt(&mut encrypted);
        }
        Box::pin(self.inner.write(&encrypted)).await
    }

    fn close(&self) {
        self.inner.close();
    }
}

/// A socket communication helper for sending and receiving raw BitTorrent peer messages.
pub struct PeerNetwork {
    socket: Arc<Socket>,
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
    pub fn new(socket: Arc<Socket>) -> Self {
        PeerNetwork { socket }
    }

    /// Retrieves a clone of the underlying socket.
    pub fn socket(&self) -> Arc<Socket> {
        self.socket.clone()
    }

    /// Configures the RC4 encryption and decryption ciphers using the `EncryptedSocket` decorator.
    #[cfg(feature = "mse")]
    pub fn set_mse_ciphers(&mut self, encrypt: crate::mse::Rc4, decrypt: crate::mse::Rc4) {
        let encrypted_sock = EncryptedSocket::new(self.socket.clone(), encrypt, decrypt);
        self.socket = Arc::new(Socket::Encrypted(encrypted_sock));
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
        let mut retries = 0;
        while offset < buffer.len() {
            let limit = buffer.len() - offset;
            match self.read(&mut buffer[offset..], limit).await {
                Ok(0) => {
                    return Err(BitTorrentError::Parse(
                        "Unexpected EOF reading socket".into(),
                    ));
                }
                Ok(n) => {
                    offset += n;
                    retries = 0;
                }
                Err(err) => {
                    let is_timeout_or_wouldblock = match &err {
                        BitTorrentError::Io(e) => {
                            e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut
                        }
                        _ => false,
                    };
                    if is_timeout_or_wouldblock && offset > 0 && retries < 15 {
                        retries += 1;
                        #[cfg(feature = "std")]
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                    return Err(err);
                }
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

    /// Reads and decodes a BitTorrent connection handshake from the network stream into fixed-size byte arrays.
    pub async fn read_handshake_arrays(
        &self,
    ) -> Result<([u8; 20], [u8; 20], [u8; 8]), BitTorrentError> {
        let mut buffer = [0u8; crate::constants::INITIAL_HANDSHAKE_LENGTH];
        let mut offset = 0;
        let mut retries = 0;
        while offset < buffer.len() {
            let limit = buffer.len() - offset;
            match self.read(&mut buffer[offset..], limit).await {
                Ok(0) => {
                    return Err(BitTorrentError::Parse(
                        "Unexpected EOF reading handshake".into(),
                    ));
                }
                Ok(n) => {
                    offset += n;
                    retries = 0;
                }
                Err(err) => {
                    let is_timeout_or_wouldblock = match &err {
                        BitTorrentError::Io(e) => {
                            e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut
                        }
                        _ => false,
                    };
                    if is_timeout_or_wouldblock && retries < 3 {
                        retries += 1;
                        #[cfg(feature = "std")]
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        PeerMessage::handshake_decode_arrays(&buffer)
    }

    /// Reads and decodes a BitTorrent connection handshake from the network stream.
    pub async fn read_handshake(&self) -> Result<(Vec<u8>, Vec<u8>, [u8; 8]), BitTorrentError> {
        let (info_hash, peer_id, reserved) = self.read_handshake_arrays().await?;
        Ok((info_hash.to_vec(), peer_id.to_vec(), reserved))
    }

    /// Encodes and writes a high-level `PeerMessage` to the stream.
    pub async fn write_message(&self, message: PeerMessage<'_>) -> Result<usize, BitTorrentError> {
        let buffer = message.encode();
        Ok(self.write(&buffer).await?)
    }

    /// Reads the next message length prefix and body from the stream, returning the decoded `PeerMessage`.
    pub async fn read_message<'a>(
        &self,
        read_buffer: &'a mut [u8],
    ) -> Result<PeerMessage<'a>, BitTorrentError> {
        let mut length_buf = [0u8; 4];
        self.read_exact(&mut length_buf).await?;
        let length: usize = u32::from_be_bytes(length_buf).try_into().map_err(|_| {
            BitTorrentError::Parse("Message length exceeds memory pointer representation".into())
        })?;
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

        // Once the 4-byte length prefix has been read, the connection framing has committed
        // to a message of `length` bytes. Read the full payload into `read_buffer[..length]`.
        // If timeouts occur while reading the payload, retry so packet delivery delay
        // doesn't tear down framing.
        let mut offset = 0;
        let mut retries = 0;
        while offset < length {
            let limit = length - offset;
            match self.read(&mut read_buffer[offset..length], limit).await {
                Ok(0) => {
                    return Err(BitTorrentError::Parse(
                        "Unexpected EOF reading message payload".into(),
                    ));
                }
                Ok(n) => {
                    offset += n;
                    retries = 0;
                }
                Err(err) => {
                    let is_timeout_or_wouldblock = match &err {
                        BitTorrentError::Io(e) => {
                            e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut
                        }
                        _ => false,
                    };
                    if is_timeout_or_wouldblock && retries < 15 {
                        retries += 1;
                        #[cfg(feature = "std")]
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Failed to read full message payload (got {} of {} bytes): {}",
                        offset,
                        length,
                        err
                    )));
                }
            }
        }

        let id = read_buffer[0];
        // Enforce strict length limits by message ID
        match id {
            0..=3 | 14 | 15 => {
                if length != 1 {
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Invalid length {} for control message ID {}",
                        length,
                        id
                    )));
                }
            }
            4 | 13 | 17 => {
                if length != 5 {
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Invalid length {} for ID {}",
                        length,
                        id
                    )));
                }
            }
            6 | 8 | 16 => {
                if length != 13 {
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Invalid length {} for ID {}",
                        length,
                        id
                    )));
                }
            }
            9 => {
                if length != 3 {
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Invalid length {} for Port ID",
                        length
                    )));
                }
            }
            7 => {
                // Piece message: 9 bytes header + up to 16 KiB block payload
                if length < 9 || length > 16384 + 9 {
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Invalid length {} for Piece ID",
                        length
                    )));
                }
            }
            _ => {
                // Other IDs (Bitfield, Extended, etc.) can be up to buffer size
                if length > read_buffer.len() {
                    return Err(BitTorrentError::Parse(alloc::format!(
                        "Message length {} exceeds buffer size",
                        length
                    )));
                }
            }
        }

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
    stream: Arc<TcpStream>,
}

#[cfg(feature = "std")]
impl TcpSocket {
    /// Creates a new `TcpSocket` wrapping the standard `TcpStream`.
    pub fn new(stream: TcpStream) -> Self {
        // Ensure read/write timeouts are configured to prevent indefinite blocking
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(5)));
        TcpSocket {
            stream: Arc::new(stream),
        }
    }
}

#[cfg(feature = "std")]
impl AsyncSocket for TcpSocket {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        match (&*self.stream).read(buf) {
            Ok(n) => Ok(n),
            Err(e) => Err(BitTorrentError::Io(e)),
        }
    }

    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        match (&*self.stream).write_all(buf) {
            Ok(_) => Ok(buf.len()),
            Err(e) => Err(BitTorrentError::Io(e)),
        }
    }

    fn close(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// A standard TCP socket factory implementing `SocketFactory`.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct TcpSocketFactory {
    /// Connection establishment timeout.
    pub connect_timeout: std::time::Duration,
    /// Read timeout configured on the connected stream.
    pub read_timeout: std::time::Duration,
    /// Write timeout configured on the connected stream.
    pub write_timeout: std::time::Duration,
}

#[cfg(feature = "std")]
impl crate::io_traits::SocketFactory for TcpSocketFactory {
    fn connect(&self, ip: &str, port: u16) -> Result<Arc<Socket>, BitTorrentError> {
        let address = crate::util::format_socket_addr(ip, port);
        let addr = address
            .parse::<std::net::SocketAddr>()
            .map_err(|err| BitTorrentError::Parse(err.to_string()))?;
        let stream = TcpStream::connect_timeout(&addr, self.connect_timeout)?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(self.read_timeout));
        let _ = stream.set_write_timeout(Some(self.write_timeout));
        Ok(Arc::new(Socket::Tcp(TcpSocket::new(stream))))
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
        let socket = Arc::new(Socket::Tcp(TcpSocket::new(stream)));
        let network = PeerNetwork::new(socket);
        futures::executor::block_on(async {
            let written = network.write(&vec![0xAB; 64]).await.unwrap();
            assert_eq!(written, 64);
        });

        handle.join().unwrap();
    }
}
