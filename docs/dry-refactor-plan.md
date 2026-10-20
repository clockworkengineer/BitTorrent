# Concrete DRY Refactoring Plan for BitTorrent Workspace

> **Objective:** Eliminate code duplication, consolidate shared domain logic, and maximize code reuse across the `bittorrent-rs` library, client applications (`torrent_client`, `bt_daemon`, `bt_webui`, `sync_hub`, `torrent_client_shared`), examples, and test suites while preserving strict `#![no_std]` core compatibility, API stability, and zero clippy warnings.

---

## 1. Executive Summary & Duplication Audit

A comprehensive static analysis across the BitTorrent workspace revealed recurring duplication patterns across 9 distinct functional areas. By extracting reusable abstractions and unifying redundant implementations, we can eliminate an estimated **1,200+ lines of duplicate boilerplate**, enhance maintainability, and ensure single sources of truth.

### Duplication Impact Matrix

| # | Duplication Category | Affected Files | Redundant Lines | Proposed Abstraction |
|---|----------------------|----------------|-----------------|----------------------|
| **1** | **IPC Client & Socket Framing** | `bt_cli.rs`, `bt_webui/main.rs` | ~120 lines | `torrent_client_shared::ipc::IpcClient` |
| **2** | **HTTP Streaming Server & Range Logic** | `stream_server.rs`, `bt_daemon.rs`, `torrent_stream_example` | ~340 lines | `torrent_client_shared::stream::HttpStreamer` |
| **3** | **Media Detection & Primary File Resolution** | `bt_daemon.rs`, `torrent_client_shared`, `app.rs` | ~85 lines | `torrent_client_shared::media` |
| **4** | **Hex String Encoding & Decoding** | 15+ files across `library`, `clients`, `tests` | ~90 lines | `bittorrent_rs::util::{to_hex, from_hex}` |
| **5** | **Torrent Building & Recursive Directory Scan** | `sync_hub/torrent_gen.rs`, `torrent_creator_example` | ~160 lines | `TorrentBuilder::add_dir` in `library` |
| **6** | **`TorrentContext` Struct Initialization** | `core/torrent_context.rs` (`new` vs `new_magnet_bootstrap`) | ~140 lines | `TorrentContext::new_base()` helper |
| **7** | **Session Lifecycle & Executor Wiring** | `session/session.rs` (`new_with_options` vs `new_magnet`) | ~60 lines | `TorrentSession::assemble_session()` |
| **8** | **Peer Wire Message & Action Emission** | `peer_message.rs`, `network/peer.rs` | ~75 lines | Message encode helpers & peer action macros/helpers |
| **9** | **WebUI Axum REST IPC Handlers** | `bt_webui/src/main.rs` | ~110 lines | `handle_ipc_command()` response mapper |
| **10**| **Test Suite Fixtures & Path Helpers** | 8 test files in `library/tests/` | ~65 lines | `library/tests/common/mod.rs` |

---

## 2. Phase-by-Phase Concrete Refactoring Plan

---

### Phase 1: IPC Client & Protocol Transport Consolidation [COMPLETED]

#### Problem Statement
Both the CLI companion (`clients/bt_daemon/src/bin/bt_cli.rs`, lines 9–47) and the WebUI gateway (`clients/bt_webui/src/main.rs`, lines 27–83) re-implement the exact same IPC client logic:
- Connecting to Unix domain sockets on `/tmp/{name}.sock` (Unix) or Named Pipes `\\.\pipe\{name}` with retry backoff for error 231 `ERROR_PIPE_BUSY` (Windows).
- Appending trailing `\n` delimiters.
- Flushing and reading single-line JSON responses with `BufReader`.
- Serializing `IpcMessage` and deserializing `IpcReply`.

#### Target Architecture
Move the connection and message exchange logic into `clients/torrent_client_shared/src/ipc.rs` (or `clients/torrent_client_shared/src/lib.rs`).

