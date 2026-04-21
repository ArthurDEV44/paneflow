# PaneFlow release runbook

Step-by-step checklist for cutting a new PaneFlow release. Written for
the maintainer coming back cold after a month away — every step has a
time budget, a clear pass/fail signal, and a "what to do if it
breaks" box. Referenced by US-021.

**Target total time: ≤ 15 minutes for a happy-path release.** If any
step pushes you past that, check the step's troubleshooting box before
plowing on — the runbook has probably already anticipated the failure.

**Last validated on:** _pending — first end-to-end dry run at v0.2.0._
(Update this line every release so a returning maintainer knows the
runbook has actually been exercised recently. See the "Dry-run
validation" section at the bottom for the contract.)

Related runbooks:

- [`docs/pkg-repo-runbook.md`](./pkg-repo-runbook.md) — one-time R2 +
  repo bootstrap, GPG key creation.
- [`docs/release-signing.md`](./release-signing.md) — deep-dive on the
  signing pipeline, key rotation.
- [`docs/validation-aarch64.md`](./validation-aarch64.md) — on-device
  aarch64 validation (execute for releases that aarch64 users depend
  on).

Prerequisites (one-time, not part of the per-release cadence):

- `gh` CLI authenticated with `repo` scope (`gh auth status`).
- GitHub secrets populated: `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE`,
  `GPG_KEY_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`,
  `R2_ENDPOINT`, `R2_BUCKET`. See `docs/pkg-repo-runbook.md` §4.
- Docker installed locally (used in Step 6 smoke tests).

---

## Supported release targets (US-008)

Authoritative status of every platform target PaneFlow releases can
ship to. Cross-reference `.github/workflows/release.yml`'s matrix.

| Target | Status | Ships | Gate level | Restore requires |
|---|---|---|---|---|
| `x86_64-unknown-linux-gnu` | **Active** | .deb, .rpm, AppImage, .tar.gz | Hard-required (gates the whole release) | — |
| `aarch64-unknown-linux-gnu` | **Active** | .deb, .rpm, AppImage, .tar.gz | Hard-required (gates the whole release) | — |
| `aarch64-apple-darwin` | **Best-effort** | (none today — leg fails at codesign) | `continue-on-error: true` (in matrix, does not block release) | Apple Dev secrets provisioning (see `memory/project_macos_signing.md`) → flip `continue-on-error: false` |
| `x86_64-apple-darwin` | **Closed — pending v0.3.0** | — | Removed from matrix entirely | (a) Apple Dev secrets + (b) macos-13 queue-SLA improvement OR matrix `needs:` refactor so `Publish` doesn't block on best-effort legs |
| `x86_64-pc-windows-msvc` | **Best-effort** | .msi (unsigned until Azure Trusted Signing lands) | `continue-on-error: true` | Azure Trusted Signing secrets (see `memory/project_windows_signing.md`) → flip `continue-on-error: false` |
| `aarch64-pc-windows-msvc` | **Closed — pending v0.3.0** | — | Not in matrix | Scope decision at v0.3.0 cut (Windows on ARM — real hardware is rare; evaluate demand before committing runner hours) |

**Interpretation:**
- *Hard-required*: a failure blocks the release. The `Publish GitHub
  Release` job's `needs:` waits for a green result.
- *Best-effort*: in the matrix with `continue-on-error: true`.
  Catches Rust-level cross-compile regressions at PR time but a
  failure does NOT prevent the release from being published.
- *Closed — pending vX.Y.Z*: deliberately not in the matrix. Restoring
  the entry is tracked against the listed version's release PRD. Do
  not silently re-add a closed target — adding back requires the
  listed prerequisites AND a status update to this table.

**v0.3.0 commitment:** both macOS legs (`aarch64-apple-darwin`
restored to hard-required, `x86_64-apple-darwin` re-added to matrix)
land together in the first signed macOS release cut, alongside Apple
Dev secrets provisioning. Tracked in `tasks/prd-macos-port.md`.

---

## Step 1 — Pre-flight (≈ 3 min, manual judgement required)

Work on `main`. All changes for this release must already be merged.

```bash
git switch main
git pull --ff-only
git status                       # working tree clean? if not, stop.
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

**Manual judgement:** read the test output, not just the exit code.
`cargo test` exits 0 even when a test is marked `#[ignore]` and quietly
skipped — scan the summary for unexpected ignores, new warnings, or
flaky tests that passed this run but failed the previous one.

Bump versions. Single source of truth for the Rust version is the
workspace `Cargo.toml`:

```bash
# Run this block from the repo root (cd to /home/arthur/dev/paneflow
# or wherever you cloned it). The `sed` and the changelog write use
# relative paths and silently misbehave if CWD is a subdirectory.
cd "$(git rev-parse --show-toplevel)"

# Set the new version ONCE, then reuse via the shell var. Pasting the
# block verbatim without setting VERSION will fail loudly — the guard
# below enforces a valid semver.
VERSION="X.Y.Z"   # <-- EDIT THIS before running. No `v` prefix.
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || { echo "invalid VERSION='$VERSION' (expected N.N.N)"; return 1 2>/dev/null || exit 1; }

# Only the workspace root Cargo.toml carries a literal `version = "..."`;
# src-app/Cargo.toml and crates/*/Cargo.toml use `version.workspace = true`
# and will inherit automatically. This sed targets workspace root only.
sed -i "s/^version = \".*\"$/version = \"$VERSION\"/" Cargo.toml

# Debian changelog needs a new stanza — format matters (dpkg-parsechangelog is strict).
# Force C locale on `date` so the trailer is RFC-2822 English even on
# a host with a non-English LANG setting (dpkg-parsechangelog rejects
# localized date strings).
cat > /tmp/paneflow-changelog-entry.txt <<EOF
paneflow ($VERSION-1) stable; urgency=medium

  * Release v$VERSION. See GitHub release notes for the full diff.

 -- Arthur Jean <arthur.jean@strivex.fr>  $(LC_TIME=C date -R)

EOF
cat /tmp/paneflow-changelog-entry.txt debian/changelog > debian/changelog.new
mv debian/changelog.new debian/changelog

# Let cargo rewrite Cargo.lock for the new version
cargo build -p paneflow-app --release 2>/dev/null || true
cargo check --workspace

# Commit the bump
git add Cargo.toml Cargo.lock debian/changelog
git commit -m "chore: bump version to v$VERSION"
git push origin main
```

Pass signal: the commit lands on `main` via a green push (pre-commit
hooks or required checks don't block).

### Troubleshooting — Step 1

| Symptom | Top 3 recoveries |
|---|---|
| `cargo test` fails on a flaky test | 1. Re-run the specific test with `cargo test <name> -- --nocapture`. 2. If genuinely flaky, file an issue and mark `#[ignore]` in a separate commit BEFORE tagging — don't tag a known-broken release. 3. If the failure is real, fix it and restart Step 1. |
| Working tree not clean (leftover unstaged changes) | 1. `git stash` to park the noise. 2. `git diff` to audit each change — uncommitted work from a different branch should be committed or stashed, never force-discarded. 3. Only after `git status` is clean do you proceed. |
| `dpkg-parsechangelog: error: no version` when validating `debian/changelog` | 1. The changelog stanza format is strict — `name (VERSION-REVISION) DISTRIBUTION; urgency=LEVEL`. Copy the previous stanza and edit only the version/date. 2. The `-- <name> <email>  <date>` trailer needs **two** spaces before the date. 3. Validate locally with `dpkg-parsechangelog -l debian/changelog` before pushing. |

---

## Step 2 — Tag and push (≈ 1 min)

```bash
# Tag the bump commit with an annotated tag
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

**Pre-release convention:** if you need on-device aarch64 validation
before promoting to `latest`, tag as `vX.Y.Z-rc.1` first. The
`release.yml` workflow matches `rc`/`beta`/`alpha` in the tag name and
publishes as `prerelease: true`. After validation (the Pre-announce section or
`docs/validation-aarch64.md`), retag as `vX.Y.Z` to cut the final
release.

### Troubleshooting — Step 2

| Symptom | Top 3 recoveries |
|---|---|
| Tag already exists on the remote | 1. Do NOT `git push --force` — that overwrites an immutable release marker and breaks anyone's `git fetch`. 2. Decide if the existing tag should be consumed as-is (rare) or if you need a new version number (usual). 3. If a broken tag was pushed and the release workflow produced bad artifacts, bump to the next patch and re-tag — the dud release stays on GitHub as a historical record. |
| Push rejected because `main` moved after you committed | 1. `git pull --rebase origin main`, re-inspect `git log` to confirm your bump commit is still the tip. 2. Re-run `cargo test` locally — a racing merge may have introduced conflicts that Git resolved cleanly but the test suite doesn't. 3. Only then `git push` and re-`git push origin vX.Y.Z`. |
| Pushed the tag but forgot to commit the version bump | 1. **IRREVERSIBLE for anyone who already `git fetch`-ed** — even a few seconds of exposure means downstream clones may carry the bad tag indefinitely. `git tag -d vX.Y.Z` locally and `git push --delete origin vX.Y.Z` to remove the remote tag. 2. If the release workflow already created a GitHub Release object for the tag, delete that separately: `gh release delete vX.Y.Z --cleanup-tag --yes`. 3. Make the version-bump commit, re-tag on the new commit, push. Announce in release notes that the original tag was retracted so downstream forks update. |

---

## Step 3 — Monitor release.yml (≈ 15 min, manual judgement required)

```bash
# `gh run list --branch=` does NOT match tag refs — for tag-triggered
# workflows we filter by event=push and take the most recent run.
# The tag name is informational only (to sanity-check that the run
# you're watching is actually the one you just pushed).
RUN_ID=$(gh run list --workflow=release.yml --event=push --limit=1 \
          --json databaseId,headBranch --jq '.[0].databaseId')
gh run watch --exit-status "$RUN_ID"
```

Budget from US-019 AC4: **total matrix wall-clock < 25 min** for both
arches combined (`fail-fast: true` means both legs must finish for the
release job to run). First aarch64 run after an `ubuntu-22.04-arm`
runner cold start may burn 2–3 extra minutes on queueing — acceptable.

**Manual judgement:** if the workflow is green but a step emitted a
`::warning::` annotation (`lintian`, `dpkg-sig` verify-tail output, or
GPG fingerprint mismatch warning), stop here and read the annotation
before proceeding — a warning on the signing leg is often a silent
"unsigned release would have shipped if we hadn't tripped the
fingerprint guard" near-miss.

### Troubleshooting — Step 3

| Symptom | Top 3 recoveries |
|---|---|
| aarch64 leg queues for > 5 min on `ubuntu-22.04-arm` | 1. Wait — GitHub's free-tier ARM queue is bursty and usually clears within 10 min. 2. If genuinely stuck > 20 min, cancel via `gh run cancel <id>` and re-run the workflow from the Actions UI. 3. Persistent queueing may mean the runner label is wrong post-GitHub-maintenance; see the Runner Availability notes in [`tasks/spike-aarch64-build.md`](../tasks/spike-aarch64-build.md) §Residual-unknowns. |
| GPG signing step exits with `GPG signing failed: key not loaded` | 1. A required secret (`GPG_PRIVATE_KEY`, `GPG_KEY_ID`, `GPG_PASSPHRASE`) is missing or malformed. Check the run log for the exact secret referenced. 2. Re-populate the secret from the password manager (see `docs/pkg-repo-runbook.md` §4). 3. Re-run the failed job — don't re-tag. |
| `fail-fast: true` cancelled the x86_64 leg after aarch64 failed | 1. Click through to the aarch64 log and find the real failure — `fail-fast` makes the reported failure cascade-prone. 2. If aarch64 is the blocker and you need an x86_64-only release, patch the matrix in a hotfix commit to skip aarch64, re-tag to a patch number, and note the aarch64 gap in release notes. 3. If aarch64 is a first-time failure since a GPUI bump, see [`tasks/spike-aarch64-build.md`](../tasks/spike-aarch64-build.md) — may indicate an upstream GPUI regression. |

---

## Step 4 — Verify artifacts attached (≈ 2 min)

```bash
gh release view vX.Y.Z --json assets --jq '.assets[].name' | sort
```

Expected asset count: **12** (two arches × six files each):

```
paneflow-vX.Y.Z-aarch64.AppImage
paneflow-vX.Y.Z-aarch64.AppImage.zsync
paneflow-vX.Y.Z-aarch64.deb
paneflow-vX.Y.Z-aarch64.rpm
paneflow-vX.Y.Z-aarch64.tar.gz
paneflow-vX.Y.Z-aarch64.tar.gz.sha256
paneflow-vX.Y.Z-x86_64.AppImage
paneflow-vX.Y.Z-x86_64.AppImage.zsync
paneflow-vX.Y.Z-x86_64.deb
paneflow-vX.Y.Z-x86_64.rpm
paneflow-vX.Y.Z-x86_64.tar.gz
paneflow-vX.Y.Z-x86_64.tar.gz.sha256
```

A missing or renamed asset breaks the in-app updater's asset matcher
(it looks up by `-<arch>.<format>` suffix, see
`src-app/src/update_checker.rs`).

### Troubleshooting — Step 4

| Symptom | Top 3 recoveries |
|---|---|
| Asset count is 6 (one arch missing) | 1. The `fail-fast: true` matrix should have prevented this. Check whether the `release` job in `release.yml` ran — if one matrix leg silently skipped artifact upload, that's a release.yml bug. 2. Re-run the missing leg via `gh run rerun --failed`, which re-uploads without re-tagging. 3. If neither arch's asset set is complete, delete the broken release (`gh release delete vX.Y.Z --cleanup-tag`) and restart at Step 1 with a patch bump. |
| Asset names have the wrong arch suffix (e.g., `_amd64.deb` instead of `-x86_64.deb`) | 1. The staging step in `release.yml` renames to the canonical form; a missing rename is a workflow regression. 2. Do NOT publish — the updater will not find assets and user upgrades break. 3. Patch the staging step, cut a new patch tag. |
| Pre-release ended up on `latest` | 1. The tag contains `rc`/`beta`/`alpha` but the workflow's prerelease boolean is false — double-check the `contains(...)` expression in `release.yml`. 2. Manually flip the release to pre-release: `gh release edit vX.Y.Z --prerelease`. 3. Fix the workflow expression in a follow-up commit. |

---

## Step 5 — Verify repo-publish.yml (≈ 10 min, auto-chained from release.yml)

`repo-publish.yml` auto-chains off `release.yml` completion via GitHub
Actions' `workflow_run` trigger (US-003). A successful tag-push
`release.yml` run fires `repo-publish.yml` within ~30 s of the parent
job finishing — no manual `gh workflow run` is needed on the happy path.
Prerelease tags (`-rc.N`, `-alpha.N`, `-beta.N`) are filtered out on the
auto-chain path, so they do NOT push to the stable repo (intentional:
stable is stable-only).

```bash
gh run watch --exit-status $(gh run list --workflow=repo-publish.yml --limit=1 --json databaseId --jq '.[0].databaseId')
```

Then verify the repo metadata is fresh:

```bash
# APT — InRelease file must include today's date
curl --fail --silent https://pkg.paneflow.dev/apt/dists/stable/InRelease \
  | grep -E '^Date:'
# RPM — repomd.xml should list a paneflow-<version>.rpm whose version
# matches what you just tagged. We use a semver regex so the grep
# works even if the maintainer forgot to substitute X.Y.Z.
curl --fail --silent https://pkg.paneflow.dev/rpm/repodata/repomd.xml \
  | grep -oE 'paneflow-v[0-9]+\.[0-9]+\.[0-9]+[^"]*' | head -1
```

### Troubleshooting — Step 5

| Symptom | Top 3 recoveries |
|---|---|
| `repo-publish.yml` didn't fire | 1. Check whether the tag is a pre-release (`-rc.N`/`-alpha.N`/`-beta.N`) — the auto-chain filters those out by design; manually re-publish via `gh workflow run repo-publish.yml -f tag=vX.Y.Z` if you really want a prerelease on the stable repo. 2. Check the `release.yml` parent run succeeded — the `workflow_run` guard requires `conclusion == 'success'` AND `event == 'push'` (a `gh run rerun` of release.yml will NOT re-fire repo-publish). 3. If release.yml succeeded on a final tag but repo-publish still didn't fire, confirm `release.yml`'s `name:` field is still `Release` (verbatim match required by `workflow_run`), then fall back to `gh workflow run repo-publish.yml -f tag=vX.Y.Z`. |
| `rclone sync` step fails with 403 | 1. R2 credentials rotated. Refresh `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` from Cloudflare (see `docs/pkg-repo-runbook.md` §2) and update the GitHub secrets. 2. Re-run the workflow. 3. If the bucket itself was deleted or renamed, restore from the rclone `--dry-run` diff before re-syncing — never blast-write an empty local staging dir to a bucket that still has user-visible content. |
| `InRelease` returns stale date | 1. Cloudflare edge cache — invalidate via the dashboard (or wait 60s for the default TTL). 2. Check whether `reprepro` actually ran by inspecting the workflow log for the "signing Release file" step. 3. If reprepro ran but the InRelease has wrong dists/version, the repo DB on R2 may be corrupted; rebuild from scratch per `docs/pkg-repo-runbook.md` §Bootstrap. |

---

## Step 6 — Smoke-test install in containers (≈ 4 min)

CI already runs `smoke-test-ubuntu` and `smoke-test-fedora` jobs
against the built artifacts. This step exercises the **published**
release from a user's perspective — "does a fresh `apt install` off
`pkg.paneflow.dev` actually work?"

> **Trust-on-first-use on the Docker base images.** The `ubuntu:22.04`
> and `fedora:40` tags below are mutable — `docker run` pulls whatever
> Docker Hub currently serves under those names. If you need
> reproducibility across releases (e.g., when diagnosing a failure that
> only reproduces against a specific base-image build), pin by digest:
> `docker run ubuntu@sha256:<hash> ...`. For routine smoke tests, the
> mutable tag is fine — we're validating our packages, not Ubuntu.

```bash
# Ubuntu 22.04 — apt-repo path
docker run --rm -it ubuntu:22.04 bash -c '
  set -euo pipefail
  apt-get update -qq
  apt-get install -y --no-install-recommends ca-certificates curl gnupg
  curl -fsSL https://pkg.paneflow.dev/gpg \
    | gpg --dearmor -o /usr/share/keyrings/paneflow-archive.gpg
  echo "deb [signed-by=/usr/share/keyrings/paneflow-archive.gpg] https://pkg.paneflow.dev/apt stable main" \
    > /etc/apt/sources.list.d/paneflow.list
  apt-get update
  apt-get install -y paneflow
  paneflow --version
'

# Fedora 40 — dnf-repo path
docker run --rm -it fedora:40 bash -c '
  set -euo pipefail
  dnf install -y dnf-plugins-core
  dnf config-manager --add-repo https://pkg.paneflow.dev/rpm/paneflow.repo
  rpm --import https://pkg.paneflow.dev/gpg
  dnf install -y paneflow
  paneflow --version
'
```

Both commands must exit 0 with the correct version string. A failure
here is the "Cursor regression" scenario — the release is live but
users can't install it.

### Troubleshooting — Step 6

| Symptom | Top 3 recoveries |
|---|---|
| `apt-get install` fails with `Hash Sum mismatch` | 1. Cloudflare edge still serving a stale InRelease or Packages file. Wait 60 s for TTL or purge cache manually via the Cloudflare dashboard. 2. Inside the container, force a no-cache refresh: `apt-get update -o Acquire::Check-Valid-Until=false`. 3. If the hash mismatch persists after 5 min, the reprepro build is inconsistent — re-run Step 5. |
| `dnf install` fails with `Errors during downloading metadata` | 1. `.repo` file points at the wrong base URL — cross-check against `.github/workflows/repo-publish.yml`'s published path. 2. `gpgkey=` URL 404s — confirm `https://pkg.paneflow.dev/gpg` is reachable from outside the container (`curl -I`). 3. RPM repo metadata in R2 lacks a matching signature — fix is the same as InRelease staleness above. |
| `paneflow --version` prints an unexpected version string | 1. The apt/dnf cache returned an older package. Purge and re-install: `apt remove paneflow && apt update && apt install paneflow`. 2. Two concurrent release workflows overwrote each other — check the repo-publish.yml concurrency group and make sure the most recent one actually ran. 3. Release was cut from the wrong branch/commit; abandon and re-cut at a patch bump. |

---

## Pre-announce — aarch64 on-device validation (conditional)

Only execute for releases where aarch64 users are expected (i.e.,
most public releases). Follow
[`docs/validation-aarch64.md`](./validation-aarch64.md) end-to-end on
at least one aarch64 device (RPi 5 or Asahi Linux). Attach evidence
(asciinema cast + screenshot) to the release notes per that runbook's
§5.

If validation fails, take the escape hatch documented in
`docs/validation-aarch64.md` §6 (remove the aarch64 assets from the
release, or hold as a pre-release) BEFORE announcing in Step 7 — an
announcement that links to broken aarch64 bits is the worst
failure mode possible for ARM users.

---

## Step 7 — Announce (≈ 2 min, manual judgement required)

Write the release notes on the GitHub Release page. `release.yml` sets
`generate_release_notes: true`, so GitHub has pre-filled the changelog
from merged PRs since the previous tag — your job is to polish that
default, not write it from scratch.

Suggested structure:

```markdown
## Highlights

- One-sentence summary of the biggest user-visible change.
- Second highlight.

## Install

See the [Installation section in the README](https://github.com/ArthurDEV44/paneflow#install).

## Validation

- x86_64 smoke tests: ✅ (CI: link to workflow run)
- aarch64 on-device: ✅ <distro / date / maintainer> OR ⏳ pending
  (see pre-release policy in `docs/validation-aarch64.md`)

## Full changelog

{the auto-generated list GitHub prepended}
```

**Manual judgement:** read the auto-generated changelog end-to-end.
Re-order items so user-visible changes come first (refactors and
chores go last), and drop anything that's pure-noise (dependabot bumps
that users don't need to know about).

Post-announce:

- Share the release link in whatever channels matter (GitHub
  Discussions, Discord, Mastodon, Reddit). Keep the post short — "new
  release, highlights, install link" — not a wall of text.
- Update the memory note
  `~/.claude/projects/-home-arthur-dev-paneflow/memory/research_linux_packaging.md`
  if the release changed the current-release-state lines (version,
  tag list).

### Troubleshooting — Step 7

| Symptom | Top 3 recoveries |
|---|---|
| Auto-generated release notes are empty | 1. No PRs merged since the previous tag — the notes generator has nothing to list. Write a manual entry explaining the release (direct-commit patch release, dependency-only bump, etc.). 2. The previous tag used a different naming scheme (e.g., missing `v` prefix) and the generator didn't find it — manually supply `--generate-notes --notes-start-tag=<previous-tag>` via `gh release edit`. 3. GitHub outage on the notes generator — retry in 10 min or fall back to `git log --oneline <prev-tag>..vX.Y.Z`. |
| Announced, then discovered a critical bug | 1. Flip the release to pre-release: `gh release edit vX.Y.Z --prerelease`. Users on `latest` fall back to the previous stable. 2. Pin a known-issue note at the top of the release notes and link a tracking issue. 3. Cut a patch release as quickly as feasible — the repo-publish workflow updates the apt/dnf streams automatically, so `apt upgrade` fixes most users without manual intervention. |
| Forgot to promote an `-rc.N` tag to the final release | 1. Run through Steps 0–5 with the non-rc tag — the workflow will produce a fresh set of artifacts. 2. Don't delete the `-rc.N` release; it stays as a historical pre-release record. 3. Update the "latest" link consumers by making sure the new final release is marked as `latest` (`gh release edit vX.Y.Z --latest`). |

---

## Dry-run validation (AC6 — once per runbook revision)

This runbook is considered validated when a maintainer has executed
it end-to-end at least once for a real release AND the three Phase 1
formats (plus the Phase 2 `.rpm`) installed cleanly in fresh
containers during Step 6. The first execution should treat any
friction as a bug in the runbook, not in the maintainer — open a PR
to fix the step that went wrong.

Track dry-run validation at the top of the runbook ("Last validated
on: vX.Y.Z, YYYY-MM-DD") — keep that line updated every release so a
maintainer returning after a long break knows the runbook still
works.

Last validated on: _pending — first dry run at v0.2.0_.
