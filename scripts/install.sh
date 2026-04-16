#!/usr/bin/env bash
# PaneFlow installer — run from the extracted release directory
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY="$SCRIPT_DIR/paneflow"
DESKTOP="$SCRIPT_DIR/paneflow.desktop"
ICONS_DIR="$SCRIPT_DIR/icons"

if [ ! -f "$BINARY" ]; then
    echo "Error: paneflow binary not found in $SCRIPT_DIR"
    exit 1
fi

echo "Installing PaneFlow v$("$BINARY" --version | awk '{print $2}')..."

# Install binary
mkdir -p "$HOME/.local/bin"
cp "$BINARY" "$HOME/.local/bin/paneflow"
chmod +x "$HOME/.local/bin/paneflow"
echo "  Binary  → ~/.local/bin/paneflow"

# Install desktop entry (patch Exec to use full path)
mkdir -p "$HOME/.local/share/applications"
sed "s|Exec=paneflow|Exec=$HOME/.local/bin/paneflow|" "$DESKTOP" \
    > "$HOME/.local/share/applications/paneflow.desktop"
echo "  Desktop → ~/.local/share/applications/paneflow.desktop"

# Install icons
for size in 16 32 48 128 256 512; do
    icon="$ICONS_DIR/paneflow-${size}.png"
    if [ -f "$icon" ]; then
        dest="$HOME/.local/share/icons/hicolor/${size}x${size}/apps"
        mkdir -p "$dest"
        cp "$icon" "$dest/paneflow.png"
    fi
done
echo "  Icons   → ~/.local/share/icons/hicolor/"

# Update caches
if command -v gtk-update-icon-cache &>/dev/null; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
fi
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
fi

echo ""
echo "Done! PaneFlow is installed."
echo ""
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo "Note: ~/.local/bin is not in your PATH."
    echo "Add this to your shell profile (~/.bashrc or ~/.zshrc):"
    echo ""
    echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
fi
echo "Launch from terminal: paneflow"
echo "Or find 'PaneFlow' in your application launcher."
