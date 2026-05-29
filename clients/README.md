# Clients

This folder contains executable client applications built on top of the `library` crate.

## Torrent client

- `clients/torrent_client` — a desktop torrent client built with `eframe` / `egui`

Run it from the repository root:

```bash
cargo run -p torrent_client --release -- <torrent-file> <download-dir>
```
