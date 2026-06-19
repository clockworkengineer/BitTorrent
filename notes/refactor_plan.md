# bittorrent-rs Library Refactor Plan

> Mapped to the 10 attributes of a well-written software library (see `notes/attributes.md`).

---

## 1. Intuitive API Design

**Current state**

- `TorrentSession` exposes a clean builder pattern and a `builder()` factory — good.
- However, many internal types leak through the public API (`TorrentContext`, raw bitfield `Vec<u8>`, `Arc<Mutex<…>>` handles). Users must lock mutexes themselves to read progress.
- `Tracker` is constructed independently from `TorrentSession` and must be wired up manually with a `peer_swarm_queue` sender — this two-step setup is error-prone.
- `TorrentSession::new()` and `TorrentSession::new_with_options()` duplicate the builder, creating three ways to construct the same type.
- `session.context()` returns `Arc<Mutex<TorrentContext>>` — callers must lock it and navigate internal fields to observe state.
- Magnet-link sessions use separate constructors (`new_magnet`, `new_magnet_with_options`) instead of a unified builder entry point.

**Proposed changes**

| File | Change |
|---|---|
| `session/session.rs` | Remove bare `new()` and `new_with_options()` — make the builder the **only** entry point. Add a `from_magnet(link, path)` builder method so all four constructors collapse into one type. |
| `session/session.rs` | Add a thin `SessionStats` value-type (no locks) that `progress()`, `status()`, and `bytes_downloaded()` return, so callers never touch the mutex directly. |
| `network/tracker.rs` | Fold `Tracker::new()` into `TorrentSession::tracker()` (already exists) and remove the need to call `set_peer_swarm_queue` manually — wire it automatically inside `start_download()`. |
| `lib.rs` | Hide `TorrentContext` from the top-level re-exports; move it under `bittorrent_rs::internals` with a doc note that it is not part of the stable API. |
| `core/torrent_context.rs` | Remove the `pub` from all raw internal fields that are not intended for external use (`pieces_missing`, `piece_data`, `assembler`, `bad_peer_scores`, `paused`, `call_back`, etc.). Provide accessor methods instead. |

---

## 2. Comprehensive Documentation

**Current state**

- `lib.rs` has a reasonable crate-level example but the second code snippet (`MockSocket`/`MemStorage`) uses `in_tx.send(…)` which does not match the actual API signature (takes `Vec<u8>` not a byte literal directly — needs `.to_vec()`).
- Most public types have module-level doc comments but many *methods* on `TorrentContext` lack `# Errors` / `# Panics` sections.
- `set_piece_length()` calls `panic!()` on an out-of-range value — this is undocumented.
- `tracker.rs` has a module-level example but `log_announce()` uses `println!` (not the logging macro) making it inconsistent.
- `README.md` references `docs/refactor-plan.md` which exists elsewhere, but the project has no CHANGELOG or migration guide.

**Proposed changes**

| File | Change |
|---|---|
| `lib.rs` | Fix the `MockSocket` code example so it compiles with `--test` (`in_tx.send(b"…".to_vec())`). Add `# Examples` to `TorrentSessionBuilder`. |
| `core/torrent_context.rs` | Add `# Panics` to `set_piece_length`. Add `# Errors` to every `Result`-returning method. |
| `session/session.rs` | Add `# Errors` / `# Panics` / `# Examples` to `start_download`, `stop`, `pause`, `resume`. |
| `network/tracker.rs` | Replace `println!` in `log_announce` with `log_debug!` for consistency. Add `# Errors` to `announce_*` methods. |
| [NEW] `CHANGELOG.md` (project root) | Create a changelog tracking the refactor phases. |
| [NEW] `docs/api-stability.md` | Document which items are stable API vs. internal (sealed behind `#[doc(hidden)]` or `internals` module). |

---

## 3. High Reliability

**Current state**

- `set_piece_length()` calls `panic!()` rather than returning an error — this can crash callers.
- `is_space_in_swarm()` acquires the `peer_swarm` `RwLock` **twice** in sequence — a TOCTOU window where a second check may see a different state than the first.
- The executor loop in `new_with_options()` and `new_magnet_with_options()` is copy-pasted verbatim — a bug fixed in one will be missed in the other.
- `MockReceiver::recv()` spin-loops without any backoff, which can starve the CPU in test environments.
- Duplicate lock acquisition patterns (`context.lock().unwrap()` followed immediately by another `context.lock().unwrap()`) across `session.rs` can deadlock in re-entrant scenarios.

**Proposed changes**

