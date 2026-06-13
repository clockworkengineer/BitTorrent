# DRY Refactoring Plan II for Torrent Client

This document outlines the second phase of DRY (Don't Repeat Yourself) refactoring for the graphical BitTorrent client application located in `clients/torrent_client/src/main.rs`.

---

## 1. Identified Redundancies and Code Polish Areas

### A. Nested Match Blocks in Session Creation
In `add_session_by_path` (lines 285–301), the code matches on the result of `new_magnet` and `new` separately. Both branches contain identical `Ok` and `Err` structure, leading to duplicate error logging and return statements.
- **Goal**: Consolidate the initialization by assigning the `Result<TorrentSession, BitTorrentError>` first, then matching on it once.

### B. Inconsistent Tracker Error Logging
In `add_session_by_path` (lines 351–356), the tracker announce error handler manually formats, logs to the channel, and prints to stderr, ignoring the helper closure `log_err` defined on line 279.
- **Goal**: Reuse the `log_err` helper closure to unify the logging format and reduce code duplication.

### C. UI Rendering Variables
In `update` (lines 147–155), the local variables (`file_name`, `progress`, `status`, etc.) are assigned from the `SessionState` struct fields and then used immediately. Since the fields are public and simple, they can be read directly during UI rendering, reducing variable assignment noise.

---

## 2. Proposed Changes

### A. Consolidating Session Initialization

```rust
            let session_res = if torrent_path.starts_with("magnet:?") {
                TorrentSession::new_magnet(&torrent_path, &download_dir_buf)
            } else {
                TorrentSession::new(&torrent_path_buf, &download_dir_buf, false)
            };

            let mut session = match session_res {
                Ok(s) => s,
                Err(e) => {
                    log_err(format!("Failed to create session: {}", e));
                    return;
                }
            };
```

---

### B. Unifying Tracker Error Logging

```rust
                Err(e) => {
                    log_err(format!("Tracker announce failed: {}", e));
                    let _ = session_tx.send(session);
                }
```

---

## 3. Concrete DRY Refactor Changes

Below are the exact code modifications proposed.

```diff
diff --git a/clients/torrent_client/src/main.rs b/clients/torrent_client/src/main.rs
index 26b5854..3e6024d 100644
--- a/clients/torrent_client/src/main.rs
+++ b/clients/torrent_client/src/main.rs
@@ -285,21 +285,11 @@ impl TorrentClientApp {
-            let mut session = if torrent_path.starts_with("magnet:?") {
-                match TorrentSession::new_magnet(&torrent_path, &download_dir_buf) {
-                    Ok(s) => s,
-                    Err(e) => {
-                        log_err(format!("Failed to create magnet session: {}", e));
-                        return;
-                    }
-                }
-            } else {
-                match TorrentSession::new(&torrent_path_buf, &download_dir_buf, false) {
-                    Ok(s) => s,
-                    Err(e) => {
-                        log_err(format!("Failed to create session: {}", e));
-                        return;
-                    }
-                }
-            };
+            let session_res = if torrent_path.starts_with("magnet:?") {
+                TorrentSession::new_magnet(&torrent_path, &download_dir_buf)
+            } else {
+                TorrentSession::new(&torrent_path_buf, &download_dir_buf, false)
+            };
+
+            let mut session = match session_res {
+                Ok(s) => s,
+                Err(e) => {
+                    log_err(format!("Failed to create session: {}", e));
+                    return;
+                }
+            };
@@ -351,6 +341,2 @@ impl TorrentClientApp {
                 Err(e) => {
-                    let err_msg = format!("[{}] Tracker announce failed: {}", session_id, e);
-                    let _ = msg_tx.send(err_msg.clone());
-                    eprintln!("{}", err_msg);
+                    log_err(format!("Tracker announce failed: {}", e));
                     let _ = session_tx.send(session);
                 }
```