#### Implementation Spec
```rust
// clients/torrent_client_shared/src/ipc.rs
pub struct IpcClient;

impl IpcClient {
    /// Sends an IpcMessage to the specified daemon socket/pipe and awaits an IpcReply.
    pub fn send(socket_name: &str, msg: &IpcMessage) -> Result<IpcReply, String> {
        let serialized = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        let response_str = Self::send_raw(socket_name, &serialized)?;
        serde_json::from_str(&response_str).map_err(|e| e.to_string())
    }

    pub fn send_raw(name: &str, payload: &str) -> Result<String, String> {
        #[cfg(unix)]
        {
            use std::io::{BufRead, BufReader, Write};
            use std::os::unix::net::UnixStream;
            let path = format!("/tmp/{}.sock", name);
            let mut stream = UnixStream::connect(path).map_err(|e| e.to_string())?;
            let mut bytes = payload.as_bytes().to_vec();
            bytes.push(b'\n');
            stream.write_all(&bytes).map_err(|e| e.to_string())?;
            stream.flush().map_err(|e| e.to_string())?;
            let mut reader = BufReader::new(stream);
            let mut reply = String::new();
            reader.read_line(&mut reply).map_err(|e| e.to_string())?;
            Ok(reply)
        }
        #[cfg(windows)]
        {
            use std::fs::OpenOptions;
            use std::io::{BufRead, BufReader, Write};
            let pipe_path = format!(r"\\.\pipe\{}", name);
            let mut file = None;
            for _ in 0..10 {
                match OpenOptions::new().read(true).write(true).open(&pipe_path) {
                    Ok(f) => { file = Some(f); break; }
                    Err(e) if e.raw_os_error() == Some(231) => {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            let mut file = file.ok_or_else(|| "All pipe instances are busy".to_string())?;
            let mut bytes = payload.as_bytes().to_vec();
            bytes.push(b'\n');
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
            let mut reader = BufReader::new(file);
            let mut reply = String::new();
            reader.read_line(&mut reply).map_err(|e| e.to_string())?;
            Ok(reply)
        }
    }
}
```

#### Affected Files to Update
- `clients/torrent_client_shared/src/lib.rs` (export `IpcClient`).
- `clients/bt_daemon/src/bin/bt_cli.rs` (delete `mod ipc_client`, replace calls with `IpcClient::send("bt-daemon", &msg)`).
- `clients/bt_webui/src/main.rs` (delete `execute_ipc`, replace with `IpcClient::send("bt-daemon", &msg)`).

---

### Phase 2: Media Utilities & HTTP Streaming Range Engine

#### Problem Statement
1. **Media Detection Duplication**:
   - `is_media_filename()` and `detect_mime_type()` are defined in both `clients/torrent_client_shared/src/lib.rs` (lines 288–335) and `clients/bt_daemon/src/bin/bt_daemon.rs` (lines 288–329).
2. **Primary Media Finder Duplication**:
   - Finding the largest media file in a torrent context is duplicated across `clients/torrent_client/src/app.rs` (lines 119–148) and `clients/bt_daemon/src/bin/bt_daemon.rs` (lines 468–489).
3. **HTTP Range Streaming Duplication**:
   - Raw HTTP parsing (`GET / HEAD / OPTIONS`), `Range: bytes=start-end` math, HTTP 206 formatting, CORS header generation, and chunk loop over `TorrentStream` are repeated across:
     - `clients/torrent_client/src/stream_server.rs`
     - `clients/bt_daemon/src/bin/bt_daemon.rs`
     - `examples/torrent_stream_example/src/main.rs`

#### Target Architecture
- Consolidate all media definitions and MIME lookups in `torrent_client_shared::media`.
- Create a reusable streaming handler in `torrent_client_shared::stream::serve_http_range_stream`.

#### Implementation Spec
```rust
// clients/torrent_client_shared/src/media.rs
pub fn is_media_filename(name: &str) -> bool { ... }
pub fn detect_mime_type(name: &str) -> &'static str { ... }

/// Finds the best (largest) media file index from a list of torrent files.
pub fn find_primary_media_file<T: AsRef<str>>(files: &[(T, u64)]) -> Option<usize> {
    let mut best_media = None;
    let mut max_media_len = 0;
    let mut best_any = None;
    let mut max_any_len = 0;

    for (idx, (path, len)) in files.iter().enumerate() {
        if *len > max_any_len {
            max_any_len = *len;
            best_any = Some(idx);
        }
        if is_media_filename(path.as_ref()) && *len > max_media_len {
            max_media_len = *len;
            best_media = Some(idx);
        }
    }
    best_media.or(best_any)
}
```

