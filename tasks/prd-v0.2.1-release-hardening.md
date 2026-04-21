[PRD]
# PRD: PaneFlow v0.2.1 Release Hardening

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-21 | Arthur Jean | Initial draft — 10-story follow-up to prd-v0.2.0-release-hardening.md capturing debt accumulated during the v0.2.0 release execution (6 tag retries + 1 repo-publish bug-fix). |

## Problem Statement

PaneFlow v0.2.0 shipped on 2026-04-21 after 6 tag retries, 1 repo-publish bug-fix, and 1 manual workflow dispatch. The release is live and validated end-to-end on Ubuntu 24.04 + Fedora 40. Every friction point encountered during that release trace-backs to one of the following 10 debt items — each one has a concrete file reference and a failure signature from the CI logs. Left untracked, they will resurface at the worst possible moment (next release, first external user, first security audit).

1. **Per-package RPM signing is disabled in release.yml.** `rpmsign --addsign` on ubuntu-22.04 (rpm 4.17.0) fails with `error: /dev/stdin: import read failed(0)` despite a fully-populated `~/.rpmmacros`. The current workaround is a commented-out signing loop and `gpgcheck=0` in `packaging/rpm/postinst.sh` — dnf clients only verify repo-level signatures via `repomd.xml.asc`, not per-package signatures. This halves the signing defense-in-depth and closes the door on Fedora users who enable `gpgcheck=1` manually.
2. **reprepro idempotency check was arch-blind.** Commit `83a3333` fixed the awk predicate `$NF == v` that caused `binary-amd64/Packages` to stay empty after the arm64 deb was included first. The regression is fixed but no CI guard exists to prevent a re-introduction.
3. **`repo-publish.yml` requires manual dispatch after every release.** `release: types: [published]` does not fire when the release is created via `GITHUB_TOKEN` in the same workflow run (GitHub anti-loop protection). Every release ends with an "oh right, I need to `gh workflow run repo-publish.yml -f tag=...`" moment that breaks the "tag, push, done" release contract.
4. **No end-to-end smoke test in release.yml.** The two Podman-based tests (Ubuntu 24.04 apt install + Fedora 40 dnf install) were run manually after v0.2.0 shipped — they revealed the reprepro arch-blind bug, which would otherwise have reached production. A CI step reproducing those tests post-publish closes the loop.
5. **`gh secret set GPG_PRIVATE_KEY` stdin-pipe quirk is undocumented.** `gpg --armor --export-secret-keys | gh secret set` stored a truncated/malformed blob that failed `gpg --import` on CI. `gh secret set < /tmp/file.asc` with stdin-redirect works. The operator runbook does not warn about this.
6. **No dependency advisory scanning.** The 2026-04-20 swarm audit flagged the absence of `deny.toml` / `audit.toml`. No CI step surfaces RustSec vulnerabilities, stale yanked crates, or unknown-source git deps. A fail-forward (warnings-only) integration is feasible today and blocks nothing.
7. **US-011 Windows Explorer argument concatenation bug.** `Command::new("explorer").arg("/select,").arg(path)` in `workspace_ops/mod.rs:688-691` passes two args, but Windows Explorer expects `/select,<path>` concatenated into a single arg. Flagged as SHOULD_FIX in the v0.2.0 status.json review note. Will silently no-op on Windows 11 when a user clicks "Reveal in File Manager".
8. **Intel Mac (`x86_64-apple-darwin`) dropped from release.yml matrix.** `macos-13` runner queue times (30+ min) blocked the v0.2.0 `Publish GitHub Release` gate on a best-effort leg. The fix was commenting out the matrix entry. This leaves the Intel Mac build path silently inactive and creates a "who removed this?" question for a future maintainer.
9. **GitHub Actions using deprecated Node.js 20.** `actions/checkout@v4`, `dorny/paths-filter@v3`, `ilammy/msvc-dev-cmd@v1` surface Node.js 20 deprecation warnings. GitHub will force Node.js 24 from 2026-06-02 and remove Node.js 20 runtime on 2026-09-16.
10. **Three residual `review_note` items in `prd-v0.2.0-release-hardening-status.json`.** US-003 CONSIDER (cf-cache-status smoke assertion), US-009 CONSIDER (`cp -R` → `ditto --rsrc`), US-010 CONSIDER (msiexec exit 1618 handling). None block v0.2.0 but all will re-surface on the next code review.

**Why now:** Every item is FRESH — the failure signatures are in the CI logs we just pulled, the file:line references are one `git diff` away, and the operator runbook writer (me) still has full context. Deferring past v0.2.1 means re-deriving each from scratch after 3+ months of context loss. The v0.3.0 macOS/Windows release cut is also coming, and several items (US-007 Explorer fix, US-008 Intel Mac reintegration path, US-005 docs) become prerequisites for that release's success.

## Overview

v0.2.1 is a 10-story patch release focused exclusively on release-engineering debt surfaced by the v0.2.0 execution. No new user-facing features. No GPUI / terminal / config changes. Target ship: within 1 week of v0.2.0 (2026-04-28) so the runbook + workflow changes land before the next release cut.

The PRD splits into three epics:

**EP-001 (P0 — 3 stories):** Correctness fixes that affect published artifacts. Per-package RPM signing comes back (story 1), the reprepro arch-blind bug gets a regression guard (story 2), and `repo-publish.yml` auto-triggers after a release (story 3). These close the last three gaps between "tag a release" and "users can install it".

**EP-002 (P1 — 4 stories):** Process and observability. A post-publish smoke test (story 4) catches repo-level bugs in CI instead of post-mortem. The `gh secret set` stdin quirk is documented (story 5). A fail-forward `cargo-deny` integration lights up the advisory stream (story 6). The Windows Explorer arg bug is fixed (story 7).

**EP-003 (P2 — 3 stories):** Maintenance. The Intel Mac matrix entry gets a deliberate resolution (story 8). Node.js 24 action bumps (story 9). Residual v0.2.0 review notes are either actioned or formally closed with rationale (story 10).

Key design decisions grounded in the research agent's findings:

- **`workflow_run` over a PAT-based chain (story 3):** GitHub's anti-loop protection blocks `release.published` from firing on a GITHUB_TOKEN-created release. `workflow_run: workflows: ["Release"] types: [completed]` with `if: github.event.workflow_run.conclusion == 'success'` chains cleanly with no PAT to rotate. Max chain depth is 3, well within our needs. Tag is available via `github.event.workflow_run.head_branch` (for a tag push, `head_branch` holds the tag name).
- **rpmsign root-cause path (story 1):** The `import read failed(0)` error is a known rpm upstream issue ([rpm#3740](https://github.com/rpm-software-management/rpm/issues/3740)) tied to GPG agent state at invocation time, not to the macros file. Spike will verify that pre-warming the agent with `gpg --list-secret-keys` after import + keeping loopback pinentry is sufficient. Falls back to `rpm --addsign` alias or `--define`-only CLI invocation if the macros path cannot be made to work.
- **Warn-only cargo-deny (story 6):** `advisories.yanked = "warn"` + `advisories.unmaintained = "workspace"` + `licenses.allow = [MIT, Apache-2.0, ...]` with `unknown-git = "warn"` and `unknown-registry = "warn"`. No check set to `deny` initially. The RustSec `vulnerabilities` check has no downgrade path (upstream removed it), so if a CVE shows up it WILL fail the job — acceptable for a real vulnerability.

## Goals

| Goal | v0.2.0 (today) | v0.2.1 (within 1 week) |
|------|----------------|-------------------------|
| Per-package RPM signing operational | 0% (disabled) | 100% (`rpmsign --addsign` re-enabled, `gpgcheck=1` restored) |
| `rpm --checksig` on shipped .rpm returns `OK` | NOKEY (no signature) | OK for all signature components |
| Release → repo-publish auto-chain | Manual `gh workflow run` required | `workflow_run` chain fires within 30s of release publish |
| End-to-end CI smoke coverage (Ubuntu + Fedora apt/dnf install) | 0 steps | 2 steps (one per distro) passing |
| CI advisory scanning | None | `cargo-deny check` runs on every PR + push, warnings surface in job summary |
| Deprecated Node.js 20 actions | 3 flagged | 0 flagged |
| Residual v0.2.0 review notes | 3 open | 0 open (actioned or explicitly closed) |

## Target Users

### PaneFlow maintainer (Arthur Jean — solo dev shipping releases)
- **Role:** Tags versions, runs the release pipeline, owns the GPG + R2 + Cloudflare infra. Operates `pkg.paneflow.dev`. Reads `docs/release-runbook.md` once every version bump.
- **Behaviors:** Bumps Cargo.toml + debian/changelog + metainfo.xml, pushes tag, watches CI for ~20 min, smoke-tests in a container, announces. Works solo, no code review before shipping.
- **Pain points:** Manual `gh workflow run` step after every release. Re-running smoke tests by hand. Re-deriving the `gh secret set < file` incantation from Stack Overflow each time a secret gets rotated. Rediscovering that `rpmsign` silently ships unsigned RPMs.
- **Current workaround:** Maintains a mental checklist of "things that need to happen after `gh run watch` turns green" — incomplete, partially documented, at risk of drift after 3 months away from release engineering.
- **Success looks like:** `git push origin vX.Y.Z` → 20 min later, a verified release is live at `pkg.paneflow.dev`, the CI has already run the apt + dnf install smoke tests, and Slack/email has a green check. Zero follow-up manual actions.

### PaneFlow end user on Fedora / RHEL / openSUSE (dnf-based)
- **Role:** Installs PaneFlow via `dnf install paneflow` from `pkg.paneflow.dev/rpm`. Expects the same security posture as `dnf install` from a first-party repo.
- **Behaviors:** Runs `dnf install paneflow`, trusts the repomd.xml signature (because they set `repo_gpgcheck=1`), and may or may not keep `gpgcheck=1` (per-package signature verification).
- **Pain points:** v0.2.0 shipped with `gpgcheck=0` because per-package signing was disabled. A security-conscious user who flips back to `gpgcheck=1` in their local `/etc/yum.repos.d/paneflow.repo` will get NOKEY errors on `dnf install`.
- **Current workaround:** Leave `gpgcheck=0` (security regression) or install via `.rpm` manually with `--nogpgcheck`.
- **Success looks like:** `gpgcheck=1` works out-of-the-box. `rpm --checksig paneflow-0.2.1-1.x86_64.rpm` reports all signature components `OK`.

### Future PaneFlow maintainer (any collaborator, or Arthur in 6 months)
- **Role:** Arrives at the repo, needs to cut a release, reads `docs/release-runbook.md` for the procedure.
- **Behaviors:** Follows the runbook verbatim. Stops the first time something is unclear or surprising.
- **Pain points:** Runbook says "tag, push" but reality requires a manual `gh workflow run repo-publish.yml -f tag=...` step that is buried in a comment. Secret-setting has a pipe-vs-stdin gotcha that is undocumented. Intel Mac matrix entry is commented with a TODO but no owner or target version.
- **Current workaround:** Pings Arthur. Gets blocked on response time.
- **Success looks like:** Runbook is self-contained. Every commented-out code block either has a tracked v0.2.x+ story or a documented "closed — decision: keep disabled indefinitely" rationale.

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **Zed editor**'s release pipeline uses `workflow_run` to chain its release-notification workflow after a signed artifact upload completes. Same pattern we adopt for repo-publish.
- **rust-lang/rust**'s advisory scanning uses `cargo-deny` in warn-only mode for license + sources, deny-mode for vulnerabilities. We mirror that posture.
- **Debian** and **Fedora** both sign individual packages AND repo metadata. Our v0.2.0 shipped only repo metadata signed — story 1 closes the gap.

### Best Practices Applied
- **Chain workflows via `workflow_run`, not PAT** — avoids a long-lived token that needs rotation, survives the GITHUB_TOKEN anti-loop protection, and scopes runner permissions to the downstream workflow's `permissions:` block. Max chain depth is 3 in GitHub Actions — well above our 2-level need.
- **Pre-warm the GPG agent before invoking rpmsign** — the `import read failed(0)` error is tied to agent state at invocation time, not to the macros file. `gpg --list-secret-keys` after `gpg --batch --import` loads the secret into the running agent.
- **Warn-only dependency scanning on first integration** — `cargo-deny` supports per-check severity (`yanked = "warn"`, `unknown-git = "warn"`). No global downgrade flag; set each relevant field explicitly. The `vulnerabilities` subcheck does not offer a warn-mode (upstream removed it in 0.14+) — treat as deny.
- **Pin GitHub Actions by major tag, bump on deprecation** — `actions/checkout@v4` → `@v5` when available. Use `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` as a temporary opt-in for actions that haven't cut a Node.js 24 release yet.

### Technical Recommendations (research-verified)
- **`workflow_run` exact syntax:** `on: workflow_run: workflows: ["Release"] types: [completed]` — the `workflows:` value must match the parent's `name:` field, NOT the filename. `github.event.workflow_run.head_branch` contains the tag name for a tag-triggered parent run. `if: github.event.workflow_run.conclusion == 'success'` is the canonical success guard (no `failure` / `cancelled` chain).
- **rpmsign CI recipe:** (1) `echo "$GPG_PRIVATE_KEY" | gpg --batch --import`, (2) `gpg --list-secret-keys` to warm the agent cache, (3) write `/tmp/gpg-pass` 0600 with the passphrase, (4) populate `~/.rpmmacros` with `%__gpg_sign_cmd` using `--pinentry-mode loopback --passphrase-file /tmp/gpg-pass`, (5) `rpmsign --addsign package.rpm`. `rpm --addsign` is a legacy alias with no different behavior.
- **Minimal `deny.toml` fail-forward:**
  ```toml
  [advisories]
  unmaintained = "workspace"
  yanked       = "warn"
  maximum-db-staleness = "P30D"

  [licenses]
  allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception",
           "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib"]

  [sources]
  unknown-registry = "warn"
  unknown-git      = "warn"
  allow-registry   = ["https://github.com/rust-lang/crates.io-index"]

  [sources.allow-org]
  github = ["zed-industries", "smol-rs", "proptest-rs"]
  ```

*Full research sources: [GitHub Actions workflow_run docs](https://docs.github.com/actions/using-workflows/events-that-trigger-workflows#workflow_run), [rpm issue #3740](https://github.com/rpm-software-management/rpm/issues/3740), [cargo-deny docs](https://embarkstudios.github.io/cargo-deny/), and the v0.2.0 release execution CI logs from 2026-04-21 (run IDs 24707864961, 24708485673, 24711775068, 24712582475, 24712666733, 24713221127, 24713557279).*

## Assumptions & Constraints

### Assumptions (to validate)
- **rpmsign root cause is GPG-agent-state related, not macros-related.** If the spike proves it's a macros issue that the research didn't predict, US-001 size bumps from M to L.
- **`workflow_run` completes within 30s of parent success.** GitHub's event bus is typically sub-second but has no published SLA; 30s is a conservative upper bound based on community reports.
- **Advisory DB refresh over the network is acceptable in CI.** If GitHub's RustSec-DB fetch is rate-limited, fall back to `--no-fetch` + periodic cache warming via a scheduled workflow.
- **`macos-13` runner queues will remain slow.** GitHub Actions macOS runners have had 30-60 min queues for 6+ months. Story 8's Intel Mac reintegration assumes this doesn't change.
- **No new dependency vulnerabilities are active in the current lockfile.** If `cargo-deny` surfaces an unexpected critical advisory, story 6 may require emergency patching before v0.2.1 ships.

### Hard Constraints
- **Cross-platform mandate** (per `CLAUDE.md`): every change compiles on all six targets. No regression allowed.
- **No GPUI pin bump** — out of scope for v0.2.1.
- **No macOS / Windows release scope reopened** — that's v0.3.0. Stories 7 and 8 touch Windows/Mac code but stop short of enabling production builds.
- **Atomic commits per story**; branch naming `feat/...` or `fix/...`.
- **Version bump:** Cargo.toml + Cargo.lock + debian/changelog + `assets/io.github.arthurdev44.paneflow.metainfo.xml` as a single commit at the end, before tagging.
- **Release target:** within 7 days of v0.2.0 (ship by 2026-04-28).

## Quality Gates

These commands must pass for every user story:

- `cargo fmt --check` — formatting enforced
- `cargo clippy --workspace -- -D warnings` — no new warnings on rustc 1.95+
- `cargo test --workspace` — all tests pass
- `cargo check --target x86_64-pc-windows-msvc --target x86_64-apple-darwin` — cross-compile sanity (touched-code stories only)
- `actionlint .github/workflows/*.yml` — workflow YAML + action reference linting (any story that edits a workflow)

For the post-publish smoke (US-004), additionally:
- `podman run --rm ubuntu:24.04 bash -c '... apt install paneflow && paneflow --version'` must include the literal string `paneflow 0.2.1`
- `podman run --rm fedora:40 bash -c '... dnf install paneflow && paneflow --version'` must include `paneflow 0.2.1`

For the RPM signing re-enable (US-001), additionally:
- `rpm --checksig -v paneflow-0.2.1-1.x86_64.rpm` exits 0 with every signature component reported `OK`
- `packaging/rpm/postinst.sh` has `gpgcheck=1` (not `gpgcheck=0`)

For the advisory scanning (US-006):
- `cargo deny check advisories licenses sources` in CI produces a structured job summary — ERRORS (if any) on `vulnerabilities`, WARNINGS elsewhere

## Epics & User Stories

### EP-001: Published-artifact correctness (v0.2.1 must-ship)

Close the last three correctness gaps between "tag push" and "user installs": per-package RPM signing, reprepro regression guard, and release → repo-publish auto-chain.

**Definition of Done:** Per-package `rpmsign --addsign` succeeds in CI with the rpm on ubuntu-22.04. `gpgcheck=1` restored in `packaging/rpm/postinst.sh`. A CI step asserts both `binary-amd64/Packages` and `binary-arm64/Packages` in the published `InRelease` are non-empty. `repo-publish.yml` auto-triggers within 30s of `release.yml` success without manual dispatch.

#### US-001: Root-cause and re-enable per-package RPM signing
**Description:** As a security-conscious Fedora user, I want `rpm --checksig paneflow-0.2.1.x86_64.rpm` to return `OK` on every signature component, so that I can keep `gpgcheck=1` in my repo config without falling back to `--nogpgcheck` on every install.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a local Podman ubuntu:22.04 container with rpm 4.17.0, when the spike runs the v0.2.0 `rpmsign --addsign` invocation + `strace -f -e trace=openat,read`, then the specific `/dev/stdin` read that fails is identified and documented in the commit message
- [ ] Given the root cause, when the release.yml GPG signing step is updated, then it implements the research-validated recipe: `gpg --batch --import`, `gpg --list-secret-keys` agent warm-up, `/tmp/gpg-pass` passphrase file, `~/.rpmmacros` with `--pinentry-mode loopback --passphrase-file /tmp/gpg-pass`
- [ ] Given the updated workflow, when `rpmsign --addsign target/<arch>/generate-rpm/paneflow-0.2.1-1.<arch>.rpm` runs for both `x86_64` and `aarch64`, then both exit 0 without the `Could not set GPG_TTY` warning AND produce a signed rpm
- [ ] Given the signed rpm, when `rpm --checksig -v paneflow-0.2.1-1.<arch>.rpm` runs, then every line reports `OK` (`Header V4 RSA/SHA256 Signature ... OK`, `Header SHA1 digest ... OK`, etc.)
- [ ] Given the signing is restored, when `packaging/rpm/postinst.sh` is inspected, then the `gpgcheck=` line reads `gpgcheck=1` (not `gpgcheck=0`)
- [ ] Given the release.yml verify block at the `for rpm_path in ...; rpm --checksig -v ...` loop, when the step runs, then it is UNCOMMENTED (restored from the v0.2.0 disabled state) and fails the job if any rpm reports `NOKEY` or `BAD`
- [ ] Unhappy path: if the spike cannot resolve the stdin-read issue within 4 hours of investigation, fall back to `--define` CLI flags (no `~/.rpmmacros`) as a documented workaround; if that also fails, revert to unsigned rpms with `gpgcheck=0` and downgrade this story to P1 deferred to v0.2.2

#### US-002: CI regression guard for reprepro arch idempotency
**Description:** As a maintainer, I want a CI step that fails the release if any `binary-<arch>/Packages` in the published InRelease is empty, so that the arch-blind reprepro bug fixed in commit 83a3333 cannot silently regress.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `.github/workflows/repo-publish.yml`, when a new step is added after the rclone-upload-to-R2 step, then it fetches `curl -s https://pkg.paneflow.dev/apt/dists/stable/InRelease`
- [ ] Given the fetched InRelease, when parsed with `awk '/binary-amd64\/Packages$/ {print $1, $2}' `, then both `binary-amd64/Packages` and `binary-arm64/Packages` must NOT have the empty-file MD5 `d41d8cd98f00b204e9800998ecf8427e` AND must NOT have size `0`
- [ ] Given a deliberate regression (simulating 83a3333 rollback), when the test runs, then the step fails with a clear error `"binary-amd64/Packages is empty — reprepro arch-idempotency regression"`
- [ ] Given the fix is intact, when the step runs on a real publish, then it passes silently
- [ ] Given `actionlint .github/workflows/repo-publish.yml` runs after the change, then it passes
- [ ] Unhappy path: if `curl` fails (Cloudflare edge transient error), retry 3× with exponential backoff (1s, 2s, 4s) before failing; the last failure surfaces in the job summary

#### US-003: Auto-chain repo-publish.yml via workflow_run
**Description:** As a maintainer, I want `repo-publish.yml` to run automatically within 30s of a successful `release.yml` completion, so that `git push origin vX.Y.Z` is the ONLY manual step of the release process.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `.github/workflows/repo-publish.yml`, when the `on:` block is updated, then it contains `workflow_run: workflows: ["Release"] types: [completed]` (the value of `workflows:` MUST match `release.yml`'s `name:` field verbatim, not the filename)
- [ ] Given the job definition, when its `if:` is inspected, then it is `if: ${{ github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'push' }}` (filters: success + originally triggered by tag push, not re-runs)
- [ ] Given the job needs the tag name, when it extracts the ref, then it uses `TAG: ${{ github.event.workflow_run.head_branch }}` in the env (for a tag-push parent, `head_branch` holds the tag name)
- [ ] Given the existing `workflow_dispatch` trigger with `inputs.tag`, when the workflow is triggered manually, then it still works (kept as a fallback path)
- [ ] Given a successful `release.yml` completion on a future v0.2.2+ tag, when the tag is pushed, then `repo-publish.yml` fires within 30s and succeeds without any manual `gh workflow run` command
- [ ] Given `docs/release-runbook.md` references the manual dispatch, when the doc is updated, then the manual dispatch step is removed from the "happy path" runbook and kept only in a "Manual re-publish" troubleshooting subsection
- [ ] Unhappy path: if `release.yml` fails, `repo-publish.yml` MUST NOT run (the `if:` conclusion guard handles this); a failed release should not partially update the live repo

---

### EP-002: Process + observability (v0.2.1 should-ship)

Operational improvements: post-publish end-to-end smoke in CI, runbook doc updates, advisory scanning, and the Windows Explorer arg bug. Non-blocking individually, but together they close the feedback loop for every future release.

**Definition of Done:** Post-publish smoke tests run in CI and fail the release pipeline if install breaks. `docs/release-signing.md` has the `gh secret set` stdin-redirect section. `cargo-deny check` runs on every PR. Windows Explorer correctly highlights the target file in the reveal-in-file-manager action.

#### US-004: Post-publish end-to-end smoke test in CI
**Description:** As a maintainer, I want a CI step that installs PaneFlow via `apt` (Ubuntu 24.04) and `dnf` (Fedora 40) in ephemeral containers after every release, so that a repo-level bug (like the v0.2.0 reprepro arch-blindness) is caught before I'm alerted by a user issue.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003 (workflow_run chain) — the smoke must run AFTER repo-publish populates the pool

**Acceptance Criteria:**
- [ ] Given a new job `smoke-post-publish` in `.github/workflows/repo-publish.yml`, when it runs, then it has `needs: publish` so it only fires after the R2 sync + Cloudflare purge completes
- [ ] Given the Ubuntu leg, when it runs `podman run --rm ubuntu:24.04 bash -c '...'`, then the script: installs curl + ca-certificates + gnupg, fetches `https://pkg.paneflow.dev/gpg`, adds the repo via `deb [signed-by=...]`, runs `apt update && apt install -y paneflow`, runs `paneflow --version`
- [ ] Given the Fedora leg, when it runs `podman run --rm fedora:40 bash -c '...'`, then the script: writes `/etc/yum.repos.d/paneflow.repo` with `gpgcheck=1 repo_gpgcheck=1`, runs `dnf install -y paneflow`, runs `paneflow --version`
- [ ] Given either leg, when `paneflow --version` runs, then the output MUST contain the literal string `paneflow 0.2.1` (version-bumped for this PRD); the step fails with a `grep -q` guard if it doesn't
- [ ] Given the step completes, when the job summary is inspected, then both distro results are logged with a ✅/❌ marker
- [ ] Given a deliberate regression (e.g., wrong repo URL), when the smoke runs, then it fails fast with the distro-specific error from `apt update` / `dnf install`
- [ ] Unhappy path: if `podman run` itself fails (runner image issue), fall back to `docker run` (both are available on ubuntu-22.04 runners); if both fail, mark the step as `continue-on-error: false` and fail the release pipeline

#### US-005: Document `gh secret set < file` stdin-redirect quirk
**Description:** As a future maintainer rotating a secret, I want `docs/release-signing.md` to warn about the pipe-vs-stdin-redirect behavior of `gh secret set` with multi-line values, so that I don't spend 45 minutes debugging a "GPG_PRIVATE_KEY failed to import" error like v0.2.0's tag retry #5 did.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `docs/release-signing.md`, when a new subsection titled `### Setting GPG secrets on GitHub` is added, then it contains the specific warning: "DO NOT use `gpg ... | gh secret set GPG_PRIVATE_KEY` — the pipe can truncate the value at certain buffer boundaries. USE `gh secret set GPG_PRIVATE_KEY < /tmp/private.asc` (stdin redirect from file) instead."
- [ ] Given the same section, when the full correct incantation is listed, then it covers: (1) `gpg --armor --export-secret-keys <uid> > /tmp/paneflow-release-private.asc`, (2) `chmod 600 /tmp/...`, (3) `gh secret set GPG_PRIVATE_KEY -R <repo> < /tmp/...`, (4) `shred -u /tmp/...`
- [ ] Given the v0.2.0 retry #5 incident, when referenced in the doc, then a single-sentence historical note cites the symptom (`"GPG_PRIVATE_KEY failed to import. Secret may be truncated or missing the ASCII-armor header."`) so future maintainers can match on the error text
- [ ] Given the doc changes, when `markdownlint docs/` runs (if wired), then it passes; if not wired, a visual inspection confirms the section renders cleanly on GitHub
- [ ] Unhappy path: if a future maintainer still hits the truncation (e.g., via a different CLI like `gh secret set --body`), the doc's "symptom → fix" recipe applies unchanged — the fix is always "use stdin-redirect from a file"

#### US-006: cargo-deny integration with warn-only posture
**Description:** As a maintainer, I want `cargo-deny check advisories licenses sources` to run on every PR and main push, so that a new RustSec vulnerability or a copyleft license creeping into a transitive dep is surfaced within hours of the change landing.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given the repo root, when a new file `deny.toml` is created, then it contains the research-validated minimal config: `[advisories] yanked = "warn" unmaintained = "workspace" maximum-db-staleness = "P30D"`, `[licenses] allow = [MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib]`, `[sources] unknown-registry = "warn" unknown-git = "warn"`
- [ ] Given `ci.yml`, when a new `security-audit` job is added, then it `runs-on: ubuntu-22.04`, uses `Swatinem/rust-cache@v2 prefix-key: "v1-rust"` (consistent with existing cache config), installs `cargo-deny` via `cargo install cargo-deny --locked --version 0.14.x`, and runs `cargo deny check advisories licenses sources`
- [ ] Given the job runs on a clean lockfile (v0.2.0 state), when it completes, then it surfaces findings in the job summary but does NOT fail the overall CI pipeline (per the fail-forward posture); a `continue-on-error: true` on the step OR a `|| true` tail ensures this
- [ ] Given a future transitive dep adds a GPL-licensed crate, when the job runs on that PR, then the `licenses` check surfaces it as a warning but does not block the merge (until v0.2.2 can flip severity to `deny`)
- [ ] Given a RustSec vulnerability is discovered in a transitive dep, when the job runs, then the `advisories.vulnerabilities` check FAILS the step (this check has no warn-mode in cargo-deny 0.14+ — failing is correct security posture)
- [ ] Given the `deny.toml` lists `[sources.allow-org] github = ["zed-industries", "smol-rs", "proptest-rs"]`, when the `sources` check runs against the current Cargo.lock, then no warnings are produced for the known GPUI-pulled git deps
- [ ] Unhappy path: if the advisory DB fetch fails (GitHub RustSec-DB rate-limit or outage), the step fails with `cargo-deny check advisories --help` output logged for debugging; the workflow has a retry with 30s backoff

#### US-007: Fix Windows Explorer argument concatenation + sidebar comment
**Description:** As a future Windows user, I want "Reveal in File Manager" to correctly open Windows Explorer with the target file highlighted, so that the feature advertised in the UI actually works on my OS.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `src-app/src/app/workspace_ops/mod.rs` around line 688-691, when the Windows branch is read, then `Command::new("explorer").arg("/select,").arg(path)` is replaced with: `let mut flag = OsString::from("/select,"); flag.push(path.as_os_str()); Command::new("explorer").arg(flag).spawn()` — passing the `/select,<path>` token as a SINGLE argument
- [ ] Given `src-app/src/app/sidebar/mod.rs` around line 504-509, when the comment block claiming `open::that` is "byte-identical" to `cmd /C start ""` on Windows is read, then it is rewritten to clarify: `open::that` uses `ShellExecuteW` (not a subprocess), which is functionally equivalent for `https://` URLs but is NOT the same mechanism as `cmd /C start`
- [ ] Given the code changes, when `cargo check --target x86_64-pc-windows-msvc` runs, then it passes cleanly
- [ ] Given the changes, when `cargo clippy --workspace -- -D warnings` runs on Linux (cfg'd code paths), then it passes cleanly
- [ ] Given a manual smoke test on a Windows 11 VM or physical host, when the "Reveal in File Manager" action is triggered on a file path, then Windows Explorer opens the PARENT directory AND the target file is visually highlighted (the standard Windows convention)
- [ ] Given the v0.2.0 status.json US-011 review note, when the note is re-read after this story lands, then the SHOULD_FIX item is verifiable as DONE
- [ ] Unhappy path: if a Windows hardware smoke test is not available (maintainer-gated), the story stays in `IN_REVIEW` until hardware validation. The review note should be updated with the validation result.

---

### EP-003: Maintenance (v0.2.1 could-ship or deferred)

Bookkeeping and long-runway items: Intel Mac matrix entry resolution, Node.js 24 upgrades, residual v0.2.0 review-note triage.

**Definition of Done:** Every matrix entry in `release.yml` is either active or has a dated "closed as won't-restore-until-vX.Y.Z" comment. Zero Node.js 20 deprecation warnings in any CI run. `prd-v0.2.0-release-hardening-status.json` has no `review_note` fields left that aren't explicitly closed with rationale.

#### US-008: Resolve Intel Mac matrix entry status
**Description:** As a future maintainer, I want a clear decision on whether `x86_64-apple-darwin` is a supported target, so that I don't either (a) silently ship without it and leave Intel Mac users stuck, or (b) uncomment it and spend 45 min debugging the macos-13 runner queue again.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `.github/workflows/release.yml:58-73`, when inspected, the commented-out `x86_64-apple-darwin` block is either RESTORED (with a fix for the runner queue blocking) or REMOVED with a dated "v0.3.0 decision — restore after Apple Dev secrets land" comment referencing the Apple signing memory
- [ ] If RESTORED: the `Publish GitHub Release` job's `needs:` is refactored to NOT wait for all matrix legs — only Linux hard-required legs gate the publish; macOS + Windows results attach opportunistically via a post-publish `gh release upload` step that skips if the artifact is missing
- [ ] If REMOVED: a new section in `docs/release-runbook.md` titled "Supported release targets" lists what's shipping, what's not, and what would restore the dropped targets (for Intel Mac: Apple Dev secrets provisioning)
- [ ] Given either path, when a v0.2.1 release is tagged, then the `Publish GitHub Release` job completes within 20 min of the 2 Linux legs finishing (no blocking on Intel Mac runner queue)
- [ ] Given `actionlint .github/workflows/release.yml` runs, then it passes
- [ ] Given the aarch64-apple-darwin entry (currently also `continue-on-error: true` pending Apple secrets), when reviewed, then it follows the same resolution as Intel Mac (consistent posture across both macOS legs)
- [ ] Unhappy path: if the refactor proves risky (matrix dependency logic is fragile), fall back to the REMOVED path with a hard commitment to restore both macOS legs in v0.3.0's release cut

#### US-009: Upgrade GitHub Actions away from Node.js 20
**Description:** As a maintainer, I want zero Node.js 20 deprecation warnings in CI, so that the forced Node.js 24 migration on 2026-06-02 and the Node.js 20 removal on 2026-09-16 don't surprise me in the middle of a release.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given all workflow files under `.github/workflows/`, when audited for `uses:` lines, then every action is either: (a) at a version that supports Node.js 24 natively, or (b) pinned with a short-term `env: { FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: 'true' }` and a comment tracking the upstream release timeline
- [ ] Given `actions/checkout@v4`, when the current latest tag is checked, then it is bumped to `@v5` (or the current Node.js 24 major) across all workflows
- [ ] Given `dorny/paths-filter@v3`, when the current latest tag is checked, then it is bumped or the `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` env override is applied with a comment
- [ ] Given `ilammy/msvc-dev-cmd@v1`, same treatment
- [ ] Given a fresh CI run after the changes, when the annotations are inspected, then zero "Node.js 20 actions are deprecated" warnings appear
- [ ] Given `actionlint .github/workflows/*.yml` runs, then it passes (actionlint will flag invalid action pins)
- [ ] Unhappy path: if an action has no Node.js 24 release available AND the FORCE_JAVASCRIPT flag does not work (action uses native Node.js 20 APIs), file an upstream issue on the action's repo and document the blocker in a comment; the maintainer keeps the current version until upstream ships a fix

#### US-010: Close residual v0.2.0 review notes
**Description:** As an auditor re-reading the v0.2.0 status.json, I want every `review_note` field to be either ACTIONED (with a linked v0.2.1 story) or EXPLICITLY CLOSED (with a rationale), so that a fresh audit doesn't keep re-discovering the same CONSIDER items.

**Priority:** P2
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `tasks/prd-v0.2.0-release-hardening-status.json` US-003 review note ("CONSIDER cf-cache-status smoke assertion"), when updated, then the note is replaced with "Addressed by prd-v0.2.1-release-hardening US-002 (regression guard) + US-004 (smoke test)"
- [ ] Given US-009 review note ("CONSIDER `cp -R` → `ditto --rsrc`"), when updated, then the note is replaced with "Deferred to v0.3.0 macOS release cut — `cp -R` is adequate for the current PaneFlow.app layout (no nested frameworks with resource forks). Reopen if bundle gains Swift dylibs or embedded frameworks."
- [ ] Given US-010 review note ("CONSIDER msiexec exit 1618 handling"), when updated, then the note is replaced with "Deferred to v0.3.0 Windows release cut — requires hardware-validated test case. Track as WIN-001 in the v0.3.0 port PRD."
- [ ] Given the edits, when `jq . tasks/prd-v0.2.0-release-hardening-status.json` runs, then it parses cleanly with no syntax errors
- [ ] Given the v0.2.0 PRD itself, when the status field is re-computed, then it remains `DONE` (no stories reopened, only notes cleared)
- [ ] Unhappy path: if any of the three items are re-surfaced by an external audit within v0.2.1's 1-week window, promote from "deferred" to an active v0.2.1 follow-up story

---

## Functional Requirements

- FR-01: Every `.rpm` artifact attached to a PaneFlow GitHub Release must have a detached GPG signature embedded in its header (verified by `rpm --checksig -v` returning `OK` on every signature component).
- FR-02: Every `.deb` package in the APT repo pool at `pkg.paneflow.dev/apt/pool/main/p/paneflow/` must be referenced by a non-empty `Packages` file under `dists/stable/main/binary-<arch>/` for every architecture present (`amd64` and `arm64` for v0.2.1).
- FR-03: The `repo-publish.yml` workflow must trigger automatically within 30 seconds of a successful `release.yml` completion on a tag push, without any maintainer action beyond `git push origin vX.Y.Z`.
- FR-04: CI must run an end-to-end install smoke test on Ubuntu 24.04 (apt) and Fedora 40 (dnf) after every release publish; the smoke test must assert the installed binary reports the expected version string.
- FR-05: The operator runbook must describe the exact `gh secret set < file` incantation for multi-line secrets and warn against the pipe-based alternative.
- FR-06: CI must run `cargo-deny check advisories licenses sources` on every PR and main push; findings must surface in the job summary. Active RustSec vulnerabilities must fail the step; other checks run in warn-only mode.
- FR-07: `Reveal in File Manager` on Windows must invoke `explorer.exe` with the `/select,<path>` token as a single argument, not as two separate arguments.
- FR-08: Every workflow file must use actions at versions that support Node.js 24 (native or via `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` override) before 2026-06-02.

## Non-Functional Requirements

- **Performance:** `repo-publish.yml` auto-trigger latency P95 ≤ 30 seconds from `release.yml` success. Smoke-test CI step total duration ≤ 5 minutes (both distros combined).
- **Reliability:** `rpm --checksig` returns `OK` on 100% of published `.rpm` artifacts. Post-publish smoke test passes on 100% of releases unless a real install regression has landed.
- **Security:** All third-party GitHub Actions use versions that support Node.js 24 by 2026-06-02. No PAT introduced for workflow chaining. `cargo-deny` vulnerabilities check blocks any RustSec advisory on direct or transitive dep.
- **Observability:** Every CI job (release.yml, repo-publish.yml, ci.yml) produces a structured job summary that surfaces the top-level pass/fail of each matrix leg and every security finding with severity.
- **Compatibility:** Every code change must compile on all 6 targets (x86_64 + aarch64 × linux-gnu + apple-darwin + pc-windows-msvc) without regression.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User/Operator Message |
|---|----------|---------|-------------------|-----------------------|
| 1 | rpm signing spike finds unfixable root cause | Story 1 blocked | Fall back to `--define` CLI flags; if still broken, revert to `gpgcheck=0` and downgrade to P1 v0.2.2 | Commit message documents fallback; release notes flag unsigned rpms for v0.2.1 only |
| 2 | workflow_run fires but parent was re-run manually | `workflow_dispatch` re-runs release.yml | `if: workflow_run.event == 'push'` filters this out — only tag-push re-triggers repo-publish | Silent skip (expected) |
| 3 | workflow_run depth exceeds 3 levels | Hypothetical — doesn't apply today | Chain breaks; workflow simply doesn't fire | N/A (not reached in v0.2.1) |
| 4 | Smoke test `podman run` fails on runner image drift | GitHub updates ubuntu-22.04 image | Fall back to `docker run`; both are preinstalled | Smoke step retries once |
| 5 | cargo-deny RustSec DB fetch rate-limited | GitHub mirror overloaded | Step retries 3× with exponential backoff, then fails with DB-fetch-specific error | Job summary shows "advisory DB unreachable — retry later" |
| 6 | cargo-deny finds a real vulnerability in a transitive dep | New RustSec advisory matches a dep | `advisories.vulnerabilities` fails the step; PR/push is blocked | GitHub check fails with RUSTSEC-YYYY-XXXX reference |
| 7 | Windows Explorer reveal still doesn't highlight file after story 7 fix | Unusual Windows 11 config (e.g., Explorer replaced by a 3rd-party shell) | The spawn exits non-zero OR the target is not highlighted | Toast: "Could not highlight file in Explorer — using parent directory fallback" |
| 8 | `paneflow --version` fails in a smoke container (GL/Vulkan init required) | Binary needs GPU and runner has no GPU | Run `paneflow --help` or `paneflow --print-version` as a lighter-weight check instead | Smoke test uses a minimal invocation that doesn't need a display |
| 9 | Regression guard (story 2) curl to pkg.paneflow.dev returns stale cache | Cloudflare cache not purged within 30s | Retry loop in the step waits up to 60s before failing | Step logs: "Cloudflare cache not fresh — retrying" |
| 10 | Node.js 24 upgrade breaks a pinned action behavior | Action rewrites internals in the Node 24 port | Pin back to a known-good version OR file upstream issue + use `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=false` temporarily | Commit message explains the regression + upstream link |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|-------------|--------|------------|
| 1 | Story 1 rpm spike finds a hard upstream bug with no CI-workable fix | Medium (rpm #3740 is still open upstream) | High (v0.2.1 ships with `gpgcheck=0` again, security debt carried forward) | Time-box spike to 4 hours; fallback paths documented; acceptable to downgrade to v0.2.2 if required |
| 2 | `workflow_run` event has a subtle edge case (e.g., cancelled parent still fires child) that causes a bad publish | Low (GitHub docs say `types: [completed]` + `conclusion == 'success'` is the canonical guard) | Medium (bad repo state) | Double-guard: `conclusion == 'success' && event == 'push'` filters re-runs; test on a pre-release tag first |
| 3 | cargo-deny surfaces a vulnerability on day 1 that requires dep bumping | Medium (GPUI pulls in many transitive deps) | Medium (delays v0.2.1 until the advisory is patched) | Fail-forward posture means warnings; vulnerabilities will show but not auto-block until we flip severity; stage a hot-patch if a real advisory appears |
| 4 | Node.js 24 action upgrades introduce a subtle behavior change (e.g., checkout fetch depth) | Low (semver expectations on major actions) | Low (easy revert) | Land each bump in a separate commit; test CI on a branch before merge; keep `@v4` pins as fallback in git history |
| 5 | Smoke test false positives due to Cloudflare cache lag | Medium (observed 30-60s max in v0.2.0) | Low (smoke retries; doesn't block publish) | Sleep 60s between repo-publish end and smoke start; retry up to 3x |
| 6 | Intel Mac refactor (story 8 RESTORED path) introduces race in Publish job dependencies | Low | Medium (bad release state) | Prefer the REMOVED path for v0.2.1; defer RESTORED to v0.3.0 when signing secrets are in place and the full macOS path is exercised |
| 7 | `gh secret set` quirk doc (story 5) not discovered by future maintainer because they don't read docs | Medium | Low (they rediscover via v0.2.0 status.json or CI error) | Cross-link from the CI error message itself: update the GPG signing step in release.yml to suggest the file-redirect method in its error output |
| 8 | Hardware smoke test for story 7 Windows fix cannot be performed | High (no Windows CI with UI tests) | Medium (ships unverified) | Mark story 7 `IN_REVIEW` until Windows hardware smoke; block merge to main for that story only |
| 9 | v0.2.1 ships with story 1 still P1-deferred | Low | Medium (rpm signing debt persists) | Accept as known limitation; story remains P0 in v0.2.2; document in release notes |
| 10 | workflow_run `head_branch` does not populate as expected for tag-push events | Low (GitHub docs contradict a few community posts on this) | Medium (tag unknown → wrong artifacts) | Spike: trigger a workflow_run on a test tag, log `github.event.workflow_run.head_branch` to verify; fall back to parsing from `workflow_run.artifacts_url` if needed |

## Non-Goals

Explicit boundaries — this PRD does NOT include:

- **Shipping the first signed macOS or Windows release.** Stories 7 + 8 touch that code but stop short of actually publishing macOS/Windows artifacts for v0.2.1. That's v0.3.0's scope.
- **Provisioning Apple Dev secrets or Azure Trusted Signing.** Out of scope; tracked in the macOS/Windows release PRDs.
- **Bumping the GPUI pin.** Frozen for this release train.
- **Migrating away from reprepro to a different APT repo tool.** reprepro works; the arch-blind bug was in our wrapper script, not reprepro itself.
- **Rewriting the release runbook from scratch.** Story 5 and story 8 add sections; story 3 removes a section. Full runbook rewrite deferred to v0.3.0 when the macOS/Windows paths land.
- **Setting up a release announcement process.** Not a release-engineering concern — that's a v0.3.0 / v1.0 marketing concern.
- **Automating version bumps via a tool like `cargo-release` or `cliff`.** Manual bump in 3 files remains the process for v0.2.1. Automation considered for v0.3.0.

## Files NOT to Modify

- `Cargo.toml` `[patch.crates-io]` block (async-task, calloop) — load-bearing for GPUI.
- `src-app/src/main.rs` bootstrap top — out of scope; no code path in EP-001 or EP-002 touches it.
- `tasks/prd-linux-packaging-migration.md` — historical PRD (v0.2.0 predecessor), do not edit text.
- `tasks/prd-v0.2.0-release-hardening.md` — historical PRD (DONE), do not edit text; only the companion status.json is updated by US-010.
- `GPUI source (external git dep)` — frozen.
- `assets/io.github.arthurdev44.paneflow.metainfo.xml` `<release>` entries for v0.2.0 and earlier — only the new v0.2.1 entry is appended by the final version-bump commit.
- `.github/workflows/release.yml` GPG signing section lines 213-402 — these are correctness-tested by v0.2.0 shipping; only the rpmsign block (story 1) and the matrix entry (story 8) are touched.

## Technical Considerations

Framed as questions for engineering input:

- **Architecture (US-001 rpmsign):** GPG agent warming via `gpg --list-secret-keys` after `gpg --batch --import` is the research-recommended path. If the spike finds a different root cause (e.g., rpm tries to read from TTY for something other than pinentry), the fallback ranking is: (a) `rpm --addsign` legacy alias, (b) `--define` CLI-only invocation, (c) disable per-package signing, revert to `gpgcheck=0`. Recommendation: take path (a) → (b) → (c) in order, time-boxed to 4 hours per step.
- **API Design (US-003 workflow_run):** `workflow_run: workflows: ["Release"] types: [completed]` is the canonical chain pattern. Decision on `types:`: use `[completed]` + guard on `conclusion == 'success'` (what the research recommends), NOT `[success]` (which doesn't exist as an event type). Engineering to verify the exact enum values on a pre-release test tag.
- **Dependencies (US-006 cargo-deny):** `cargo-deny 0.14.x` is the current stable. Pin the version in CI (`cargo install cargo-deny --locked --version 0.14.x`) to avoid surprise-deny-ing on a new release. Consider caching `~/.cargo/advisory-db` via `actions/cache` separately from the Swatinem rust-cache to avoid fetching the DB on every run.
- **Platform dispatch (US-007 Windows arg concat):** `OsString::from("/select,"); flag.push(path.as_os_str()); Command::new("explorer").arg(flag)` is the canonical Windows pattern (per the `std::process::Command` docs on Windows argument parsing). Engineering to verify on a real Windows 11 host that the target file is highlighted, not just the parent directory opened.
- **Matrix strategy (US-008 Intel Mac):** The REMOVED path is lower-risk for v0.2.1. The RESTORED path requires re-architecting the `Publish GitHub Release` job's `needs:` to not block on best-effort legs, which is a non-trivial refactor. Engineering recommendation: REMOVE for v0.2.1, RESTORE in v0.3.0 alongside Apple Dev secrets provisioning.
- **Ordering (US-004 smoke depends on US-003 chain):** US-004's smoke test must run AFTER repo-publish completes (R2 sync + Cloudflare purge). US-003's workflow_run chain is the enabling primitive. Implement US-003 first, then US-004.

## Success Metrics

| Metric | Baseline (v0.2.0) | Target (v0.2.1) | Timeframe | How Measured |
|--------|-------------------|-----------------|-----------|--------------|
| RPM signature verification pass rate | 0% (gpgcheck=0) | 100% | v0.2.1 ship date | `rpm --checksig -v paneflow-0.2.1-1.x86_64.rpm` reports OK |
| Manual steps per release (post-tag) | 1 (`gh workflow run repo-publish.yml`) | 0 | v0.2.1 ship date | Count of maintainer commands between `git push origin vX.Y.Z` and "release live at pkg.paneflow.dev" |
| End-to-end CI smoke coverage | 0 distros | 2 distros (Ubuntu + Fedora) | v0.2.1 ship date | `.github/workflows/repo-publish.yml` `smoke-post-publish` job exists and runs on release |
| Advisory DB refresh frequency | N/A (no scanning) | Every PR + every main push | v0.2.1 ship date | `ci.yml` `security-audit` job exists and runs |
| Node.js 20 deprecation warnings per CI run | 3 (checkout, paths-filter, msvc-dev-cmd) | 0 | v0.2.1 ship date | GitHub Actions run annotations |
| Residual v0.2.0 review notes | 3 open | 0 open | v0.2.1 ship date | `jq '.stories[].review_note' tasks/prd-v0.2.0-release-hardening-status.json` returns only "DONE — see v0.2.1" notes |
| Workflow_run chain latency (release→repo-publish) | N/A (manual) | P95 ≤ 30s | first v0.2.2+ release | Timestamp delta between release.yml completion and repo-publish.yml start |

## Open Questions

- **US-001 spike outcome:** What's the actual rpm root cause? Is the 4-hour budget enough? Decision needed at the end of the spike: accept, fall back, or defer.
- **US-003 workflow_run `head_branch`:** Does it populate as the tag name for a tag-push parent run? Community posts are split. Verify on a pre-release tag before merging.
- **US-004 smoke `paneflow --version`:** Does the binary require GPU to print the version? If yes, use `--print-version` (needs implementation) or `paneflow --help` as a lighter invocation. Engineering to verify during implementation.
- **US-006 cargo-deny `vulnerabilities` severity:** Initial posture: let the `advisories.vulnerabilities` check fail the step (no warn-mode available upstream). If the current lockfile has an active advisory, v0.2.1 ships with a patched dep or defers US-006 to v0.2.2.
- **US-008 Intel Mac:** REMOVED (recommended, v0.2.1) vs RESTORED (deferred v0.3.0). Decision: confirm REMOVED path is acceptable to the maintainer (me) at implementation time; if RESTORE is preferred, re-scope to L+ with a refactor story.
- **US-009 action Node.js 24:** Do all three actions (checkout, paths-filter, msvc-dev-cmd) have a Node.js 24 release available at implementation time? If not, which ones need the `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24=true` fallback?

[/PRD]
