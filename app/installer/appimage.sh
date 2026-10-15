#!/usr/bin/env bash
# Build WINCI as a Linux AppImage: target/appimage/WINCI-<version>-x86_64.AppImage
#
#   app/installer/appimage.sh        (run from anywhere)
#
# Build on the OLDEST distro you want to support (e.g. Ubuntu 22.04): an
# AppImage only runs where the system glibc is at least as new as the one it
# was built against. The first run downloads appimagetool into target/appimage/.
set -euo pipefail

APP="$(cd "$(dirname "$0")/.." && pwd)"
cd "$APP"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT="$APP/target/appimage"
DIR="$OUT/WINCI.AppDir"
TOOL="$OUT/appimagetool-x86_64.AppImage"
TOOL_URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"

[ "$(uname -m)" = x86_64 ] || { echo "only x86_64 is set up"; exit 1; }

cargo build --release

# AppDir layout: the binary, a desktop entry + icon at the root, and AppRun
rm -rf "$DIR"
mkdir -p "$DIR/usr/bin"
install -m755 target/release/winci "$DIR/usr/bin/winci"
install -m644 installer/winci.desktop "$DIR/winci.desktop"
install -m644 icons/128x128@2x.png "$DIR/winci.png"
ln -s winci.png "$DIR/.DirIcon"
ln -s usr/bin/winci "$DIR/AppRun"

if [ ! -x "$TOOL" ]; then
    echo "downloading appimagetool…"
    curl -fL -o "$TOOL" "$TOOL_URL"
    chmod +x "$TOOL"
fi

# --appimage-extract-and-run: works on build machines without FUSE 2
ARCH=x86_64 "$TOOL" --appimage-extract-and-run "$DIR" "$OUT/WINCI-$VERSION-x86_64.AppImage"
echo "built $OUT/WINCI-$VERSION-x86_64.AppImage"
