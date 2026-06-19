# Concrete DRY Refactoring Plan for BitTorrent Library

This document outlines the concrete steps to apply DRY (Don't Repeat Yourself) principles across the core `bittorrent-rs` library. By centralizing byte-manipulation helpers, consolidating peer list deserializers, and unifying hex formatting, we will reduce code redundancy, minimize maintenance overhead, and enhance code readability.

---

## 1. Consolidate Compact Peer List Parsing
* **Current State:** Both `HttpAnnouncer` (in `announcer.rs`), `UdpAnnouncer` (in `announcer.rs`), and the `TrackerAnnounceContext` helper `get_compact_peer_list` (in `tracker.rs`) replicate or access compact peer decoding details.
* **Refactor Plan:**
  * Define a single, highly optimized function in [util.rs](file:///c:/Projects/BitTorrent/library/src/utils/util.rs):
    ```rust
    pub fn decode_compact_ipv4_peers(peers: &[u8], offset: usize) -> Vec<(String, u16)>
    ```
  * Replace the separate implementations in `tracker.rs` and `announcer.rs` with calls to this function, converting the returned tuples into `PeerDetails` blocks.

## 2. Devirtualize Native Byte-Order Operations
* **Current State:** [util.rs](file:///c:/Projects/BitTorrent/library/src/utils/util.rs) implements custom shifting logic for `pack_u32`, `unpack_u32`, `pack_u64`, and `unpack_u64`.
* **Refactor Plan:**
  * Refactor these functions to internally delegate to standard `u32::to_be_bytes()`, `u32::from_be_bytes()`, `u64::to_be_bytes()`, and `u64::from_be_bytes()`.
  * Eventually migrate call sites to use the standard library methods directly, phasing out the custom helpers.

## 3. Unify Hex String Formatting
* **Current State:** Manual loops converting byte arrays (e.g. `info_hash`) to hex strings are found in `tracker.rs` (under non-HTTP-tracker targets) and various logs.
* **Refactor Plan:**
  * Consolidate all byte-to-hex formatting to use the existing `info_hash_to_string` function in [util.rs](file:///c:/Projects/BitTorrent/library/src/utils/util.rs).

## 4. Unify Error conversion boilerplate
* **Current State:** The codebase repeats `.map_err(|e| BitTorrentError::Parse(e.to_string()))` in dozens of places.
* **Refactor Plan:**
  * Implement `From<String>` and `From<&str>` for `BitTorrentError` or use a standard helper mapping to cleanly convert errors.
