#!/usr/bin/env bash
# US-005: end-to-end auto-update test harness.
#
# Simulates a real user upgrading from `OLD_VERSION` (default 0.2.10) to
# `NEW_VERSION` (default = workspace package version) by:
#
#   1. Building the OLD paneflow at its tag in a git worktree
#   2. Building the NEW paneflow from the current tree
#   3. Bundling NEW into a tar.gz with cargo-bundle layout
#   4. Generating a fixture `latest.json` matching the GitHub Releases API
#   5. Serving the fixture + tarball from localhost via `python3 -m http.server`
#   6. Running OLD paneflow with `--update-and-exit` and
#      `PANEFLOW_UPDATE_FEED_URL` pointing at localhost
#   7. Asserting the binary at the install path now reports NEW_VERSION
#
# Three scenarios are exercised:
#   (a) tar.gz happy path  — exit 0, version bumps
#   (b) hash mismatch      — exit 4 (UpdateError::IntegrityMismatch)
#   (c) feed unreachable   — exit 3 (HTTP server killed before invocation)
#
# AC3a (AppImage swap) is deferred: appimageupdatetool isn't part of the
# default CI image, has no in-process SHA verify, and would test the same
# atomic-swap regression surface as tar.gz with extra ceremony. The
# `--update-and-exit` Rust handler keeps the wiring in place for a
# follow-up that opts in by installing the tool.
#
# Exit code: 0 = all scenarios pass, 1 = any scenario failed.

set -euo pipefail

# -----------------------------------------------------------------------------
# Configuration — env-overridable so the same script works in CI and locally.
# -----------------------------------------------------------------------------
OLD_VERSION="${OLD_VERSION:-0.2.10}"
OLD_TAG="${OLD_TAG:-v${OLD_VERSION}}"
WORK_DIR="${WORK_DIR:-/tmp/paneflow-e2e}"
HTTP_PORT="${HTTP_PORT:-0}"      # 0 = pick an ephemeral port
SCENARIO="${SCENARIO:-all}"      # all|happy|hash_mismatch|feed_unreachable

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
NEW_VERSION="$(awk -F'"' '/^version = / { print $2; exit }' "$REPO_ROOT/Cargo.toml")"

# -----------------------------------------------------------------------------
# Logging helpers — keep CI logs greppable.
# -----------------------------------------------------------------------------
log()  { printf '[e2e] %s\n' "$*" >&2; }
fail() { printf '[e2e] FAIL: %s\n' "$*" >&2; exit 1; }
ok()   { printf '[e2e] PASS: %s\n' "$*" >&2; }

# -----------------------------------------------------------------------------
# Cleanup — always run, even on early exit.
# -----------------------------------------------------------------------------
HTTP_PID=""
HTTP_LOG=""
WORKTREE_PATH=""
cleanup() {
    local rc=$?
    if [ -n "${HTTP_PID}" ] && kill -0 "${HTTP_PID}" 2>/dev/null; then
        kill "${HTTP_PID}" 2>/dev/null || true
        wait "${HTTP_PID}" 2>/dev/null || true
    fi
    if [ -n "${WORKTREE_PATH}" ] && [ -d "${WORKTREE_PATH}" ]; then
        git -C "${REPO_ROOT}" worktree remove --force "${WORKTREE_PATH}" 2>/dev/null || true
    fi
    if [ "${rc}" -ne 0 ] && [ -n "${HTTP_LOG}" ] && [ -f "${HTTP_LOG}" ]; then
        log "http.server log:"
        sed 's/^/[http] /' "${HTTP_LOG}" >&2 || true
    fi
    return "${rc}"
}
trap cleanup EXIT INT TERM

# -----------------------------------------------------------------------------
# Phase 0 — workspace prep.
# -----------------------------------------------------------------------------
log "OLD_VERSION=${OLD_VERSION}  NEW_VERSION=${NEW_VERSION}  WORK_DIR=${WORK_DIR}"
rm -rf "${WORK_DIR}"
mkdir -p "${WORK_DIR}"/{home,fixture,install-bin}

