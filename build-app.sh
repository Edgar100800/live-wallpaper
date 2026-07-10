#!/bin/zsh
# Builds ParticleWall.app from the Swift package.
# Usage: ./build-app.sh [debug|release]   (default: release)
set -euo pipefail

CONFIG="${1:-release}"
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP="$ROOT/build/ParticleWall.app"

cd "$ROOT"
swift build -c "$CONFIG"

BIN="$(swift build -c "$CONFIG" --show-bin-path)"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN/ParticleWall" "$APP/Contents/MacOS/ParticleWall"
cp "$ROOT/Info.plist" "$APP/Contents/Info.plist"
cp -R "$BIN/ParticleWall_ParticleWall.bundle" "$APP/Contents/Resources/"

codesign --force --deep --sign - "$APP"

# Install outside TCC-protected folders (Desktop/Documents/Downloads): an app
# running from those paths blocks on a folder-access prompt when reading its
# own bundle resources.
INSTALL_DIR="$HOME/Applications"
mkdir -p "$INSTALL_DIR"
rm -rf "$INSTALL_DIR/ParticleWall.app"
cp -R "$APP" "$INSTALL_DIR/ParticleWall.app"

echo "Built:     $APP"
echo "Installed: $INSTALL_DIR/ParticleWall.app"
echo "Run:       open \"$INSTALL_DIR/ParticleWall.app\""
