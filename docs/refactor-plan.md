# Concrete Refactoring Plan: Implementing Library Attributes

This document outlines a concrete, actionable refactoring plan to align the `bittorrent-rs` library with the 10 high-quality library attributes defined in [attributes.md](file:///c:/Projects/BitTorrent/notes/attributes.md).

---

## 1. Intuitive API Design
*The interfaces and APIs should be easy to understand and use, with names that clearly reflect their purpose.*

### Current Status
* `TorrentSession` requires the caller to manually manage background reannounce loops, join peer workers, and handle synchronization locks (`Arc<Mutex<TorrentContext>>`) for basic progress tracking.
* The builder pattern requires file path inputs directly, limiting flexibility.

### Concrete Refactor Plan
- **Unified Event-Driven/Async API:**
  - Introduce a `TorrentSession::events(&mut self) -> impl Stream<Item = TorrentEvent>` or a callback-based API.
  - Automate the background worker management and re-announce loops internally when `start_download()` is called, removing the need for manual thread spawning or joining by the library user.
- **Improved Builder Options:**
  - Allow the builder to accept `MetaInfoFile` directly, in addition to file paths, for better testability and in-memory operations.
- **Type-Safe Primitives:**
  - Wrap raw `u16` ports, `u32` piece indices, and `[u8; 20]` info-hashes into distinct strongly typed wrappers (e.g., `struct PieceIndex(u32)`, `struct InfoHash([u8; 20])`) to prevent developers from accidentally swapping parameters.

---

## 2. Comprehensive Documentation
*Great libraries provide clear readme files, manuals, tutorials, and code comments to help users get started and troubleshoot.*

### Current Status
* Standard documentation exists in `docs/` and root `README.md`.
* Gaps exist in inline rustdocs for internal modules (`core/*`, `network/*`, `storage/*`, etc.), and some public structs/methods have brief or missing comments.

### Concrete Refactor Plan
- **Enforce Documentation Linting:**
  - Add `#![warn(missing_docs)]` in `lib.rs` to ensure every public item is thoroughly documented.
- **Integrate Doc Tests:**
  - Convert `no_run` examples in `lib.rs` and other modules into active runnable doc tests (`cargo test --doc`) by using mock objects so that they compile and verify automatically.
- **Internal Design Comments:**
  - Document the state machines of `utp.rs`, `mse.rs`, and the piece selector with structural comments explaining data-flow and invariants.

---

## 3. High Reliability
*The code must work predictably and consistently, with a low failure rate even under varied conditions.*

### Current Status
* Errors are represented by `BitTorrentError` which heavily relies on `String` descriptions (e.g., `BitTorrentError::Parse(String)`). This makes programmatically parsing and responding to errors difficult.
* The parser and wire protocols lack explicit boundary checks, raising potential panic risks on corrupted peer packets.

### Concrete Refactor Plan
- **Structured Error Handling:**
  - Refactor `BitTorrentError` to be a structured, categorised enum:
    ```rust
    pub enum BitTorrentError {
        Bencode(BencodeError),
        Network(NetworkError),
        Storage(StorageError),
        Protocol(ProtocolError),
        // ...
    }
    ```
  - Remove generic `String` allocations inside error variants to make errors cheap to construct and easy to match.
- **Panic-Free Parsing Gaps:**
  - Wrap all wire framing and Bencode parsing in explicit overflow/bounds checks (`try_into()`, `get()`) to guarantee that malformed network data never causes a panic.

---

## 4. Performance and Efficiency
*It should execute tasks quickly while minimizing resource consumption like memory, CPU, and network bandwidth.*

### Current Status
* Implements zero-copy Bencode parsing and heap-free piece verification.
* Frequent locking of `TorrentContext` (`Arc<Mutex<...>>`) causes CPU lock contention as peer counts scale.
* Static buffer pool has a fallback allocation strategy that does not limit memory allocation bounds under high load.

### Concrete Refactor Plan
- **Lock Contention Reduction:**
  - Transition from a single global Mutex over `TorrentContext` to a message-passing channel architecture. Have a single owner for `TorrentContext` (the event loop) and communicate state changes via lock-free channels.
- **Bounded Buffer Pool:**
  - Limit the heap fallback allocation of `StaticBufferPool` to prevent memory exhaustion under high peer loads, introducing backpressure on the read sockets.

---

## 5. Maintainability
*A well-crafted library is easy to repair, improve, or modify without introducing new bugs or breaking existing functionality.*

### Current Status
* Non-standard module mapping using `#[path = "..."]` in `lib.rs` diverges from modern idiomatic Rust practices.
* Some components are coupled (e.g., peer handling depends directly on specific session details).

### Concrete Refactor Plan
- **Normalize Crate Module Structure:**
  - Remove all `#[path = "..."]` annotations from [lib.rs](file:///c:/Projects/BitTorrent/library/src/lib.rs).
  - Move files to match standard Rust module layout (e.g., `src/core/mod.rs` to `src/core.rs`, etc.).
- **Decouple Modules:**
  - Decouple `peer` logic from `session` by using clean, trait-based boundaries or events, allowing easier modifications to the peer wire protocol without affecting the session runner.

---

## 6. Flexibility and Customization
*It should be specific enough to solve a problem but flexible enough to allow for basic customization and adaptation to future needs.*

### Current Status
* Re-usable traits (`AsyncSocket`, `BlockStorage`) are defined, but they allocate futures dynamically on the heap (`Pin<Box<dyn Future>>`), which is not ideal for bare-metal `#![no_std]` targets.

### Concrete Refactor Plan
- **Zero-Allocation Async Traits:**
  - Refactor I/O traits to avoid dynamic `Box` allocations for futures. Use custom poll-based traits (`poll_read`, `poll_write` similar to Tokio's `AsyncRead`/`AsyncWrite`) or use static async-traits using GATs/`impl Future` features when compiling in `#![no_std]`.
- **Dynamic Configuration Tuning:**
  - Expand `SessionConfig` to expose internal constants such as block sizes, maximum concurrent requests per peer, and timeout variables.

---

## 7. Strong Security
*The library must safeguard data and block unauthorized or malicious actions that could negatively affect the user's system.*

### Current Status
* `mse.rs` uses standard 64-bit (`u64`) integers for Diffie-Hellman key exchange, which is cryptographically insecure and easily brute-forced.
* Bencode decoding does not enforce recursion limits, risking stack overflow attacks.

### Concrete Refactor Plan
- **Strengthen Handshake Obfuscation:**
  - Increase Diffie-Hellman parameter sizes in `mse.rs` to use standard 768-bit or 1024-bit key sizes (using big integer representations safe for `#![no_std]`) instead of 64-bit types.
- **Enforce Parser Resource Limits:**
  - Add a maximum nesting depth limit (e.g., 50 levels) in `bencode.rs` parser to prevent stack overflows on malicious deeply nested structures.
  - Limit the maximum allowed size of keys and values inside dictionaries to prevent memory exhaustion attacks.

---

## 8. High Testability
*Code should be thoroughly tested and designed so that others can also easily verify its correctness.*

### Current Status
* Test suite relies on `#![cfg(feature = "std")]` for mocks because `MockSocket` and `MemStorage` use standard library `std::sync` synchronization structures.

### Concrete Refactor Plan
- **`no_std` Test Mocks:**
  - Refactor `MockSocket` and `MemStorage` to use `alloc` collection synchronization wrappers or atomic operations (e.g. using `spin` locks or bare cell abstractions) so they can compile and run in a bare-metal environment.
- **Fuzz Testing Support:**
  - Set up structured entry points (e.g., `fuzz_bencode`, `fuzz_peer_message`) to allow running cargo-fuzz/libFuzzer on the parsers.

---

## 9. Compatibility and Portability
*It should operate correctly across different platforms, devices, and environments with minimal modification.*

### Current Status
* The codebase assumes 32-bit or 64-bit target pointer sizes for index conversions (`u64` to `usize`).

### Concrete Refactor Plan
- **Pointer-Size Safety:**
  - Perform safe, checked conversions (`try_into()`) when casting between file/piece sizes (`u64`) and memory buffer sizes (`usize`) to guarantee correctness on 16-bit or 32-bit embedded platforms.
- **Gated Core Feature Set:**
  - Split tracker features into separate sub-features (`http-tracker`, `udp-tracker`) so that compilation targets lacking socket support can build a pure offline DHT/PEX client without compiling ureq or other external protocol crates.

---

## 10. Low Dependency Footprint
*A good library minimizes its own dependencies, ensuring that users don't have to include a "zillion other things" to use a single module.*

### Current Status
* The crate depends on standard features of several crates, pulling in transient dependencies like `ureq` and `url`.

### Concrete Refactor Plan
- **Dependency Minimization:**
  - Make `url` and `ureq` entirely optional and disabled by default.
  - Allow users who do not require HTTP trackers to compile the library without the `url` crate.
  - Consolidate hashing utilities so that only the necessary cryptography features are enabled.
