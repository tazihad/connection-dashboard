#!/usr/bin/env bash
# Build a portable AppImage of the connection dashboard.
#
# Usage:
#   packaging/build-appimage.sh            # build release binary, then package
#   SKIP_BUILD=1 packaging/build-appimage.sh
#   ARCH=aarch64 packaging/build-appimage.sh
#
# The AppImage bundles the application binary and its resources only. The
# windowing stack (X11/Wayland), OpenGL driver and glibc come from the host,
# because bundling them would tie the image to one distribution and GPU driver.
set -euo pipefail

cd "$(dirname "$(readlink -f "$0")")/.."

APP=connection-dashboard
ARCH=${ARCH:-$(uname -m)}
VERSION=${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)}
APPDIR="target/appimage/$APP.AppDir"
OUT="target/appimage/$APP-$VERSION-$ARCH.AppImage"
TOOLDIR="target/appimage-tools"
TOOL="$TOOLDIR/appimagetool-$ARCH.AppImage"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
    cargo build --release
fi

rm -rf "$APPDIR"
install -Dm755 "target/release/$APP" "$APPDIR/usr/bin/$APP"
install -Dm644 icon.png "$APPDIR/usr/share/icons/hicolor/512x512/apps/$APP.png"
install -Dm644 "packaging/$APP.desktop" "$APPDIR/$APP.desktop"
install -Dm644 "packaging/$APP.desktop" "$APPDIR/usr/share/applications/$APP.desktop"
install -Dm644 "packaging/io.github.tazihad.connection_dashboard.metainfo.xml" "$APPDIR/usr/share/metainfo/io.github.tazihad.connection_dashboard.metainfo.xml"
install -Dm755 packaging/AppRun "$APPDIR/AppRun"
cp icon.png "$APPDIR/$APP.png"
ln -sf "$APP.png" "$APPDIR/.DirIcon"

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$APPDIR/$APP.desktop"
fi

if [ ! -x "$TOOL" ]; then
    echo "Downloading appimagetool for $ARCH..."
    mkdir -p "$TOOLDIR"
    URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 3 -o "$TOOL" "$URL"
    else
        wget -q -O "$TOOL" "$URL"
    fi
    chmod +x "$TOOL"
fi

# Prefer a real FUSE mount, but fall back to extraction on systems where FUSE is
# unavailable (containers, hardened kernels).
if ! ARCH="$ARCH" VERSION="$VERSION" "$TOOL" "$APPDIR" "$OUT"; then
    rm -f "$OUT"
    echo "Retrying appimagetool with extraction (FUSE unavailable)..."
    APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" VERSION="$VERSION" \
        "$TOOL" "$APPDIR" "$OUT"
fi

echo "Built $OUT"
