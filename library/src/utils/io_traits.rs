use crate::error::BitTorrentError;

/// A hardware-agnostic asynchronous socket trait.
#[allow(async_fn_in_trait)]
pub trait AsyncSocket: Send + Sync {
    /// Reads up to `buf.len()` bytes asynchronously from the socket.
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError>;

    /// Writes raw bytes asynchronously to the socket.
    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError>;

    /// Shuts down the socket connection.
    fn close(&self);
}

/// Interface Segregation Principle: Read-only block storage view.
pub trait BlockReader: Send + Sync {
    /// Reads up to `buffer.len()` bytes from the specified absolute byte offset.
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError>;
}

/// Interface Segregation Principle: Write-only block storage view.
pub trait BlockWriter: Send + Sync {
    /// Writes a data block at the specified absolute byte offset.
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError>;

    /// Flushes any pending dirty or buffered data to underlying storage.
    fn flush(&self) -> Result<(), BitTorrentError> {
        Ok(())
    }

    /// Discards cached uncommitted data for the specified range without flushing (default: no-op).
    fn discard_range(&self, _offset: u64, _length: u64) {}
}

/// A hardware-agnostic block storage trait combining reading and writing views.
pub trait BlockStorage: BlockReader + BlockWriter {}

impl<T: BlockReader + BlockWriter + ?Sized> BlockStorage for T {}

/// A lightweight spinlock implementation for no_std environments.
#[derive(Debug)]
pub struct SpinLock<T> {
    lock: core::sync::atomic::AtomicBool,
    data: core::cell::UnsafeCell<T>,
}

unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub fn new(data: T) -> Self {
        Self {
            lock: core::sync::atomic::AtomicBool::new(false),
            data: core::cell::UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self
            .lock
            .compare_exchange_weak(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinLockGuard { parent: self }
    }
}

pub struct SpinLockGuard<'a, T> {
    parent: &'a SpinLock<T>,
}

impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.parent.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.parent.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.parent
            .lock
            .store(false, core::sync::atomic::Ordering::Release);
    }
}

/// A channel sender for the mock socket.
#[derive(Clone, Debug)]
pub struct MockSender {
    queue: alloc::sync::Arc<SpinLock<alloc::collections::VecDeque<alloc::vec::Vec<u8>>>>,
    closed: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
}

impl MockSender {
    /// Sends a packet to the mock socket.
    pub fn send(&self, data: alloc::vec::Vec<u8>) -> Result<(), BitTorrentError> {
        if self.closed.load(core::sync::atomic::Ordering::Relaxed) {
            return Err(BitTorrentError::Parse("Mock socket closed".into()));
        }
        self.queue.lock().push_back(data);
        Ok(())
    }
}

/// A channel receiver for the mock socket.
#[derive(Debug)]
pub struct MockReceiver {
    queue: alloc::sync::Arc<SpinLock<alloc::collections::VecDeque<alloc::vec::Vec<u8>>>>,
    closed: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
}

impl MockReceiver {
    /// Receives a packet from the mock socket, spinning until data is available or the socket closes.
    /// Returns `Err` after a fixed number of empty spin iterations to prevent infinite loops in tests.
    pub fn recv(&self) -> Result<alloc::vec::Vec<u8>, BitTorrentError> {
        const MAX_SPIN_ITERS: usize = 100_000;
        let mut spins = 0usize;
        loop {
            if let Some(data) = self.queue.lock().pop_front() {
                return Ok(data);
            }
            if self.closed.load(core::sync::atomic::Ordering::Relaxed) {
                return Err(BitTorrentError::Protocol("Mock socket closed".into()));
            }
            spins += 1;
            if spins >= MAX_SPIN_ITERS {
                return Err(BitTorrentError::Protocol(
                    "MockReceiver::recv timed out (no data)".into(),
                ));
            }
            core::hint::spin_loop();
        }
    }
}

/// A mock socket implementation for testing without real network I/O.
#[derive(Debug)]
pub struct MockSocket {
    pub rx: alloc::sync::Arc<SpinLock<alloc::collections::VecDeque<alloc::vec::Vec<u8>>>>,
    pub tx: alloc::sync::Arc<SpinLock<alloc::collections::VecDeque<alloc::vec::Vec<u8>>>>,
    pub read_buf: SpinLock<alloc::vec::Vec<u8>>,
    pub closed: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
}

