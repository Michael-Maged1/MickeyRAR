#!/usr/bin/env bash
set -euo pipefail

echo "=== Building Mickey RAR for macOS ==="

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

# 1. Verify release binary exists
BIN_PATH="target/release/mickey-rar"
if [ ! -f "$BIN_PATH" ]; then
    echo "Error: $BIN_PATH not found. Run cargo build --release first."
    exit 1
fi

# 2. Find 7zz binary
SEVEN_ZIP="$(which 7zz || true)"
if [ -z "$SEVEN_ZIP" ]; then
    echo "7zz not in PATH, checking common brew paths..."
    if [ -f "/opt/homebrew/bin/7zz" ]; then
        SEVEN_ZIP="/opt/homebrew/bin/7zz"
    elif [ -f "/usr/local/bin/7zz" ]; then
        SEVEN_ZIP="/usr/local/bin/7zz"
    else
        echo "Error: 7zz not found. Please install it (brew install sevenzip)."
        exit 1
    fi
fi
echo "Using 7zz from: $SEVEN_ZIP"

# 3. Generate AppIcon.icns using native macOS tools
echo "Generating macOS AppIcon.icns..."
ICON_SRC="assets/icon.png"
ICONSET="AppIcon.iconset"
rm -rf "$ICONSET" AppIcon.icns
mkdir -p "$ICONSET"

sips -z 16 16     "$ICON_SRC" --out "$ICONSET/icon_16x16.png" >/dev/null
sips -z 32 32     "$ICON_SRC" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
sips -z 32 32     "$ICON_SRC" --out "$ICONSET/icon_32x32.png" >/dev/null
sips -z 64 64     "$ICON_SRC" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 128 128   "$ICON_SRC" --out "$ICONSET/icon_128x128.png" >/dev/null
sips -z 256 256   "$ICON_SRC" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256   "$ICON_SRC" --out "$ICONSET/icon_256x256.png" >/dev/null
sips -z 512 512   "$ICON_SRC" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512   "$ICON_SRC" --out "$ICONSET/icon_512x512.png" >/dev/null
sips -z 1024 1024 "$ICON_SRC" --out "$ICONSET/icon_512x512@2x.png" >/dev/null

iconutil -c icns "$ICONSET" -o AppIcon.icns
rm -rf "$ICONSET"
echo "Generated AppIcon.icns successfully."

# 4. Construct MickeyRAR.app
APP_BUNDLE="MickeyRAR.app"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS"
mkdir -p "$APP_BUNDLE/Contents/Resources"

cp "$BIN_PATH" "$APP_BUNDLE/Contents/MacOS/mickey-rar"
chmod +x "$APP_BUNDLE/Contents/MacOS/mickey-rar"

cp "$SEVEN_ZIP" "$APP_BUNDLE/Contents/MacOS/7zz"
chmod +x "$APP_BUNDLE/Contents/MacOS/7zz"

cp AppIcon.icns "$APP_BUNDLE/Contents/Resources/AppIcon.icns"
cp macos/Info.plist "$APP_BUNDLE/Contents/Info.plist"
echo -n "APPL????" > "$APP_BUNDLE/Contents/PkgInfo"

echo "App bundle created at: $APP_BUNDLE"

# 5. Create DMG (Disk Image) with Applications symlink
echo "Creating MickeyRAR-macOS.dmg..."
DMG_STAGING="dmg_staging"
rm -rf "$DMG_STAGING" MickeyRAR-macOS.dmg
mkdir -p "$DMG_STAGING"

cp -R "$APP_BUNDLE" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

hdiutil create -volname "Mickey RAR" -srcfolder "$DMG_STAGING" -ov -format UDZO MickeyRAR-macOS.dmg

rm -rf "$DMG_STAGING" AppIcon.icns
echo "=== Done! Created MickeyRAR-macOS.dmg successfully ==="
ls -lh MickeyRAR-macOS.dmg
