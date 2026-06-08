# Embedded Systems Optimization & Refactoring Plan

This document outlines a concrete plan to optimize and refactor the `bittorrent-rs` library to make it suitable for resource-constrained embedded systems (e.g., ESP32, STM32, Cortex-M, and bare-metal environments).

---

## 1. Core Objectives for Embedded Systems
Embedded development imposes strict constraints on memory footprint, execution runtime, networking stack, and concurrency models:
1. **Minimize Memory Footprint**: Eliminate dynamic allocations (`no_std` and allocation-free options).
2. **Decouple OS Dependencies**: Abstract away standard file IO (`std::fs`), socket networking (`std::net`), and native OS threading (`std::thread`).
3. **Single-Threaded / Cooperative Concurrency**: Replace OS threads with an async-based state machine or cooperative multitasking executor (e.g., `embassy`).
4. **Reduce Binary Size (Code Bloat)**: Remove dependency graph components and formatting overhead.

---

## 2. Phase 1: `no_std` Core Extraction
Split the library into a clean `#![no_std]` core crate containing protocol logic, and a standard-library driver wrapper.

### Steps
1. **Declare `#![no_std]`**: Add `#![no_std]` to `library/src/lib.rs`.
2. **Abstract Dynamic Allocations**:
   - Use `extern crate alloc` for systems that support a heap allocator (e.g. RTOS environments).
   - Wrap data structures in `alloc::vec::Vec`, `alloc::string::String`, and `alloc::sync::Arc`.
3. **Enable Allocation-Free Configuration (Optional)**:
   - Provide a `no-alloc` compile-time feature flag.
   - Under `no-alloc`, replace `alloc` collections with fixed-capacity stack-allocated collections from the `heapless` crate (e.g., `heapless::Vec`, `heapless::String`).
4. **Decouple Error Types**: Refactor `bittorrent_rs::error::BitTorrentError` to avoid `std::error::Error` when `std` is disabled, relying instead on `core::fmt::Display`.

---

## 3. Phase 2: Async Executor & Cooperative Multitasking
spawning OS threads for each peer and stats loop is impossible on bare-metal systems and extremely expensive on RTOS.

### Steps
1. **Transition to Async/Await**:
   - Refactor the peer read/write loop in [peer.rs](file:///c:/Projects/BitTorrent/library/src/peer.rs) and [peer_network.rs](file:///c:/Projects/BitTorrent/library/src/peer_network.rs) to use `async fn`.
   - Implement peer state machines using standard futures.
2. **Cooperative Peer Swarm Manager**:
   - Replace thread-spawning loops in [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs) with an async task runner.
   - Run the entire BitTorrent client swarm concurrently on a single thread using an async executor like `embassy-executor` or `pollster`.
3. **Timer Abstraction**:
   - Replace `std::thread::sleep` loops in [session.rs](file:///c:/Projects/BitTorrent/library/src/session.rs) stats and announcing loops with an abstract `Delay` trait.

---

## 4. Phase 3: Hardware-Agnostic I/O Abstractions
Embedded systems do not use standard POSIX sockets or file systems. They rely on hardware registers, Ethernet/WiFi controller drivers (e.g., `lwip`), and flash storage chips.

### Steps
1. **Network Abstraction**:
   - Decouple `PeerNetwork` from `std::net::TcpStream`.
   - Define a generic `AsyncRead` and `AsyncWrite` trait boundary (or use standard `embedded-io-async` traits) to support hardware TCP/IP sockets.
2. **Storage/Disk I/O Abstraction**:
   - Currently, [disk_io.rs](file:///c:/Projects/BitTorrent/library/src/disk_io.rs) uses `std::fs::File` and `std::fs::OpenOptions`.
   - Abstract this by introducing a `BlockStorage` trait:
     ```rust
     pub trait BlockStorage {
         fn write_block(&mut self, offset: u64, data: &[u8]) -> Result<(), StorageError>;
         fn read_block(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize, StorageError>;
     }
     ```
   - Implement storage drivers for standard files (`std` target) and raw SPI Flash / EEPROM / SD Cards (`no_std` target).

---

## 5. Phase 4: Fixed-Size Buffer Pools & Zero-Copy Optimizations
The current peer loop downloads 16 KB blocks and verifies whole pieces (256 KB - 1 MB) by aggregating them in memory. In an embedded device with 512 KB total RAM, this will cause Out-Of-Memory (OOM) failures.

### Steps
1. **Static Buffer Allocator**:
   - Replace dynamic heap vectors with a global pre-allocated static buffer pool (e.g., using `static_cell` or global memory pools).
   - Reuse a single block buffer slice across network reads, hash validation, and flash writing.
2. **Streaming SHA-1 Verification**:
   - Currently, hash checking is done after loading a full piece into memory.
   - Refactor validation to run a streaming SHA-1 hasher (`sha1::Digest::update`) block-by-block as they arrive from the peer, avoiding the need to cache the whole piece in RAM.

---

## 6. Phase 5: Reducing Code Bloat (Binary Size)
Standard formatting and large dependencies cause compiled binaries to exceed embedded flash limits (typically 1-2 MB).

### Steps
1. **Formatting & Panic Reduction**:
   - Replace `format!` and string-based logs with a lightweight logging macro (like `defmt` for embedded debugging, or simple serial print).
   - Minimize panic paths by replacing `.unwrap()` and `.expect()` calls with structured error propagates (`?`).
2. **Remove Floating Point Math**:
   - Replace progress percentage floats and bytes-per-second calculations with fixed-point integer arithmetic to prevent pulling in float emulation libraries on hardware without FPU.
