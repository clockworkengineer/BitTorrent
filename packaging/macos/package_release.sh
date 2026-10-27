#!/usr/bin/env bash
# ==============================================================================
# BitTorrent Client - macOS Release Packaging Script (.app & .dmg)
# ==============================================================================
set -euo pipefail

VERSION="${1:-0.3.0}"
SKIP_BUILD="${2:-false}"
ARCH="$(uname -m)"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
APP_DIR="${DIST_DIR}/BitTorrent.app"
STAGE_DIR="${DIST_DIR}/BitTorrent-v${VERSION}-macos-${ARCH}"

echo "=========================================================="
echo "  BitTorrent Client - macOS Release Packaging Script     "
echo "  Version: ${VERSION} (${ARCH})                          "
echo "=========================================================="

# 1. Build release binaries
if [ "${SKIP_BUILD}" != "true" ] && [ "${SKIP_BUILD}" != "--skip-build" ]; then
    echo -e "\n[1/4] Compiling release binaries with cargo..."
    cd "${ROOT_DIR}"
    cargo build --release -p torrent_client -p bt_daemon -p sync_hub -p bt_webui
else
    echo -e "\n[1/4] Skipping cargo build (skip-build requested)..."
fi

# 2. Construct macOS .app bundle
echo -e "\n[2/4] Assembling BitTorrent.app bundle..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"

# Copy executable
cp "${ROOT_DIR}/target/release/torrent_client" "${APP_DIR}/Contents/MacOS/torrent_client"
chmod +x "${APP_DIR}/Contents/MacOS/torrent_client"

# Copy Info.plist
cp "${SCRIPT_DIR}/Info.plist" "${APP_DIR}/Contents/Info.plist"

# Generate ICNS if on macOS with iconutil/sips, otherwise copy png as fallback
if command -v iconutil >/dev/null 2>&1 && command -v sips >/dev/null 2>&1; then
    ICONSET_DIR="/tmp/bittorrent.iconset"
    rm -rf "${ICONSET_DIR}"
    mkdir -p "${ICONSET_DIR}"

    sips -z 16 16     "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_16x16.png" >/dev/null
    sips -z 32 32     "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_16x16@2x.png" >/dev/null
    sips -z 32 32     "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_32x32.png" >/dev/null
    sips -z 64 64     "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_32x32@2x.png" >/dev/null
    sips -z 128 128   "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_128x128.png" >/dev/null
    sips -z 256 256   "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_128x128@2x.png" >/dev/null
    sips -z 256 256   "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_256x256.png" >/dev/null
    sips -z 512 512   "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_256x256@2x.png" >/dev/null
    sips -z 512 512   "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_512x512.png" >/dev/null
    sips -z 1024 1024 "${ROOT_DIR}/assets/icon.png" --out "${ICONSET_DIR}/icon_512x512@2x.png" >/dev/null

    iconutil -c icns "${ICONSET_DIR}" -o "${APP_DIR}/Contents/Resources/icon.icns"
    rm -rf "${ICONSET_DIR}"
    echo "  + Converted and embedded native icon.icns"
else
    cp "${ROOT_DIR}/assets/icon.png" "${APP_DIR}/Contents/Resources/icon.png"
    echo "  + Embedded icon.png"
fi

# 3. Create portable archive
echo -e "\n[3/4] Creating portable distribution staging..."
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}/bin"
cp -R "${APP_DIR}" "${STAGE_DIR}/"

BINARIES=("bt_daemon" "bt_cli" "bt_webui" "sync_hub")
for bin in "${BINARIES[@]}"; do
    SRC="${ROOT_DIR}/target/release/${bin}"
    if [ -f "${SRC}" ]; then
        cp "${SRC}" "${STAGE_DIR}/bin/"
        chmod +x "${STAGE_DIR}/bin/${bin}"
    fi
done

cp "${ROOT_DIR}/README.md" "${STAGE_DIR}/"
cp "${ROOT_DIR}/LICENSE-MIT" "${STAGE_DIR}/"
cp "${ROOT_DIR}/install.sh" "${STAGE_DIR}/"
chmod +x "${STAGE_DIR}/install.sh"

TAR_PATH="${DIST_DIR}/BitTorrent-v${VERSION}-macos-${ARCH}-portable.tar.gz"
cd "${DIST_DIR}"
tar -czf "${TAR_PATH}" -C "${DIST_DIR}" "BitTorrent-v${VERSION}-macos-${ARCH}"
echo "  [OK] Portable package created: ${TAR_PATH}"

# 4. Generate Apple Disk Image (.dmg) if hdiutil is present
echo -e "\n[4/4] Checking macOS disk image generator (hdiutil)..."
DMG_PATH="${DIST_DIR}/BitTorrent-v${VERSION}-macos-${ARCH}.dmg"
if command -v hdiutil >/dev/null 2>&1; then
    DMG_STAGE="/tmp/dmg_stage_$$"
    rm -rf "${DMG_STAGE}"
    mkdir -p "${DMG_STAGE}"
    cp -R "${APP_DIR}" "${DMG_STAGE}/"
    ln -s /Applications "${DMG_STAGE}/Applications"

    rm -f "${DMG_PATH}"
    hdiutil create -volname "BitTorrent" -srcfolder "${DMG_STAGE}" -ov -format UDZO "${DMG_PATH}" >/dev/null
    rm -rf "${DMG_STAGE}"
    echo "  [OK] macOS Disk Image created: ${DMG_PATH}"
else
    echo "  [INFO] hdiutil not available; DMG generation skipped."
fi

echo -e "\nmacOS release packaging complete!"
