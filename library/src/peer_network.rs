use crate::error::BitTorrentError;
use crate::peer_message::PeerMessage;
use std::io::{Read, Result as IoResult, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct PeerNetwork {
    stream: Arc<Mutex<TcpStream>>,
    pub read_buffer: Vec<u8>,
    pub packet_length: usize,
}

impl PeerNetwork {
    pub fn new(stream: TcpStream) -> Self {
        PeerNetwork {
            stream: Arc::new(Mutex::new(stream)),
            read_buffer: vec![0u8; 1024 * 16 + 2 * 4 + 1],
            packet_length: 4,
        }
    }

    pub fn write(&self, buffer: &[u8]) -> IoResult<usize> {
        let mut lock = self.stream.lock().unwrap();
        lock.write_all(buffer)?;
        Ok(buffer.len())
    }

    pub fn read(&self, buffer: &mut [u8], length: usize) -> IoResult<usize> {
        let mut lock = self.stream.lock().unwrap();
        let read = lock.read(&mut buffer[..length])?;
        Ok(read)
    }

    pub fn read_exact(&self, buffer: &mut [u8]) -> IoResult<()> {
        let mut lock = self.stream.lock().unwrap();
        lock.read_exact(buffer)
    }

    pub fn write_handshake(
        &self,
        info_hash: &[u8],
        peer_id: &[u8],
    ) -> Result<usize, BitTorrentError> {
        let buffer = PeerMessage::handshake_encode(info_hash, peer_id)?;
        Ok(self.write(&buffer)?)
    }

    pub fn read_handshake(&self) -> Result<(Vec<u8>, Vec<u8>), BitTorrentError> {
        let mut buffer = [0u8; crate::constants::INITIAL_HANDSHAKE_LENGTH];
        self.read_exact(&mut buffer)?;
        PeerMessage::handshake_decode(&buffer)
    }

    pub fn write_message(&self, message: PeerMessage) -> Result<usize, BitTorrentError> {
        let buffer = message.encode();
        Ok(self.write(&buffer)?)
    }

    pub fn read_message(&mut self) -> Result<PeerMessage, BitTorrentError> {
        let mut length_buf = [0u8; 4];
        self.read_exact(&mut length_buf)?;
        let length = u32::from_be_bytes(length_buf) as usize;
        if length == 0 {
            self.packet_length = 0;
            self.read_buffer[..4].copy_from_slice(&length_buf);
            return Ok(PeerMessage::KeepAlive);
        }
        if length > self.read_buffer.len() {
            self.read_buffer.resize(length, 0);
        }
        let mut lock = self.stream.lock().unwrap();
        lock.read_exact(&mut self.read_buffer[..length])?;
        self.packet_length = length;
        PeerMessage::decode(&self.read_buffer[..length])
    }

    pub fn start_reads(&self) {
        // current implementation reads on demand through read_message()
    }

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
        let written = network.write(&vec![0xAB; 64]).unwrap();
        assert_eq!(written, 64);

        handle.join().unwrap();
    }
}
