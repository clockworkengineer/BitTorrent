#!/usr/bin/env bash
# ==============================================================================
# BitTorrent Client - Debian / Ubuntu (.deb) Package Generator
# ==============================================================================
set -euo pipefail

VERSION="${1:-0.3.0}"
ARCH="${2:-amd64}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
PKG_NAME="bittorrent_${VERSION}_${ARCH}"
PKG_DIR="${DIST_DIR}/${PKG_NAME}"

echo "Building Debian package: ${PKG_NAME}.deb..."

mkdir -p "${PKG_DIR}/DEBIAN"
mkdir -p "${PKG_DIR}/usr/bin"
mkdir -p "${PKG_DIR}/usr/share/applications"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps"
mkdir -p "${PKG_DIR}/usr/share/icons/hicolor/512x512/apps"
mkdir -p "${PKG_DIR}/usr/share/doc/bittorrent"
mkdir -p "${PKG_DIR}/lib/systemd/user"

# Control file
cat <<EOF > "${PKG_DIR}/DEBIAN/control"
Package: bittorrent
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Maintainer: BitTorrent-rs Contributors <maintainers@bittorrent-rs.org>
Description: High-performance BitTorrent v1/v2, WebTorrent, and LAN Sync Suite
 Comprehensive BitTorrent protocol implementation in Rust, featuring BitTorrent v2
 (BEP 52) SHA-256 Merkle trees, WebTorrent WebRTC browser bridging, seekable media
 streaming, zero-allocation buffer pooling, and full desktop client GUI.
Homepage: https://github.com/clockworkengineer/BitTorrent
EOF

# Copy binaries
for bin in torrent_client bt_daemon bt_cli bt_webui sync_hub; do
    if [ -f "${ROOT_DIR}/target/release/${bin}" ]; then
        cp "${ROOT_DIR}/target/release/${bin}" "${PKG_DIR}/usr/bin/"
        chmod 755 "${PKG_DIR}/usr/bin/${bin}"
    else
        echo "Warning: binary ${bin} not found in target/release"
    fi
done

# Copy assets and configs
cp "${SCRIPT_DIR}/bittorrent.desktop" "${PKG_DIR}/usr/share/applications/"
cp "${SCRIPT_DIR}/bt_daemon.service" "${PKG_DIR}/lib/systemd/user/"
cp "${ROOT_DIR}/assets/icon.png" "${PKG_DIR}/usr/share/icons/hicolor/512x512/apps/bittorrent.png"
cp "${ROOT_DIR}/assets/icon.png" "${PKG_DIR}/usr/share/icons/hicolor/scalable/apps/bittorrent.png"
cp "${ROOT_DIR}/README.md" "${PKG_DIR}/usr/share/doc/bittorrent/"
cp "${ROOT_DIR}/LICENSE-MIT" "${PKG_DIR}/usr/share/doc/bittorrent/copyright"

# Post-install script
cat <<'EOF' > "${PKG_DIR}/DEBIAN/postinst"
#!/bin/sh
set -e
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t /usr/share/icons/hicolor || true
fi
if command -v xdg-mime >/dev/null 2>&1; then
    xdg-mime default bittorrent.desktop application/x-bittorrent || true
    xdg-mime default bittorrent.desktop x-scheme-handler/magnet || true
fi
exit 0
EOF
chmod 755 "${PKG_DIR}/DEBIAN/postinst"

# Build package
if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb --build --root-owner-group "${PKG_DIR}" "${DIST_DIR}/${PKG_NAME}.deb"
    echo "[OK] Debian package created: ${DIST_DIR}/${PKG_NAME}.deb"
else
    echo "[INFO] dpkg-deb not installed; staged directory left at: ${PKG_DIR}"
fi
