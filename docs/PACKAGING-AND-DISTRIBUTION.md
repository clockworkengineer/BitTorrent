# Windows Packaging & Distribution Guide

This document describes how **BitTorrent-rs** desktop releases, Windows installers, and portable packages are compiled, configured, and published.

---

## 1. Release Artifacts Overview

The project generates two official distribution formats for Windows 64-bit systems:

| Distribution Artifact | File Path | Description |
| :--- | :--- | :--- |
| **Windows Setup Installer** | [`dist/BitTorrent-Setup-v0.1.1.exe`](file:///c:/Projects/BitTorrent/dist/BitTorrent-Setup-v0.1.1.exe) | Professional Inno Setup 6 installer with start menu shortcuts, uninstaller, and `.torrent` / `magnet:` protocol associations. |
| **Portable ZIP Release** | [`dist/BitTorrent-v0.1.1-windows-x86_64-portable.zip`](file:///c:/Projects/BitTorrent/dist/BitTorrent-v0.1.1-windows-x86_64-portable.zip) | Standalone portable archive containing all release binaries, documentation, and icons (zero installation required). |

Both artifacts package all 5 compiled workspace binaries:
- `torrent_client.exe` (Desktop GUI)
- `bt_daemon.exe` (Server Daemon)
- `bt_cli.exe` (CLI management utility)
- `bt_webui.exe` (Web UI gateway)
- `sync_hub.exe` (Encrypted LAN folder sync)

---

## 2. Windows Packaging Script (`package_release.ps1`)

The automated PowerShell packaging script is located at [`packaging/windows/package_release.ps1`](file:///c:/Projects/BitTorrent/packaging/windows/package_release.ps1).

### 2.1 Execution Steps
1. **Compilation**: Invokes `cargo build --release` across all targets.
2. **Staging**: Gathers binaries, licenses, README, and multi-resolution icon assets into `dist/BitTorrent-v0.1.1-windows-x86_64`.
3. **Portable ZIP Creation**: Compresses the staged payload into `dist/BitTorrent-v0.1.1-windows-x86_64-portable.zip`.
4. **Installer Compilation**: Discovers `ISCC.exe` (Inno Setup 6) on the system PATH or standard install paths (`C:\Program Files (x86)\Inno Setup 6` or `C:\Users\<User>\AppData\Local\Programs\Inno Setup 6`) and compiles `dist/BitTorrent-Setup-v0.1.1.exe`.

### 2.2 Running the Packaging Script
```powershell
# Full release build and installer compilation
powershell -ExecutionPolicy Bypass -File .\packaging\windows\package_release.ps1

# Quick packaging (skips cargo build if target/release is already compiled)
powershell -ExecutionPolicy Bypass -File .\packaging\windows\package_release.ps1 -SkipBuild
```

---

## 3. Inno Setup 6 Configuration (`installer.iss`)

The installer configuration is located in [`packaging/windows/installer.iss`](file:///c:/Projects/BitTorrent/packaging/windows/installer.iss).

### Key Features
- **Modern Wizard Style**: Configured with `WizardStyle=modern` and solid-state compression (`Compression=lzma2/ultra64`).
- **File Associations**:
  - Automatically associates `.torrent` files with `torrent_client.exe`.
  - Registers the `magnet:` protocol handler in the Windows Registry (`HKCR\magnet\shell\open\command`).
- **Start Menu & Desktop Shortcuts**: Optional user tasks to create Start Menu programs shortcuts and a Desktop icon.
- **Run on Finish**: Checkbox option to immediately launch BitTorrent Client upon setup completion.
- **Clean Uninstallation**: Removes registry keys, program files, and shortcuts cleanly.

---

## 4. Console Suppression for Desktop GUI

By default, Rust compiles Windows binaries with the `console` subsystem, which pops up an unwanted black command prompt window behind graphical applications.

In [`clients/torrent_client/src/main.rs`](file:///c:/Projects/BitTorrent/clients/torrent_client/src/main.rs), this is suppressed using:
```rust
#![windows_subsystem = "windows"]
```

This guarantees that launching `torrent_client.exe` directly, via desktop shortcut, or by double-clicking a `.torrent` file creates only the native graphical window with zero console flicker.

---

## 5. Application Icon Assets

The project includes custom branded multi-resolution icons:
- **`assets/icon.ico`**: Multi-layer Windows icon embedding 16×16, 32×32, 48×48, 64×64, 128×128, and 256×256 resolutions with PNG/RGBA compression for crisp scaling across Windows taskbars, Start Menu tiles, and Explorer file listings.
- **`assets/icon.png`**: High-resolution 1024×1024 master image used for installer wizard graphics and Web UI favicon/branding.
- **In-App Dynamic Icon**: `torrent_client` embeds RGBA icon bytes at compile-time and installs the window icon dynamically into the `egui`/`eframe` viewport on startup.
