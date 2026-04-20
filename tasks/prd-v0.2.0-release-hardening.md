[PRD]
# PRD: PaneFlow v0.2.0 Release Hardening

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-20 | Arthur Jean | Initial draft — 3-tier release hardening plan surfaced by the 2026-04-20 swarm audit (5 parallel agent-explore runs) |

## Problem Statement

The 2026-04-20 5-agent swarm audit of PaneFlow produced a **GO** verdict for shipping v0.2.0 (Linux packaging migration), but surfaced one pre-tag correctness bug and 13 post-release / future-platform hardening items that must be tracked explicitly rather than absorbed silently.

1. **Auto-updater hangs indefinitely on flaky networks.** `ureq::get(...).call()` in `src-app/src/update/checker.rs:181`, `targz.rs:127`, and `appimage.rs:120` uses ureq 3's default no-timeout configuration. A user on intermittent connectivity will see the update-checker thread stall without timing out, producing a zombie background thread and a stuck "Checking…" UI pill. This will ship to every v0.2.0 user if not fixed before `git push origin v0.2.0`.
2. **Release artifacts lack integrity sidecars.** Only `.tar.gz` has a `.sha256` sibling on the GitHub Release (`release.yml:750-752`). The `.deb`, `.rpm`, and `.AppImage` artifacts ship raw, preventing out-of-band verification by downstream mirrors or security scanners.
3. **Repo metadata served stale from the Cloudflare edge.** `repo-publish.yml` uploads to R2 but does not invalidate the CDN cache. Clients hitting an edge that cached the old `InRelease` / `Packages.gz` / `repomd.xml` see hash mismatches on new packages.
4. **CI runner drift.** `ci.yml:45` uses `ubuntu-latest` while `release.yml` pins `ubuntu-22.04`. When `ubuntu-latest` rolls to 24.04, a regression could pass CI but fail release builds silently.
5. **Third-party tool download is TOFU-HTTPS only.** `appimageupdatetool` is fetched from the AppImage project's rolling `continuous` channel with no SHA256 verification (`src-app/src/update/appimage.rs:68-71`). A compromised CDN or channel regression would be silently trusted.
6. **Pre-release dependency on the production path.** `sha2 = "0.11"` (`src-app/Cargo.toml:57`) is a pre-release API line; Cargo resolves both 0.10.9 and 0.11.0 in the lockfile. Two copies of sha2 ship in every binary.
7. **Stale PRD review notes create false audit signals.** Two stories in the predecessor PRD (`prd-linux-packaging-migration-status.json`) carry review notes describing gaps that have since been fixed in code. A fresh re-audit produces false blockers.
8. **Legacy `.run` installer code path unreachable but not compile-gated.** `update/mod.rs:82` `download_installer` and `run_installer` have no `#[cfg(unix)]` guard — correctness depends on runtime dispatch logic rather than the type system.
9. **macOS / Windows install flows are dead code.** `update/macos/dmg.rs:9` and `update/windows/msi.rs:9` both `bail!("not yet implemented")`, and `self_update_flow.rs` never dispatches to them — the `AppBundle` and `WindowsMsi` install methods fall through to the legacy `.run` path, which is Linux-shaped.
10. **Reveal-in-file-manager silent no-op on macOS and Windows.** `workspace_ops/mod.rs:502` and `sidebar/mod.rs:506` call `xdg-open` unconditionally. The feature appears in the UI but does nothing outside Linux.
11. **Fonts enumeration empty on macOS without fontconfig.** `fonts.rs` calls `fc-list` on both Linux and macOS; macOS ships without fontconfig, so the settings font picker is empty unless Homebrew is installed.
12. **Orphaned AI-hook extraction code.** `terminal/pty_session.rs:749` iterates over `["claude", "codex", "paneflow-hook"]` extracting from `assets/bin/`, but the directory does not exist and no scripts are embedded. Dead code that touches `PATH` on every shell spawn.
13. **Stale architecture documentation.** `CLAUDE.md` describes `assets/bin/{claude,codex,paneflow-hook}` as extant; reality is the directory is empty. Re-audits produce false positives.

**Why now:** Item 1 is a release blocker — it must be fixed before `git push origin v0.2.0` (see `RELEASE_V0.2.0_PLAN.md` Phase B.6). Items 2–8 are Linux-specific and should ship in v0.2.1 before the first real production install base accumulates. Items 9–13 must be captured before the macOS / Windows release plan (`RELEASE_MACOS_PLAN.md`, `tasks/prd-windows-port.md`) kicks off — otherwise they will be rediscovered one-by-one during release prep, each blocking its own mini-audit.

## Overview

This PRD splits the hardening work into three tiers, each shipped in a distinct release train:

**Tier 1 (v0.2.0, 1 story)** — fix the ureq timeout regression before the tag is pushed. A 15-minute patch across three files using ureq 3's `timeout_global` config builder.

**Tier 2 (v0.2.1, 7 stories)** — a post-release patch focused on Linux packaging robustness: signed integrity sidecars on every artifact, CDN cache purge in the publish workflow, CI base-image pinning, third-party tool pinning, dependency cleanup, documentation corrections, and legacy-path compile gating. Ships within one week of v0.2.0.

**Tier 3 (ongoing, 6 stories)** — cross-platform correctness items that are prerequisites to the first macOS and Windows releases. Do not gate on a specific date; close individually as implemented. Paves the way for `RELEASE_MACOS_PLAN.md` and `tasks/prd-windows-port.md` execution.

Key design decisions, all grounded in research and source-verified APIs:

