#!/bin/sh
# Installs the ParticleWall Linux daemon for the current user:
#   1. release build
#   2. systemd --user unit + enable (graphical-session.target)
# Requires: gtk4, gtk4-layer-shell, rustup/cargo. webkitgtk-6.0 is optional:
# particle and ASCII-video wallpapers render with wgpu; WebKit only adds the
# HTML fallback. Without it (or with --gpu-only) the daemon builds without it.
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"

FEATURES=""
if [ "$1" = "--gpu-only" ] || ! pkg-config --exists webkitgtk-6.0; then
    echo "==> GPU-only build (no WebKitGTK: HTML wallpaper fallback disabled)"
    FEATURES="--no-default-features --features gpu,power"
fi

command -v cargo >/dev/null 2>&1 || [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> building (release)"
# shellcheck disable=SC2086 # FEATURES is a word list
cargo build --release --manifest-path "$DIR/Cargo.toml" -p particlewall-linux $FEATURES

cargo build --release --manifest-path "$DIR/../tools/ascii-converter/Cargo.toml"
cp "$DIR/../tools/ascii-converter/target/release/particlewall-ascii-converter" "$DIR/target/release/"

BIN_PATH="$DIR/target/release/particlewall"

echo "==> installing systemd unit"
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"
sed "s|@BIN@|$BIN_PATH|" "$DIR/particlewall.service" > "$UNIT_DIR/particlewall.service"

echo "==> installing desktop entry + icon"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
mkdir -p "$APP_DIR" "$ICON_DIR"
sed "s|@BIN@|$BIN_PATH|" "$DIR/assets/particlewall.desktop" > "$APP_DIR/particlewall.desktop"
cp "$DIR/assets/icon.png" "$ICON_DIR/particlewall.png"
gtk-update-icon-cache -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "==> stopping manual instances"
pkill -f "$BIN_PATH" 2>/dev/null || true

systemctl --user daemon-reload
systemctl --user enable --now particlewall.service
sleep 2
systemctl --user --no-pager status particlewall.service | head -6
echo "==> done. Launcher: buscar ParticleWall en el menu | CLI: particlewall --status"
