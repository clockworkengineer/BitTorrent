#!/usr/bin/env sh
# ==============================================================================
# BitTorrent-rs Universal Cross-Platform Installer (Linux, macOS, Unix)
# ==============================================================================
# Usage:
#   ./install.sh [OPTIONS]
#   curl -fsSL https://raw.githubusercontent.com/clockworkengineer/BitTorrent/main/install.sh | sh
#
# Options:
#   --prefix=<PATH>     Custom install prefix (default: /usr/local or ~/.local)
#   --systemd           Install and enable systemd user service for bt_daemon
#   --daemon-only       Install CLI and daemon tools only, skip Desktop GUI
#   --uninstall         Uninstall all binaries, icons, and desktop integrations
#   --help              Display this help message
# ==============================================================================

set -e

COLOR_RESET="\033[0m"
COLOR_CYAN="\033[1;36m"
COLOR_GREEN="\033[1;32m"
COLOR_YELLOW="\033[1;33m"
COLOR_RED="\033[1;31m"
COLOR_GRAY="\033[0;90m"

log_info() {
    printf "${COLOR_CYAN}[INFO]${COLOR_RESET} %s\n" "$1"
}

log_ok() {
    printf "${COLOR_GREEN}[OK]${COLOR_RESET} %s\n" "$1"
}

log_warn() {
    printf "${COLOR_YELLOW}[WARN]${COLOR_RESET} %s\n" "$1"
}

log_err() {
    printf "${COLOR_RED}[ERROR]${COLOR_RESET} %s\n" "$1" >&2
}

# ------------------------------------------------------------------------------
# 1. Platform Detection
# ------------------------------------------------------------------------------
OS_NAME="$(uname -s 2>/dev/null || echo "Unknown")"
ARCH_NAME="$(uname -m 2>/dev/null || echo "Unknown")"

case "${OS_NAME}" in
    Linux*)   PLATFORM="linux" ;;
    Darwin*)  PLATFORM="macos" ;;
    FreeBSD*) PLATFORM="freebsd" ;;
    OpenBSD*) PLATFORM="openbsd" ;;
    *)        PLATFORM="unix" ;;
esac

# ------------------------------------------------------------------------------
# 2. Defaults & CLI Arguments Parsing
# ------------------------------------------------------------------------------
DEFAULT_PREFIX="/usr/local"
if [ "$(id -u)" != "0" ]; then
    DEFAULT_PREFIX="${HOME}/.local"
fi

INSTALL_PREFIX="${DEFAULT_PREFIX}"
ENABLE_SYSTEMD=false
DAEMON_ONLY=false
DO_UNINSTALL=false

for arg in "$@"; do
    case "$arg" in
        --prefix=*)
            INSTALL_PREFIX="${arg#*=}"
            ;;
        --systemd)
            ENABLE_SYSTEMD=true
            ;;
        --daemon-only)
            DAEMON_ONLY=true
            ;;
        --uninstall)
            DO_UNINSTALL=true
            ;;
        --help|-h)
            printf "${COLOR_CYAN}BitTorrent-rs Cross-Platform Installer${COLOR_RESET}\n"
            printf "Usage: %s [OPTIONS]\n\n" "$0"
            printf "Options:\n"
            printf "  --prefix=<PATH>   Set target directory (default: %s)\n" "${DEFAULT_PREFIX}"
            printf "  --systemd         Install and enable background daemon service\n"
            printf "  --daemon-only     Install CLI and daemon tools without GUI\n"
            printf "  --uninstall       Remove all installed files and associations\n"
            printf "  --help, -h        Show this help message\n\n"
            exit 0
            ;;
        *)
            log_err "Unknown argument: $arg"
            exit 1
            ;;
    esac
done

BIN_DIR="${INSTALL_PREFIX}/bin"
SHARE_DIR="${INSTALL_PREFIX}/share"
APP_DIR="${SHARE_DIR}/applications"
ICON_DIR="${SHARE_DIR}/icons/hicolor/512x512/apps"
SYSTEMD_USER_DIR="${HOME}/.config/systemd/user"