# Pin a fake $HOME so install_method::detect() classifies the OLD binary
# as `TarGz { app_dir: $HOME/.local/paneflow.app }`. Anything else and
# `--update-and-exit` exits 5.
export HOME="${WORK_DIR}/home"
mkdir -p "${HOME}/.local"

# -----------------------------------------------------------------------------
# Phase 1 — build NEW paneflow + bundle into tar.gz fixture.
# -----------------------------------------------------------------------------
log "phase 1: building NEW paneflow at v${NEW_VERSION}"
( cd "${REPO_ROOT}" && cargo build --release -p paneflow-app --quiet )
log "phase 1: bundling tar.gz with bundle-tarball.sh"
( cd "${REPO_ROOT}" && ARCH=x86_64 bash scripts/bundle-tarball.sh "${NEW_VERSION}" >/dev/null )
NEW_TARBALL="${REPO_ROOT}/target/bundle/paneflow-${NEW_VERSION}-x86_64.tar.gz"
[ -s "${NEW_TARBALL}" ] || fail "expected tarball not produced: ${NEW_TARBALL}"

# Copy into fixture dir and emit the .sha256 sidecar `download_with_verification`
# fetches before downloading the tarball body.
cp "${NEW_TARBALL}" "${WORK_DIR}/fixture/"
( cd "${WORK_DIR}/fixture" && sha256sum "paneflow-${NEW_VERSION}-x86_64.tar.gz" \
      > "paneflow-${NEW_VERSION}-x86_64.tar.gz.sha256" )

# -----------------------------------------------------------------------------
# Phase 2 — build OLD paneflow in a git worktree.
# -----------------------------------------------------------------------------
WORKTREE_PATH="${WORK_DIR}/old-src"
log "phase 2: checking out ${OLD_TAG} into ${WORKTREE_PATH}"
git -C "${REPO_ROOT}" worktree add --detach "${WORKTREE_PATH}" "${OLD_TAG}"
# `rust-toolchain.toml` was introduced in commit 1884237 (post-v0.2.11),
# so the OLD worktree at v0.2.10 / v0.2.11 has no toolchain pin. In CI
# the dtolnay/rust-toolchain action installs 1.95 but does NOT set a
# rustup default — running plain `cargo` in a directory without a
# toolchain file fails with "rustup could not choose a version of cargo
# to run, because one wasn't specified explicitly".
#
# Read the channel from main's toolchain file and pass it as
# RUSTUP_TOOLCHAIN to the OLD build. This is more idiomatic than
# copying the file (rust-lang.github.io/rustup/overrides.html lists
# RUSTUP_TOOLCHAIN env above directory-file overrides), avoids
# polluting the OLD worktree's git state, and surfaces the chosen
# toolchain in CI logs. Future-proof: when main bumps the pin, the
# e2e auto-follows without a script edit.
OLD_BUILD_TOOLCHAIN=""
if [ -f "${REPO_ROOT}/rust-toolchain.toml" ]; then
    OLD_BUILD_TOOLCHAIN="$(awk -F'"' '/^channel/ { print $2; exit }' "${REPO_ROOT}/rust-toolchain.toml")"
fi
log "phase 2: building OLD paneflow at v${OLD_VERSION} (toolchain=${OLD_BUILD_TOOLCHAIN:-system default}, slow step)"
(
    cd "${WORKTREE_PATH}"
    if [ -n "${OLD_BUILD_TOOLCHAIN}" ]; then
        RUSTUP_TOOLCHAIN="${OLD_BUILD_TOOLCHAIN}" cargo build --release -p paneflow-app --quiet
    else
        cargo build --release -p paneflow-app --quiet
    fi
)
OLD_BIN_SRC="${WORKTREE_PATH}/target/release/paneflow"
[ -x "${OLD_BIN_SRC}" ] || fail "OLD binary not built at ${OLD_BIN_SRC}"

