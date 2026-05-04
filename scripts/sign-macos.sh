#!/usr/bin/env bash
# Sign a PaneFlow .app bundle with the Apple Developer ID certificate.
#
# US-015 / US-023. Run this on a macOS runner AFTER `scripts/bundle-macos.sh`
# has produced `dist/PaneFlow.app`, and BEFORE `scripts/notarize-macos.sh`.
#
# The build keychain is intentionally ephemeral: created with a random
# password at the start of this script and deleted in the EXIT trap. Nothing
# is persisted across CI runs, so a compromised runner image cannot replay
# the signing certificate later.
#
# Required env vars (sourced from GitHub Secrets in release.yml):
#   APPLE_DEVELOPER_CERT_P12        base64-encoded .p12 file
#   APPLE_DEVELOPER_CERT_PASSWORD   password to decrypt the .p12
#   APPLE_TEAM_ID                   10-char team ID (for sanity checks)
#
# Usage:
#   scripts/sign-macos.sh                      # signs dist/PaneFlow.app with release entitlements
#   scripts/sign-macos.sh path/to/MyApp.app
#   scripts/sign-macos.sh --entitlements packaging/macos/paneflow.dev.entitlements
#   scripts/sign-macos.sh --entitlements packaging/macos/paneflow.nightly.entitlements path/to/MyApp.app
#
# Arguments:
#   --entitlements <path>    Entitlements plist to embed at signing time.
#                            Defaults to packaging/macos/paneflow.entitlements
#                            (see US-023 for the three supported variants:
#                            release, dev, nightly).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"

ENTITLEMENTS="$REPO_ROOT/packaging/macos/paneflow.entitlements"
APP=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --entitlements)
            [ "$#" -ge 2 ] || { echo "error: --entitlements requires a path" >&2; exit 1; }
            ENTITLEMENTS="$2"
            shift 2
            ;;
        -h|--help)
            sed -n '1,30p' "$0" >&2
            exit 0
            ;;
        --*)
            echo "error: unknown flag: $1" >&2
            exit 1
            ;;
        *)
            [ -z "$APP" ] || { echo "error: unexpected positional arg: $1" >&2; exit 1; }
            APP="$1"
            shift
            ;;
    esac
done

APP="${APP:-dist/PaneFlow.app}"

[ -d "$APP" ] || { echo "error: bundle not found: $APP" >&2; exit 1; }
[ -f "$ENTITLEMENTS" ] || { echo "error: entitlements file not found: $ENTITLEMENTS" >&2; exit 1; }

# Validate the entitlements plist before we feed it to codesign — codesign's
# own error on malformed XML ("error reading entitlements") is hard to
# correlate with the source file. plutil exits non-zero on any parse error.
plutil -lint "$ENTITLEMENTS" >/dev/null

# Fail fast if any required secret is missing / empty. `:?` syntax prints
# the variable name alongside the error, so the CI log tells you exactly
# which secret needs populating.
: "${APPLE_DEVELOPER_CERT_P12:?APPLE_DEVELOPER_CERT_P12 env var is required}"
: "${APPLE_DEVELOPER_CERT_PASSWORD:?APPLE_DEVELOPER_CERT_PASSWORD env var is required}"
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID env var is required}"

# --- Ephemeral keychain ---------------------------------------------------
# A random password; never re-used, never stored. `openssl rand -hex 32`
# yields 64 hex chars = 256 bits of entropy — well above what the Keychain
# encrypts with.
KEYCHAIN_PASSWORD="$(openssl rand -hex 32)"
KEYCHAIN="build.keychain"
CERT_P12="$(mktemp -t paneflow-cert.XXXXXX).p12"

cleanup() {
    security delete-keychain "$KEYCHAIN" 2>/dev/null || true
    rm -f "$CERT_P12"
}
trap cleanup EXIT

# Decode the cert. macOS ships BSD base64 which uses `-D` (not GNU `--decode`)
# — the script always runs on a macOS runner so we can hard-code BSD syntax.
# Feeding the secret over a here-string keeps it out of argv/ps.
base64 -D > "$CERT_P12" <<< "$APPLE_DEVELOPER_CERT_P12"

# Create + unlock keychain, then add it to the search list so find-identity
# / codesign can see it. `-lut 3600` sets an auto-lock timeout; since the
# trap deletes the keychain on exit, the timeout is belt-and-braces.
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 3600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"

# `security list-keychains` without `-s` appends to the default list only
# for the current session; `-s` persists the change — safe here because the
# trap deletes the keychain on exit, so no stale reference survives.
# shellcheck disable=SC2046
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')

# Import cert with explicit tool access for codesign. `-T /usr/bin/codesign`
# whitelists the codesign binary to use the key without pinentry prompts.
security import "$CERT_P12" \
    -k "$KEYCHAIN" \
    -P "$APPLE_DEVELOPER_CERT_PASSWORD" \
    -T /usr/bin/codesign \
    -T /usr/bin/productbuild

# macOS 10.12+ requires this extra unlock before codesign can use an
# imported private key non-interactively. Without it, codesign triggers a
# GUI pinentry and hangs the CI job. The `-S` partition list grants apple
# tooling persistent access.
security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s -k "$KEYCHAIN_PASSWORD" \
    "$KEYCHAIN" > /dev/null