| File | Change |
|---|---|
| `core/torrent_context.rs` | Replace `panic!()` in `set_piece_length` with `return` (logging an error) or change the method signature to `Result<(), BitTorrentError>`. |
| `core/torrent_context.rs` | Refactor `is_space_in_swarm` to acquire the lock once: `let swarm = self.peer_swarm.read().unwrap(); !ip.is_empty() && !swarm.contains_key(ip) && swarm.len() < self.maximum_swarm_size`. |
| `session/session.rs` | Extract the executor-loop spawn code into a private `fn spawn_executor(context, task_rx) -> thread::JoinHandle<()>` to eliminate duplication between `new_with_options` and `new_magnet_with_options`. Extract stats-loop into `fn spawn_stats_loop(context, task_tx)`. |
| `session/session.rs` | Consolidate consecutive `context.lock().unwrap()` calls into a single guard per logical block. |
| `utils/io_traits.rs` | Add a maximum retry count or timeout to `MockReceiver::recv()` to avoid infinite spin in pathological tests. |

---

## 4. Performance and Efficiency

**Current state**

- `RarestFirstSelector::select_piece()` and `next_block_request_for_peer()` both do a full linear scan (`0..number_of_pieces`) on every call — O(n) per peer per loop iteration.
- `number_of_unchoked_peers()` acquires the swarm `RwLock`, then calls `peer.try_lock()` inside the iterator — potentially O(n) lock attempts per stats tick.
- Block buffer (`PieceBuffer`) stores a `Vec<bool>` for tracking presence — a compact bitset would halve memory and improve cache locality for large piece counts.
- `bytes_per_second()` recomputes under a `Mutex` every 500 ms while holding the guard during floating-point arithmetic.
- `process_piece_block()` acquires `assembler.piece_buffers.lock()` three times for one block write.

**Proposed changes**

| File | Change |
|---|---|
| `core/selector.rs` | Add a `PiecePriorityQueue` (binary heap keyed on `peer_count`) maintained incrementally in `merge_piece_bitfield` / `unmerge_piece_bitfield` instead of scanning every time. Document the trade-off. |
| `storage/piece_buffer.rs` | Replace `Vec<bool>` block-presence tracker with a `u64` bitset (a piece has at most 1024 blocks given 16 KiB blocks and a 16 MiB piece limit). |
| `core/torrent_context.rs` | Reduce lock scope in `process_piece_block`: acquire `piece_buffers` lock once, do all reads and writes, release. |
| `core/torrent_context.rs` | Cache the unchoked-peer count as an `AtomicUsize` updated on choke/unchoke events rather than computing it with a full scan + lock per stats interval. |
| `core/torrent_context.rs` | Move the floating-point division in `bytes_per_second()` outside the mutex guard — copy the raw values out, release the lock, then compute. |

---

## 5. Maintainability

**Current state**

- The executor-spawn and stats-loop code is duplicated verbatim across `new_with_options` and `new_magnet_with_options` (~120 lines each).
- `worker.rs`'s `handle_peer_session()` is 490 lines in a single function handling: MSE, handshake, PEX, magnet bootstrap, metadata assembly, block requesting, and keep-alive. This violates single-responsibility.
- `torrent_context.rs` is 949 lines. It combines state storage, piece scheduling, hash verification, peer swarm management, and speed tracking.
- `metainfo.rs` uses a flat `HashMap<String, Vec<u8>>` for parsed data, encoding multi-file entries as NUL-delimited strings — this internal encoding is fragile and undocumented.
- `session.rs` has both `TorrentSession` and `TorrentSessionBuilder` in the same file alongside `SessionConfig` and `spawn_choking_loop` — a single 992-line file.

**Proposed changes**

| File | Change |
|---|---|
| [SPLIT] `session/session.rs` | Split into: `session/config.rs` (SessionConfig + Default), `session/builder.rs` (TorrentSessionBuilder), `session/session.rs` (TorrentSession methods only). |
| [REFACTOR] `session/worker.rs` | Decompose `handle_peer_session` into sub-functions: `perform_handshake(…)`, `run_peer_loop(…)`, `handle_magnet_bootstrap(…)`, `send_block_requests(…)`. Each ≤ 80 lines. |
| [SPLIT] `core/torrent_context.rs` | Extract speed-tracking into `core/speed_tracker.rs` (`DownloadSpeedTracker` struct). Extract swarm management into `core/peer_swarm.rs`. |
| [REFACTOR] `core/metainfo.rs` | Replace the NUL-delimited string encoding for multi-file entries with a typed `ParsedFileEntry { path: String, length: u64, md5sum: Option<String> }` stored in a `Vec` field rather than the flat `HashMap`. |
| [REFACTOR] `session/session.rs` | Extract `spawn_executor()` and `spawn_stats_loop()` as private free functions shared by both construction paths. |