# ------------------------------------------------------------------------------
# 3. Uninstallation Flow
# ------------------------------------------------------------------------------
if [ "${DO_UNINSTALL}" = "true" ]; then
    log_info "Uninstalling BitTorrent-rs from ${INSTALL_PREFIX}..."

    # Stop and disable systemd service if running
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user stop bt_daemon.service 2>/dev/null || true
        systemctl --user disable bt_daemon.service 2>/dev/null || true
        rm -f "${SYSTEMD_USER_DIR}/bt_daemon.service"
        systemctl --user daemon-reload 2>/dev/null || true
    fi

    # Remove binaries
    for bin in torrent_client bt_daemon bt_cli bt_webui sync_hub; do
        if [ -f "${BIN_DIR}/${bin}" ]; then
            rm -f "${BIN_DIR}/${bin}"
            log_ok "Removed binary: ${BIN_DIR}/${bin}"
        fi
    done

    # Remove desktop entry and icons on Linux
    if [ "${PLATFORM}" = "linux" ]; then
        rm -f "${APP_DIR}/bittorrent.desktop"
        rm -f "${ICON_DIR}/bittorrent.png"
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "${APP_DIR}" 2>/dev/null || true
        fi
    fi

    # Remove macOS app bundle
    if [ "${PLATFORM}" = "macos" ]; then
        if [ -d "/Applications/BitTorrent.app" ]; then
            rm -rf "/Applications/BitTorrent.app"
            log_ok "Removed /Applications/BitTorrent.app"
        fi
        if [ -d "${HOME}/Applications/BitTorrent.app" ]; then
            rm -rf "${HOME}/Applications/BitTorrent.app"
            log_ok "Removed ${HOME}/Applications/BitTorrent.app"
        fi
    fi

    log_ok "Uninstallation complete!"
    exit 0
fi

# ------------------------------------------------------------------------------
# 4. Installation Banner & Pre-flight
# ------------------------------------------------------------------------------
printf "${COLOR_CYAN}==========================================================${COLOR_RESET}\n"
printf "${COLOR_CYAN}  BitTorrent Client - Universal Cross-Platform Installer ${COLOR_RESET}\n"
printf "${COLOR_CYAN}  Platform: %-8s  Arch: %-10s  Prefix: %s${COLOR_RESET}\n" "${PLATFORM}" "${ARCH_NAME}" "${INSTALL_PREFIX}"
printf "${COLOR_CYAN}==========================================================${COLOR_RESET}\n\n"

SCRIPT_PATH="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo "")"
SOURCE_ROOT="${SCRIPT_PATH}"

# Check for pre-built binaries in script directory or target/release
locate_binary() {
    name="$1"
    if [ -f "${SOURCE_ROOT}/bin/${name}" ]; then
        echo "${SOURCE_ROOT}/bin/${name}"
    elif [ -f "${SOURCE_ROOT}/${name}" ]; then
        echo "${SOURCE_ROOT}/${name}"
    elif [ -f "${SOURCE_ROOT}/target/release/${name}" ]; then
        echo "${SOURCE_ROOT}/target/release/${name}"
    else
        echo ""
    fi
}

# ------------------------------------------------------------------------------
# 5. Build from Source if Binaries Not Found
# ------------------------------------------------------------------------------
CLIENT_BIN="$(locate_binary torrent_client)"
DAEMON_BIN="$(locate_binary bt_daemon)"

if [ -z "${DAEMON_BIN}" ] && [ -f "${SOURCE_ROOT}/Cargo.toml" ]; then
    log_info "Pre-built binaries not found. Building release binaries with cargo..."
    if ! command -v cargo >/dev/null 2>&1; then
        log_err "Rust and Cargo are required to compile from source. Please install via https://rustup.rs"
        exit 1
    fi
    (cd "${SOURCE_ROOT}" && cargo build --release -p bt_daemon -p sync_hub -p bt_webui)
    if [ "${DAEMON_ONLY}" != "true" ]; then
        (cd "${SOURCE_ROOT}" && cargo build --release -p torrent_client)
    fi
    CLIENT_BIN="$(locate_binary torrent_client)"
    DAEMON_BIN="$(locate_binary bt_daemon)"
