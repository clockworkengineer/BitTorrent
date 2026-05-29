# Examples

This folder is reserved for example projects and demonstration code that use the `library` crate.

## Running the example

```bash
cargo run -p torrent_session_example --release -- <torrent-file> <download-dir>
```

The example demonstrates the `TorrentSession` lifecycle API for:

- loading a torrent file
- validating local download state
- starting a download
- pausing and resuming
- stopping the session