impl MockSocket {
    /// Creates a mock socket and returns it along with channels to write to its input
    /// and read from its output externally.
    pub fn new() -> (Self, MockSender, MockReceiver) {
        let rx_queue = alloc::sync::Arc::new(SpinLock::new(alloc::collections::VecDeque::new()));
        let tx_queue = alloc::sync::Arc::new(SpinLock::new(alloc::collections::VecDeque::new()));
        let closed = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));

        let socket = MockSocket {
            rx: rx_queue.clone(),
            tx: tx_queue.clone(),
            read_buf: SpinLock::new(alloc::vec::Vec::new()),
            closed: closed.clone(),
        };

        let sender = MockSender {
            queue: rx_queue,
            closed: closed.clone(),
        };

        let receiver = MockReceiver {
            queue: tx_queue,
            closed,
        };

        (socket, sender, receiver)
    }

    /// Creates a pair of interconnected mock sockets where writes to one socket are immediately
    /// readable from the other, enabling high-speed in-memory peer loopback without OS sockets.
    pub fn pair() -> (Self, Self) {
        let q1 = alloc::sync::Arc::new(SpinLock::new(alloc::collections::VecDeque::new()));
        let q2 = alloc::sync::Arc::new(SpinLock::new(alloc::collections::VecDeque::new()));
        let c1 = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));
        let c2 = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));

        let socket_a = MockSocket {
            rx: q1.clone(),
            tx: q2.clone(),
            read_buf: SpinLock::new(alloc::vec::Vec::new()),
            closed: c1,
        };

        let socket_b = MockSocket {
            rx: q2,
            tx: q1,
            read_buf: SpinLock::new(alloc::vec::Vec::new()),
            closed: c2,
        };

        (socket_a, socket_b)
    }
}

impl AsyncSocket for MockSocket {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        if self.closed.load(core::sync::atomic::Ordering::Relaxed) {
            return Ok(0);
        }
        let mut read_guard = self.read_buf.lock();
        if read_guard.is_empty() {
            let mut rx_guard = self.rx.lock();
            if let Some(data) = rx_guard.pop_front() {
                read_guard.extend_from_slice(&data);
            } else if self.closed.load(core::sync::atomic::Ordering::Relaxed) {
                return Ok(0);
            }
        }
        let to_read = buf.len().min(read_guard.len());
        if to_read > 0 {
            buf[..to_read].copy_from_slice(&read_guard[..to_read]);
            read_guard.drain(..to_read);
        }
        Ok(to_read)
    }

    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        if self.closed.load(core::sync::atomic::Ordering::Relaxed) {
            return Err(BitTorrentError::Parse("Socket closed".into()));
        }
        self.tx.lock().push_back(buf.to_vec());
        Ok(buf.len())
    }

    fn close(&self) {
        self.closed
            .store(true, core::sync::atomic::Ordering::Relaxed);
    }
}

/// An in-memory block storage implementation.
#[derive(Debug)]
pub struct MemStorage {
    data: SpinLock<alloc::vec::Vec<u8>>,
}

impl MemStorage {
    /// Creates a new in-memory storage of the given size.
    pub fn new(size: usize) -> Self {
        MemStorage {
            data: SpinLock::new(alloc::vec![0u8; size]),
        }
    }

    /// Returns the capacity of the memory storage in bytes.
    pub fn len(&self) -> usize {
        self.data.lock().len()
    }

    /// Returns true if the storage buffer has 0 capacity.
    pub fn is_empty(&self) -> bool {
        self.data.lock().is_empty()
    }
}

impl BlockReader for MemStorage {
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        if buffer.is_empty() {
            return Ok(0);
        }

        let guard = self.data.lock();
        let capacity = guard.len() as u64;

        let end = match offset.checked_add(buffer.len() as u64) {
            Some(e) => e,
            None => {
                #[cfg(feature = "std")]
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "read offset + length exceeds storage capacity (overflow)",
                )));
                #[cfg(not(feature = "std"))]
                return Err(BitTorrentError::Protocol(
                    "read offset + length exceeds storage capacity (overflow)".into(),
                ));
            }
        };

        if end > capacity || offset > capacity {
            #[cfg(feature = "std")]
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "read offset + length exceeds storage capacity",
            )));
            #[cfg(not(feature = "std"))]
            return Err(BitTorrentError::Protocol(
                "read offset + length exceeds storage capacity".into(),
            ));
        }

        let start = offset as usize;
        let end_idx = end as usize;
        buffer.copy_from_slice(&guard[start..end_idx]);
        Ok(buffer.len())
    }
}

impl BlockWriter for MemStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        if data.is_empty() {
            return Ok(());
        }

        let mut guard = self.data.lock();
        let capacity = guard.len() as u64;

        let end = match offset.checked_add(data.len() as u64) {
            Some(e) => e,
            None => {
                #[cfg(feature = "std")]
                return Err(BitTorrentError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "write offset + length exceeds storage capacity (overflow)",
                )));
                #[cfg(not(feature = "std"))]
                return Err(BitTorrentError::Protocol(
                    "write offset + length exceeds storage capacity (overflow)".into(),
                ));
            }
        };

        if end > capacity || offset > capacity {
            #[cfg(feature = "std")]
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "write offset + length exceeds storage capacity",
            )));
            #[cfg(not(feature = "std"))]
            return Err(BitTorrentError::Protocol(
                "write offset + length exceeds storage capacity".into(),
            ));
        }

        let start = offset as usize;
        let end_idx = end as usize;
        guard[start..end_idx].copy_from_slice(data);
        Ok(())
    }

    fn flush(&self) -> Result<(), BitTorrentError> {
        Ok(())
    }

    fn discard_range(&self, _offset: u64, _length: u64) {}
}

