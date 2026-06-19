#[cfg(feature = "std")]
pub mod config;
#[cfg(feature = "std")]
pub mod builder;
#[cfg(feature = "std")]
pub mod manager;
#[cfg(feature = "std")]
pub mod session;
#[cfg(all(feature = "std", feature = "webseed"))]
pub mod webseed;
#[cfg(feature = "std")]
pub mod worker;
#[cfg(feature = "std")]
pub mod client;

#[cfg(feature = "std")]
pub use self::session::*;
#[cfg(feature = "std")]
pub use self::client::*;
#[cfg(feature = "std")]
pub use self::config::SessionConfig;
#[cfg(feature = "std")]
pub use self::builder::{TorrentSessionBuilder, MagnetSessionBuilder};