```rust
// clients/torrent_client_shared/src/stream.rs
pub fn handle_http_range_stream<R: Read + Seek>(
    mut socket: std::net::TcpStream,
    mut stream: R,
    file_name: &str,
    total_len: u64,
) -> std::io::Result<()> {
    // 1. Read request headers and extract Range
    // 2. Compute range_start and range_end
    // 3. Emit 206 Partial Content or 200 OK headers
    // 4. Stream chunks (64 KiB) to socket until range_end
}
```

#### Affected Files to Update
- `clients/torrent_client_shared/src/lib.rs` (re-export `media` and `stream` modules).
- `clients/bt_daemon/src/bin/bt_daemon.rs` (remove duplicate functions; delegate to `torrent_client_shared`).
- `clients/torrent_client/src/stream_server.rs` (use `handle_http_range_stream`).
- `clients/torrent_client/src/app.rs` (use `find_primary_media_file`).

---

### Phase 3: Hex Encoding/Decoding Standardisation

#### Problem Statement
Over 20 sites perform ad-hoc hexadecimal transformations:
- Formatting byte slices to hex: `bytes.iter().map(|b| format!("{:02x}", b)).collect::<String>()`
- Uppercase hex: `bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>()`
- Hex parsing loops with `u8::from_str_radix` in `state.rs` and `bt_daemon.rs`.

#### Target Architecture
Provide zero-allocation and fast string formatting helpers in `library/src/utils/util.rs`.

#### Implementation Spec
```rust
// library/src/utils/util.rs

/// Encodes a byte slice into a lowercase hexadecimal string.
pub fn to_hex(bytes: &[u8]) -> alloc::string::String {
    let mut s = alloc::string::String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use core::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Encodes a byte slice into an uppercase hexadecimal string.
pub fn to_hex_upper(bytes: &[u8]) -> alloc::string::String {
    let mut s = alloc::string::String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        use core::fmt::Write;
        let _ = write!(s, "{:02X}", b);
    }
    s
}

/// Decodes a hexadecimal string slice into raw bytes.
pub fn from_hex(hex: &str) -> Result<alloc::vec::Vec<u8>, crate::error::BitTorrentError> {
    if hex.len() % 2 != 0 {
        return Err(crate::error::BitTorrentError::Parse("Hex string has odd length".into()));
    }
    let mut bytes = alloc::vec::Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|e| crate::error::BitTorrentError::Parse(e.to_string()))?;
        bytes.push(byte);
    }
    Ok(bytes)
}
```

#### Affected Files to Update
- `library/src/core/metainfo.rs` (replace `pieces_root_hex`).
- `library/src/core/merkle.rs` (replace `root_hex`).
- `library/src/core/magnet.rs` (replace `v1_hex` and `v2_hex` construction).
- `library/src/storage/piece_layers_cache.rs` (replace manual format loop).
- `clients/torrent_client/src/state.rs` (replace `from_str_radix` decoding and `to_hex`).
- `clients/bt_daemon/src/bin/bt_daemon.rs` (replace `from_str_radix` and `to_hex`).
- `clients/sync_hub/src/manager.rs` (replace `info_hash_hex`).

---

### Phase 4: Torrent Generation & Directory Walking Unification

#### Problem Statement
`sync_hub/src/torrent_gen.rs` reimplements the entire torrent file generation pipeline (`visit_dirs`, Sha1 piece hashing, Bencode encoding) despite `bittorrent_rs::TorrentBuilder` being available in `library/src/core/torrent_builder.rs`. Furthermore, recursive directory traversal is duplicated between `sync_hub` and `examples/torrent_creator_example/src/main.rs`.

#### Target Architecture
Add recursive directory ingestion natively to `TorrentBuilder` in `library/src/core/torrent_builder.rs`.

#### Implementation Spec
```rust
// library/src/core/torrent_builder.rs

#[cfg(feature = "std")]
impl TorrentBuilder {
    /// Recursively adds all files from a directory on the local filesystem.
    pub fn add_dir(mut self, root_dir: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let root = root_dir.as_ref();
        let mut stack = vec![root.to_path_buf()];

        while let Some(current_dir) = stack.pop() {
            for entry in std::fs::read_dir(current_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    let rel_path = path
                        .strip_prefix(root)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?
                        .to_string_lossy()
                        .replace('\\', "/");
                    let data = std::fs::read(&path)?;
                    self = self.add_file(rel_path, data);
                }
            }
        }
        Ok(self)
    }
}
```

