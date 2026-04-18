#!/usr/bin/env bash
# Package a signed+notarized .app bundle into a drag-to-Applications .dmg.
#
# US-016. Run this on a macOS runner AFTER `scripts/sign-macos.sh` and
# `scripts/notarize-macos.sh` have produced a signed, stapled bundle at
# `dist/PaneFlow.app`.
#
# Output filename convention:
#   dist/paneflow-<version>-<arch>-apple-darwin.dmg
#
# The `-apple-darwin` suffix is required: `update_checker.rs::pick_asset`
# (US-008) matches `.dmg` assets with `AssetFormat::target_qualifier() ==
# "-apple-darwin"`, so any other filename shape would silently prevent
# in-app update prompts from finding the asset.
#
# Usage:
#   scripts/create-dmg.sh --version 0.2.0 --arch aarch64
#   scripts/create-dmg.sh --version 0.2.0 --arch x86_64 --app path/to/X.app
set -euo pipefail

VERSION=""
ARCH=""
APP="dist/PaneFlow.app"

usage() {
    cat >&2 <<EOF
Usage: $0 --version <ver> --arch {aarch64|x86_64} [--app <path>]
EOF
}

die() {
    echo "error: $*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)  [ "$#" -ge 2 ] || die "--version requires an argument"; VERSION="$2"; shift 2 ;;
        --arch)     [ "$#" -ge 2 ] || die "--arch requires an argument";    ARCH="$2";    shift 2 ;;
        --app)      [ "$#" -ge 2 ] || die "--app requires an argument";     APP="$2";     shift 2 ;;
        -h|--help)  usage; exit 0 ;;
        *)          usage; die "unknown argument: $1" ;;
    esac
done

# --- Validate inputs ------------------------------------------------------
[ -n "$VERSION" ] || { usage; die "--version is required"; }
[ -n "$ARCH" ]    || { usage; die "--arch is required"; }
case "$ARCH" in
    aarch64|x86_64) ;;
    *) die "--arch must be 'aarch64' or 'x86_64' (got '$ARCH')" ;;
esac
[ -d "$APP" ] || die "bundle not found: $APP"

# hdiutil + osascript are macOS-native and have no portable equivalent.
# Fail loudly on other OSes rather than producing a broken DMG.
command -v hdiutil   >/dev/null 2>&1 || die "hdiutil not found (this script only runs on macOS)"
command -v osascript >/dev/null 2>&1 || die "osascript not found (this script only runs on macOS)"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

VOLNAME="PaneFlow"
FINAL_DMG="$REPO_ROOT/dist/paneflow-${VERSION}-${ARCH}-apple-darwin.dmg"
BG_SRC="$REPO_ROOT/assets/dmg-background.png"

[ -f "$BG_SRC" ] || die "DMG background PNG not found at $BG_SRC"

# --- Prepare staging dir -------------------------------------------------
# Every DMG run starts from a clean staging directory so stale symlinks or
# leftover `.Trash-*` files can't leak into the image.
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING" "$TEMP_DMG"' EXIT
TEMP_DMG="$STAGING/temp.dmg"

# The enclosed app bundle — `cp -R` preserves the embedded code signature
# and extended attributes (notarization ticket). `ditto` would also work
# but `cp -R` keeps the dependency surface minimal.
cp -R "$APP" "$STAGING/"
BUNDLE_NAME="$(basename "$APP")"

# Drag target — a symlink to /Applications makes the familiar macOS
# "drag here to install" UX. `ln -s /Applications` creates a dangling
# symlink on the staging fs (Linux-style absolute path); when the DMG is
# mounted on macOS, /Applications resolves correctly.
ln -s /Applications "$STAGING/Applications"

# The background image lives in a hidden `.background/` subdir so Finder
# renders it but doesn't display it as a normal icon.
mkdir -p "$STAGING/.background"
cp "$BG_SRC" "$STAGING/.background/background.png"

# --- Create a writable staging DMG ---------------------------------------
# Size = 2 × the app bundle's footprint, rounded up to the next 50 MB.
# hdiutil will tighten the image at convert time (UDZO drops unused
# sectors), so over-provisioning here just gives us headroom during
# osascript's Finder writes without ENOSPC.
APP_BYTES=$(du -sk "$APP" | awk '{print $1}')
SIZE_MB=$(( (APP_BYTES * 2 / 1024) + 50 ))
# Cap at something sensible so a misread `du` can't request a 10 GB image.
if [ "$SIZE_MB" -gt 1024 ]; then
    SIZE_MB=1024
