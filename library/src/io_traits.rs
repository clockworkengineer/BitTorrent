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
