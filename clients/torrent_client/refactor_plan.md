# DRY Refactoring Plan for Torrent Client

This document outlines the concrete DRY (Don't Repeat Yourself) refactoring plan for the graphical BitTorrent client application located in `clients/torrent_client/src/main.rs`.

---

## 1. Identified Redundancies (Violations of DRY)

### A. Session State Synchronization
The logic to fetch data from a locked `TorrentContext` and update the local `SessionState` fields (file name, progress, status, connected peers, active peers, download speed, total downloaded, total uploaded, and total size) is duplicated exactly in:
1. `SessionState::new` (lines 71–84) using `.lock().unwrap()`
2. `TorrentClientApp::update` (lines 186–200) using `.try_lock()`

### B. Torrent Session Instantiation Error Handling
The error handling and logging pattern during background torrent session instantiation is duplicated in `add_session_by_path` (lines 344–364):
- Printing to standard error.
- Sending the error message to the GUI message queue thread.
- Early returning from the spawned thread.

---

## 2. Refactoring Proposals

### A. Extract State Synchronization to `SessionState` Method
Implement an `update_from_context` helper method on `SessionState` that accepts a reference to `TorrentContext` and centralizes the field synchronization logic.

#### Implementation Draft:
```rust
impl SessionState {
    /// Synchronizes the session display state with the underlying TorrentContext.
    /// This keeps UI stats updated (progress, active peers, speed, etc.).
    fn update_fields(&mut self, ctx: &bittorrent_rs::TorrentContext) {
        self.last_file_name = std::path::Path::new(&ctx.file_name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| ctx.file_name.clone());
        self.last_progress = ctx.progress_percent() / 100.0;
        self.last_status = format!("{:?}", ctx.status);
        self.last_peers_connected = ctx.peer_swarm.read().unwrap().len();
        self.last_peers_active = ctx.number_of_unchoked_peers();
        self.last_bps = ctx.bytes_per_second() as u64;
        self.last_downloaded = ctx.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
        self.last_total = ctx.total_bytes_to_download;
        self.last_uploaded = ctx.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed);
    }
}
```

Then refactor the callers:
* In `SessionState::new`:
  ```rust
  if let Ok(ctx_guard) = state.session.context().lock() {
      state.update_fields(&ctx_guard);
  }
  ```
* In `update`:
  ```rust
  if let Ok(ctx_guard) = session_state.session.context().try_lock() {
      session_state.update_fields(&ctx_guard);
  }
  ```

---

### B. Simplify Session Creation Error Handlers
Use a local helper inside the spawned thread to handle failures uniformly during session creation.

#### Implementation Draft:
```rust
// A helper closure to report session initialization errors to UI logs and stderr.
let handle_error = |err_msg: String| {
    let _ = msg_tx.send(err_msg.clone());
    eprintln!("{}", err_msg);
};
```

This reduces code replication and simplifies error routing.

---

## 3. Concrete DRY Refactor Changes

Below are the exact code modifications proposed.

```diff
diff --git a/clients/torrent_client/src/main.rs b/clients/torrent_client/src/main.rs
index 17555..2a0b1 100644
--- a/clients/torrent_client/src/main.rs
+++ b/clients/torrent_client/src/main.rs
@@ -58,21 +58,11 @@ impl SessionState {
             last_total: 0,
             last_uploaded: 0,
         };
         if let Ok(ctx_guard) = state.session.context().lock() {
-            state.last_file_name = std::path::Path::new(&ctx_guard.file_name)
-                .file_name()
-                .map(|n| n.to_string_lossy().to_string())
-                .unwrap_or_else(|| ctx_guard.file_name.clone());
-            state.last_progress = ctx_guard.progress_percent() / 100.0;
-            state.last_status = format!("{:?}", ctx_guard.status);
-            state.last_peers_connected = ctx_guard.peer_swarm.read().unwrap().len();
-            state.last_peers_active = ctx_guard.number_of_unchoked_peers();
-            state.last_bps = ctx_guard.bytes_per_second() as u64;
-            state.last_downloaded = ctx_guard.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
-            state.last_total = ctx_guard.total_bytes_to_download;
-            state.last_uploaded = ctx_guard.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed);
+            state.update_fields(&ctx_guard);
         }
         state
     }
+
+    /// Synchronizes display fields from the underlying TorrentContext.
+    fn update_fields(&mut self, ctx_guard: &bittorrent_rs::TorrentContext) {
+        self.last_file_name = std::path::Path::new(&ctx_guard.file_name)
+            .file_name()
+            .map(|n| n.to_string_lossy().to_string())
+            .unwrap_or_else(|| ctx_guard.file_name.clone());
+        self.last_progress = ctx_guard.progress_percent() / 100.0;
+        self.last_status = format!("{:?}", ctx_guard.status);
+        self.last_peers_connected = ctx_guard.peer_swarm.read().unwrap().len();
+        self.last_peers_active = ctx_guard.number_of_unchoked_peers();
+        self.last_bps = ctx_guard.bytes_per_second() as u64;
+        self.last_downloaded = ctx_guard.total_bytes_downloaded.load(std::sync::atomic::Ordering::Relaxed);
+        self.last_total = ctx_guard.total_bytes_to_download;
+        self.last_uploaded = ctx_guard.total_bytes_uploaded.load(std::sync::atomic::Ordering::Relaxed);
+    }
 }
```
