use core::pin::Pin;
use core::future::Future;
use alloc::boxed::Box;
use crate::error::BitTorrentError;

/// A hardware-agnostic asynchronous socket trait.
pub trait AsyncSocket: Send + Sync {
    /// Reads up to `buf.len()` bytes asynchronously from the socket.
    fn read<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>>;

    /// Writes raw bytes asynchronously to the socket.
    fn write<'a>(
        &'a self,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>>;

    /// Shuts down the socket connection.
    fn close(&self);
}

/// A hardware-agnostic block storage trait.
pub trait BlockStorage: Send + Sync {
    /// Writes a data block at the specified absolute byte offset.
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError>;

    /// Reads up to `buffer.len()` bytes from the specified absolute byte offset.
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError>;
}

/// A mock socket implementation for testing without real network I/O.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct MockSocket {
    pub rx: std::sync::Mutex<std::sync::mpsc::Receiver<alloc::vec::Vec<u8>>>,
    pub tx: std::sync::Mutex<std::sync::mpsc::Sender<alloc::vec::Vec<u8>>>,
    pub read_buf: std::sync::Mutex<alloc::vec::Vec<u8>>,
    pub closed: std::sync::atomic::AtomicBool,
}

#[cfg(feature = "std")]
impl MockSocket {
    /// Creates a mock socket and returns it along with channels to write to its input
    /// and read from its output externally.
    pub fn new() -> (
        Self,
        std::sync::mpsc::Sender<alloc::vec::Vec<u8>>,
        std::sync::mpsc::Receiver<alloc::vec::Vec<u8>>,
    ) {
        let (in_tx, in_rx) = std::sync::mpsc::channel();
        let (out_tx, out_rx) = std::sync::mpsc::channel();
        (
            MockSocket {
                rx: std::sync::Mutex::new(in_rx),
                tx: std::sync::Mutex::new(out_tx),
                read_buf: std::sync::Mutex::new(alloc::vec::Vec::new()),
                closed: std::sync::atomic::AtomicBool::new(false),
            },
            in_tx,
            out_rx,
        )
    }
}

#[cfg(feature = "std")]
impl AsyncSocket for MockSocket {
    fn read<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>> {
        Box::pin(async move {
            if self.closed.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(0);
            }
            let mut read_guard = self.read_buf.lock().unwrap();
            if read_guard.is_empty() {
                let rx_guard = self.rx.lock().unwrap();
                match rx_guard.recv() {
                    Ok(data) => {
                        read_guard.extend_from_slice(&data);
                    }
                    Err(_) => {
                        return Ok(0); // Sender dropped / EOF
                    }
                }
            }
            let to_read = buf.len().min(read_guard.len());
            buf[..to_read].copy_from_slice(&read_guard[..to_read]);
            read_guard.drain(..to_read);
            Ok(to_read)
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
            let tx_guard = self.tx.lock().unwrap();
            match tx_guard.send(buf.to_vec()) {
                Ok(_) => Ok(buf.len()),
                Err(_) => Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Receiver dropped",
                ))),
            }
        })
    }

    fn close(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// An in-memory block storage implementation.
#[derive(Debug)]
pub struct MemStorage {
    #[cfg(feature = "std")]
    data: std::sync::RwLock<alloc::vec::Vec<u8>>,
    #[cfg(not(feature = "std"))]
    data: core::cell::UnsafeCell<alloc::vec::Vec<u8>>,
}

#[cfg(not(feature = "std"))]
unsafe impl Send for MemStorage {}
#[cfg(not(feature = "std"))]
unsafe impl Sync for MemStorage {}

impl MemStorage {
    /// Creates a new in-memory storage of the given size.
    pub fn new(size: usize) -> Self {
        MemStorage {
            #[cfg(feature = "std")]
            data: std::sync::RwLock::new(alloc::vec![0u8; size]),
            #[cfg(not(feature = "std"))]
            data: core::cell::UnsafeCell::new(alloc::vec![0u8; size]),
        }
    }
}

impl BlockStorage for MemStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        let start = offset as usize;
        let end = start + data.len();
        #[cfg(feature = "std")]
        {
            let mut guard = self.data.write().unwrap();
            if end > guard.len() {
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "MemStorage write out of bounds",
                )));
            }
            guard[start..end].copy_from_slice(data);
            Ok(())
        }
        #[cfg(not(feature = "std"))]
        {
            // SAFETY: In no_std, we assume single-threaded context.
            unsafe {
                let ptr = self.data.get();
                let vec_ref = &mut *ptr;
                let len = vec_ref.len();
                if end > len {
                    return Err(BitTorrentError::Parse("MemStorage write out of bounds".into()));
                }
                vec_ref[start..end].copy_from_slice(data);
            }
            Ok(())
        }
    }

    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        let start = offset as usize;
        let end = start + buffer.len();
        #[cfg(feature = "std")]
        {
            let guard = self.data.read().unwrap();
            if end > guard.len() {
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "MemStorage read out of bounds",
                )));
            }
            buffer.copy_from_slice(&guard[start..end]);
            Ok(buffer.len())
        }
        #[cfg(not(feature = "std"))]
        {
            // SAFETY: In no_std, we assume single-threaded context.
            unsafe {
                let ptr = self.data.get();
                let vec_ref = &*ptr;
                let len = vec_ref.len();
                if end > len {
                    return Err(BitTorrentError::Parse("MemStorage read out of bounds".into()));
                }
                buffer.copy_from_slice(&vec_ref[start..end]);
            }
            Ok(buffer.len())
        }
    }
}

/// A hardware-agnostic socket factory trait.
#[cfg(feature = "std")]
pub trait SocketFactory: Send + Sync + std::fmt::Debug {
    /// Establishes a socket connection to target IP and port.
    fn connect(&self, ip: &str, port: u16) -> Result<alloc::sync::Arc<dyn AsyncSocket>, BitTorrentError>;
}

/// A hardware-agnostic HTTP client trait.
#[cfg(all(feature = "std", feature = "http-tracker"))]
pub trait HttpClient: Send + Sync + std::fmt::Debug {
    /// Performs an HTTP GET request to the target URL, returning the response body bytes.
    fn get(&self, url: &str) -> Result<alloc::vec::Vec<u8>, BitTorrentError>;
}

/// Default HTTP client implementation using `ureq`.
#[cfg(all(feature = "std", feature = "http-tracker"))]
#[derive(Debug)]
pub struct UreqHttpClient;

#[cfg(all(feature = "std", feature = "http-tracker"))]
impl HttpClient for UreqHttpClient {
    fn get(&self, url: &str) -> Result<alloc::vec::Vec<u8>, BitTorrentError> {
        let response = ureq::get(url).call().map_err(|err| {
            BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                err.to_string(),
            ))
        })?;
        let mut body = alloc::vec::Vec::new();
        use std::io::Read;
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|err| {
                BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err.to_string(),
                ))
            })?;
        Ok(body)
    }
}
