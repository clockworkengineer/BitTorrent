#[cfg(feature = "std")]
pub mod assembler;
#[cfg(feature = "std")]
pub mod disk_cache;
#[cfg(feature = "std")]
pub mod disk_io;
#[cfg(feature = "std")]
pub mod fast_resume;
#[cfg(feature = "std")]
pub mod piece_buffer;
#[cfg(feature = "std")]
pub mod piece_request;
#[cfg(feature = "std")]
pub mod stream;

#[cfg(feature = "std")]
pub use disk_cache::{CacheBlock, CacheMetrics, CachedBlockStorage};