/// A null/placeholder block storage implementation for uninitialized or bootstrap sessions (LSP).
///
/// Implements `BlockStorage` contract safely without allocating memory or throwing panics:
/// - Zero-length reads/writes succeed (no-op).
/// - Non-zero reads return error indicating bootstrap uninitialized state.
/// - Non-zero writes return error indicating bootstrap uninitialized state.
/// - Flush and discard operations are idempotent no-ops.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullBlockStorage;

impl BlockReader for NullBlockStorage {
    fn read_block(&self, _offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        if buffer.is_empty() {
            return Ok(0);
        }
        #[cfg(feature = "std")]
        return Err(BitTorrentError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Storage uninitialized in bootstrap state",
        )));
        #[cfg(not(feature = "std"))]
        return Err(BitTorrentError::Protocol(
            "Storage uninitialized in bootstrap state".into(),
        ));
    }
}

impl BlockWriter for NullBlockStorage {
    fn write_block(&self, _offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        if data.is_empty() {
            return Ok(());
        }
        #[cfg(feature = "std")]
        return Err(BitTorrentError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Storage uninitialized in bootstrap state",
        )));
        #[cfg(not(feature = "std"))]
        return Err(BitTorrentError::Protocol(
            "Storage uninitialized in bootstrap state".into(),
        ));
    }

    fn flush(&self) -> Result<(), BitTorrentError> {
        Ok(())
    }

    fn discard_range(&self, _offset: u64, _length: u64) {}
}

#[derive(Debug)]
pub enum Socket {
    #[cfg(feature = "std")]
    Tcp(crate::network::peer_network::TcpSocket),
    #[cfg(feature = "utp")]
    Utp(crate::network::utp::UtpSocketAdapter),
    #[cfg(all(feature = "std", feature = "mse"))]
    Encrypted(crate::network::peer_network::EncryptedSocket),
    #[cfg(all(feature = "std", feature = "webrtc"))]
    WebRtc(crate::network::webrtc::WebRtcSocketAdapter),
    Mock(MockSocket),
}

impl AsyncSocket for Socket {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        match self {
            #[cfg(feature = "std")]
            Socket::Tcp(s) => s.read(buf).await,
            #[cfg(feature = "utp")]
            Socket::Utp(s) => s.read(buf).await,
            #[cfg(all(feature = "std", feature = "mse"))]
            Socket::Encrypted(s) => s.read(buf).await,
            #[cfg(all(feature = "std", feature = "webrtc"))]
            Socket::WebRtc(s) => s.read(buf).await,
            Socket::Mock(s) => s.read(buf).await,
        }
    }

    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        match self {
            #[cfg(feature = "std")]
            Socket::Tcp(s) => s.write(buf).await,
            #[cfg(feature = "utp")]
            Socket::Utp(s) => s.write(buf).await,
            #[cfg(all(feature = "std", feature = "mse"))]
            Socket::Encrypted(s) => s.write(buf).await,
            #[cfg(all(feature = "std", feature = "webrtc"))]
            Socket::WebRtc(s) => s.write(buf).await,
            Socket::Mock(s) => s.write(buf).await,
        }
    }

    fn close(&self) {
        match self {
            #[cfg(feature = "std")]
            Socket::Tcp(s) => s.close(),
            #[cfg(feature = "utp")]
            Socket::Utp(s) => s.close(),
            #[cfg(all(feature = "std", feature = "mse"))]
            Socket::Encrypted(s) => s.close(),
            #[cfg(all(feature = "std", feature = "webrtc"))]
            Socket::WebRtc(s) => s.close(),
            Socket::Mock(s) => s.close(),
        }
    }
}

/// A hardware-agnostic socket factory trait.
#[cfg(feature = "std")]
pub trait SocketFactory: Send + Sync + std::fmt::Debug {
    /// Establishes a socket connection to target IP and port.
    fn connect(&self, ip: &str, port: u16) -> Result<alloc::sync::Arc<Socket>, BitTorrentError>;
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
        let response = ureq::get(url)
            .timeout(std::time::Duration::from_secs(5))
            .call()
            .map_err(|err| BitTorrentError::Parse(err.to_string()))?;
        let mut body = alloc::vec::Vec::new();
        use std::io::Read;
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|err| BitTorrentError::Parse(err.to_string()))?;
        Ok(body)
    }
}
