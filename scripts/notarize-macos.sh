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

# --- Submit (non-blocking) -----------------------------------------------
# Originally this script used `notarytool submit --wait`, which blocks the
# CI runner inside a single API call until Apple returns a terminal status.
# `--wait` produces no output during the wait — when Apple's notary backend
# is in a deep queue (observed 30+ min for first-time submissions from a
# new Developer account), the runner appears frozen with no heartbeat in
# the CI log, no submission ID exposed for out-of-band recovery, and no
# upper bound on duration short of the GitHub-default 6h job timeout.
#
# The replacement pattern submits without `--wait`, captures the
# submission ID immediately, and polls `notarytool info` on a fixed
# interval. This buys three things the `--wait` path could not:
#   - Heartbeat: each poll prints `[+MM:SS] In Progress...` to the CI log
#     so reviewers can see Apple is still processing rather than guessing
#     whether the runner deadlocked.
#   - Recoverability: the submission ID is logged at line 1 after submit,
#     so even if the runner is killed mid-poll the ID survives in the CI
#     transcript. A maintainer with any Mac + the same credentials can
#     run `xcrun notarytool info <id>` later to pick up where we left off.
#   - Bounded waits: a 90-min ceiling matches Apple's documented P99
#     (~30 min) plus first-submission scrutiny margin (typically 45-60
#     min) without consuming a 6h runner reservation when Apple stalls.
#
# `--output-format json` keeps parsing deterministic — the human-readable
# default text shifts between Xcode releases.
echo "Submitting $ZIP to notarytool..."
SUBMIT_JSON="$(xcrun notarytool submit "$ZIP" \
    --apple-id "$APPLE_ID" \
    --password "$APPLE_APP_SPECIFIC_PASSWORD" \
    --team-id "$APPLE_TEAM_ID" \
    --output-format json)"

# Parse with python3 (stdlib, always present on macOS runners).
SUBMISSION_ID="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])' <<< "$SUBMIT_JSON")"

echo "Submission ID: $SUBMISSION_ID"
echo "(If this job is killed mid-poll, recover status from any Mac with:"
echo "    xcrun notarytool info $SUBMISSION_ID --apple-id <APPLE_ID> --team-id $APPLE_TEAM_ID --password <APP_SPECIFIC_PASSWORD>)"

# --- Poll for terminal status --------------------------------------------
# Loop runs until status reaches Accepted / Invalid / Rejected, or the
# elapsed wait exceeds MAX_WAIT_SECONDS. POLL_INTERVAL=30s gives reviewers
# a heartbeat every half-minute without making the API call rate
# meaningfully expensive (Apple does not document a per-account rate
# limit on notarytool, but ~10 polls in the typical 5 min case and ~180
# polls at the 90 min ceiling are well below any plausible threshold).
POLL_INTERVAL=30
MAX_WAIT_SECONDS=$((90 * 60))
START_TIME=$(date +%s)

while true; do
    INFO_JSON="$(xcrun notarytool info "$SUBMISSION_ID" \
        --apple-id "$APPLE_ID" \
        --password "$APPLE_APP_SPECIFIC_PASSWORD" \
        --team-id "$APPLE_TEAM_ID" \
        --output-format json)"
    STATUS="$(python3 -c 'import json,sys; print(json.load(sys.stdin).get("status", "Unknown"))' <<< "$INFO_JSON")"

    NOW=$(date +%s)
    ELAPSED=$((NOW - START_TIME))
    ELAPSED_FMT="$(printf '%02d:%02d' $((ELAPSED / 60)) $((ELAPSED % 60)))"

    case "$STATUS" in
        Accepted)
            echo "[+${ELAPSED_FMT}] Accepted by Apple"
            break
            ;;
        Invalid|Rejected)
            echo "::error title=Notarization::Apple rejected submission (status=$STATUS, id=$SUBMISSION_ID)"
            # AC6: surface the developer log on rejection so the root
            # cause lands in the CI transcript. Common causes: missing
            # hardened runtime, unsigned nested binary, missing secure
            # timestamp, entitlements mismatch.
            echo "--- notarytool log $SUBMISSION_ID ---" >&2
            xcrun notarytool log "$SUBMISSION_ID" \
                --apple-id "$APPLE_ID" \
                --password "$APPLE_APP_SPECIFIC_PASSWORD" \
                --team-id "$APPLE_TEAM_ID" \
                >&2 || echo "(failed to retrieve log — Apple may still be processing)" >&2
            exit 1
            ;;
        "In Progress")
            echo "[+${ELAPSED_FMT}] In Progress... (next poll in ${POLL_INTERVAL}s)"
            ;;
        *)
            # Any unexpected status (network blip mid-poll, unknown
            # transient Apple state) — keep polling until terminal or
            # timeout, but flag the anomaly in the log.
            echo "[+${ELAPSED_FMT}] Unexpected status: $STATUS — continuing to poll"
            ;;
    esac

    if [ "$ELAPSED" -ge "$MAX_WAIT_SECONDS" ]; then
        echo "::error title=Notarization timeout::Submission $SUBMISSION_ID still pending after $((MAX_WAIT_SECONDS / 60)) minutes."
        echo "::error::Apple's notary backend is in deep queue. Recover later with:"
        echo "::error::  xcrun notarytool info $SUBMISSION_ID --apple-id <APPLE_ID> --team-id $APPLE_TEAM_ID --password <APP_SPECIFIC_PASSWORD>"
        echo "::error::If the submission later reaches Accepted, staple manually with:"
        echo "::error::  xcrun stapler staple $APP"
        exit 1
    fi

    sleep "$POLL_INTERVAL"
done

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