fi

echo "Creating staging DMG (${SIZE_MB} MB)..."
hdiutil create \
    -srcfolder "$STAGING" \
    -volname "$VOLNAME" \
    -fs HFS+ \
    -fsargs "-c c=64,a=16,e=16" \
    -format UDRW \
    -size "${SIZE_MB}m" \
    "$TEMP_DMG" >/dev/null

# --- Mount, lay out, detach ----------------------------------------------
MOUNT_INFO="$(hdiutil attach -nobrowse -readwrite -noautoopen "$TEMP_DMG")"
MOUNT_DEV="$(echo "$MOUNT_INFO" | awk 'NR==1 {print $1}')"
MOUNT_POINT="/Volumes/$VOLNAME"

# Tight-scoped detach helper for the osascript trap — if the AppleScript
# blows up mid-layout, we still need to detach cleanly or the next CI run
# will see a stale /Volumes/PaneFlow mount.
detach_volume() {
    hdiutil detach "$MOUNT_DEV" -quiet 2>/dev/null \
        || hdiutil detach "$MOUNT_DEV" -force 2>/dev/null \
        || true
}
trap 'rm -rf "$STAGING" "$TEMP_DMG"; detach_volume' EXIT

# AppleScript configures icon view, window bounds, background image, and
# icon positions. `update without registering applications` commits the
# layout to the volume's .DS_Store before we detach.
#
# Coordinate choices:
#   window 440×340 content area (bounds 400,100 → 1060,500 includes
#   titlebar). Icon grid at y=200 places them vertically centred. PaneFlow
#   on the left at x=165, Applications symlink on the right at x=495 —
#   standard macOS drag-to-install spacing.
osascript <<APPLESCRIPT
tell application "Finder"
    tell disk "$VOLNAME"
        open
        set current view of container window to icon view
        set toolbar visible of container window to false
        set statusbar visible of container window to false
        set the bounds of container window to {400, 100, 1060, 500}
        set viewOptions to the icon view options of container window
        set arrangement of viewOptions to not arranged
        set icon size of viewOptions to 128
        set background picture of viewOptions to file ".background:background.png"
        set position of item "$BUNDLE_NAME" of container window to {165, 200}
        set position of item "Applications" of container window to {495, 200}
        close
        open
        update without registering applications
        delay 1
    end tell
end tell
APPLESCRIPT

# Sync + detach so the volume's .DS_Store is flushed before we convert.
sync
detach_volume

# --- Convert to compressed read-only final image -------------------------
# UDZO = zlib-compressed, universally readable. UDBZ (bzip2) shaves a few
# percent off the final size but needs macOS ≥10.11 and the readback is
# slower — UDZO is the project-appropriate default.
mkdir -p "$(dirname "$FINAL_DMG")"
rm -f "$FINAL_DMG"
hdiutil convert "$TEMP_DMG" \
    -format UDZO \
    -imagekey zlib-level=9 \
    -o "$FINAL_DMG" >/dev/null

# --- Verify -------------------------------------------------------------
# `hdiutil verify` checksums the compressed image — catches truncation.
hdiutil verify "$FINAL_DMG" >/dev/null

# AC3: codesign inside the DMG must still verify. Mount the final image
# read-only and run codesign against the embedded .app — any signature
# drift (e.g., from a buggy hdiutil that rewrote extended attributes)
# would surface here, not at Gatekeeper time on a user's Mac.
VERIFY_MOUNT="$(hdiutil attach -nobrowse -readonly -noautoopen "$FINAL_DMG")"
VERIFY_DEV="$(echo "$VERIFY_MOUNT" | awk 'NR==1 {print $1}')"
VERIFY_PT="/Volumes/$VOLNAME"
if ! codesign --verify --deep --strict "$VERIFY_PT/$BUNDLE_NAME"; then
    hdiutil detach "$VERIFY_DEV" -force 2>/dev/null || true
    die "codesign verification failed on enclosed bundle"
fi
hdiutil detach "$VERIFY_DEV" -quiet 2>/dev/null \
    || hdiutil detach "$VERIFY_DEV" -force 2>/dev/null \
    || true

echo "Created: $FINAL_DMG ($(du -h "$FINAL_DMG" | awk '{print $1}'))"