- **ureq v3 timeout (US-001):** use `.config().timeout_global(Some(Duration::from_secs(30))).build()` on the `RequestBuilder` — confirmed from ureq 3.3.0 source at `~/.cargo/registry/src/.../ureq-3.3.0/src/config.rs:663-748` and `src/request.rs:349-351`. Returns `ureq::Error::Timeout(ureq::Timeout)` which is a direct enum arm, no downcasting.
- **appimageupdatetool pinning (US-005):** hardcode the SHA256 of the pinned tag `2.0.0-alpha-1-20251018` from the canonical `AppImageCommunity/AppImageUpdate` repo (the older `AppImage/AppImageUpdate` URL is deprecated; the `continuous` tag is a rolling pre-release unsuitable for production). No GPG signing is offered upstream (open issue #16), so SHA256 is the only available integrity mechanism.
- **Cloudflare cache purge (US-003):** use `nathanvaughn/actions-cloudflare-purge@v4.0.0` (maintained; `jakejarvis/cloudflare-purge-action` was archived 2025-02-01). Purge specific URLs (`InRelease`, `Packages.gz`, `repomd.xml`) rather than `purge_everything` (which is rate-limited to once/second). Instant Purge propagates globally in under 150 ms per Cloudflare's documented operational behavior.
- **macOS / Windows install flow dispatch (US-009, US-010):** implement both stubs with real code now; validate on hardware before the first macOS / Windows release cut. The Linux v0.2.0 release is unaffected either way.

## Goals

| Goal | v0.2.0 (this week) | v0.2.1 (within 1 week of v0.2.0) | First macOS/Windows release |
|------|--------------------|----------------------------------|-----------------------------|
| Auto-updater never hangs > 30 seconds on any HTTP call | 100% of update-path calls bounded | maintained | maintained |
| Artifact integrity sidecars on every GitHub Release asset | — | 6/6 artifacts have `.sha256` | 8/8 (adds dmg + msi) |
| Cloudflare edge serves fresh metadata within 60s of publish | — | < 60s P95 after purge step | maintained |
| CI failure surface matches release failure surface | — | ci.yml + release.yml both on ubuntu-22.04 | maintained |
| macOS / Windows install flows functional end-to-end | — | — | 100% of `AppBundle` + `WindowsMsi` install methods route through real code (no `bail!`) |
| Reveal-in-file-manager works on all 3 OSes | — | — | 3/3 OSes |
| Zero stale review notes in PRD status files | — | 0 stale notes | maintained |

## Target Users

### PaneFlow maintainer (Arthur Jean, solo dev)
- **Role:** Ships PaneFlow releases, owns CI/CD and GPG signing infra, operates the Cloudflare R2 repo hosted at `pkg.paneflow.dev`.
- **Behaviors:** Tags a version, watches `release.yml` + `repo-publish.yml` run, smoke-tests a container on Ubuntu 24.04 and Fedora 40, then announces on Hacker News / r/linux. Works in terminal + browser; reviews PRs on GitHub.
- **Pain points:** Manual re-audit after every release keeps producing false positives (stale review notes, dead code referenced in docs); silent CDN propagation delays force defensive `sleep 60` in runbook; no single place to track deferred work surfaced by an audit.
- **Current workaround:** Keeps a mental list of deferred items; audits surface the same items repeatedly across different sessions.
- **Success looks like:** A fresh audit passes green, the status JSON matches code reality, and every known hardening item is tracked with an owner and a target release.

### PaneFlow end user on Linux (Ubuntu 24.04 / Fedora 40 target)
- **Role:** Downloads PaneFlow via `apt install paneflow` from `pkg.paneflow.dev` or a direct `.deb` / `.AppImage` download from GitHub Releases. Runs it on a laptop with variable connectivity (home WiFi, tethered phone, office VPN).
- **Behaviors:** Installs once, launches often, expects the app to not hang on startup even if the home WiFi just went down. Trusts the OS package manager to verify integrity and re-verifies SHA256 when downloading artifacts manually.
- **Pain points:** v0.1.x `.run` installer was opaque; user expects modern per-distro packaging. An app that hangs on network failure is a 1-star review trigger. No `.sha256` sidecar means manual integrity verification is impossible.
- **Current workaround:** Disables WiFi before launching unreliable apps; runs `dpkg --verify paneflow` on installed `.deb` and skips integrity-check on direct downloads.
- **Success looks like:** Launching PaneFlow on a flaky network produces a visible "update check failed" toast within 30 seconds, then the app works normally. Every artifact download comes with a verifiable `.sha256`.

### PaneFlow end user on macOS / Windows (future)
- **Role:** Will install PaneFlow from a signed `.dmg` (macOS) or `.msi` (Windows) once those ship. Expects auto-update to "just work" with native platform UX (Gatekeeper allow, UAC prompt).
- **Behaviors:** Downloads a DMG from GitHub Releases or installs from `winget` / Homebrew, drags to /Applications or runs MSI installer, launches. Expects "Check for updates" in the app menu to download and apply the update.
- **Pain points:** Current in-app updater would fail opaquely on macOS/Windows if the flow shipped today (Tier 3 items) — silent fall-through to Linux `.run` execution.
- **Current workaround:** No macOS / Windows build currently ships.
- **Success looks like:** In-app "Check for updates" downloads the new DMG/MSI, verifies its integrity, prompts the user via the platform-native UAC / Gatekeeper flow, and installs.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **Firefox / Thunderbird:** use Mozilla's MAR updater with per-channel GPG + SHA256 manifest. PaneFlow's approach is simpler (per-artifact SHA256 sidecar + GPG on repo metadata only) but covers the same integrity surface.
- **Zed editor:** ships pre-built tarballs with per-artifact hash manifest; uses `self-update` crate for auto-update. PaneFlow diverges by using `ureq` + custom update flow per install method (AppImage via `appimageupdatetool`, tar.gz via atomic swap, `.deb`/`.rpm` via system package manager hint).
- **Market gap:** Linux terminal multiplexers with auto-update are rare (`alacritty`, `kitty`, `wezterm` delegate to distro packaging). PaneFlow shipping a built-in updater puts it ahead, but means every edge case (flaky network, SHA mismatch, rate limit) is ours to handle.

### Best Practices Applied
- **Pin external build tools by tag + hash, not by channel name** — follows Debian `debian-installer` and Fedora `koji` practice. Applied in US-005.
- **Purge specific URLs after R2 sync, don't rely on edge TTL** — pattern used by every production APT/RPM mirror behind a CDN. Applied in US-003.
- **Use structured error variants for timeouts, don't match on error strings** — follows `reqwest` and `hyper` conventions. Applied in US-001.
- **Keep `ubuntu-*` CI images pinned**, never `ubuntu-latest`, when release artifacts must be reproducible — documented by GitHub Actions team in 2024. Applied in US-004.

### Technical Recommendations (source-verified)
- **ureq 3.3.0 API** (from `~/.cargo/registry/src/.../ureq-3.3.0/src/config.rs:663-748`): use `.config().timeout_global(Some(Duration))` on `RequestBuilder`. Error variant is `ureq::Error::Timeout(ureq::Timeout)` — a direct enum arm, not wrapped in `Io`.
- **AppImageUpdate canonical repo**: `AppImageCommunity/AppImageUpdate` (not the deprecated `AppImage/AppImageUpdate`). Latest dated tag: `2.0.0-alpha-1-20251018`. No `SHA256SUMS` bundle file — hashes are per-file on the release page. No GPG signing available upstream.
- **Cloudflare Cache Purge API**: `POST /client/v4/zones/{zone_id}/purge_cache` with `{"files": [...]}` body. Minimum scope: `Zone → Cache Purge → Purge`. Propagation: < 150 ms globally (Instant Purge, operational behavior).
- **Core Text on macOS**: `font-kit` (Zed's GPUI dep tree already pulls transitively) has a Core Text backend; alternative is the `core-text` crate v20+ directly.

*Full research sources: ureq 3.3.0 source files, [Cloudflare Cache Purge API docs](https://developers.cloudflare.com/api/resources/cache/methods/purge/), [Cloudflare Instant Purge blog](https://blog.cloudflare.com/instant-purge/), [AppImageCommunity/AppImageUpdate Releases](https://github.com/AppImageCommunity/AppImageUpdate/releases), [nathanvaughn/actions-cloudflare-purge](https://github.com/NathanVaughn/actions-cloudflare-purge), and the 2026-04-20 internal 5-agent swarm audit summary (top of this conversation).*

## Assumptions & Constraints

### Assumptions (to validate)
- **ureq 3.x semver stability:** the `timeout_global` API remains stable through the 3.x line. Based on `Cargo.lock` pinning `3.3.0` and ureq's semver policy.
- **Cloudflare Instant Purge applies to R2 custom domains:** documented for standard zones; R2 custom domains traverse the same edge layer. No contractual SLA, but operational < 150 ms is reproducible.
- **AppImageUpdate tag stability:** the `2.0.0-alpha-1-20251018` tag is immutable on GitHub (dated tags are by convention not force-pushed). Only the `continuous` tag rolls.
- **Cloudflare API token scope:** maintainer already has a token in the Cloudflare account used in `RELEASE_V0.2.0_PLAN.md` Phase B, and can add `Zone → Cache Purge → Purge` scope to it (or mint a new purge-only token).
- **`update/mod.rs` legacy `.run` path is unreachable from non-Linux after Tier 3 `#[cfg(unix)]` guard:** requires a full call-graph trace during implementation of US-008. If a reachable path exists, US-008 is blocked and spawns a follow-up fix story.

### Hard Constraints
- **Cross-platform mandate** (per `CLAUDE.md`): every change compiles on `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`, or has documented per-OS branches.
- **No backwards-compat shims** (per `CLAUDE.md`): no renaming unused `_vars`, no re-exporting types, no `// removed for compat` comments. Delete what's dead.
- **Atomic commits per story; branch naming `feat/...` or `fix/...`** (project convention).
- **Tier 1 must ship in v0.2.0:** `Cargo.toml` + `debian/changelog` + `assets/io.github.arthurdev44.paneflow.metainfo.xml` version bumps happen in `RELEASE_V0.2.0_PLAN.md` Phase B.5, after this PRD's US-001 merges.
- **Tier 2 requires v0.2.1 version bump** in the same 3 files, as a single atomic commit alongside the changes.
- **Tier 3 ships alongside the first macOS/Windows release** — target version TBD (likely v0.3.0).
- **GPUI pin frozen:** GPUI is a git dep pinned to commit `0b984b5`; any change that requires bumping the pin is out of scope for this PRD.

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` — formatting enforced
- `cargo clippy --workspace -- -D warnings` — no new warnings
- `cargo test --workspace` — all tests pass
- `cargo check --target x86_64-pc-windows-msvc --target x86_64-apple-darwin` — cross-compile sanity (EP-003 stories and US-008 specifically)

For release-workflow stories (EP-002: US-002, US-003, US-004), additionally:

- `actionlint .github/workflows/*.yml` — YAML + action reference linting
- Dry-run the workflow by pushing a pre-release tag to a fork before merging to `main`

For the Tier 1 network-timeout fix (US-001), additionally:

- **Smoke test:** Disable WiFi (or `nmcli dev disconnect`), `RUST_LOG=info cargo run`, confirm the update-checker thread emits a `Timeout` error within 30–35 seconds (not indefinite hang). Verify via `log::info!` output from the error classifier.

For cross-platform UI stories (US-011, US-012), additionally:

- **Manual verification per OS:** test the reveal-in-file-manager / font picker on real Linux (Ubuntu 24.04), macOS 14+, Windows 11 hosts or VMs before marking `IN_REVIEW → DONE`.

## Epics & User Stories

### EP-001: v0.2.0 pre-tag hardening

Single release-blocker fix before `git push origin v0.2.0`. Bounds auto-updater network calls so a flaky connection never produces a zombie background thread.

**Definition of Done:** All three `ureq::get(...).call()` sites in `src-app/src/update/` complete within 30 seconds or return `ureq::Error::Timeout`. Smoke test confirmed with network disabled.

#### US-001: Bound auto-updater network calls with a 30-second timeout
**Description:** As a PaneFlow user on a flaky network, I want the auto-update check to fail fast with a clear error instead of hanging my update-checker thread indefinitely, so that the app doesn't show a stuck "Checking…" pill when my home WiFi drops.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given an online host, when the update check runs, then `src-app/src/update/checker.rs:181`, `src-app/src/update/targz.rs:127`, and `src-app/src/update/appimage.rs:120` each invoke `ureq::get(...).config().timeout_global(Some(Duration::from_secs(30))).build().call()` (confirmed from ureq 3.3.0 source)
- [ ] Given network is disabled (e.g., WiFi off, firewall drops DNS/TCP), when the update check runs, then the call returns `Err(ureq::Error::Timeout(_))` within 30–35 seconds
- [ ] Given a timeout occurs, when the UI observes the result, then the `SelfUpdateStatus::Checking` state transitions to `Errored` with a user-visible "Update check timed out" toast (reuses existing error toast path)
- [ ] Given the fix is applied, when `ureq::Error::Timeout` is matched in `update/error.rs` classification, then the error is classified as `UpdateError::Network` variant (not misclassified as `Other`)
- [ ] Given the fix is applied, when `cargo clippy --workspace -- -D warnings` runs, then it passes with no new warnings
- [ ] Given the fix is applied, when `cargo test --workspace` runs, then all existing tests pass (no regression)
- [ ] Unhappy path: if the timeout fires mid-download (partial response received), the partial body is discarded and `UpdateError::Network` is returned — no partial file left on disk, no panic

---

### EP-002: v0.2.1 post-release hardening

Post-v0.2.0 patch release focused on Linux packaging robustness: integrity sidecars, CDN hygiene, CI pinning, third-party tool pinning, dependency cleanup, and compile-time gating of the legacy `.run` path. Ships within one week of v0.2.0.

**Definition of Done:** All 7 stories merged, version bumped to 0.2.1 in `Cargo.toml` + `debian/changelog` + `metainfo.xml`, tag pushed, `curl -I https://pkg.paneflow.dev/apt/dists/stable/InRelease` returns fresh `cf-cache-status: MISS` within 60 seconds of publish, every GitHub Release asset has a `.sha256` sibling, `Cargo.lock` contains only one major version of `sha2`, and both stale review notes in the predecessor PRD are cleared.

#### US-002: Emit SHA256 sidecars for .deb, .rpm, and .AppImage release artifacts
**Description:** As a downstream package mirror operator, I want a `.sha256` file for every artifact on a PaneFlow GitHub Release, so that I can verify the artifact integrity out-of-band before mirroring.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `.github/workflows/release.yml` runs on a tag push, when the artifact staging step runs, then `paneflow-*.deb`, `paneflow-*.rpm`, and `paneflow-*.AppImage` each have a sibling `<file>.sha256` generated via `sha256sum <file> > <file>.sha256` containing the canonical format `<64-char-hex>  <filename>`
- [ ] Given the release is published, when a user downloads both `<file>` and `<file>.sha256` and runs `sha256sum -c <file>.sha256`, then verification succeeds and prints `<filename>: OK`
- [ ] Given the matrix runs for both `x86_64` and `aarch64`, when 6 primary artifacts total are produced (3 formats × 2 arches), then 6 `.sha256` siblings are produced (not 4, not 3)
- [ ] Given `release.yml:750-752` already emits `.sha256` for `.tar.gz`, when the new logic is added, then the existing tar.gz sidecar behavior is not regressed
- [ ] Given `gh release view v0.2.1 --json assets | jq '.assets[].name'` runs post-release, when the output is inspected, then every non-`.sha256` asset has a corresponding `.sha256` asset
- [ ] Unhappy path: if `sha256sum` is missing from the runner image (`ubuntu-22.04` ships it by default, but defensive-coding), the step fails loudly with `command not found` (not silently skipped)

#### US-003: Purge Cloudflare cache after R2 repo sync
**Description:** As an end-user running `apt update` immediately after a PaneFlow release, I want fresh repo metadata (not stale edge cache), so that I don't see hash mismatches on new packages.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `.github/workflows/repo-publish.yml` completes the R2 sync step, when the next step runs, then `nathanvaughn/actions-cloudflare-purge@v4.0.0` is invoked with a `files:` list containing at minimum: `https://pkg.paneflow.dev/apt/dists/stable/InRelease`, `https://pkg.paneflow.dev/apt/dists/stable/Release`, `https://pkg.paneflow.dev/apt/dists/stable/Release.gpg`, `https://pkg.paneflow.dev/apt/dists/stable/main/binary-amd64/Packages.gz`, `https://pkg.paneflow.dev/apt/dists/stable/main/binary-arm64/Packages.gz`, `https://pkg.paneflow.dev/rpm/repodata/repomd.xml`, `https://pkg.paneflow.dev/rpm/repodata/repomd.xml.asc`
- [ ] Given the purge step runs, when `curl -I https://pkg.paneflow.dev/apt/dists/stable/InRelease` is sent within 60 seconds of the step completing, then the response header includes `cf-cache-status: MISS` or `DYNAMIC` (not `HIT`)
- [ ] Given the API token has minimum scope `Zone → Cache Purge → Purge` (no broader permissions), when the step runs, then no 403 authorization errors occur
- [ ] Given the `CLOUDFLARE_ZONE` and `CLOUDFLARE_AUTH_KEY` secrets are not configured, when the workflow runs, then the purge step fails loudly with an actionable error (not silently skipped — catches misconfiguration early before real users see stale metadata)
- [ ] Given Cloudflare returns a 5xx or rate-limit error, when the step runs, then it retries up to 3 times with exponential backoff (1s, 2s, 4s) before failing
- [ ] Given the step is added, when `actionlint .github/workflows/repo-publish.yml` runs, then it passes
- [ ] Unhappy path: if the Cloudflare API is completely unreachable (DNS failure, zone not found), the step fails after the 3 retries with a clear "Cloudflare API unreachable — metadata may be stale up to the edge TTL" message in the workflow summary

#### US-004: Pin CI workflow base image to ubuntu-22.04
**Description:** As a release engineer, I want `ci.yml` and `release.yml` to run on the same Ubuntu base image, so that a test passing in CI means the exact same code will build in release.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `.github/workflows/ci.yml:45`, when the `runs-on:` key is read, then the value is `ubuntu-22.04` (matching `release.yml`'s `runs-on: ubuntu-22.04`)
- [ ] Given this change is merged, when `cargo build` runs in both CI and release, then both use the same glibc version (2.35) and the same system library set
- [ ] Given `ubuntu-latest` rolls to 24.04 in the future (GitHub started rolling this in early 2025), when the release workflow runs, then it is unaffected (still on 22.04)
- [ ] Given the change, when `actionlint .github/workflows/ci.yml` runs, then it passes
- [ ] Unhappy path: if CI-on-22.04 is too permissive and a regression surfaces that `ubuntu-24.04` would catch, a follow-up story adds a separate `ubuntu-24.04` matrix job to CI (but main CI stays on 22.04 for parity with release)

#### US-005: Pin appimageupdatetool to a versioned tag with SHA256 verification
**Description:** As a PaneFlow user, I want the in-app updater to refuse to run a tampered `appimageupdatetool`, so that a compromised CDN or channel regression cannot replace the tool with malicious code that then runs with my AppImage as its argument.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/update/appimage.rs:68-71`, when the download URL is constructed, then it points to `https://github.com/AppImageCommunity/AppImageUpdate/releases/download/2.0.0-alpha-1-20251018/appimageupdatetool-<arch>.AppImage` (canonical repo, pinned dated tag — NOT `continuous`, NOT `AppImage/AppImageUpdate`)
- [ ] Given the tool is downloaded, when the code computes its SHA256 using the existing `sha2` dependency, then the digest is compared to a hardcoded `const APPIMAGEUPDATETOOL_SHA256_X86_64: [u8; 32]` and `const APPIMAGEUPDATETOOL_SHA256_AARCH64: [u8; 32]` in source
- [ ] Given the digest does not match, when the update flow continues, then the tool is deleted from disk and the flow returns `UpdateError::IntegrityMismatch { expected, actual }`
- [ ] Given the digest matches, when the flow continues, then the tool is `chmod +x`'d and invoked as before
- [ ] Given a bump to a newer tool version is needed, when the hardcoded constants are updated, then a tracking comment adjacent to the constants documents the exact procedure: `// To bump: (1) pick new dated tag from https://github.com/AppImageCommunity/AppImageUpdate/releases, (2) download both arch binaries, (3) sha256sum each, (4) paste hex bytes here, (5) advance tag in URL.`
- [ ] Given the network request to GitHub fails mid-download, when the failure occurs, then the partial file is deleted (not left on disk) and `UpdateError::Network` is returned
- [ ] Given a unit test, when it simulates a digest mismatch, then it asserts `UpdateError::IntegrityMismatch` is returned and the file is not present on disk
- [ ] Unhappy path: if upstream removes the pinned tag (unlikely but possible), the 404 surface produces a clear `UpdateError::ReleaseAssetMissing` error, not a silent fall-through

#### US-006: Downgrade sha2 from 0.11 → 0.10 stable line
**Description:** As a PaneFlow maintainer, I want to ship on the sha2 0.10.x stable line instead of the 0.11 pre-release, so that the binary doesn't carry two incompatible copies of sha2 and we don't depend on an unstable API.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/Cargo.toml:57`, when the `sha2` dependency is read, then the version spec is `"0.10"` (not `"0.11"`)
- [ ] Given `cargo build --release` runs after the change, when `Cargo.lock` is inspected, then only one major.minor line of `sha2` is resolved (`0.10.x`), with no `sha2 0.11.x` entry
- [ ] Given `cargo tree -p sha2` is run, when the output is inspected, then no transitive dep forces `sha2 ^0.11`
- [ ] Given the downgrade, when `cargo test --workspace` runs, then all tests pass (no 0.11-only API used in the codebase)
- [ ] Given the codebase calls `sha2::Sha256::new()`, `update(...)`, `finalize()`, when the 0.10 API is applied, then these calls compile without modification (the core Sha256 API is unchanged between 0.10 and 0.11)
- [ ] Given US-005 also uses `sha2::Sha256`, when US-005 merges, then it uses the 0.10 API consistently (this story must land before US-005 or concurrently)
- [ ] Unhappy path: if a transitive dep (likely a test dep) forces 0.11 back into the lockfile, the story is blocked and a spike story is filed to identify the offending crate via `cargo tree -p sha2 --invert` output

#### US-007: Clear stale review notes from the Linux packaging migration status file
**Description:** As an auditor running a fresh review of the predecessor PRD, I want `prd-linux-packaging-migration-status.json` to reflect current code reality, so that audits don't surface false-positive blockers that were fixed weeks ago.

**Priority:** P2
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `tasks/prd-linux-packaging-migration-status.json` US-002, when the `review_note` is inspected, then either: (a) it is removed, `status` set to `"DONE"`, and `"reviewed_at": "2026-04-20"` is added — because `appstreamcli validate` IS wired in CI at `release.yml:112-118`; OR (b) the note is rewritten to reflect a remaining genuine gap discovered during the edit
- [ ] Given US-017, when the `review_note` is inspected, then all three sub-gaps are re-verified against current code: (i) `packaging/paneflow-release.asc` contains a real `-----BEGIN PGP PUBLIC KEY BLOCK-----` (not a stub) → if true, drop sub-gap (a); (ii) `repo-publish.yml:102` enforces `^[0-9A-Fa-f]{40}$` → if true, drop sub-gap (b); (iii) `docs/pkg-repo-runbook.md:65` uses `/tmp/paneflow-release-private.asc` → if true, drop sub-gap (c). If all three drop, `status` → `"DONE"`
- [ ] Given the edits, when `jq . tasks/prd-linux-packaging-migration-status.json` runs, then the JSON parses cleanly with no syntax errors
- [ ] Given epic roll-up, when `EP-001` and `EP-003` story counts are recomputed, then their `status` advances to `"DONE"` if all their stories reach `"DONE"` (Epic Status Roll-up rule from `prd-template.md:316-320`)
- [ ] Given the PRD-level status, when all 5 epics reach `"DONE"` (except EP-004 which has the hardware-gated US-020), then the PRD `status` stays `"IN_PROGRESS"` only if EP-004 still has open stories
- [ ] Unhappy path: if any review note describes a gap that is NOT actually resolved in code upon re-verification, the note is preserved verbatim and the story status stays `"IN_REVIEW"` — this story does not force-close notes, it only drops ones that are demonstrably stale

#### US-008: Gate the legacy .run installer code path with #[cfg(unix)]
**Description:** As a maintainer reading the codebase fresh, I want the legacy `.run` installer functions to be compile-time gated to Unix, so that it's obvious at a glance that this code path cannot reach Windows and the compiler enforces the invariant.

**Priority:** P2
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/update/mod.rs:82`, when `download_installer` is defined, then it has a `#[cfg(unix)]` attribute immediately above the `fn`
- [ ] Given `src-app/src/update/mod.rs` around line 122, when `run_installer` is defined, then it also has `#[cfg(unix)]`
- [ ] Given `installed_binary_path()` at `update/mod.rs:149` (returns `~/.local/bin/paneflow`), when it is defined, then it also has `#[cfg(unix)]` if it is only used by the legacy path (verify via call-graph)
- [ ] Given `cargo check --target x86_64-pc-windows-msvc --target aarch64-pc-windows-msvc` runs after the change, then it passes cleanly with no unresolved-symbol errors
- [ ] Given all call sites of these functions, when they are audited, then every call site is either: (a) already `#[cfg(unix)]` gated, or (b) dispatches at runtime and the Windows/macOS arm does not reach it. If (b) and the Windows/macOS arm IS reachable at runtime, this story is blocked and a follow-up bug-fix story is filed against the dispatch logic
- [ ] Given the story lands, when `rg '#\[cfg\(unix\)\]' src-app/src/update/mod.rs` runs, then at least the 3 expected functions show matches
- [ ] Unhappy path: if adding `#[cfg(unix)]` breaks `cargo check --target x86_64-unknown-linux-gnu` (unexpected — Linux IS Unix), investigate immediately and do not proceed

---

### EP-003: Cross-platform release readiness

Six stories that must ship before the first macOS / Windows release. Not gated on a specific date; close individually as implemented. Paves the way for `RELEASE_MACOS_PLAN.md` and `tasks/prd-windows-port.md` execution. Target release: v0.3.0 (TBD).

**Definition of Done:** `InstallMethod::AppBundle` and `InstallMethod::WindowsMsi` dispatch to real code (no `bail!`). Reveal-in-file-manager works on all 3 OSes. macOS font picker is populated without Homebrew fontconfig. Dead AI-hook code removed. `CLAUDE.md` reflects reality.

#### US-009: Wire macOS .dmg update install flow into self-update dispatch
**Description:** As a macOS user on a PaneFlow build with auto-update enabled, I want the in-app "Install update" button to correctly mount the downloaded DMG and replace the installed `/Applications/PaneFlow.app`, so that the update succeeds without any opaque failures or fall-through to Linux `.run` execution.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/update/macos/dmg.rs:9`, when the install function is called, then it no longer contains `bail!("not yet implemented")` and instead executes the sequence: `hdiutil attach -nobrowse -readonly <dmg>` → `cp -R <volume>/PaneFlow.app /Applications/PaneFlow.app.new` → atomic rename → `hdiutil detach <volume>`
- [ ] Given `src-app/src/app/self_update_flow.rs`, when `InstallMethod::AppBundle` is matched in the dispatch, then execution routes to `update::macos::dmg::install(...)` (not to the legacy `.run` path at `update/mod.rs:82`)
- [ ] Given `installed_binary_path()` is called on macOS, when the path is returned, then it points to `/Applications/PaneFlow.app/Contents/MacOS/paneflow` (not `~/.local/bin/paneflow`)
- [ ] Given the DMG SHA256 verification fails (US-005 pattern applied to the DMG), when the install is attempted, then `UpdateError::IntegrityMismatch` is returned, the DMG and mounted volume are cleaned up, and the installed app is untouched
- [ ] Given the DMG install completes successfully, when the app restarts via `cx.restart()`, then the new binary at `/Applications/PaneFlow.app/Contents/MacOS/paneflow` launches
- [ ] Given `cargo check --target x86_64-apple-darwin --target aarch64-apple-darwin` runs, then both compile cleanly
- [ ] Given a unit test stubs `hdiutil` failures, when integrity/mount/copy errors occur, then each maps to a specific `UpdateError` variant (not generic `Other`)
- [ ] Unhappy path: if `/Applications/` is not writable (e.g., user installed in `~/Applications/` or SIP blocks the replacement), return `UpdateError::InstallDeclined` with a user-visible message "Unable to replace /Applications/PaneFlow.app — reinstall manually"
- [ ] Given the story is DONE per code review, when a macOS smoke test is run on real hardware (maintainer-gated), then the update flow completes end-to-end and an `asciinema` cast is attached to the story review note

#### US-010: Wire Windows .msi update install flow into self-update dispatch
**Description:** As a Windows user on a PaneFlow build with auto-update enabled, I want the in-app "Install update" button to correctly invoke `msiexec` with the downloaded MSI, so that the update installs with the same UAC elevation semantics as a fresh install and routes through Windows Installer's transaction log.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/update/windows/msi.rs:9`, when the install function is called, then it no longer contains `bail!("not yet implemented")` and instead executes `msiexec.exe /i <msi> /qb /norestart /l*v <log>` via `std::process::Command`, where `<log>` is a temp path under `%TEMP%`
- [ ] Given `src-app/src/app/self_update_flow.rs`, when `InstallMethod::WindowsMsi` is matched in the dispatch, then execution routes to `update::windows::msi::install(...)` (not to the legacy `.run` path)
- [ ] Given `installed_binary_path()` is called on Windows, when the path is returned, then it points to `%ProgramFiles%\PaneFlow\paneflow.exe` via the `PATHEXT`-aware resolver (not a hardcoded `C:\Program Files\PaneFlow\paneflow.exe` path)
- [ ] Given the MSI SHA256 verification fails, when the install is attempted, then `UpdateError::IntegrityMismatch` is returned and the MSI is deleted
- [ ] Given the MSI install completes with `msiexec` exit code 0, when the app restarts via `cx.restart()`, then the new binary launches with the updated version (`paneflow --version` reflects new version)
- [ ] Given `msiexec` returns exit code 1602 (user declined UAC), when the update flow handles it, then `UpdateError::InstallDeclined` is returned with a "Update cancelled — administrator permission required" message
- [ ] Given `msiexec` returns exit code 1603 (fatal install error), when handled, then `UpdateError::InstallFailed { log_path }` includes the path to the verbose log for post-mortem
- [ ] Given `cargo check --target x86_64-pc-windows-msvc --target aarch64-pc-windows-msvc` runs, then both compile cleanly
- [ ] Unhappy path: if `msiexec.exe` is not found in `PATH` (pathological Windows state), return `UpdateError::EnvironmentBroken` with an explanatory message
- [ ] Given the story is DONE per code review, when a Windows 11 smoke test is run on real hardware or VM (maintainer-gated), then the update flow completes end-to-end

#### US-011: Dispatch xdg-open / open / explorer.exe per-OS in reveal-in-file-manager sites
**Description:** As a macOS or Windows user, I want the "Reveal in file manager" and port/URL open actions to actually open my native file manager or browser, so that a UI feature advertised as working actually works on my platform.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/app/workspace_ops/mod.rs:502`, when the reveal action is triggered on Linux, then `Command::new("xdg-open").arg(<path>)` is spawned (current behavior preserved)
- [ ] Given the same site on macOS, when triggered, then `Command::new("open").arg(<path>)` is spawned
- [ ] Given the same site on Windows, when triggered, then `Command::new("explorer").arg("/select,").arg(<path>)` is spawned (explorer with `/select,` highlights the target file in its parent folder — standard Windows convention)
- [ ] Given `src-app/src/app/sidebar/mod.rs:506`, when a port or URL is clicked on Linux, then `xdg-open <url>` runs (current behavior preserved)
- [ ] Given the same site on macOS, when triggered, then `open <url>` runs
- [ ] Given the same site on Windows, when triggered, then `cmd.exe /C start "" <url>` runs (empty string as window title is the documented idiom to open URLs via the default handler)
- [ ] Given a spawn fails (binary not found, permission denied), when the failure occurs, then a user-visible toast appears (current behavior is silent `log::warn!` only — this is a small UX upgrade)
- [ ] Given the dispatch is implemented, when `cargo check --target x86_64-pc-windows-msvc --target x86_64-apple-darwin` runs, then both compile cleanly
- [ ] Unhappy path: on a minimal Linux install without `xdg-utils` package, spawn fails; toast reads "xdg-open not found — install xdg-utils to use this feature"
- [ ] Manual verification: test reveal-in-file-manager on Linux (Nautilus / Dolphin / Nemo), macOS (Finder), and Windows 11 (File Explorer with file highlighted) before marking DONE

#### US-012: Fall back to Core Text for font enumeration on macOS
**Description:** As a macOS user without Homebrew's fontconfig installed, I want PaneFlow's settings font picker to show the system fonts, so that I can actually customize my terminal font without installing third-party tooling.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/fonts.rs:13`, when the existing `#[cfg(not(windows))]` branch is split, then Linux keeps the `fc-list` path and macOS switches to a Core Text enumeration path (via either the `core-text` crate v20+ directly, or `font-kit`'s Core Text backend — decision deferred to implementation based on GPUI's existing font-loading dep tree)
- [ ] Given a fresh macOS install without Homebrew, when the settings font picker opens, then at least the 15 standard macOS monospace system fonts (SF Mono, Monaco, Menlo, Courier, Courier New, Andale Mono, etc.) are enumerated and selectable
- [ ] Given a macOS install WITH Homebrew fontconfig present, when the picker opens, then the behavior is equivalent to the fallback case (no dual-listing, no empty picker — Core Text is the authoritative source on macOS regardless of whether fontconfig is installed)
- [ ] Given `cargo check --target x86_64-apple-darwin --target aarch64-apple-darwin` runs, then it compiles cleanly on both Intel and Apple Silicon
- [ ] Given the new Core Text path, when it is exercised under Instruments, then font enumeration completes in under 200 ms on a warm cache (no perf regression vs fc-list which takes ~100 ms)
- [ ] Given `cargo test --workspace` runs, then any new unit tests for font enumeration pass
- [ ] Unhappy path: if Core Text fails for an unexpected reason (sandbox restriction, framework not loaded), the function returns an empty list + `log::warn!("Core Text font enumeration failed: {err}")` — the picker shows empty rather than panicking (current Linux fallback-on-failure behavior preserved for macOS)

#### US-013: Remove orphaned AI-hook extraction code
**Description:** As a maintainer, I want `terminal/pty_session.rs:749` to not extract `claude`, `codex`, `paneflow-hook` scripts that don't exist anywhere in the embed set, so that shell spawn isn't wasting syscalls on a dead code path and future readers aren't misled by the CLAUDE.md reference.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/terminal/pty_session.rs:749`, when the `ensure_wrapper_scripts` function (or equivalent) is inspected, then either: (a) the function is deleted entirely and its callers updated, OR (b) the function remains but the iteration list is empty and a comment above it reads `// AI hook system currently unwired — tracked in a future PRD for cross-platform implementation`
- [ ] Given the decision is (a), when `rg 'ensure_wrapper_scripts' src-app/` runs, then the output is empty (no callers remain)
- [ ] Given the decision is (a), when `rg 'assets/bin' src-app/` runs, then the output is empty
- [ ] Given the decision is (b), when `rg 'assets/bin' src-app/` runs, then the sole match is the annotated comment
- [ ] Given `cargo test --workspace` runs, then all existing tests pass (no test depends on the hook scripts being extracted)
- [ ] Given the change, when PaneFlow is launched and a new terminal is spawned, then no `PATH` injection occurs for the hook-script directory (confirmed via `echo $PATH` inside the spawned terminal — it does not contain a paneflow-specific entry)
- [ ] Given the decision is (a) deletion, when the `rust-embed` attribute for `assets/bin/` is inspected, then it is also deleted (no orphan embed target)
- [ ] Unhappy path: if `ensure_wrapper_scripts` has external callers beyond `pty_session.rs`, all of them are audited and updated in the same commit (scope is the orphan code, not a partial removal)

#### US-014: Purge stale assets/bin reference from CLAUDE.md
**Description:** As an auditor re-reading `CLAUDE.md` for architectural context, I want it to describe files that actually exist in the repo, so that re-audits don't surface the absence of `assets/bin/` as a blocker.

**Priority:** P2
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-013 (decision to delete or defer affects CLAUDE.md wording)

**Acceptance Criteria:**
- [ ] Given `/home/arthur/dev/paneflow/CLAUDE.md` in the "Gotchas" section, when the line `"AI hook scripts at `assets/bin/{claude,codex,paneflow-hook}` are Unix-only shell scripts; Windows equivalents are tracked in `prd-windows-port.md`"` is read, then it is either: (a) removed entirely if US-013 deleted the code, OR (b) rewritten to `"AI hook system is currently unwired (dead extraction code removed by US-013 / retained as empty scaffold per future PRD)"` if US-013 chose the defer path
- [ ] Given `grep 'assets/bin' CLAUDE.md` runs after the edit, then the output matches US-013's resolution (0 matches for deletion; 1 annotated line for deferral)
- [ ] Given the edit is committed, when `git log -1 --format=%s` is read, then the commit message is of the form `docs(CLAUDE): remove stale assets/bin reference` (scope limited to `CLAUDE.md`)
- [ ] Given predecessor PRD files (`tasks/prd-linux-packaging-migration.md`, `tasks/prd-windows-port.md`), when `grep 'assets/bin' tasks/` runs, then any stale references there are also reconciled in the same commit if they exist (scope creep is acceptable for this specific kind of cleanup — they're all the same kind of stale-doc issue)
- [ ] Given the change, when a fresh `/meta-audit` or `/review-story` runs on the repo, then the `assets/bin` cluster of findings no longer surfaces
- [ ] Unhappy path: if a future PRD re-introduces `assets/bin/` with real scripts (the future AI-hook PRD), this story's change is reverted as part of that PRD — it is not preserved as legacy

---

## Functional Requirements

- FR-01: The system must bound every outbound HTTP request made from the auto-updater subsystem with a 30-second total timeout (`ureq` `timeout_global`).
- FR-02: The system must produce a SHA256 integrity sidecar (`<file>.sha256`) for every release artifact published to GitHub Releases.
- FR-03: The system must invalidate the Cloudflare CDN cache for repo metadata files (`InRelease`, `Release`, `Release.gpg`, `Packages.gz`, `repomd.xml`, `repomd.xml.asc`) within 60 seconds of an R2 sync.
- FR-04: The system must verify third-party binary tools downloaded at runtime against a hardcoded SHA256 before executing them.
- FR-05: The `InstallMethod::AppBundle` dispatch path must invoke macOS-native install logic (hdiutil + atomic replace), never the Linux `.run` path.
- FR-06: The `InstallMethod::WindowsMsi` dispatch path must invoke `msiexec /i` with UAC semantics, never the Linux `.run` path.
- FR-07: Reveal-in-file-manager on macOS must call `open`, on Windows `explorer /select,`, on Linux `xdg-open`.
- FR-08: Font enumeration on macOS must return at least the system default monospace fonts without requiring third-party fontconfig.
- FR-09: The system must NOT extract AI-hook wrapper scripts from the embedded asset tree if the source files are not present in the tree (current state: orphaned iteration touches `PATH` for no benefit).
- FR-10: The system must NOT reference filesystem paths in documentation (`CLAUDE.md`, PRD files) that do not exist in the repository.
- FR-11: The system must compile-gate the legacy `.run` installer functions with `#[cfg(unix)]` so the type system enforces their unreachability on Windows.

## Non-Functional Requirements

- **Performance:** auto-updater network timeout ≤ 30s total per request; Cloudflare purge-to-fresh-edge latency ≤ 60s P95 (typical 150 ms per Cloudflare operational data); macOS Core Text font enumeration ≤ 200 ms warm.
- **Security:** 100% of third-party binary downloads verified via SHA256 before execution; no TOFU-HTTPS-only downloads in the update path; Cloudflare API token minimum-scoped to `Zone → Cache Purge → Purge`.
- **Reliability:** auto-updater never produces a zombie background thread (every thread exits within 30 seconds of its last network operation); Cloudflare purge step retries 3× with exponential backoff (1s, 2s, 4s) before failing.
- **Integrity:** 100% of release artifacts have a publicly-downloadable `.sha256` sidecar; `sha256sum -c <file>.sha256` succeeds for every asset.
- **Compatibility:** every code change must compile on 6 targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`.
- **Reproducibility:** CI and release workflows run on identical Ubuntu base image (`ubuntu-22.04` pinned on both).
- **Observability:** every new error path emits a structured `log::warn!` or `log::error!` with enough context for post-mortem (error class, file, remote URL, HTTP status or exit code).

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Network timeout on update check | WiFi drops during background check | Thread exits with `ureq::Error::Timeout` within 30–35s, UI shows `Errored` state | "Update check timed out" |
| 2 | Partial download interrupted | Connection drops mid-download of `.tar.gz` / AppImage / DMG / MSI | Partial file deleted from disk, `UpdateError::Network` returned, retry offered | "Download interrupted — retry?" |
| 3 | SHA256 mismatch on downloaded artifact | Mirror tampering, corruption, upstream retag | Artifact deleted, `UpdateError::IntegrityMismatch { expected, actual }` returned, install aborted | "Downloaded file failed integrity check — please try again" |
| 4 | Cloudflare purge rate-limited | `purge_everything` called more than once per second (shouldn't happen with `files:` purge, but defensive) | Step retries with exponential backoff (3×), then fails with clear message | N/A (CI-only, surfaces in workflow summary) |
| 5 | Cloudflare API token lacks permission | Secret misconfigured post-rotation | Purge step returns HTTP 403, workflow fails loudly at that step with a `"Cloudflare API token missing Zone → Cache Purge → Purge scope"` message | N/A (CI-only) |
| 6 | macOS /Applications not writable | SIP, Gatekeeper quarantine, or user installed to ~/Applications | Install fails cleanly, `UpdateError::InstallDeclined` returned, installed app unchanged | "Unable to replace /Applications/PaneFlow.app — reinstall manually" |
| 7 | Windows UAC declined | User clicks "No" on elevation prompt | `msiexec` returns exit code 1602, `UpdateError::InstallDeclined` returned | "Update cancelled — administrator permission required" |
| 8 | Windows install fatal error | `msiexec` returns 1603 (corrupted MSI, DB lock, etc.) | `UpdateError::InstallFailed { log_path }` returned with path to `msiexec` verbose log | "Update failed — see log at {path}" |
| 9 | macOS without Homebrew fontconfig | Fresh macOS install | Core Text fallback populates font list | N/A (picker just works) |
| 10 | `xdg-open` missing on minimal Linux | User installed without `xdg-utils` package | Spawn fails, toast shown | "xdg-open not found — install xdg-utils" |
| 11 | `explorer.exe` spawn fails on Windows | Very unusual (explorer not in PATH, security software interference) | Spawn fails, toast shown | "Could not open file manager" |
| 12 | AppImageUpdateTool SHA256 mismatch | CDN serves tampered binary, or pinned tag removed upstream | Tool deleted, `UpdateError::IntegrityMismatch` returned, update aborted, next retry tries fresh download | "Update tool failed integrity check — please update PaneFlow manually" |
| 13 | `sha2 0.11` transitive dep forces 0.11 back into lockfile | US-006 blocker scenario | Story blocked; spike story filed to identify offending crate via `cargo tree -p sha2 --invert` | N/A (maintainer-visible only) |
| 14 | AppImageUpdate pinned tag removed upstream | Unusual (dated tags are conventionally immutable) but possible | Download 404s; `UpdateError::ReleaseAssetMissing` returned with tracking message | "Update tool unavailable — please update PaneFlow manually" |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|-------------|--------|------------|
| 1 | appimageupdatetool SHA256 pin goes stale (upstream retags, or force-pushes to `continuous`) | Low (dated tag, not rolling channel) | Medium (in-app updater breaks silently on every user) | Hardcode dated tag + per-arch SHA256 as a `const`; document bump procedure in inline comment; quarterly review cadence on maintainer's calendar; fail-closed on mismatch |
| 2 | Cloudflare Instant Purge does not propagate globally within 60 seconds | Very low (operational < 150 ms) | Medium (first 60 seconds of clients hit stale edge) | Add 60-second `sleep` post-purge in `repo-publish.yml` as defense-in-depth; smoke-test step runs `curl -I ...` and asserts `cf-cache-status: MISS` or `DYNAMIC` |
| 3 | Wiring DMG/MSI install flows requires hardware validation | High (no macOS/Windows CI with full UI tests today) | High (ship-blocker for first macOS/Windows release) | Implement now; mark stories `IN_REVIEW` until maintainer runs hardware smoke test; attach `asciinema` cast or screen recording to review note |
| 4 | `sha2` 0.10 downgrade fails to compile (API differs in a used path) | Low (core `Sha256` API stable across 0.10/0.11) | Low (trivial to revert Cargo.toml change) | `cargo tree -p sha2` before + after; code review of all `sha2::` call sites |
| 5 | `CLAUDE.md` edits conflict with ongoing monolith-refactor activity on `main` | Medium (`main` is active) | Low (trivial merge conflict resolution) | Land US-014 after US-013 is merged; rebase branch immediately before merging |
| 6 | ureq 3.x timeout API changes in a minor bump | Low (ureq follows semver, `Cargo.lock` pins `3.3.0`) | Low (breakage would be caught by CI on next `cargo update`) | Lockfile committed; deliberate `cargo update` reviewed; `cargo clippy` strict catches breakage |
| 7 | Cloudflare API token accidentally over-scoped | Low (operator-controlled; new token can be minted purge-only) | Medium (exposure surface if token leaks) | Document exact `Zone → Cache Purge → Purge` scope in `docs/pkg-repo-runbook.md`; rotate tokens annually; never commit token value anywhere |
| 8 | US-008 `#[cfg(unix)]` gate breaks a call site that is actually reachable on Windows (dispatch bug) | Low (dispatch has been audited, falls through only for AppBundle/WindowsMsi) | High (Windows users crash on update) | US-009 + US-010 must ship BEFORE US-008 in the same v0.2.1 release; cross-compile check in CI catches symbol resolution errors |
| 9 | Core Text font enumeration on macOS is slower than fc-list (regression) | Low (Core Text is the native API, typically fastest) | Low (200 ms budget is generous) | Measure during implementation; fall back to cached list if enumeration > 200 ms |
| 10 | Removing `ensure_wrapper_scripts` regresses some hidden external integration | Very low (no known caller beyond the PTY loop; audit performed) | Low (easily reverted) | Grep call sites before deletion; stage behind a config flag for one release if uncertain |

## Non-Goals

Explicit boundaries — this PRD does NOT include:

- **Re-creating the AI-hook system with Unix + Windows scripts.** US-013 removes the dead code; a future PRD will re-add proper AI hook support across all 3 OSes from scratch.
- **Shipping a macOS or Windows release.** Tier 3 stories prepare the code for a future release cut but do not include code signing, notarization, `winget` / Homebrew submission, or release-cutting itself. Those are covered by `RELEASE_MACOS_PLAN.md` and `tasks/prd-windows-port.md`.
- **Hardening the GPG signing infrastructure beyond what US-017 (predecessor PRD) already delivered.** US-007 clears stale review notes but does not add GPG key rotation automation, client-side fingerprint pinning, or a `paneflow-archive-keyring` package (all deferred).
- **Automated CVE scanning of the dependency tree.** `cargo audit` / `cargo deny` integration is tracked as a separate future improvement (surfaced in Agent 5 of the 2026-04-20 swarm audit).
- **Bumping the GPUI pin.** GPUI version is frozen for this release train; bump will be a dedicated PRD.
- **Validating aarch64 artifacts on real hardware.** Tracked by US-020 in the predecessor PRD; handled by `RELEASE_V0.2.0_PLAN.md` Phase B.8.
- **Adding a `sha2` `deny.toml` policy.** Deferred to the future cargo-deny PRD.

## Files NOT to Modify

- `Cargo.toml` `[patch.crates-io]` block (`async-task`, `calloop`) — load-bearing for the current GPUI pin; any change risks breaking the GPUI build.
- `src-app/src/main.rs` bootstrap top-level (lines 1–200) — untouched by any story in this PRD; only `src-app/src/app/self_update_flow.rs` and `src-app/src/update/` submodules are in scope.
- `tasks/prd-linux-packaging-migration.md` — the predecessor PRD markdown text is historical; US-007 edits the status JSON only, not the PRD document itself.
- `assets/io.github.arthurdev44.paneflow.metainfo.xml` v0.2.0 entry — added by `RELEASE_V0.2.0_PLAN.md` Phase B.5, not by this PRD. This PRD's v0.2.1 bump (if/when Tier 2 ships) adds a new `<release version="0.2.1">` entry in a single atomic commit alongside the version bump.
- `.github/workflows/release.yml` GPG signing section (lines 213–402) — US-017 hardening is complete; do not regress the 40-char fingerprint enforcement or the cleanup `trap`.
- GPUI source (external git dep) — frozen.

## Technical Considerations

Framed as questions for engineering input:

- **Architecture (US-001):** Use `ureq` v3's `timeout_global` builder vs. switching to `reqwest` or `hyper`. Recommended: stay on `ureq` — the project already depends on `ureq 3.3.0`, switching stacks is out of proportion for a 3-line fix. Engineering to confirm no other crate in the workspace requires a different HTTP client.
- **API Design (US-003):** Cloudflare purge implemented via third-party GitHub Action (`nathanvaughn/actions-cloudflare-purge`) vs. hand-rolled `curl` step. Trade-off: the action has better error messages and retry logic built in; a `curl` step has zero third-party supply-chain risk. Recommended: action, given the workflow already uses third-party actions (`actions/checkout`, `actions/setup-rust`, etc.). Engineering to confirm the action's pinned commit SHA (not just the `@v4.0.0` tag) during implementation for supply-chain hygiene.
- **Dependencies (US-005):** `sha2` already in `src-app/Cargo.toml:57` for other purposes — reuse it for the AppImageUpdateTool integrity check, don't add a new hashing crate. Engineering to confirm.
- **Dependencies (US-012):** Core Text on macOS — `font-kit` (likely already in the GPUI transitive dep tree) vs. direct `core-text` crate binding. Trade-off: font-kit is higher-level and cross-platform (could simplify US-012 API); core-text is lower-level and has less code surface. Engineering to decide based on existing GPUI font-loading path.
- **Migration (US-006):** `sha2 0.10` vs. `0.11` — run `cargo tree -p sha2` before and after the downgrade to confirm no transitive 0.11 requirement. Rollback: single-line revert of Cargo.toml, no data migration needed.
- **Platform dispatch (US-009, US-010):** The dispatch in `self_update_flow.rs` currently has explicit branches for `AppImage`, `TarGz`, `Unknown`, `SystemPackage`. Add explicit branches for `AppBundle` and `WindowsMsi`, routing to the new `update::macos::dmg::install` / `update::windows::msi::install` functions. Remove the fall-through to the legacy `.run` path at `update/mod.rs:82` for these methods (combined with US-008 `#[cfg(unix)]` gates). Engineering to verify the fall-through is truly eliminated via cargo-check on all 6 targets.
- **Error taxonomy (US-009, US-010):** New error variants needed — `UpdateError::InstallDeclined`, `UpdateError::InstallFailed { log_path }`, `UpdateError::ReleaseAssetMissing`. Engineering to confirm these align with existing `UpdateError` enum conventions in `src-app/src/update/error.rs`.

## Success Metrics

| Metric | Baseline (pre-hardening) | Target | Timeframe | How Measured |
|--------|--------------------------|--------|-----------|--------------|
| Update-checker thread never hangs unboundedly | Unbounded (ureq 3 default) | ≤ 35s worst case | v0.2.0 ship date | Manual offline smoke test; `RUST_LOG=info` log inspection |
| GitHub Release asset SHA256 sidecar coverage | 1 / 6 artifacts (tar.gz only) | 6 / 6 artifacts | v0.2.1 ship date | `gh release view v0.2.1 --json assets \| jq '[.assets[].name]' \| grep -c ".sha256"` = 6 |
| Cloudflare edge freshness P95 after release publish | TTL-dependent (minutes) | < 60s post-purge | v0.2.1 ship date | `curl -I https://pkg.paneflow.dev/apt/dists/stable/InRelease \| grep cf-cache-status` returns MISS/DYNAMIC |
| CI / release base-image drift | `ubuntu-latest` vs. `ubuntu-22.04` | Aligned on `ubuntu-22.04` | v0.2.1 ship date | `grep 'runs-on:' .github/workflows/ci.yml .github/workflows/release.yml` shows identical values |
| Third-party tool downloads verified | 0 / 1 (appimageupdatetool TOFU) | 1 / 1 | v0.2.1 ship date | Code inspection; integrity-mismatch unit test present in `update/appimage.rs` test module |
| `sha2` version slots in Cargo.lock | 2 (0.10 + 0.11) | 1 (0.10 only) | v0.2.1 ship date | `grep 'name = "sha2"' Cargo.lock \| wc -l` = 1 |
| Stale review notes in predecessor PRD status JSON | 3 stale (US-002, US-006, US-017) | 0 stale | v0.2.1 ship date | Manual re-audit via fresh `agent-explore` pass; all `review_note` fields reflect current code |
| macOS/Windows install-flow coverage | 0% (both `bail!`) | 100% (real code; hardware-validated) | First macOS/Windows release | Smoke test on real hardware; attached `asciinema` cast or screen recording in release notes |
| Reveal-in-file-manager cross-OS coverage | 1 / 3 (Linux only works) | 3 / 3 | First macOS/Windows release | Manual smoke test per OS |
| macOS font picker populated without Homebrew | 0 fonts listed | ≥ 15 system fonts | First macOS release | Launch on fresh macOS VM, inspect picker |
| CLAUDE.md accuracy (stale file references) | 1 stale (`assets/bin/`) | 0 stale | v0.2.1 ship date | `grep 'assets/bin' CLAUDE.md` returns 0 lines |

## Open Questions

- **US-005:** Who maintains the `appimageupdatetool` SHA256 pin bump cadence? Options: (a) calendar reminder to Arthur every 6 months, (b) a Dependabot-equivalent custom GitHub Action that opens a PR when the upstream tag advances. Decision needed before US-005 merges so the bump procedure is documented in the inline constant comment.
- **US-007:** Should the predecessor PRD status file be edited in-place or migrated to a v2 schema? Tentative decision: edit in-place — the existing schema is already compatible with the intended edits.
- **US-013:** Delete the `ensure_wrapper_scripts` function entirely (option a) vs. keep the function scaffolding for the future AI-hook PRD (option b)? Tentative decision: defer to implementation — whichever is simpler at the moment the story runs. Prefer (a) unless the scaffolding is reusable for the future PRD.
- **US-011 (Windows URL opener):** Does `cmd /C start "" <url>` open in the user's default browser reliably on Windows 10 + 11? Alternative: use the `webbrowser` crate (adds a dep). Engineering to verify `cmd /C start` empirically or pick the crate.
- **US-012 (macOS font API):** Core Text direct binding vs. font-kit — final decision pending dependency-tree inspection (see Technical Considerations above). Engineering to pick during implementation.
- **US-003 (Cloudflare secrets naming):** The nathanvaughn action expects secrets `CLOUDFLARE_ZONE` and `CLOUDFLARE_AUTH_KEY`. Current `RELEASE_V0.2.0_PLAN.md` Phase B.4 defines `R2_*` secrets but no Cloudflare zone secrets. US-003 must be merged AFTER the maintainer adds `CLOUDFLARE_ZONE` and `CLOUDFLARE_AUTH_KEY` secrets (Zone ID from the Cloudflare dashboard; API token with `Zone → Cache Purge → Purge` scope).
- **EP-003 target version:** v0.3.0 vs v0.2.x? Tentative: v0.3.0, shipped alongside the first macOS/Windows release. Maintainer to confirm when that release plan concretizes.

[/PRD]