---

## 6. Flexibility and Customization

**Current state**

- `PieceSelector` is pluggable — good.
- `SocketFactory` and `HttpClient` are injectable via `SessionConfig` — good.
- The port used for LSD, DHT, and NAT-PMP is hardcoded to `6881` in `session.rs` (multiple call sites) — callers cannot change the listen port without rebuilding.
- The choking algorithm (`spawn_choking_loop`) is hardcoded; there is no trait to swap it.
- The block size (16 KiB) is a compile-time constant but `SessionConfig::block_size` exists yet is not used anywhere in piece scheduling — the field is dead code.

**Proposed changes**

| File | Change |
|---|---|
| `session/session.rs` | Replace magic `6881` literals with `config.listen_port` (add `listen_port: u16` to `SessionConfig` defaulting to `6881`). |
| `session/session.rs` | Expose `ChokingStrategy` trait with a default `StandardChoking` impl; add `choking_strategy: Arc<dyn ChokingStrategy>` to `SessionConfig`. |
| `core/torrent_context.rs` | Thread `config.block_size` through `next_block_request_for_peer` and `next_pending_block` so the field is actually used rather than being dead code. |
| `session/session.rs` | Add `SessionConfig::with_listen_port(u16)` builder-style setter. |

---

## 7. Strong Security

**Current state**

- `metainfo.rs` validates against path traversal, null bytes, absolute paths, Windows reserved names — good.
- `TorrentContext` uses a bad-peer scoring system (score ≥ 3 → blacklist) — adequate.
- However, `MemStorage::write_block()` error message uses `BitTorrentError::Parse("MemStorage write out of bounds")` — this exposes the implementation type in user-facing errors.
- `process_piece_block()` validates bounds before writing, but the alignment check (`begin % BLOCK_SIZE`) is done *after* the piece-local check — a misaligned block from a local-complete piece silently returns `Ok(false)` without flagging the sender as bad.
- `TorrentContext::report_bad_peer()` holds `bad_peer_scores` lock while also writing to `peer_swarm` — a potential deadlock if `peer_swarm.write()` blocks while `bad_peer_scores` is held.

**Proposed changes**

| File | Change |
|---|---|
| `utils/io_traits.rs` | Change `MemStorage` error messages to use `BitTorrentError::Io(…)` wrapping an `io::ErrorKind::InvalidInput` rather than `Parse`. |
| `core/torrent_context.rs` | Move the alignment check (`begin % BLOCK_SIZE != 0`) to *before* the `is_piece_local` early-return guard. |
| `core/torrent_context.rs` | Fix `report_bad_peer`: release `bad_peer_scores` lock before acquiring `peer_swarm` write lock. Capture score in a local variable, then check outside the lock scope. |
| `utils/error.rs` | Add `BitTorrentError::Protocol(String)` variant to distinguish peer protocol violations from generic parse errors. |
| `session/worker.rs` | Add a check that the remote peer's `info_hash` is not all-zero before accepting the handshake. |

---

## 8. High Testability

**Current state**

- 23 test files with solid coverage of bencode, metainfo, tracker, peer messages, disk I/O, session lifecycle — good.
- `MockSocket` and `MemStorage` are `pub` in `lib.rs` enabling unit testing without real I/O — good.
- However, `worker.rs::handle_peer_session` is untestable as-is (spawns real threads, requires real socket factory).
- `spawn_choking_loop` has no way to inject a mock clock or fake swarm state.
- `process_piece_block()` writes to storage and validates hash in one call — no way to test hash-fail handling without a full context.
- Several tests require real `.torrent` files from `tests/files/` — no CI guard to skip them.

**Proposed changes**

| File | Change |
|---|---|
| `session/worker.rs` | After decomposing `handle_peer_session` (§5), write unit tests for each sub-function using `MockSocket` + `MemStorage`. |
| `core/torrent_context.rs` | Extract `check_piece_hash()` call from `update_bitfield_from_buffer()` so the two steps can be tested separately. Add `TorrentContext::process_verified_block(piece, begin, data)` that skips hash-checking for use in tests. |
| [NEW] `tests/mock_peer_tests.rs` | Add tests for full peer handshake + message exchange using `MockSocket`, covering: bitfield exchange, choke/unchoke, request/piece flow, and PEX. |
| `session/session.rs` | Add a `TorrentSession::new_from_context(context, config)` constructor that bypasses file I/O, enabling tests to inject a pre-built `TorrentContext` directly. |
| `tests/session_tests.rs` | Annotate tests requiring real `.torrent` files with `#[cfg(test_real_files)]` to allow CI to skip them by default. |

