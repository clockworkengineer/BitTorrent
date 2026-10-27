#!/usr/bin/env bash
# ==============================================================================
# BitTorrent Client - Linux & Unix Release Packaging Script
# ==============================================================================
set -euo pipefail

VERSION="${1:-0.3.0}"
SKIP_BUILD="${2:-false}"
ARCH="$(uname -m)"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
STAGE_DIR="${DIST_DIR}/BitTorrent-v${VERSION}-linux-${ARCH}"

echo "=========================================================="
echo "  BitTorrent Client - Linux Release Packaging Script     "
echo "  Version: ${VERSION} (${ARCH})                          "
echo "=========================================================="

# 1. Build optimized release binaries
if [ "${SKIP_BUILD}" != "true" ] && [ "${SKIP_BUILD}" != "--skip-build" ]; then
    echo -e "\n[1/4] Compiling release binaries with cargo..."
    cd "${ROOT_DIR}"
    cargo build --release -p torrent_client -p bt_daemon -p sync_hub -p bt_webui
else
    echo -e "\n[1/4] Skipping cargo build (skip-build requested)..."
fi

# 2. Stage distribution directory
echo -e "\n[2/4] Staging files into ${STAGE_DIR}..."
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}/bin"
mkdir -p "${STAGE_DIR}/share/applications"
mkdir -p "${STAGE_DIR}/share/icons/hicolor/512x512/apps"
mkdir -p "${STAGE_DIR}/share/systemd"

BINARIES=("torrent_client" "bt_daemon" "bt_cli" "bt_webui" "sync_hub")
for bin in "${BINARIES[@]}"; do
    SRC="${ROOT_DIR}/target/release/${bin}"
    if [ -f "${SRC}" ]; then
        cp "${SRC}" "${STAGE_DIR}/bin/"
        chmod +x "${STAGE_DIR}/bin/${bin}"
        echo "  + Staged: ${bin}"
    else
        echo "  ! Warning: missing binary ${SRC}"
    fi
done

# Copy resources and integration files
cp "${SCRIPT_DIR}/bittorrent.desktop" "${STAGE_DIR}/share/applications/"
cp "${SCRIPT_DIR}/bt_daemon.service" "${STAGE_DIR}/share/systemd/"
cp "${ROOT_DIR}/assets/icon.png" "${STAGE_DIR}/share/icons/hicolor/512x512/apps/bittorrent.png"
cp "${ROOT_DIR}/README.md" "${STAGE_DIR}/"
cp "${ROOT_DIR}/LICENSE-MIT" "${STAGE_DIR}/"
cp "${ROOT_DIR}/LICENSE-APACHE" "${STAGE_DIR}/"
cp "${ROOT_DIR}/install.sh" "${STAGE_DIR}/"
chmod +x "${STAGE_DIR}/install.sh"

# 3. Create portable tar.gz archive
echo -e "\n[3/4] Creating portable .tar.gz release..."
TAR_PATH="${DIST_DIR}/BitTorrent-v${VERSION}-linux-${ARCH}-portable.tar.gz"
mkdir -p "${DIST_DIR}"
cd "${DIST_DIR}"
tar -czf "${TAR_PATH}" -C "${DIST_DIR}" "BitTorrent-v${VERSION}-linux-${ARCH}"
echo "  [OK] Portable package created: ${TAR_PATH}"

# 4. Generate Debian package (.deb) if dpkg-deb is present
echo -e "\n[4/4] Checking Debian package generator..."
DEB_ARCH="amd64"
if [ "${ARCH}" = "aarch64" ] || [ "${ARCH}" = "arm64" ]; then
    DEB_ARCH="arm64"
fi

if bash "${SCRIPT_DIR}/build_deb.sh" "${VERSION}" "${DEB_ARCH}"; then
    echo "  [OK] Debian package generation completed."
else
    echo "  [INFO] Debian package generation skipped."
fi

echo -e "\nLinux release packaging complete!"
