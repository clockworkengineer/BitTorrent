# Concrete Attributes-based Refactoring Plan

This document outlines the concrete steps to align the core `bittorrent-rs` library with the 10 quality attributes listed in [attributes.md](file:///c:/Projects/BitTorrent/notes/attributes.md).

---

## 1. Intuitive API Design
*   **Problem:** Initializing a session requires coordinating multiple objects: `TorrentSession`, `Tracker`, and managing announcing/peer workers manually.
*   **Plan:** Provide a unified `TorrentClient` facade or builder that orchestrates the session, tracker, and peer connection pool automatically.

## 2. Comprehensive Documentation
*   **Problem:** Advanced modules (like Message Stream Encryption `mse`, `utp` LEDBAT, and `dht`) lack inline Rustdoc examples.
*   **Plan:** Write module-level and function-level documentation with runnable examples for all public endpoints.

## 3. High Reliability
*   **Problem:** Network message decoders might panic or overflow on corrupted inputs.
*   **Plan:** Audit all decoders (specifically `PeerMessage::decode` and uTP header parsing) to enforce boundary constraints and return a `Result` instead of panicking.

## 4. Performance and Efficiency
*   **Problem:** Peer traffic statistics require locking the global `TorrentContext`.
*   **Plan:** Shift stats collection strictly to `AtomicU64` and `AtomicU32` structures, avoiding thread synchronization locks in the fast worker path.

## 5. Maintainability
*   **Problem:** `metainfo.rs` is a large file handling file details, v1 parsing, v2 parsing, and path validation.
*   **Plan:** Extract parsing sub-tasks into separate files under a `metainfo` module (e.g. `metainfo/v1.rs`, `metainfo/v2.rs`, `metainfo/path.rs`).

## 6. Flexibility and Customization
*   **Problem:** Hardcoded peer limits, block sizes, and request timeouts.
*   **Plan:** Expose these parameters in `SessionConfig` to allow clients to customize networking behavior for different execution environments.

## 7. Strong Security
*   **Problem:** Malicious torrents could use directory traversal or illegal characters in file names.
*   **Plan:** Verify and reinforce `validate_relative_path` to guarantee protection against directory traversals, null bytes, and reserved Windows device names case-insensitively.

## 8. High Testability
*   **Problem:** Tests for timers require actual thread sleeps, leading to slow and flaky tests.
*   **Plan:** Introduce a mockable clock/time source trait so tests can simulate time progression instantly.

## 9. Compatibility and Portability
*   **Problem:** Code might assume a standard library environment (`std`) implicitly in features that should support `#![no_std]`.
*   **Plan:** Ensure clean build verification for the workspace with and without default features enabled.

## 10. Low Dependency Footprint
*   **Problem:** Unused transitives.
*   **Plan:** Keep the current lightweight dependency stack and avoid adding third-party crates unless strictly necessary.
