# Performance & Resource Optimization

The BitTorrent library implements several optimization strategies to minimize memory allocations, reduce binary size, and limit CPU utilization. This makes it suitable for execution on low-memory embedded devices and high-throughput desktop configurations.

---

## 1. Zero-Copy Lifetime Parsing

To eliminate heap allocations and buffer copies when parsing protocol packets and torrent metadata files, the library utilizes zero-copy structures that borrow slices directly from underlying read buffers.

### BNode & Bencode Decoders
Instead of copying Bencode strings and integers into new heap-allocated structures, `BNode<'a>` contains lifetime-bound references:
- Dictionary keys and string values are returned as slice references (`&'a [u8]` or `&'a str`).
- The parser avoids allocating new memory during decoding.

### PeerMessage Framing
Peer wire messages carry block payloads (up to 16 KiB per block). The decoder decodes messages to `PeerMessage<'a>`:
- The `Bitfield` and `Piece` variants reference payload slices borrowed directly from the thread's TCP socket read buffer.
- This ensures zero memory allocations are made during active file transfers.

---

## 2. Heap-Free Piece Verification

Aggregating all sub-blocks of a piece in RAM before running SHA-1 validation can trigger Out-of-Memory (OOM) conditions on systems with limited heap space (e.g., pieces can range from 256 KiB to 16 MiB+).
- **RAM Elimination**: The `PieceBuffer` aggregates block metadata but does not store block data itself. Blocks are written directly to `BlockStorage` as they arrive.
- **Streaming SHA-1**: When a piece is fully written, the client verifies its hash using a streaming checker. It reads blocks back from storage one-by-one into a running SHA-1 hash calculation using a small, reusable 16 KiB buffer on the stack. This completely eliminates piece assembly RAM overhead.

---

## 3. Buffer Pools & Churn Reduction

Spawning peer connection workers triggers frequent buffer allocations for network writes and reads:
- **`StaticBufferPool`**: Implements a lock-free buffer allocator pre-allocating a static set of 16 KiB buffers.
- **Acquire and Release**: Peer connections acquire a buffer from the pool at the start of read loops, and release it back to the pool once done.
- **Heap Fallback**: If all static pool buffers are occupied, the client cleanly falls back to standard heap allocation. This minimizes allocator fragmentation.

---

## 4. Integer Arithmetic Metrics

To eliminate floats and floating-point emulation libraries (which bloat binary size and CPU cycles on targets without hardware FPUs), all calculations are performed using integer math:
- **Download Progress**: Replaced percentage calculations with `progress_ppm(&self) -> u32`, which returns progress in parts-per-ten-thousand (0 to 10000). The standard feature wraps this metric into `progress_percent() -> f32` for GUI displays.
- **Transfer Rates**: Speeds and averages (e.g., `bytes_per_second`) are computed using millisecond deltas and integer division.

---

## 5. Embed-Safe Zero-Allocation Logging

With logging statements active across peer connections, string allocations and format evaluation thrashes the heap and CPU.
- **Conditional Logging Macro**: The library exposes the `log_debug!` macro.
- **Feature Gate**: Under `std`, this routes to a thread-safe static logger. Under `no_std`, the macro resolves to an empty block `()`, allowing the Rust compiler to fully optimize away log format strings and associated variables, significantly reducing binary size.