# Stage OLD into the canonical TarGz install layout
# ($HOME/.local/paneflow.app/bin/paneflow).
INSTALL_DIR="${HOME}/.local/paneflow.app"
mkdir -p "${INSTALL_DIR}/bin"
cp "${OLD_BIN_SRC}" "${INSTALL_DIR}/bin/paneflow"
INSTALL_BIN="${INSTALL_DIR}/bin/paneflow"

# Sanity-check the OLD binary actually reports OLD_VERSION.
actual_old="$("${INSTALL_BIN}" --version)"
[ "${actual_old}" = "paneflow ${OLD_VERSION}" ] \
    || fail "staged OLD binary reported '${actual_old}', expected 'paneflow ${OLD_VERSION}'"
log "phase 2: OLD binary staged at ${INSTALL_BIN}"

# -----------------------------------------------------------------------------
# Phase 3 — start localhost HTTP server.
# -----------------------------------------------------------------------------
HTTP_LOG="${WORK_DIR}/http-server.log"
log "phase 3: starting python3 -m http.server in ${WORK_DIR}/fixture (port=${HTTP_PORT})"
( cd "${WORK_DIR}/fixture" && exec python3 -m http.server "${HTTP_PORT}" --bind 127.0.0.1 ) \
    >"${HTTP_LOG}" 2>&1 &
HTTP_PID=$!

# Wait for the server to bind and discover the actual port (when HTTP_PORT=0).
for _ in $(seq 1 50); do
    if grep -E 'Serving HTTP on 127\.0\.0\.1 port [0-9]+' "${HTTP_LOG}" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done
HTTP_PORT_ACTUAL="$(grep -oE 'port [0-9]+' "${HTTP_LOG}" | head -n1 | awk '{print $2}')"
[ -n "${HTTP_PORT_ACTUAL}" ] || fail "http.server did not announce a port within 5s"
FEED_BASE="http://127.0.0.1:${HTTP_PORT_ACTUAL}"
log "phase 3: server up at ${FEED_BASE}"

# -----------------------------------------------------------------------------
# Phase 4 — write the fixture latest.json (mirrors GitHub Releases API).
# -----------------------------------------------------------------------------
LATEST_JSON="${WORK_DIR}/fixture/latest"
cat > "${LATEST_JSON}" <<EOF
{
  "tag_name": "v${NEW_VERSION}",
  "html_url": "${FEED_BASE}/release-page-stub",
  "assets": [
    {
      "name": "paneflow-${NEW_VERSION}-x86_64.tar.gz",
      "browser_download_url": "${FEED_BASE}/paneflow-${NEW_VERSION}-x86_64.tar.gz"
    }
  ]
}
EOF

# -----------------------------------------------------------------------------
# Helper: reset install dir to OLD between scenarios.
# -----------------------------------------------------------------------------
reset_install() {
    rm -rf "${INSTALL_DIR}" "${HOME}/.cache/paneflow"
    mkdir -p "${INSTALL_DIR}/bin"
    cp "${OLD_BIN_SRC}" "${INSTALL_BIN}"
}

# -----------------------------------------------------------------------------
# Scenario A — tar.gz happy path (AC3b).
# -----------------------------------------------------------------------------
run_happy() {
    log "scenario: tar.gz happy path"
    reset_install

    set +e
    PANEFLOW_UPDATE_FEED_URL="${FEED_BASE}/latest" \
    RUST_LOG=info \
        "${INSTALL_BIN}" --update-and-exit \
        > "${WORK_DIR}/happy.stdout" 2> "${WORK_DIR}/happy.stderr"
    rc=$?
    set -e

    [ "${rc}" -eq 0 ] || {
        log "happy: stderr:"; cat "${WORK_DIR}/happy.stderr" >&2
        fail "happy: --update-and-exit returned ${rc}, expected 0"
    }
    actual_new="$("${INSTALL_BIN}" --version)"
    [ "${actual_new}" = "paneflow ${NEW_VERSION}" ] \
        || fail "happy: post-swap version is '${actual_new}', expected 'paneflow ${NEW_VERSION}'"
    ok "tar.gz happy path: v${OLD_VERSION} → v${NEW_VERSION}"
}

