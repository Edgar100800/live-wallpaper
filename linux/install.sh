#!/bin/sh
# Installs the ParticleWall Linux daemon for the current user:
#   1. release build
#   2. systemd --user unit + enable (graphical-session.target)
# Requires: webkitgtk-6.0, gtk4, gtk4-layer-shell, rustup/cargo.
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"

command -v cargo >/dev/null 2>&1 || [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> building (release)"
cargo build --release --manifest-path "$DIR/Cargo.toml" -p particlewall-linux

echo "==> installing systemd unit"
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"
sed "s|%h|$HOME|" "$DIR/particlewall.service" > "$UNIT_DIR/particlewall.service"

echo "==> stopping manual instances"
pkill -f "$HOME/Projects/live-wallpaper/linux/target/release/particlewall" 2>/dev/null || true

systemctl --user daemon-reload
systemctl --user enable --now particlewall.service
sleep 2
systemctl --user --no-pager status particlewall.service | head -6
echo "==> done. CLI: particlewall --status | --pause | --resume | --fps <N>"
