#[cfg(feature = "std")]
pub mod assembler;
#[cfg(feature = "std")]
pub mod disk_cache;
#[cfg(feature = "std")]
pub mod disk_io;
#[cfg(feature = "std")]
pub mod erasure_staging;
#[cfg(feature = "std")]
pub mod fast_resume;
#[cfg(feature = "std")]
pub mod piece_buffer;
#[cfg(all(feature = "std", feature = "v2"))]
pub mod piece_layers_cache;
#[cfg(feature = "std")]
pub mod piece_request;
#[cfg(feature = "std")]
pub mod stream;
#[cfg(feature = "std")]
pub mod verification_pool;

#[cfg(feature = "std")]
pub use disk_cache::{CacheBlock, CacheMetrics, CachedBlockStorage};
#[cfg(feature = "std")]
pub use erasure_staging::{
    AdaptiveErasureStaging, BlockArrivalOutcome, EndgameStagingMetrics, StagedBlock,
};
#[cfg(all(feature = "std", feature = "v2"))]
pub use piece_layers_cache::{PieceLayersCacheMetrics, PieceLayersDiskCache};
#[cfg(feature = "std")]
pub use verification_pool::{
    HashVerificationPool, PieceVerificationJob, PieceVerificationResult, RecheckProgress,
    VerificationPoolMetrics,
};