# -----------------------------------------------------------------------------
# Scenario B — hash mismatch (AC3c).
# -----------------------------------------------------------------------------
run_hash_mismatch() {
    log "scenario: hash mismatch"
    reset_install

    # Mutate the .sha256 sidecar to a wrong-but-well-formed hash. The
    # checker downloads the sidecar first; the tarball body is then
    # streamed and hashed with sha2::Sha256, producing
    # UpdateError::IntegrityMismatch (exit 4 in --update-and-exit).
    sha_path="${WORK_DIR}/fixture/paneflow-${NEW_VERSION}-x86_64.tar.gz.sha256"
    sha_path_backup="${sha_path}.real"
    cp "${sha_path}" "${sha_path_backup}"
    # Force the sidecar to a well-formed but wrong 64-hex-zeros hash so
    # `download_with_verification` returns `IntegrityMismatch` after
    # streaming the body — exit 4 in `--update-and-exit`.
    fake_hash="$(printf '0%.0s' {1..64})"
    printf '%s  %s\n' "${fake_hash}" "paneflow-${NEW_VERSION}-x86_64.tar.gz" > "${sha_path}"

    set +e
    PANEFLOW_UPDATE_FEED_URL="${FEED_BASE}/latest" \
    RUST_LOG=info \
        "${INSTALL_BIN}" --update-and-exit \
        > "${WORK_DIR}/mismatch.stdout" 2> "${WORK_DIR}/mismatch.stderr"
    rc=$?
    set -e

    # Restore so subsequent scenarios re-use the real sidecar.
    mv "${sha_path_backup}" "${sha_path}"

    [ "${rc}" -eq 4 ] || {
        log "mismatch: stderr:"; cat "${WORK_DIR}/mismatch.stderr" >&2
        fail "mismatch: --update-and-exit returned ${rc}, expected 4"
    }
    actual_unchanged="$("${INSTALL_BIN}" --version)"
    [ "${actual_unchanged}" = "paneflow ${OLD_VERSION}" ] \
        || fail "mismatch: post-fail version is '${actual_unchanged}', expected unchanged 'paneflow ${OLD_VERSION}'"
    ok "hash mismatch: rejected, install path unchanged"
}

# -----------------------------------------------------------------------------
# Scenario C — feed unreachable (AC6).
# -----------------------------------------------------------------------------
run_feed_unreachable() {
    log "scenario: feed unreachable"
    reset_install

    # Kill the HTTP server and pick a port no one is listening on.
    kill "${HTTP_PID}" 2>/dev/null || true
    wait "${HTTP_PID}" 2>/dev/null || true
    HTTP_PID=""

    set +e
    PANEFLOW_UPDATE_FEED_URL="http://127.0.0.1:1/latest" \
    RUST_LOG=info \
        "${INSTALL_BIN}" --update-and-exit \
        > "${WORK_DIR}/unreach.stdout" 2> "${WORK_DIR}/unreach.stderr"
    rc=$?
    set -e

    [ "${rc}" -eq 3 ] || {
        log "unreach: stderr:"; cat "${WORK_DIR}/unreach.stderr" >&2
        fail "unreach: --update-and-exit returned ${rc}, expected 3 (feed unreachable)"
    }
    grep -F "feed unreachable" "${WORK_DIR}/unreach.stderr" >/dev/null \
        || fail "unreach: stderr missing explicit 'feed unreachable' substring (AC6)"
    ok "feed unreachable: explicit error surfaced"
}

# -----------------------------------------------------------------------------
# Driver.
# -----------------------------------------------------------------------------
case "${SCENARIO}" in
    all|happy)             run_happy ;;
esac
case "${SCENARIO}" in
    all|hash_mismatch)     run_hash_mismatch ;;
esac
case "${SCENARIO}" in
    all|feed_unreachable)  run_feed_unreachable ;;
esac

log "all scenarios passed"
