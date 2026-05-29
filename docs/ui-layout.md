# Torrent Client UI Layout

## Recommended Layout

A concrete desktop UI layout for a torrent client should include:

- **Top toolbar**
  - Add torrent / magnet
  - Start / Pause / Stop
  - Settings
  - Search / filter

- **Main split view**
  - **Left: Torrent queue**
    - row per torrent
    - columns: name, progress, download rate, upload rate, ratio, status
    - sortable and selectable
  - **Right: Torrent details**
    - tabs for:
      - Overview
      - Files
      - Peers
      - Trackers
      - Logs

- **Bottom status bar**
  - total download speed
  - total upload speed
  - number of active torrents
  - global ratio
  - connection status

## Component List

### Torrent queue
- list/table component
- per-item progress bar
- status badge
- actions: open details, pause/resume, remove

### Torrent details panel
- **Overview tab**
  - torrent name
  - overall progress
  - downloaded / total
  - seeds / peers
  - time remaining
  - current speeds
- **Files tab**
  - file tree / list
  - file priorities
  - per-file selection
- **Peers tab**
  - peer IP
  - client
  - progress
  - upload/download rate
  - connection status
- **Trackers tab**
  - tracker URL
  - status
  - last announce
  - interval
- **Logs tab**
  - event list
  - errors and status messages

### Add torrent dialog
- file chooser for `.torrent`
- magnet link input
- download destination
- file selection / priority options
- start immediately toggle

### Global controls
- start all / pause all / stop all
- add torrent
- settings
- remove completed

### Settings panel
- download/upload speed limits
- maximum active torrents
- connection limits
- peer discovery options
- save path defaults
- UI theme / language

### Notifications / alerts
- toast messages for:
  - torrent added
  - download complete
  - tracker failure
  - disk error

## Layout recommendation

Use a 3-pane layout:

1. **Toolbar**
2. **Main**
   - left: torrent list
   - right: details tabs
3. **Footer status bar**

This layout gives a clean workflow:
- select a torrent from the list
- view or change details on the right
- see global stats at the bottom

## Best framework fit

- For modern cross-platform desktop UI: **Tauri** + web frontend
- For pure Rust prototype: **egui**
- For more native desktop style: **Iced**
