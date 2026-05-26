#!/usr/bin/env bash
# Generate every Paneflow icon asset from a single master PNG.
#
# Inputs (in assets/icons/master/):
#   paneflow-icon-1024.png              required to regenerate; if absent, script no-ops
#   paneflow-icon-1024-simplified.png   optional; used for sizes <=64 to avoid muddy chrome at small px
#   paneflow-icon-template-1024.png     optional; macOS menubar Template image (black silhouette + alpha)
#
# Outputs:
#   assets/icons/paneflow-{16,24,32,48,64,128,256,512}.png   hicolor sizes for cargo-deb / cargo-generate-rpm
#   assets/icons/paneflow.png                                alias of -128 used by some packaging paths
#   assets/PaneFlow.icns                                     consumed by scripts/bundle-macos.sh
#   assets/PaneFlow.ico                                      consumed by Windows MSI (cargo-wix)
#   src-app/assets/icons/paneflow.png                        runtime-embedded GPUI window icon (rust-embed)
#   assets/icons/paneflowTemplate{,@2x}.png                  macOS menubar templates (only if template master exists)
#
# Idempotent and deterministic. Run after editing a master, then commit the regenerated outputs.
#
# Backward compatible: when no master PNG is present at the required path the script logs a
# warning and exits 0. This lets the CI integration land before the masters do and keeps the
# committed (Apr 2026 baseline) icons in place until a master is dropped in.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

MASTER_DIR="$REPO_ROOT/assets/icons/master"
OUT_ICONS_DIR="$REPO_ROOT/assets/icons"
OUT_ICNS="$REPO_ROOT/assets/PaneFlow.icns"
OUT_ICO="$REPO_ROOT/assets/PaneFlow.ico"
OUT_RUNTIME_ICON="$REPO_ROOT/src-app/assets/icons/paneflow.png"

log()  { printf '%s\n' "$*" >&2; }
warn() { log "warning: $*"; }
die()  { log "error: $*"; exit 1; }

# Resolve a master by stem: accept .png (preferred), .jpg, or .jpeg so that
# raw Nano Banana / Midjourney / DALL-E exports (which default to JPG) can
# be dropped in without manual conversion. ImageMagick reads either format
# transparently and writes PNG on the output side.
resolve_master() {
    local stem="$1" path
    for ext in png jpg jpeg; do
        path="$MASTER_DIR/${stem}.${ext}"
        if [ -f "$path" ]; then
            printf '%s' "$path"
            return 0
        fi
    done
    return 1
}

MASTER="$(resolve_master "paneflow-icon-1024"             || true)"
MASTER_SIMPLE="$(resolve_master   "paneflow-icon-1024-simplified" || true)"
MASTER_TEMPLATE="$(resolve_master "paneflow-icon-template-1024"   || true)"

# --- Graceful no-op when no master is present ----------------------------
# Apr 2026 baseline shipped committed PNGs directly without a master pipeline.
# This guard lets the CI integration land before the new chrome master does.
if [ -z "$MASTER" ]; then
    warn "no master found at $MASTER_DIR/paneflow-icon-1024.{png,jpg,jpeg}"
    warn "keeping existing committed icons. To regenerate, drop a 1024x1024 master in that directory and re-run."
    exit 0
fi

# --- Resolve a resize tool ----------------------------------------------
# Lanczos is the best general-purpose resampling filter for icon downscaling.
# `magick` is ImageMagick 7 (Linux/Windows CI); `convert` is IM6 fallback;
# `sips` is built into macOS.
#
# Two flavours:
#   resize_png            -- raw resize, no mask. Used for the macOS Template
#                            silhouette which must preserve its alpha shape.
#   resize_and_mask_png   -- resize + apply a rounded-rect corner mask at
#                            ~22.37% radius. Matches the Apple icon convention
#                            (also adopted by GNOME and modern launchers): the
#                            file ships with transparent corners so dock /
#                            launcher tiles render as squircles, not as a flat
#                            #f7f7f4 square next to other apps.
#
# True G2 squircle (continuous-curvature superellipse) would require a
# precomputed SVG path; the difference vs a regular rounded-rect at <=512px
# is visually indistinguishable, and at 1024px barely so. Skip the
# complexity until someone needs pixel-perfect Apple parity.

# 22.37% expressed as basis points of 10000 for integer arithmetic
# (matches Apple's documented icon mask ratio).
MASK_RADIUS_PCT=2237

