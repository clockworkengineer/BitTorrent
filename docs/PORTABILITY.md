# Portability, Hardware Abstractions, & Port-Free Testing

To enable compilation in bare-metal and resource-constrained environments (such as microcontrollers, custom OS kernels, or simulated test rigs), the core BitTorrent library decouples protocol logic from the standard library (`std`), native OS-level threads, filesystems, and physical network sockets.

---

## `#![no_std]` Bare-Metal Core

The library is designed with a strict compilation gating hierarchy:
- **`no_std` Flag**: The root library declares `#![cfg_attr(not(feature = "std"), no_std)]` in [lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs).
- **Core Allocation (`alloc`)**: In `no_std` mode, the crate utilizes the standard `alloc` library for heap collections (such as `Vec`, `String`, and `BTreeMap`), but compiles with zero references to `std::thread`, `std::net`, or `std::fs`.
- **Feature Gating**: Standard library abstractions (disk access, standard TCP/UDP socket management, and the `futures` LocalPool executor) are gated behind the `std` target feature flag in `library/Cargo.toml`.
- **Memory-Only Core**: When compiling without `std`, developers can parse `.torrent` metadata buffers directly using `MetaInfoFile::from_bytes(data: &[u8])`, serialize/deserialize peer wire packets, and run piece selection strategies purely in memory.

---

## Hardware-Agnostic I/O Abstractions

All network socket operations and block storage writes are abstracted behind abstract Rust traits, allowing custom hardware drivers to be injected.

### 1. `AsyncSocket`
Defines asynchronous non-blocking stream interactions. Custom socket factories (like proxy tunnels or encrypted layers) implement this to handle peer traffic.

```rust
pub trait AsyncSocket: Send + Sync {
    /// Asynchronously reads up to `buf.len()` bytes into the buffer.
    fn read<'a>(&'a self, buf: &'a mut [u8]) -> Pin<Box<dyn Future<Output = Result<usize, BitTorrentError>> + Send + 'a>>;

    /// Asynchronously writes the complete byte buffer to the stream.
    fn write<'a>(&'a self, buf: &'a [u8]) -> Pin<Box<dyn Future<Output = Result<(), BitTorrentError>> + Send + 'a>>;
}
```

### 2. `BlockStorage`
Defines block-level reads and writes. This allows the client to write blocks to standard file layouts, flash partitions, or memory arrays.

```rust
pub trait BlockStorage: Send + Sync {
    /// Reads a block of data from the storage at the specified global byte offset.
    fn read_block(&self, offset: u64, buf: &mut [u8]) -> Result<usize, BitTorrentError>;

    /// Writes a block of data to the storage at the specified global byte offset.
    fn write_block(&self, offset: u64, buf: &[u8]) -> Result<(), BitTorrentError>;
}
```

---

## Port-Free & Filesystem-Free Unit Testing

To ensure high testability, the test suite verifies complex peer connection workflows and piece storage logic using in-memory mocks of the hardware traits. This allows running complete integration tests without binding actual TCP/UDP ports or creating physical files.

### 1. `MockSocket`
Simulates network socket streams using in-memory channels:
- Instantiated using `MockSocket::new()`, which returns a `MockSocket` handle alongside an input Sender and output Receiver.
- Pushing bytes to the input sender simulates receiving packets from a remote peer, which the client reads via `.read().await`.
- Writing bytes to the `MockSocket` sends them directly to the output receiver, allowing assertions on the client's output bytes.

### 2. `MemStorage`
Implements `BlockStorage` using a single thread-safe buffer inside an `RwLock` (under standard compilation) or a zero-overhead cell (under `#![no_std]`).
- Writes to `MemStorage` overwrite portions of the pre-allocated memory buffer.
- Allows verifying block writes, offsets, and piece verification without hitting physical drives.

### Example: Port-Free Test setup

```rust
use bittorrent_rs::{MemStorage, MockSocket, AsyncSocket, BlockStorage};
use std::sync::Arc;

#[test]
fn test_in_memory_session() {
    // 1. Create a 1 MB mock disk storage
    let storage = Arc::new(MemStorage::new(1024 * 1024));

    // 2. Create a mock peer socket
    let (socket, in_tx, out_rx) = MockSocket::new();
    let socket = Arc::new(socket);

    // 3. Simulate receiving a handshake from a remote peer
    let mut handshake = vec![0x13];
    handshake.extend_from_slice(b"BitTorrent protocol");
    handshake.extend_from_slice(&[0; 8]); // Reserved bytes
    handshake.extend_from_slice(&[0; 20]); // Mock Info-hash
    handshake.extend_from_slice(&[0; 20]); // Mock Peer ID
    in_tx.send(handshake).unwrap();

    // 4. Assert client reads and responds with its own handshake
    // (Run assertions on out_rx to verify client serialized output)
}
```
