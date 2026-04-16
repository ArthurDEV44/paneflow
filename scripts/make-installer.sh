#!/usr/bin/env bash
# Build a self-extracting installer for PaneFlow
# Usage: ./make-installer.sh <version> <release-dir>
# Output: paneflow-<version>-x86_64-linux.run
set -euo pipefail

VERSION="${1:?Usage: make-installer.sh <version> <release-dir>}"
RELEASE_DIR="${2:?Usage: make-installer.sh <version> <release-dir>}"
OUTPUT="paneflow-${VERSION}-x86_64-linux.run"

# Create the tar.gz payload from the release directory
PAYLOAD=$(mktemp)
tar czf "$PAYLOAD" -C "$RELEASE_DIR" paneflow/

# Build the self-extracting script
cat > "$OUTPUT" << 'INSTALLER_HEADER'
#!/usr/bin/env bash
# PaneFlow installer — self-extracting archive
# https://github.com/ArthurDEV44/paneflow
set -euo pipefail

echo ""
echo "  PaneFlow — GPU-accelerated terminal multiplexer"
echo ""

# Find where the payload starts (after the __PAYLOAD__ marker)
ARCHIVE_START=$(awk '/^__PAYLOAD__$/{print NR + 1; exit 0;}' "$0")
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# Extract payload
tail -n +"$ARCHIVE_START" "$0" | tar xzf - -C "$TMPDIR"

DIR="$TMPDIR/paneflow"
BINARY="$DIR/paneflow"

if [ ! -f "$BINARY" ]; then
    echo "Error: extraction failed"
    exit 1
fi

VERSION=$("$BINARY" --version | awk '{print $2}')
echo "  Installing v${VERSION}..."
echo ""

# Install binary
mkdir -p "$HOME/.local/bin"
cp "$BINARY" "$HOME/.local/bin/paneflow"
chmod +x "$HOME/.local/bin/paneflow"
echo "  Binary  → ~/.local/bin/paneflow"

# Install desktop entry
mkdir -p "$HOME/.local/share/applications"
sed "s|Exec=paneflow|Exec=$HOME/.local/bin/paneflow|" "$DIR/paneflow.desktop" \
    > "$HOME/.local/share/applications/paneflow.desktop"
echo "  Desktop → ~/.local/share/applications/paneflow.desktop"

# Install icons
for size in 16 32 48 128 256 512; do
    icon="$DIR/icons/paneflow-${size}.png"
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
echo "  Done! PaneFlow is installed."
echo ""
if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    echo "  Note: ~/.local/bin is not in your PATH."
    echo "  Add to your shell profile:"
    echo ""
    echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
fi
echo "  Launch: paneflow"
echo "  Or find 'PaneFlow' in your application launcher."
echo ""

exit 0
__PAYLOAD__
INSTALLER_HEADER

# Append the payload
cat "$PAYLOAD" >> "$OUTPUT"
chmod +x "$OUTPUT"
rm -f "$PAYLOAD"

echo "Created $OUTPUT ($(du -h "$OUTPUT" | cut -f1))"