---

## 9. Compatibility and Portability

**Current state**

- `#![cfg_attr(not(feature = "std"), no_std)]` with `extern crate alloc` — the no_std skeleton is in place.
- `BitTorrentError` implements `std::error::Error` only under `#[cfg(feature = "std")]` — compatible.
- However, `worker.rs` uses `std::thread::sleep` directly inside the `delay()` async fn — prevents `delay` from being used in pure async executors.
- `process_piece_block()` uses `println!` for hash pass/fail output — incompatible with no_std.
- Feature flags `dht`, `lsd`, `utp`, `nat-pmp`, `mse` are defined but the modules always compile regardless — the flags are essentially inaccurate documentation.

**Proposed changes**

| File | Change |
|---|---|
| `session/worker.rs` | Replace `std::thread::sleep` inside `delay()` with a pure async yield using `futures::task::noop_waker` or make it a no-op in no_std. |
| `core/torrent_context.rs` | Replace all `println!` calls with `log_debug!` (the macro is already defined in the crate). |
| `core/metainfo.rs` | Audit for remaining `println!` / `eprintln!` calls and replace with `log_debug!`. |
| `library/Cargo.toml` | For feature flags `dht = []`, `lsd = []`, `utp = []` etc.: add `#[cfg(feature = "dht")]` guards around the module declarations in `network/mod.rs` so the flags genuinely disable those modules. |
| All files | Run `grep -r "println!" library/src/` and replace each instance with `log_debug!` or `log::info!` as appropriate. At least 12 `println!` calls exist in `worker.rs` and `torrent_context.rs` alone. |

---

## 10. Low Dependency Footprint

**Current state**

- Core dependencies: `sha1`, `sha2` (optional), `crypto-bigint`, `rand`, `ureq`, `urlencoding`, `url`, `futures` — mostly well-controlled.
- `ureq` is only used in `UreqHttpClient` (a single 10-line impl) — pulls in a full HTTP client stack for all users.
- `futures` is used only for `LocalPool` and `LocalSpawnExt` in `session.rs` — a heavy crate for a narrow usage.
- `crypto-bigint` is used in `mse.rs` for Diffie-Hellman arithmetic but always compiled even when `mse` is disabled.
- `url` is only used in `metainfo.rs`'s `validate()` method for one `Url::parse()` call — a significant dependency for a single validation.

**Proposed changes**

| File | Change |
|---|---|
| `library/Cargo.toml` | Make `crypto-bigint` optional: `crypto-bigint = { …, optional = true }` and add it to `mse = ["dep:crypto-bigint"]`. |
| `library/Cargo.toml` | Make `url` only a dependency of the `http-tracker` or `std` feature; replace the tracker URL validation in no_std builds with a lightweight prefix check (`starts_with("http://")` / `udp://`). |
| `session/session.rs` | Replace `futures::executor::LocalPool` + `LocalSpawnExt` with a minimal hand-rolled task queue (a `VecDeque<Pin<Box<dyn Future<…>>>>` polled in a loop) or scope `futures` to an optional `futures-executor` feature. |
| `library/Cargo.toml` | Move `ureq` behind a new `ureq-http` feature flag; document that users who want to bring their own HTTP client should disable this feature and implement `HttpClient`. |

---

## Execution Phases

| Phase | Attribute(s) | Effort | Risk |
|---|---|---|---|
| **Phase 1** — Safety & Reliability | 3 (Reliability), 7 (Security) | Medium | Low — mostly local refactors, no API breaks |
| **Phase 2** — Maintainability | 5 (Maintainability) | High | Medium — large file splits require updating all imports |
| **Phase 3** — API Cleanup | 1 (Intuitive API), 6 (Flexibility) | Medium | High — breaking changes; needs semver bump |
| **Phase 4** — Documentation | 2 (Documentation) | Low | None |
| **Phase 5** — Testing | 8 (Testability) | Medium | Low — additive only |
| **Phase 6** — Portability & Deps | 9 (Portability), 10 (Dependencies) | Medium | Medium — feature flag changes can break downstream builds |
| **Phase 7** — Performance | 4 (Performance) | High | Medium — data structure changes need benchmarking |

> **Recommended starting point:** Phase 1 — eliminate the `panic!` in `set_piece_length`, fix the double-lock TOCTOU in `is_space_in_swarm`, and fix the deadlock risk in `report_bad_peer`. These are correctness issues with no API surface impact.
