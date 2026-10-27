# Cross-Platform Packaging & Distribution Guide

This document provides a comprehensive guide to building, packaging, installing, and distributing **BitTorrent-rs** across **Windows**, **macOS**, and **Linux/Unix** operating systems.

---

## 1. Supported Platforms & Distribution Formats

The project provides native packaging, automated installers, and protocol handlers tailored for each operating system:

| Operating System | Distribution Format | Primary Artifacts | Installation Tool |
| :--- | :--- | :--- | :--- |
| **Windows** (x86_64, ARM64) | Setup Wizard, Portable ZIP, PowerShell Installer | `BitTorrent-Setup-v<ver>.exe`<br>`BitTorrent-v<ver>-windows-x86_64-portable.zip` | [`install.ps1`](file:///c:/Projects/BitTorrent/install.ps1)<br>Inno Setup Wizard |
| **Linux & Unix** (x86_64, aarch64) | Debian Package, Systemd Service, Portable Tarball | `bittorrent_<ver>_<arch>.deb`<br>`BitTorrent-v<ver>-linux-<arch>-portable.tar.gz` | [`install.sh`](file:///c:/Projects/BitTorrent/install.sh)<br>`dpkg -i` |
| **macOS** (Apple Silicon, Intel) | Application Bundle, Disk Image, Launchd Agent | `BitTorrent.app`<br>`BitTorrent-v<ver>-macos-<arch>.dmg`<br>`BitTorrent-v<ver>-macos-<arch>-portable.tar.gz` | [`install.sh`](file:///c:/Projects/BitTorrent/install.sh)<br>Drag & Drop DMG |

### Distributed Binaries

All packaging workflows distribute the complete 5-binary suite:
1. `torrent_client`: Native graphical desktop application powered by `egui` and `eframe`.
2. `bt_daemon`: High-throughput headless server daemon with JSON-RPC interface.
3. `bt_cli`: Lightweight terminal command-line tool for local and remote daemon control.
4. `bt_webui`: Embedded HTTP gateway and Web dashboard for browser-based torrent administration.
5. `sync_hub`: Decentralized, encrypted LAN and peer-to-peer folder synchronization node.

---

## 2. Universal Scripted Installers

For fast local deployment and headless environments, single-command scripted installers are provided at the root of the repository.

### 2.1 Linux, macOS & Unix (`install.sh`)

The POSIX-compliant shell installer detects the underlying platform (`Linux`, `Darwin`, `FreeBSD`, `OpenBSD`), installs binaries, configures application launchers, and sets up service units:

```bash
# Standard local user installation (installs to ~/.local/bin)
./install.sh

# System-wide installation (requires sudo, installs to /usr/local/bin)
sudo ./install.sh --prefix=/usr/local

# Headless server installation (installs CLI, daemon, WebUI, and enables systemd service)
./install.sh --daemon-only --systemd

# Uninstallation (cleans binaries, icons, desktop entries, and systemd units)
./install.sh --uninstall
```

#### Installer Features on Linux
- Installs `bittorrent.desktop` into `~/.local/share/applications` or `/usr/share/applications`.
- Copies branding icons to `hicolor/512x512/apps/bittorrent.png`.
- Updates the XDG desktop database (`update-desktop-database`) and MIME database (`xdg-mime`).
- Automatically associates `.torrent` files and `magnet:` URL links.
- Generates and enables `bt_daemon.service` under `~/.config/systemd/user/`.

#### Installer Features on macOS
- Constructs `BitTorrent.app` in `/Applications` or `~/Applications`.
- Installs CLI tools and daemons to `/usr/local/bin` or `~/.local/bin`.
- Embeds Launch Services associations from [`packaging/macos/Info.plist`](file:///c:/Projects/BitTorrent/packaging/macos/Info.plist).

---

### 2.2 Windows (`install.ps1`)

The native Windows PowerShell installer installs the full suite without requiring administrative rights or third-party installer runtimes:

```powershell
# Standard user installation (installs to %LOCALAPPDATA%\Programs\BitTorrent)
powershell -ExecutionPolicy Bypass -File .\install.ps1

# Custom installation path with desktop shortcut
powershell -ExecutionPolicy Bypass -File .\install.ps1 -InstallDir "C:\Tools\BitTorrent" -DesktopShortcut

# Headless daemon / CLI only
powershell -ExecutionPolicy Bypass -File .\install.ps1 -DaemonOnly

# Complete uninstallation (removes files, shortcuts, registry keys, and PATH entry)
powershell -ExecutionPolicy Bypass -File .\install.ps1 -Uninstall
```

#### Installer Features on Windows
- Installs binaries into `$env:LOCALAPPDATA\Programs\BitTorrent` (or `$env:ProgramFiles\BitTorrent` if running elevated).
- Creates Start Menu shortcuts for `BitTorrent` and `BitTorrent WebUI`.
- Creates a Desktop shortcut using Windows Script Host COM interfaces.
- Updates the User `PATH` environment variable so `bt_cli` and `bt_daemon` can be launched from any shell.
- Registers `.torrent` file extensions and `magnet:` protocol handlers in `HKCU:\Software\Classes` (no UAC prompt needed).
- Generates a self-contained `uninstall.ps1` script within the installation folder.

---

## 3. Platform Packaging Workflows

The repository includes platform-specific packagers and a universal Python dispatcher.

### 3.1 Universal Packaging Dispatcher (`packaging/package.py`)

Run the dispatcher to automatically detect your host OS and invoke the proper packaging script:

```bash
# Build release packages for the current host OS
python packaging/package.py

# Package specific version without recompiling (if target/release already exists)
python packaging/package.py --version 0.3.0 --skip-build

# Force target packaging suite
python packaging/package.py --target-os linux
```

---

### 3.2 Windows Packaging (`packaging/windows/`)

Files:
- [`packaging/windows/package_release.ps1`](file:///c:/Projects/BitTorrent/packaging/windows/package_release.ps1): Automated compilation and packaging script.
- [`packaging/windows/installer.iss`](file:///c:/Projects/BitTorrent/packaging/windows/installer.iss): Inno Setup 6 configuration script.

```powershell
powershell -ExecutionPolicy Bypass -File .\packaging\windows\package_release.ps1
```

1. Compiles release binaries with optimizations.
2. Stages files, documentation, and icons into `dist/BitTorrent-v<ver>-windows-x86_64/`.
3. Creates a portable `.zip` archive.
4. If Inno Setup (`ISCC.exe`) is found, compiles the `BitTorrent-Setup-v<ver>.exe` installer wizard.

---

### 3.3 Linux Packaging (`packaging/linux/`)

Files:
- [`packaging/linux/package_release.sh`](file:///c:/Projects/BitTorrent/packaging/linux/package_release.sh): Builds portable `.tar.gz` and triggers `.deb` build.
- [`packaging/linux/build_deb.sh`](file:///c:/Projects/BitTorrent/packaging/linux/build_deb.sh): Packages binaries into a Debian/Ubuntu package with post-install hooks.
- [`packaging/linux/bittorrent.desktop`](file:///c:/Projects/BitTorrent/packaging/linux/bittorrent.desktop): Freedesktop desktop launcher.
- [`packaging/linux/bt_daemon.service`](file:///c:/Projects/BitTorrent/packaging/linux/bt_daemon.service): Systemd service unit definition.

```bash
bash packaging/linux/package_release.sh
```

---

### 3.4 macOS Packaging (`packaging/macos/`)

Files:
- [`packaging/macos/package_release.sh`](file:///c:/Projects/BitTorrent/packaging/macos/package_release.sh): Assembles `BitTorrent.app`, converts icons to `.icns`, creates `.dmg` disk image and `.tar.gz`.
- [`packaging/macos/Info.plist`](file:///c:/Projects/BitTorrent/packaging/macos/Info.plist): Application bundle metadata and URI/file associations.
- [`packaging/macos/com.bittorrent.daemon.plist`](file:///c:/Projects/BitTorrent/packaging/macos/com.bittorrent.daemon.plist): Launchd LaunchAgent definition.

```bash
bash packaging/macos/package_release.sh
```

---

## 4. Protocol & File Type Associations

When the desktop client is launched with a `.torrent` file path or `magnet:` URI as a CLI argument, it parses and queues the download automatically.

### System Integration Mapping

| Platform | Mechanism | Document Association (`.torrent`) | URL Scheme (`magnet:`) |
| :--- | :--- | :--- | :--- |
| **Windows** | Registry (`HKCU\Software\Classes` or `HKCR`) | `BitTorrent.TorrentFile`<br>`shell\open\command` | `magnet\shell\open\command` |
| **Linux** | Freedesktop MIME & XDG | `MimeType=application/x-bittorrent;`<br>`xdg-mime default bittorrent.desktop` | `MimeType=x-scheme-handler/magnet;`<br>`xdg-mime default bittorrent.desktop` |
| **macOS** | Launch Services (`Info.plist`) | `CFBundleDocumentTypes`<br>`LSItemContentTypes: org.bittorrent.torrent` | `CFBundleURLTypes`<br>`CFBundleURLSchemes: magnet` |

---

## 5. Background Daemon & Service Integration

### Linux Systemd User Unit
To run `bt_daemon` continuously under systemd:
```bash
# Enable and start daemon on login
systemctl --user enable --now bt_daemon.service

# View daemon logs
journalctl --user -u bt_daemon.service -f
```

### macOS Launchd Agent
To run `bt_daemon` as a macOS user daemon:
```bash
cp packaging/macos/com.bittorrent.daemon.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.bittorrent.daemon.plist
```

---

## 6. Binary Windows Console Suppression

On Windows, graphical desktop applications must suppress the standard command prompt window. In [`clients/torrent_client/src/main.rs`](file:///c:/Projects/BitTorrent/clients/torrent_client/src/main.rs), this is guaranteed via:

```rust
#![windows_subsystem = "windows"]
```

This prevents console window flicker when launching `torrent_client.exe` directly, via desktop/start menu shortcuts, or by double-clicking `.torrent` files in Windows Explorer.