# Run a `magick` (or `convert`) invocation with up to 3 attempts.
# ImageMagick 7.1.2-23 (the current Homebrew bottle on macos-14-arm64,
# and what ships preinstalled on windows-2022) has an intermittent
# SIGABRT (exit 134) during coder-module loading -- the same script, on
# the same runner image, with the same master PNG, will succeed one run
# and crash the next. The Linux apt copy on ubuntu-22.04 is older and
# doesn't hit this, but a cheap retry is worth the safety on every leg.
#
# The first arg picks the IM binary (`magick` for IM7, `convert` for
# IM6); remaining args are passed verbatim. Caller is responsible for
# the if/elif branch; this helper only adds the retry. `if run_magick`
# is set-e-safe because failure inside an `if` test is suppressed.
run_magick() {
    local bin="$1"; shift
    local attempt=0
    local max=3
    while : ; do
        if "$bin" "$@"; then
            return 0
        fi
        attempt=$((attempt + 1))
        if [ "$attempt" -ge "$max" ]; then
            warn "$bin failed after $max attempts"
            return 1
        fi
        warn "$bin transient failure (attempt $attempt/$max); retrying in 1s"
        sleep 1
    done
}

resize_png() {
    local src="$1" dst="$2" size="$3"
    if command -v magick >/dev/null 2>&1; then
        run_magick magick "$src" -filter Lanczos -resize "${size}x${size}" -strip "$dst"
    elif command -v convert >/dev/null 2>&1; then
        run_magick convert "$src" -filter Lanczos -resize "${size}x${size}" -strip "$dst"
    elif command -v sips >/dev/null 2>&1; then
        sips -Z "$size" "$src" --out "$dst" >/dev/null
    else
        die "need ImageMagick (magick/convert) or sips to resize PNGs"
    fi
}

resize_and_mask_png() {
    local src="$1" dst="$2" size="$3"
    local radius=$(( size * MASK_RADIUS_PCT / 10000 ))
    local edge=$(( size - 1 ))
    if command -v magick >/dev/null 2>&1; then
        # 3-element pipeline in a single invocation (fast, no temp files):
        #   1. resized source with `-alpha On` to ensure the alpha channel is
        #      active. `On` (vs `Set`) PRESERVES existing alpha values when
        #      the source already has them (PNG masters from Figma with
        #      transparent corners baked in) AND creates an opaque alpha
        #      channel when the source has none (raw JPG render). `-alpha
        #      Set` would force alpha=255 everywhere and destroy the
        #      master's transparency.
        #   2. rounded-rect mask drawn fresh at the target size (compose src)
        #   3. -compose DstIn -composite -> intersect alpha: result is
        #      transparent wherever EITHER the source or the mask is
        #      transparent. So master's existing transparent regions stay,
        #      and the mask additionally rounds the outer tile corners.
        #   4. `PNG32:` output prefix forces RGBA encoding -- otherwise IM
        #      may opportunistically downgrade to palette PNG when the alpha
        #      channel has only 2 distinct values (fully opaque + fully
        #      transparent), which strips the alpha back out.
        run_magick magick \
            \( "$src" -filter Lanczos -resize "${size}x${size}" -alpha On \) \
            \( -size "${size}x${size}" xc:none -fill white \
                -draw "roundrectangle 0,0 ${edge},${edge} ${radius},${radius}" \) \
            -compose DstIn -composite \
            -strip "PNG32:$dst"
    elif command -v convert >/dev/null 2>&1; then
        run_magick convert \
            \( "$src" -filter Lanczos -resize "${size}x${size}" -alpha On \) \
            \( -size "${size}x${size}" xc:none -fill white \
                -draw "roundrectangle 0,0 ${edge},${edge} ${radius},${radius}" \) \
            -compose DstIn -composite \
            -strip "PNG32:$dst"
    elif command -v sips >/dev/null 2>&1; then
        # sips can resize but cannot draw arbitrary masks. Degrade to a raw
        # resize with a visible warning so the user knows the mask was
        # silently skipped on this leg.
        warn "sips fallback: produced ${dst} without squircle mask (install ImageMagick for masked output)"
        sips -Z "$size" "$src" --out "$dst" >/dev/null
    else
        die "need ImageMagick (magick/convert) or sips to resize PNGs"
    fi
}

