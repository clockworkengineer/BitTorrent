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

/// A hardware-agnostic block storage trait.
pub trait BlockStorage: Send + Sync {
    /// Writes a data block at the specified absolute byte offset.
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError>;

    /// Reads up to `buffer.len()` bytes from the specified absolute byte offset.
    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError>;
}

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
        while self.lock.compare_exchange_weak(
            false,
            true,
            core::sync::atomic::Ordering::Acquire,
            core::sync::atomic::Ordering::Relaxed,
        ).is_err() {
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
        self.parent.lock.store(false, core::sync::atomic::Ordering::Release);
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
                return Err(BitTorrentError::Protocol("MockReceiver::recv timed out (no data)".into()));
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
        self.closed.store(true, core::sync::atomic::Ordering::Relaxed);
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
}

impl BlockStorage for MemStorage {
    fn write_block(&self, offset: u64, data: &[u8]) -> Result<(), BitTorrentError> {
        let start = offset as usize;
        let end = start + data.len();
        let mut guard = self.data.lock();
        if end > guard.len() {
            #[cfg(feature = "std")]
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "write offset + length exceeds storage capacity",
            )));
            #[cfg(not(feature = "std"))]
            return Err(BitTorrentError::Protocol("write offset + length exceeds storage capacity".into()));
        }
        guard[start..end].copy_from_slice(data);
        Ok(())
    }

    fn read_block(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, BitTorrentError> {
        let start = offset as usize;
        let end = start + buffer.len();
        let guard = self.data.lock();
        if end > guard.len() {
            #[cfg(feature = "std")]
            return Err(BitTorrentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "read offset + length exceeds storage capacity",
            )));
            #[cfg(not(feature = "std"))]
            return Err(BitTorrentError::Protocol("read offset + length exceeds storage capacity".into()));
        }
        buffer.copy_from_slice(&guard[start..end]);
        Ok(buffer.len())
    }
}

#[derive(Debug)]
pub enum Socket {
    #[cfg(feature = "std")]
    Tcp(crate::network::peer_network::TcpSocket),
    #[cfg(feature = "utp")]
    Utp(crate::network::utp::UtpSocketAdapter),
    Mock(MockSocket),
}

impl AsyncSocket for Socket {
    async fn read(&self, buf: &mut [u8]) -> Result<usize, BitTorrentError> {
        match self {
            #[cfg(feature = "std")]
            Socket::Tcp(s) => s.read(buf).await,
            #[cfg(feature = "utp")]
            Socket::Utp(s) => s.read(buf).await,
            Socket::Mock(s) => s.read(buf).await,
        }
    }

    async fn write(&self, buf: &[u8]) -> Result<usize, BitTorrentError> {
        match self {
            #[cfg(feature = "std")]
            Socket::Tcp(s) => s.write(buf).await,
            #[cfg(feature = "utp")]
            Socket::Utp(s) => s.write(buf).await,
            Socket::Mock(s) => s.write(buf).await,
        }
    }

    fn close(&self) {
        match self {
            #[cfg(feature = "std")]
            Socket::Tcp(s) => s.close(),
            #[cfg(feature = "utp")]
            Socket::Utp(s) => s.close(),
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
            .map_err(|err| {
                BitTorrentError::Parse(err.to_string())
            })?;
        let mut body = alloc::vec::Vec::new();
        use std::io::Read;
        response
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|err| {
                BitTorrentError::Parse(err.to_string())
            })?;
        Ok(body)
    }
}