fi

# ------------------------------------------------------------------------------
# 6. Install Binaries
# ------------------------------------------------------------------------------
log_info "Creating destination directories..."
mkdir -p "${BIN_DIR}"

install_bin() {
    src="$1"
    name="$2"
    if [ -n "${src}" ] && [ -f "${src}" ]; then
        cp "${src}" "${BIN_DIR}/${name}"
        chmod 755 "${BIN_DIR}/${name}"
        log_ok "Installed ${name} -> ${BIN_DIR}/${name}"
    else
        log_warn "Binary ${name} not available, skipped."
    fi
}

log_info "Installing binaries to ${BIN_DIR}..."
if [ "${DAEMON_ONLY}" != "true" ]; then
    install_bin "${CLIENT_BIN}" "torrent_client"
fi
install_bin "$(locate_binary bt_daemon)" "bt_daemon"
install_bin "$(locate_binary bt_cli)" "bt_cli"
install_bin "$(locate_binary bt_webui)" "bt_webui"
install_bin "$(locate_binary sync_hub)" "sync_hub"

# ------------------------------------------------------------------------------
# 7. Desktop & System Integration
# ------------------------------------------------------------------------------
if [ "${PLATFORM}" = "linux" ] || [ "${PLATFORM}" = "freebsd" ]; then
    if [ "${DAEMON_ONLY}" != "true" ]; then
        log_info "Installing Desktop entry and icons..."
        mkdir -p "${APP_DIR}"
        mkdir -p "${ICON_DIR}"

        # Copy or generate .desktop file
        if [ -f "${SOURCE_ROOT}/packaging/linux/bittorrent.desktop" ]; then
            cp "${SOURCE_ROOT}/packaging/linux/bittorrent.desktop" "${APP_DIR}/"
        elif [ -f "${SOURCE_ROOT}/share/applications/bittorrent.desktop" ]; then
            cp "${SOURCE_ROOT}/share/applications/bittorrent.desktop" "${APP_DIR}/"
        fi

        # Adjust Exec path if non-standard prefix
        if [ "${BIN_DIR}" != "/usr/local/bin" ] && [ "${BIN_DIR}" != "/usr/bin" ]; then
            sed -i.bak "s|^Exec=torrent_client|Exec=${BIN_DIR}/torrent_client|" "${APP_DIR}/bittorrent.desktop" 2>/dev/null || true
            rm -f "${APP_DIR}/bittorrent.desktop.bak"
        fi

        # Install icon
        if [ -f "${SOURCE_ROOT}/assets/icon.png" ]; then
            cp "${SOURCE_ROOT}/assets/icon.png" "${ICON_DIR}/bittorrent.png"
        elif [ -f "${SOURCE_ROOT}/share/icons/hicolor/512x512/apps/bittorrent.png" ]; then
            cp "${SOURCE_ROOT}/share/icons/hicolor/512x512/apps/bittorrent.png" "${ICON_DIR}/bittorrent.png"
        fi

        # Register MIME types and desktop database
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "${APP_DIR}" 2>/dev/null || true
        fi
        if command -v xdg-mime >/dev/null 2>&1; then
            xdg-mime default bittorrent.desktop application/x-bittorrent 2>/dev/null || true
            xdg-mime default bittorrent.desktop x-scheme-handler/magnet 2>/dev/null || true
            log_ok "Registered MIME handlers for .torrent and magnet: URIs"
        fi
    fi

    # Systemd daemon service integration
    if [ "${ENABLE_SYSTEMD}" = "true" ] || [ -d "${SYSTEMD_USER_DIR}" ]; then
        log_info "Setting up systemd user service..."
        mkdir -p "${SYSTEMD_USER_DIR}"
        cat <<EOF > "${SYSTEMD_USER_DIR}/bt_daemon.service"