#### Affected Files to Update
- `library/src/core/torrent_builder.rs` (add `add_dir`).
- `clients/sync_hub/src/torrent_gen.rs` (replace 130 lines of ad-hoc generation with `TorrentBuilder::new(name).piece_length(piece_length).add_dir(sync_dir)?.build_hybrid()`).
- `examples/torrent_creator_example/src/main.rs` (delete `add_files_recursive`, call `builder.add_dir(&input_path)?`).

---

### Phase 5: `TorrentContext` Construction Decomposition

#### Problem Statement
`TorrentContext::new` (lines 187–367) and `TorrentContext::new_magnet_bootstrap` (lines 373–480) duplicate approximately 140 lines initializing identical fields:
- Atomic telemetry counters (`v2_hash_requests_sent`, `holepunch_rendezvous_sent`, etc.).
- Event synchronization primitives (`ManualResetEvent`).
- Caches and profilers (`AdaptiveErasureStaging`, `SwarmLatencyProfiler`, `PieceLayersDiskCache`).

#### Target Architecture
Extract a unified internal initialization function `TorrentContext::new_base()` that initializes common fields, accepting variable components as arguments.

#### Implementation Spec
```rust
// library/src/core/torrent_context.rs

impl TorrentContext {
    fn new_base(
        info_hash: Vec<u8>,
        tracker_url: String,
        tracker_urls: Vec<String>,
        selector: Arc<dyn PieceSelector>,
        storage: Arc<dyn crate::io_traits::BlockStorage>,
        download_path: PathBuf,
        config: SessionConfig,
    ) -> Self {
        Self {
            info_hash,
            tracker_url,
            tracker_urls,
            selector,
            storage,
            download_path: download_path.clone(),
            // All shared atomic counters and synchronizers initialized once here
            paused: ManualResetEvent::new(false),
            download_finished: Arc::new(ManualResetEvent::new(false)),
            peer_swarm: RwLock::new(HashMap::new()),
            assembler: Assembler::new(),
            bad_peer_scores: Mutex::new(HashMap::new()),
            // ... all 25+ common fields initialized here
            config,
        }
    }
}
```

---

### Phase 6: Session Lifecycle & Executor Plumbing

#### Problem Statement
In `library/src/session/session.rs`, `new_with_options_internal` and `new_magnet_with_options_internal` duplicate:
- Task channel creation (`mpsc::channel`).
- Spawning the executor thread (`spawn_executor`).
- Spawning the choking loop (`config.choking_strategy.spawn_choking_loop`).
- Assembling the `TorrentSession` struct fields.

#### Target Architecture
Extract `TorrentSession::finalize_assembly(context, download_path, config)`:
```rust
// library/src/session/session.rs

impl TorrentSession {
    fn finalize_assembly(
        context: Arc<Mutex<TorrentContext>>,
        download_path: PathBuf,
        config: &SessionConfig,
    ) -> Result<Self, BitTorrentError> {
        let (task_tx, task_rx) =
            std::sync::mpsc::channel::<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>();
        let executor_thread = spawn_executor(context.clone(), task_rx);

        let session = TorrentSession {
            context: context.clone(),
            download_path,
            peer_workers: Arc::new(Mutex::new(Vec::new())),
            task_tx: task_tx.clone(),
            executor_thread: Some(executor_thread),
            manager: None,
            #[cfg(feature = "dht")]
            dht: None,
            #[cfg(feature = "nat-pmp")]
            nat_pmp: None,
        };

        config.choking_strategy.spawn_choking_loop(
            context,
            task_tx,
            None,
        );

        Ok(session)
    }
}
```

---

### Phase 7: Peer Message Wire Encoding & Action Emission Helpers

#### Problem Statement
1. `PeerMessage::encode` manually formats single-`u32` messages (`Have`, `Suggest`, `AllowedFast`) using repeated vector allocations and byte packing.
2. `PeerMessage::HashRequest` and `PeerMessage::HashReject` duplicate a 53-byte payload encoding identically.
3. `network/peer.rs` duplicates the exact 5-line block to emit super-seeding actions (`SendHave`, `SendSuggest`, `SendAllowedFast`) 4 times.