# --- Extract signing identity --------------------------------------------
# `security find-identity -v -p codesigning` prints lines like:
#   1) ABCDE12345... "Developer ID Application: Arthur Jean (TEAM1234AB)"
# We want the quoted name. awk splits on the literal double-quote.
IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" \
    | awk -F'"' '/Developer ID Application/ { print $2; exit }')"

if [ -z "$IDENTITY" ]; then
    echo "error: no 'Developer ID Application' identity in $KEYCHAIN" >&2
    echo "  --- keychain contents ---" >&2
    security find-identity -v "$KEYCHAIN" >&2 || true
    exit 1
fi

# Cross-check that the identity carries the expected team ID. A mismatch
# here would signal that APPLE_TEAM_ID and APPLE_DEVELOPER_CERT_P12 come
# from different Apple Developer accounts.
if [[ "$IDENTITY" != *"($APPLE_TEAM_ID)"* ]]; then
    echo "error: signing identity team ID does not match APPLE_TEAM_ID" >&2
    echo "  identity: $IDENTITY" >&2
    echo "  expected: ...($APPLE_TEAM_ID)" >&2
    exit 1
fi

# --- Sign (inside-out, US-023) -------------------------------------------
# Apple Technote TN3127 and the cmux reference script
# (cmux/scripts/sign-cmux-bundle.sh) both prescribe inside-out signing for
# any bundle that may grow nested binaries: every helper / framework / dylib
# inside Contents/Frameworks/, Contents/Helpers/, Contents/PlugIns/ must be
# signed BEFORE the parent .app. Signing the parent first either re-signs
# the children with the wrong entitlements (--deep) or leaves them unsigned
# (which notarytool then rejects with "The binary is not signed").
#
# bundle-macos.sh today produces a flat bundle (Contents/MacOS/paneflow +
# Contents/Resources/PaneFlow.icns), so the find loops below match nothing
# and exit immediately. Kept in place so that when PaneFlow grows a helper
# bundle (LSP sidecar, MCP host, ...), no signing-script change is required.
#
# Flags:
#   --force        replace any prior signature (idempotent re-signs).
#   --options runtime
#                  enable hardened runtime — required for notarization.
#   --timestamp    embed an Apple-supplied RFC3161 timestamp — required
#                  for notarization. Without --timestamp, notarytool rejects
#                  with "The signature does not include a secure timestamp."
#   --entitlements bind the entitlements plist to the signature so
#                  Gatekeeper enforces them at launch.
#
# --deep is INTENTIONALLY OMITTED on the parent sign call (Apple TN3127):
# the inside-out walk above already covers the children with their own,
# correct entitlements. --deep is still used on --verify below, where it
# is the right flag for recursive signature traversal.
NESTED_PATTERNS=(
    "Contents/Frameworks"
    "Contents/Helpers"
    "Contents/PlugIns"
    "Contents/XPCServices"
)

# Apple TN3127: dylibs MUST be signed without --entitlements (notarytool
# warns "entitlements are not valid for this file type" otherwise). Walk
# them in a first pass with no entitlements, then walk executables /
# .framework / .xpc bundles in a second pass with entitlements. `find -d`
# is the macOS-idiomatic depth-first flag (must precede the path on BSD
# find — placing -depth as a primary mid-expression is portable but ugly);
# the explicit \( … \) grouping keeps the alternation tight against the
# implicit -and that find inserts before -print0.
for sub in "${NESTED_PATTERNS[@]}"; do
    dir="$APP/$sub"
    [ -d "$dir" ] || continue

    # Pass 1: plain dylibs — sign WITHOUT --entitlements.
    while IFS= read -r -d '' nested; do
        codesign \
            --force \
            --options runtime \
            --timestamp \
            --sign "$IDENTITY" \
            "$nested"
    done < <(find -d "$dir" -name '*.dylib' -print0)

    # Pass 2: executables and bundle types — sign WITH --entitlements.
    # Explicit \( … \) grouping makes the alternation independent of
    # find's -a / -o precedence; the trailing -print0 binds to the whole
    # OR with implicit -and. `! -name '*.dylib'` excludes dylibs that
    # carry the executable bit so pass 1 covers them exactly once.
    while IFS= read -r -d '' nested; do
        codesign \
            --force \
            --options runtime \
            --timestamp \
            --entitlements "$ENTITLEMENTS" \
            --sign "$IDENTITY" \
            "$nested"
    done < <(find -d "$dir" \
                \( \
                    \( -name '*.framework' -o -name '*.xpc' \) \
                    -o \
                    \( -type f -perm -u+x ! -name '*.dylib' \) \
                \) -print0)
done

codesign \
    --force \
    --options runtime \
    --timestamp \
    --entitlements "$ENTITLEMENTS" \
    --sign "$IDENTITY" \
    "$APP"

# Immediate self-check. `--strict` enforces that the bundle structure
# complies with Apple's signing rules; `--verify --deep` re-walks the
# nested structure. This is AC2's embedded sanity check plus the
# precursor to AC5's `spctl --assess`.
codesign --verify --deep --strict --verbose=2 "$APP"

echo "Signed: $APP ($IDENTITY)"
echo "Entitlements: $ENTITLEMENTS"
