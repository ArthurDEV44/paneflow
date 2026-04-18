#!/usr/bin/env bash
# Sign a PaneFlow .app bundle with the Apple Developer ID certificate.
#
# US-015. Run this on a macOS runner AFTER `scripts/bundle-macos.sh` has
# produced `dist/PaneFlow.app`, and BEFORE `scripts/notarize-macos.sh`.
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
#   scripts/sign-macos.sh                      # signs dist/PaneFlow.app
#   scripts/sign-macos.sh path/to/MyApp.app
set -euo pipefail

APP="${1:-dist/PaneFlow.app}"

[ -d "$APP" ] || { echo "error: bundle not found: $APP" >&2; exit 1; }

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

# Decode the cert. base64 -d reads stdin; feeding the secret over a
# heredoc-piped stdin keeps it out of argv/ps.
base64 --decode > "$CERT_P12" <<< "$APPLE_DEVELOPER_CERT_P12"

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

# --- Sign ----------------------------------------------------------------
# --force        : replace any prior signature (idempotent re-signs).
# --options runtime : enable hardened runtime — required for notarization.
# --timestamp    : embed an Apple-supplied RFC3161 timestamp — required
#                  for notarization. Without --timestamp, notarytool rejects
#                  with "The signature does not include a secure timestamp."
#
# --deep is INTENTIONALLY OMITTED here per Apple Technote TN3127 and the
# 2024-25 community consensus: on a single-binary .app with no nested
# bundles (verified: bundle-macos.sh creates only Contents/MacOS/ +
# Contents/Resources/), --deep re-signs inside-out with the wrong entitlements
# and confuses the notarization audit trail. If PaneFlow ever grows a
# helper bundle, sign the helper explicitly first, then the parent.
# --deep is still used below on --verify, where it is the correct flag
# for recursive signature traversal.
codesign \
    --force \
    --options runtime \
    --timestamp \
    --sign "$IDENTITY" \
    "$APP"

# Immediate self-check. `--strict` enforces that the bundle structure
# complies with Apple's signing rules; `--verify --deep` re-walks the
# nested structure. This is AC2's embedded sanity check plus the
# precursor to AC5's `spctl --assess`.
codesign --verify --deep --strict --verbose=2 "$APP"

echo "Signed: $APP ($IDENTITY)"