[Unit]
Description=BitTorrent Core Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${BIN_DIR}/bt_daemon
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=default.target
EOF
        if command -v systemctl >/dev/null 2>&1; then
            systemctl --user daemon-reload 2>/dev/null || true
            if [ "${ENABLE_SYSTEMD}" = "true" ]; then
                systemctl --user enable --now bt_daemon.service 2>/dev/null || true
                log_ok "Enabled and started bt_daemon systemd service"
            fi
        fi
    fi
fi

# ------------------------------------------------------------------------------
# 8. macOS App Bundle Integration
# ------------------------------------------------------------------------------
if [ "${PLATFORM}" = "macos" ] && [ "${DAEMON_ONLY}" != "true" ]; then
    MAC_APP_TARGET="/Applications/BitTorrent.app"
    if [ "$(id -u)" != "0" ]; then
        MAC_APP_TARGET="${HOME}/Applications/BitTorrent.app"
    fi

    log_info "Configuring macOS application bundle at ${MAC_APP_TARGET}..."
    mkdir -p "${MAC_APP_TARGET}/Contents/MacOS"
    mkdir -p "${MAC_APP_TARGET}/Contents/Resources"

    if [ -n "${CLIENT_BIN}" ] && [ -f "${CLIENT_BIN}" ]; then
        cp "${CLIENT_BIN}" "${MAC_APP_TARGET}/Contents/MacOS/torrent_client"
        chmod +x "${MAC_APP_TARGET}/Contents/MacOS/torrent_client"
    fi

    if [ -f "${SOURCE_ROOT}/packaging/macos/Info.plist" ]; then
        cp "${SOURCE_ROOT}/packaging/macos/Info.plist" "${MAC_APP_TARGET}/Contents/Info.plist"
    fi

    if [ -f "${SOURCE_ROOT}/assets/icon.png" ]; then
        cp "${SOURCE_ROOT}/assets/icon.png" "${MAC_APP_TARGET}/Contents/Resources/icon.png"
    fi

    log_ok "Installed macOS Desktop Application: ${MAC_APP_TARGET}"
fi

# ------------------------------------------------------------------------------
# 9. Verification & Summary
# ------------------------------------------------------------------------------
printf "\n${COLOR_GREEN}==========================================================${COLOR_RESET}\n"
printf "${COLOR_GREEN}  Installation Successfully Completed!                   ${COLOR_RESET}\n"
printf "${COLOR_GREEN}==========================================================${COLOR_RESET}\n"
printf "Binaries installed to: ${COLOR_CYAN}%s${COLOR_RESET}\n" "${BIN_DIR}"

# Check PATH
case ":$PATH:" in
    *":${BIN_DIR}:"*) ;;
    *)
        log_warn "${BIN_DIR} is not currently in your PATH environment variable."
        printf "Add it to your shell configuration file (~/.bashrc or ~/.zshrc):\n"
        printf "  export PATH=\"%s:\$PATH\"\n\n" "${BIN_DIR}"
        ;;
esac

printf "Available commands:\n"
if [ "${DAEMON_ONLY}" != "true" ]; then
    printf "  - ${COLOR_CYAN}torrent_client${COLOR_RESET}  : Native GUI Desktop BitTorrent Client\n"
fi
printf "  - ${COLOR_CYAN}bt_daemon${COLOR_RESET}       : High-performance background headless daemon\n"
printf "  - ${COLOR_CYAN}bt_cli${COLOR_RESET}          : Command-line control interface\n"
printf "  - ${COLOR_CYAN}bt_webui${COLOR_RESET}        : Web management gateway\n"
printf "  - ${COLOR_CYAN}sync_hub${COLOR_RESET}        : Encrypted LAN folder synchronization\n\n"
printf "To uninstall in the future:\n"
printf "  %s --uninstall --prefix=%s\n\n" "$0" "${INSTALL_PREFIX}"
