#!/usr/bin/env bash
# Install PaneFlow icons and desktop entry for Linux
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ASSETS_DIR="$SCRIPT_DIR/../assets"
ICONS_DIR="$ASSETS_DIR/icons"

# Install icons to XDG hicolor theme
for size in 16 32 48 128 256 512; do
    dest="$HOME/.local/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dest"
    cp "$ICONS_DIR/paneflow-${size}.png" "$dest/paneflow.png"
    echo "Installed ${size}x${size} icon"
done

# Install desktop entry
mkdir -p "$HOME/.local/share/applications"
cp "$ASSETS_DIR/paneflow.desktop" "$HOME/.local/share/applications/paneflow.desktop"
echo "Installed desktop entry"

# Update icon cache
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi

echo "Done. PaneFlow icon is now available to your desktop environment."
