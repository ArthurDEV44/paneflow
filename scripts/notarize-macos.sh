#!/usr/bin/env bash
# Notarize + staple a signed PaneFlow .app bundle.
#
# US-015. Run this on a macOS runner AFTER `scripts/sign-macos.sh` has
# stamped a Developer ID signature on `dist/PaneFlow.app`. After this
# script succeeds, the bundle is ready for `.dmg` assembly (US-016).
#
# Required env vars (sourced from GitHub Secrets in release.yml):
#   APPLE_ID                        developer account email
#   APPLE_APP_SPECIFIC_PASSWORD     app-specific password (NOT the Apple ID
#                                   password — generate at
#                                   appleid.apple.com > App-Specific Passwords)
#   APPLE_TEAM_ID                   10-char team ID
#
# Usage:
#   scripts/notarize-macos.sh                      # notarizes dist/PaneFlow.app
#   scripts/notarize-macos.sh path/to/MyApp.app
set -euo pipefail

APP="${1:-dist/PaneFlow.app}"

[ -d "$APP" ] || { echo "error: bundle not found: $APP" >&2; exit 1; }

: "${APPLE_ID:?APPLE_ID env var is required}"
: "${APPLE_APP_SPECIFIC_PASSWORD:?APPLE_APP_SPECIFIC_PASSWORD env var is required}"
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID env var is required}"

ZIP="${APP%.app}.zip"

cleanup() {
    rm -f "$ZIP"
}
trap cleanup EXIT

# --- Build the submission archive ----------------------------------------
# `ditto -c -k --keepParent` is Apple's canonical way to archive an .app
# for notarytool. Plain `zip(1)` strips resource forks and extended
# attributes that codesign wrote during signing; submitting a `zip` archive
# instead of a `ditto` archive can yield "The binary is not signed"
# notarization rejections even on a properly signed bundle.
ditto -c -k --keepParent "$APP" "$ZIP"

# --- Submit + wait -------------------------------------------------------
# --wait blocks until Apple returns a terminal state (Accepted / Invalid /
# Rejected). Apple's SLA for notarytool is ~5 min typical, 30 min P99,
# 48 h absolute ceiling — if this step times out beyond 30 min, it's a
# sign Apple is in a backlog and re-running later usually clears it.
# --output-format json gives us deterministic fields (id, status, message)
# to parse, avoiding brittle regex against the human-readable default.
echo "Submitting $ZIP to notarytool (this usually completes in a few minutes)..."
SUBMIT_JSON="$(xcrun notarytool submit "$ZIP" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_APP_SPECIFIC_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --wait \
    --output-format json)"

echo "notarytool submit output:"
echo "$SUBMIT_JSON"

# Parse with python3 (stdlib, always present on macOS runners). The JSON
# is small and well-formed when notarytool exits 0.
STATUS="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["status"])' <<< "$SUBMIT_JSON")"
SUBMISSION_ID="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<< "$SUBMIT_JSON")"

if [ "$STATUS" != "Accepted" ]; then
    echo "::error title=Notarization::notarization failed (status=$STATUS, id=$SUBMISSION_ID)"
    # AC6: fetch the developer log so the root cause lands in the CI
    # transcript. Common causes: missing hardened runtime, unsigned
    # nested binary, missing secure timestamp, entitlements mismatch.
    # `notarytool log` prints the log JSON to stdout; piping to the
    # workflow log gives reviewers one place to look.
    echo "--- notarytool log $SUBMISSION_ID ---" >&2
    xcrun notarytool log "$SUBMISSION_ID" \
        --apple-id "$APPLE_ID" \
        --password "$APPLE_APP_SPECIFIC_PASSWORD" \
        --team-id "$APPLE_TEAM_ID" \
        >&2 || echo "(failed to retrieve log — Apple may still be processing)" >&2
    exit 1
fi

# --- Staple the ticket ---------------------------------------------------
# The ticket is fetched from Apple's CDN and attached to the .app bundle
# so Gatekeeper can validate offline (air-gapped installs, flaky wifi).
# Without stapling, first-launch needs a round-trip to Apple servers.
xcrun stapler staple "$APP"
xcrun stapler validate "$APP"

# --- Gatekeeper smoke-test (AC5) -----------------------------------------
# `spctl --assess --type exec --verbose` is the user-level Gatekeeper
# check Finder would perform on first launch. Expected pass line:
#   <path>: accepted
#   source=Notarized Developer ID
# We don't grep the output format (varies across macOS versions); the
# exit code is the authoritative pass/fail signal.
spctl --assess --type exec --verbose "$APP"

echo "Notarized + stapled: $APP (submission_id=$SUBMISSION_ID)"