# Source picker: small sizes (<=64) prefer the simplified master to avoid muddy
# chrome reflections at low resolution. Fall back to the full master if no
# simplified version exists -- the small icons will look softer than ideal but
# the release flow keeps working.
src_for_size() {
    local size="$1"
    if [ "$size" -le 64 ] && [ -f "$MASTER_SIMPLE" ]; then
        printf '%s' "$MASTER_SIMPLE"
    else
        printf '%s' "$MASTER"
    fi
}

# --- Linux hicolor PNGs + rust-embed runtime icon ------------------------
mkdir -p "$OUT_ICONS_DIR"
for size in 16 24 32 48 64 128 256 512; do
    src="$(src_for_size "$size")"
    dst="$OUT_ICONS_DIR/paneflow-${size}.png"
    log "  $dst  <- $(basename "$src")"
    resize_and_mask_png "$src" "$dst" "$size"
done

# Alias paneflow.png at 128 (used by some packaging paths as the canonical
# unsized name). 128 is large enough for the full chrome render -- always
# sourced from the master, never the simplified copy.
cp "$OUT_ICONS_DIR/paneflow-128.png" "$OUT_ICONS_DIR/paneflow.png"

# Runtime-embedded GPUI window icon -- rust-embed picks this up at compile
# time for the title-bar / about pane uses. 128px is enough today.
mkdir -p "$(dirname "$OUT_RUNTIME_ICON")"
cp "$OUT_ICONS_DIR/paneflow-128.png" "$OUT_RUNTIME_ICON"

# --- macOS .icns ---------------------------------------------------------
# Delegate to generate-icns.sh which already has the iconutil/png2icns/
# icnsutil/python3 fallback chain (US-014). It reads the hicolor PNGs we
# just wrote.
#
# Skip on Windows Git Bash: generate-icns.sh requires python3 or one of the
# native packers, and the .icns is only ever consumed by the macOS leg --
# whose CI runs natively on macos-14 with iconutil built into Xcode CLT.
# A failed Windows-side regeneration would be wasted noise.
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*)
        warn "skipping .icns regeneration on Windows (keeps the committed copy; macOS leg regenerates its own)"
        ;;
    *)
        log "  $OUT_ICNS  (via generate-icns.sh)"
        bash "$SCRIPT_DIR/generate-icns.sh" >&2
        ;;
esac

# --- Windows .ico (multi-resolution) -------------------------------------
log "  $OUT_ICO"
TMP_ICO="$(mktemp -d)"
trap 'rm -rf "$TMP_ICO"' EXIT
for size in 16 24 32 48 64 128 256; do
    src="$(src_for_size "$size")"
    resize_and_mask_png "$src" "$TMP_ICO/${size}.png" "$size"
done

# .ico is a multi-image container. ImageMagick assembles it natively and
# automatically PNG-compresses the 256px frame inside the .ico envelope (the
# rest stay BMP) for Vista+ ProgramsAndFeatures compatibility.
if command -v magick >/dev/null 2>&1; then
    run_magick magick "$TMP_ICO"/{16,24,32,48,64,128,256}.png "$OUT_ICO"
elif command -v convert >/dev/null 2>&1; then
    run_magick convert "$TMP_ICO"/{16,24,32,48,64,128,256}.png "$OUT_ICO"
else
    die "need ImageMagick to assemble $OUT_ICO"
fi

# --- macOS menubar Template PNGs (optional) ------------------------------
# AppKit auto-tints images whose filename ends in `Template.png` /
# `Template@2x.png`. The template master MUST be a black silhouette on alpha
# (no chrome render, no color). We only emit these if a template master is
# placed -- the existing release flow does not consume them yet.
if [ -f "$MASTER_TEMPLATE" ]; then
    log "  $OUT_ICONS_DIR/paneflowTemplate.png + @2x"
    resize_png "$MASTER_TEMPLATE" "$OUT_ICONS_DIR/paneflowTemplate.png"    22
    resize_png "$MASTER_TEMPLATE" "$OUT_ICONS_DIR/paneflowTemplate@2x.png" 44
fi

log ""
log "icons regenerated from $(basename "$MASTER")"
[ -f "$MASTER_SIMPLE" ]   || warn "no simplified master -- sizes <=64 use full chrome render and will look muddy"
[ -f "$MASTER_TEMPLATE" ] || log  "no template master  -- skipping menubar Template PNGs"
