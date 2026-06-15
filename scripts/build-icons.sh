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

# Serialise ImageMagick's coder-module loading. The intermittent SIGABRT
# documented on `run_magick` below is a thread race in IM7's module
# registry during first-load of a coder/delegate: two worker threads
# initialise the same module concurrently and abort. Pinning IM to a
# single thread makes module init deterministic and serial. It only
# affects parallelism inside one invocation (icon resizes are tiny, so
# the wall-clock cost is nil) and never changes a single output pixel.
export MAGICK_THREAD_LIMIT=1
export OMP_NUM_THREADS=1

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

# On Windows CI this runs under Git Bash, where pwd yields an MSYS path
# (/d/a/paneflow/...). The preinstalled ImageMagick is the NATIVE magick.exe,
# which cannot open such paths ("No such file or directory"). Convert to a
# mixed Windows path (D:/a/paneflow/...) that both magick.exe and Git Bash
# accept. cygpath exists only under Git Bash/Cygwin, so this is a no-op on
# Linux/macOS.
if command -v cygpath >/dev/null 2>&1; then
    REPO_ROOT="$(cygpath -m "$REPO_ROOT")"
fi

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
# (matches Apple's documented icon mask ratio). NOTE: this is the corner
# radius of the squircle BODY, applied relative to the body size (the inset
# artwork), not the full canvas -- see ICON_BODY_PCT below.
MASK_RADIUS_PCT=2237

# Keyline padding: the squircle body occupies this fraction of the canvas,
# with transparent margin around it. 80.47% (≈10% margin each side) is the
# value GNOME and macOS independently converge on:
#   - GNOME HIG square keyline: 103/128 = 80.47%  (developer.gnome.org/hig
#     /guidelines/app-icons.html — "drawn within 128px but shouldn't fill it")
#   - macOS Big Sur grid:       824/1024 = 80.47% (Apple rounded-rect body)
#   - KDE Breeze is close (40/48 = 83%); freedesktop mandates no padding.
# Without this inset the icon is FULL-BLEED and renders ~23% larger than
# spec-compliant peers in the GNOME Shell dash / dock, which scale every PNG
# to fill a fixed cell and ignore internal padding. Insetting fixes GNOME and
# makes the macOS .icns (sourced from these same PNGs) more correct too.
# Set to 10000 to restore the old full-bleed behaviour.
ICON_BODY_PCT=8047

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
    # 6 attempts (was 3): the IM7 coder-loader SIGABRT can recur across
    # consecutive identical invocations, so a 3-attempt budget is too
    # tight -- v0.3.6's first tag build exhausted it on the masked
    # `paneflow-512.png` step and failed the whole release. The
    # `MAGICK_THREAD_LIMIT=1` export above is the primary mitigation;
    # the wider budget + escalating backoff is the belt to its braces.
    local max=6
    while : ; do
        if "$bin" "$@"; then
            return 0
        fi
        attempt=$((attempt + 1))
        if [ "$attempt" -ge "$max" ]; then
            warn "$bin failed after $max attempts"
            return 1
        fi
        # Escalating backoff (1s, 2s, 3s, ...) gives any transient
        # module-loader / temp-file contention more room between tries
        # than a flat 1s without ballooning total wall-clock.
        warn "$bin transient failure (attempt $attempt/$max); retrying in ${attempt}s"
        sleep "$attempt"
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
    # Inset: the masked squircle body is rendered at `body` px, then centered
    # on a transparent `size` px canvas. Mask radius is relative to the BODY
    # (so the squircle's corner curvature scales with the body, not the
    # padded canvas). At ICON_BODY_PCT=10000 (full-bleed) body==size and the
    # extent is a no-op, preserving the legacy behaviour exactly.
    local body=$(( size * ICON_BODY_PCT / 10000 ))
    [ "$body" -lt 1 ] && body=1
    local radius=$(( body * MASK_RADIUS_PCT / 10000 ))
    local edge=$(( body - 1 ))
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
            \( "$src" -filter Lanczos -resize "${body}x${body}" -alpha On \) \
            \( -size "${body}x${body}" xc:none -fill white \
                -draw "roundrectangle 0,0 ${edge},${edge} ${radius},${radius}" \) \
            -compose DstIn -composite \
            +repage -compose Over -background none -gravity center \
            -extent "${size}x${size}" \
            -strip "PNG32:$dst"
    elif command -v convert >/dev/null 2>&1; then
        run_magick convert \
            \( "$src" -filter Lanczos -resize "${body}x${body}" -alpha On \) \
            \( -size "${body}x${body}" xc:none -fill white \
                -draw "roundrectangle 0,0 ${edge},${edge} ${radius},${radius}" \) \
            -compose DstIn -composite \
            +repage -compose Over -background none -gravity center \
            -extent "${size}x${size}" \
            -strip "PNG32:$dst"
    elif command -v sips >/dev/null 2>&1; then
        # sips can resize but cannot draw arbitrary masks or center-inset onto
        # a transparent canvas. Degrade to a raw full-bleed resize with a
        # visible warning so the user knows both the mask AND the keyline
        # padding were skipped on this leg.
        warn "sips fallback: produced ${dst} full-bleed, without squircle mask or keyline padding (install ImageMagick for spec-correct output)"
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
# Same MSYS->Windows conversion as REPO_ROOT above: mktemp yields /tmp/tmp.XXXX
# under Git Bash, which native magick.exe can't open. Convert so the .ico
# assembly resolves; the trap still removes it fine via the Windows path.
# No-op on Linux/macOS (cygpath absent).
if command -v cygpath >/dev/null 2>&1; then
    TMP_ICO="$(cygpath -m "$TMP_ICO")"
fi
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