#### Implementation Spec
```rust
// library/src/network/peer_message.rs
impl<'a> PeerMessage<'a> {
    #[inline]
    fn encode_single_u32(id: u8, val: u32) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(9);
        buffer.extend_from_slice(&5u32.to_be_bytes());
        buffer.push(id);
        buffer.extend_from_slice(&val.to_be_bytes());
        buffer
    }

    #[inline]
    fn encode_hash_control(
        id: u8,
        root: &[u8; 32],
        base: u32,
        index: u32,
        length: u32,
        proof_layers: u32,
    ) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(53);
        buffer.extend_from_slice(&49u32.to_be_bytes());
        buffer.push(id);
        buffer.extend_from_slice(root);
        buffer.extend_from_slice(&base.to_be_bytes());
        buffer.extend_from_slice(&index.to_be_bytes());
        buffer.extend_from_slice(&length.to_be_bytes());
        buffer.extend_from_slice(&proof_layers.to_be_bytes());
        buffer
    }
}
```

```rust
// library/src/network/peer.rs
impl Peer {
    #[inline]
    fn push_super_seed_actions(&self, piece: u32, actions: &mut Vec<PeerAction>) {
        actions.push(PeerAction::SendHave(piece));
        if self.supports_fast_extension {
            actions.push(PeerAction::SendSuggest(piece));
            actions.push(PeerAction::SendAllowedFast(piece));
        }
    }
}
```

---

### Phase 8: WebUI REST Response Handler Deduplication

#### Problem Statement
In `clients/bt_webui/src/main.rs`, 7 REST endpoints repeat a 25-line `match execute_ipc(...)` block that inspects `IpcReply::Success`, `IpcReply::Error`, and converts them into JSON with corresponding HTTP status codes.

#### Implementation Spec
```rust
// clients/bt_webui/src/main.rs

fn map_ipc_result(res: Result<IpcReply, String>) -> impl IntoResponse {
    match res {
        Ok(IpcReply::Success { message }) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "success", "message": message })),
        ).into_response(),
        Ok(IpcReply::Error { reason }) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "status": "error", "reason": reason })),
        ).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "reason": err })),
        ).into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "reason": "Unexpected daemon reply" })),
        ).into_response(),
    }
}
```
All endpoints (`add_torrent`, `pause_torrent`, `resume_torrent`, `delete_torrent`, etc.) collapse into 2–3 line functions!

---

### Phase 9: Test Suite Fixture Consolidation

#### Problem Statement
8 integration test files independently implement:
```rust
fn sample_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("files")
        .join(name)
}
```

#### Target Architecture
Create `library/tests/common/mod.rs` and import `common::sample_file` across all test files.

---

## 3. Implementation Order & Verification Matrix

| Step | Phase | Scope | Risk | Automated Verification Command |
|------|-------|-------|------|---------------------------------|
| **1** | Phase 3 | Hex utilities in `util.rs` | Zero | `cargo test -p bittorrent-rs -- util_tests` |
| **2** | Phase 1 | `IpcClient` in `torrent_client_shared` | Low | `cargo test -p torrent_client_shared` |
| **3** | Phase 2 | Media detection & stream engine | Low | `cargo test -p torrent_client_shared && cargo clippy --all-targets` |
| **4** | Phase 4 | `TorrentBuilder::add_dir` & `sync_hub` | Low | `cargo test -p sync_hub && cargo test -p bittorrent-rs -- bep52` |
| **5** | Phase 7 | Message encoding & peer action helpers | Zero | `cargo test -p bittorrent-rs -- peer_message_tests` |
| **6** | Phase 5 & 6 | `TorrentContext` & `TorrentSession` decomposition | Medium | `cargo test --workspace` |
| **7** | Phase 8 | `bt_webui` REST response deduplication | Low | `cargo check -p bt_webui` |
| **8** | Phase 9 | Test fixtures in `library/tests/common` | Zero | `cargo test --workspace` |

---

## 4. Preservation of Non-Negotiable Constraints

1. **`#![no_std]` Compatibility**:
   All new utility functions added to `library/src/utils/` (`to_hex`, `from_hex`, message encoders) must use `alloc::string::String` and `alloc::vec::Vec` without requiring `std`.
2. **API Stability**:
   Public struct signatures and methods marked stable in `docs/api-stability.md` will remain completely backward-compatible. Refactored logic is internal or strictly additive.
3. **Clippy & Lints**:
   All changes must strictly adhere to the workspace clippy rules in root `Cargo.toml` and pass `cargo clippy --workspace --all-targets -- -D warnings` with zero warnings.
