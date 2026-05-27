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
        lock.write(buffer)
    }

    pub fn read(&self, buffer: &mut [u8], length: usize) -> IoResult<usize> {
        let mut lock = self.stream.lock().unwrap();
        let read = lock.read(&mut buffer[..length])?;
        Ok(read)
    }

    pub fn start_reads(&self) {
        // Placeholder: background message processing can be added later.
    }

    pub fn close(&self) {
        let lock = self.stream.lock().unwrap();
        let _ = lock.shutdown(std::net::Shutdown::Both);
    }
}
