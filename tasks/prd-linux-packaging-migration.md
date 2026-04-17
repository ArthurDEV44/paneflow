[PRD]
# PRD: Linux Packaging Migration — `.run` → Phased Multi-Format Distribution

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-17 | Claude + Arthur | Initial draft — phased migration from single `.run` installer to tar.gz/deb/rpm/AppImage + transverse updater refactor |

## Problem Statement

PaneFlow currently ships as a single `paneflow-<version>-x86_64-linux.run` self-extracting bash script. Every released version to date (v0.1.0–v0.1.7) has used this format exclusively. Three concrete problems block reach and trust:

1. **`.run` installers are industry-rejected for GUI desktop apps.** They skip the system package manager, provide no dependency resolution, leave no uninstall manifest, don't wire into update-alternatives, and trigger AppArmor/SELinux friction. Cursor and Zed both demonstrate that serious Linux-first projects either ship distro-native packages (deb/rpm/AppImage) or a minimal `tar.gz` extracted to `~/.local`. PaneFlow does neither — the `.run` writes to `~/.local/bin/` via an opaque bash extraction step users cannot audit at a glance.

2. **The in-app updater is tightly coupled to the `.run` format and breaks on immutable distros.** `src-app/src/update_checker.rs:99` matches `"-x86_64-linux.run"` as the asset filename suffix — a single-format, single-arch assumption hardcoded into the asset selector. `src-app/src/self_update.rs:86` spawns the downloaded `.run` binary directly, which works on mutable Ubuntu but will fail or silently corrupt on Fedora Silverblue, openSUSE MicroOS, and SteamOS (read-only `/usr`). Users on those distros cannot update from within the app at all.

3. **Ubuntu/Debian and Fedora/RHEL users have no native package.** The Linux desktop user base is ~60% Ubuntu-family and ~25% Fedora-family (combined Debian+Ubuntu+Mint vs Fedora+RHEL+Rocky+openSUSE based on Steam Hardware Survey 2026 and Linux Foundation data). Every one of these users sees "`.run` file" on the download page and bounces, because their muscle memory is `sudo apt install` or `sudo dnf install`. Cursor shipping .deb/.rpm/AppImage (even with their publicly-broken .deb incident) beats PaneFlow's `.run`-only page on pure funnel conversion.

**Why now:** The app just reached v0.1.7 with a solid functional baseline (19 stories delivered across the v2-gpui-terminal and v2-title-bar PRDs). Every day PaneFlow ships exclusively as `.run`, the website converts worse than it could and immutable-distro users silently can't update. The cost of delaying the packaging refactor compounds — more releases shipped as `.run` = more users stuck on old versions with no smooth upgrade path once the format changes. Migrating before v0.2.0 is materially easier than after.

## Overview

This PRD delivers a phased migration from `.run`-only distribution to multi-format Linux packaging, with a transverse updater refactor that decouples from the `.run` format and correctly handles all target distributions (including immutable ones).

**Phase 1 (P0 — ship first)** replaces the `.run` with three x86_64 artifacts produced by the same CI run:
- `.tar.gz` — Zed-style baseline extracted to `~/.local/paneflow.app/` (works everywhere, including Silverblue/SteamOS)
- `.deb` — native Ubuntu/Debian/Mint package built via `cargo-deb` 3.6.3 with explicit dependencies (no `$auto` — issue #170)
- `AppImage` — built via `linuxdeploy` (not `appimagetool` direct — PaneFlow has shared-lib deps), with `gh-releases-zsync` update-information embedded

**Phase 2 (P1 — on demand)** adds `.rpm` x86_64 via `cargo-generate-rpm` 0.20.0 and stands up hosted APT + RPM repositories (Cloudflare R2 + reprepro + createrepo_c, GPG-signed), with a postinst script that auto-adds `pkg.paneflow.dev` on first install. This gives Ubuntu/Debian/Fedora/RHEL users automatic `apt upgrade` / `dnf upgrade` on every release — the exact pattern Cursor uses, but with the postinst thoroughly tested against a clean Ubuntu VM to avoid their broken-`apt update` incident.

**Phase 3 (P2 — gated by spike)** validates that GPUI commit `0b984b5` builds on `aarch64-unknown-linux-gnu` (no public evidence of an aarch64 Zed build exists today) and, if it does, extends the CI matrix to produce all three formats for ARM64 via native `ubuntu-22.04-arm` runners (no QEMU, no `cross` — the sysroot complexity for a Vulkan-dependent binary is not worth the emulation cost).

The **transverse updater refactor (EP-002)** runs alongside Phase 1 and is as important as the packaging itself. It replaces the hardcoded `.run` suffix matcher with a runtime install-method detector (binary path heuristic: `/usr/bin/*` → system package, `/tmp/.mount_*` → AppImage, `~/.local/paneflow.app/*` → tar.gz), an asset-selection matcher that picks the right format+arch from the GitHub release, and format-specific update flows that never write to `/usr`. For deb/rpm users the in-app update button is disabled with a hint to `apt`/`dnf`; for AppImage it invokes `appimageupdatetool` (zsync delta, ~10–30% file size); for tar.gz it performs atomic swap in `~/.local/paneflow.app/`.

Key decisions:
- **Phased, not all-at-once.** Shipping 6 artifacts day-1 like Cursor = 2× the CI time and 2× the release-signing surface before the underlying updater is rewritten. Ship Phase 1 first, validate in production, then add Phase 2/3.
- **Explicit `depends`, never `$auto`.** Cargo-deb `$auto` recursively pulls transitive shared-lib deps (issue #170), producing `.deb` files that are over-specified and fragile. We declare the 8-dep minimum set explicitly.
- **Vulkan ICDs NEVER bundled in AppImage.** GPU drivers are vendor-specific (`libvulkan_radeon.so`, `libvulkan_intel.so`, `nvidia_icd.json`) — bundling them breaks on mismatched GPUs. The loader (`libvulkan.so.1`) is bundled; ICDs come from the host `/usr/share/vulkan/icd.d/`.
- **Phase 3 is spike-gated.** If GPUI doesn't build on aarch64, Phase 3 dies cleanly in the spike story — no wasted work on CI matrix expansion or hardware testing.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Formats available | 3 (tar.gz + deb + AppImage, x86_64) | 6 (all 3 formats × x86_64 + aarch64) + hosted APT/RPM repo |
| Download page conversion | No `.run`, 3 visible formats matching Cursor layout | 6 artifacts + "Add our repo" tab with copy-pasteable snippet |
| Update flow coverage | 100% of Phase 1 formats have a working update path (or clear hint to system pm) | Users on deb/rpm auto-update via `apt`/`dnf`; AppImage uses zsync delta; tar.gz atomic swap |
| Immutable-distro support | Updater never writes to `/usr` — works on Silverblue, SteamOS, MicroOS | Documented installation path for all 3 immutable distros |
| Release pipeline time | <15 min to produce all 3 Phase 1 artifacts from a tag push | <25 min to produce all 6 artifacts including aarch64 |

## Target Users

### Ubuntu/Debian/Mint Desktop User
- **Role:** Linux developer or power user on an Ubuntu-family distribution
- **Behaviors:** Installs apps via `apt`, expects `.deb` files on vendor download pages. Uses software like VS Code, Chrome, Discord — all of which ship `.deb`.
- **Pain points:** PaneFlow's download page offers only a `.run` bash installer. No indication of how to uninstall. Running a shell script from a GitHub release raises security concerns that `apt install` does not.
- **Current workaround:** Either skips installing PaneFlow, or runs the `.run` and manually removes files later when they want to clean up.
- **Success looks like:** `sudo apt install paneflow` after a one-time `curl | sudo tee` of the repo, then `apt upgrade` handles everything forever.

### Fedora/RHEL/Rocky/openSUSE User
- **Role:** Linux developer on an RPM-based distribution
- **Behaviors:** Uses `dnf` or `zypper`. Strongly prefers RPM over alternatives because of SELinux integration and proper dependency tracking.
- **Pain points:** Same as above but with `dnf` muscle memory. AppImage works but doesn't integrate with desktop environment by default; many users see AppImages as "weird."
- **Current workaround:** Installs via the `.run`, or uses AppImage once ready (Phase 1), or simply doesn't adopt PaneFlow.
- **Success looks like:** `sudo dnf install paneflow` with auto-updates via `dnf upgrade`.

### Immutable-Distro User (Fedora Silverblue, openSUSE MicroOS, SteamOS)
- **Role:** User running a read-only rootfs distribution for reliability/security
- **Behaviors:** Cannot write to `/usr` or `/bin`. Installs GUI apps via Flatpak, AppImage, or `~/.local`-scoped tools. Never runs privileged installers that target `/usr`.
- **Pain points:** Current `.run` installer writes to `~/.local/bin/paneflow` (fine), but the in-app updater re-invokes the same script which assumes a mutable layout. Updates silently fail or corrupt state.
- **Current workaround:** Download AppImage from GitHub, update manually. Or use the `.tar.gz` from a GitHub release attached as a supplementary asset (doesn't exist yet).
- **Success looks like:** `.tar.gz` to `~/.local/paneflow.app/` works on first install AND updates; AppImage with `--appimage-extract-and-run` fallback when FUSE 2 is missing.

### ARM64 User (Asahi Linux on Apple Silicon, Raspberry Pi 5, AWS Graviton)
- **Role:** Linux user on ARM64 hardware — growing segment in 2026
- **Behaviors:** Downloads the "aarch64" variant from every vendor's Linux page. Reads release notes carefully because ARM64 Linux support is often half-finished.
- **Pain points:** PaneFlow has zero aarch64 presence. Cannot even test whether it runs.
- **Current workaround:** None — skips PaneFlow.
- **Success looks like:** `paneflow-v0.2.0-aarch64.deb` on the download page (Phase 3).

## Research Findings

Key findings that informed this PRD. Full research in `research_linux_packaging.md` (project memory).

### Competitive Context
- **Zed:** Same GPUI stack as PaneFlow. Ships `.tar.gz` only from their own site. [Explicitly documented](https://zed.dev/docs/linux). Community `.deb` at [lucasliet/zed-deb](https://github.com/lucasliet/zed-deb). Lesson: minimum-viable distribution is a tar.gz + install.sh.
- **Cursor:** Electron-based. Ships 6 artifacts (deb/rpm/AppImage × x64/aarch64) because electron-builder generates them for free. BUT: [their `.deb` broke `apt update`](https://forum.cursor.com/t/installing-cursor-via-apt-deb-package-breaks-apt-update/132008), and [lagged AppImage by months](https://github.com/cursor/cursor/issues/3550). Lesson: multi-format is desirable, but the postinst that auto-adds the APT source is the fragile part.
- **Market gap:** A Rust/GPUI terminal app that ships the full matrix with a working updater for every format including immutable distros. Not Zed (tar.gz-only), not Cursor (Electron-only pattern doesn't apply to Rust, broken postinst). PaneFlow can occupy this gap.

### Best Practices Applied
- **Explicit `Depends:` in cargo-deb** — not `$auto` (issue #170 overcollects transitive deps that break on non-Debian deriviatives). PaneFlow declares 8 deps: `libc6, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libvulkan1, libfontconfig1, libfreetype6`.
- **`linuxdeploy` for AppImage (not `appimagetool` direct)** — PaneFlow has shared-lib deps (Vulkan loader, Wayland, libxcb); `appimagetool` direct is only viable for statically-linked musl binaries.
- **Never bundle Vulkan ICDs** — GPU drivers are vendor-specific; bundle the loader (`libvulkan.so.1`), let the host resolve `/usr/share/vulkan/icd.d/`.
- **Install-method detection at runtime** — updater checks the binary path (`/usr/bin/*` vs `/tmp/.mount_*` vs `~/.local/*`) to pick the update strategy. No config file, no environment variable — self-describing state.
- **Native ARM64 runner over `cross`** — `ubuntu-22.04-arm` on GitHub Actions is available and avoids QEMU/sysroot complexity for GPU apps (no GPU on any CI runner anyway, but native linker + sysroot eliminates entire classes of cross-compile bugs).

*Full research sources available in `research_linux_packaging.md`.*

## Assumptions & Constraints

### Assumptions (to validate)
- **GPUI commit `0b984b5` builds on `aarch64-unknown-linux-gnu`** — no public evidence found. Phase 3 is gated by spike story US-018. If the spike fails, Phase 3 is cancelled and re-scoped after a GPUI rev bump. Risk: HIGH.
- **`ubuntu-22.04-arm` runners remain available on GitHub Actions under current pricing tier** — confirmed available April 2026, but billing terms change. Risk: LOW.
- **Cloudflare R2 egress stays within free tier (10 GB/month)** — with current download volume (~500/month × 40 MB average artifact), this is within free tier. Risk: LOW unless volume grows 50×.
- **Users on immutable distros want `.tar.gz` over AppImage** — based on Fedora Silverblue documentation recommendations and anecdotal community posts. Not formally surveyed. Risk: MEDIUM — AppImage with `--appimage-extract-and-run` is an equivalent fallback.

### Hard Constraints
- **Linux-only.** No macOS or Windows packaging work. The existing CLAUDE.md explicitly says this.
- **GPUI stays pinned** at `0b984b5` throughout this PRD. No upstream rev bumps during packaging work.
- **MIT license** declared in Cargo.toml must match the restored LICENSE file at the repo root.
- **No `$auto`** for cargo-deb dependencies (issue #170).
- **Binary name stays `paneflow`** — do not rename to `paneflow-app` or anything else; `[[bin]]` in `src-app/Cargo.toml` defines this.
- **Installer never writes to `/usr`** from user context. `.deb` and `.rpm` install to `/usr` at `dpkg`/`rpm` time (privileged) — that's fine. The *in-app updater* must not.

## Quality Gates

These commands must pass for every user story:

- `cargo check --workspace` — workspace compiles
- `cargo clippy --workspace -- -D warnings` — zero lint warnings
- `cargo fmt --check` — formatting correct
- `cargo test --workspace` — tests pass (mostly paneflow-config crate)

For packaging stories (EP-001, EP-003, EP-004), additional gates:
- `cargo deb -p paneflow-app --target x86_64-unknown-linux-gnu` succeeds
- `dpkg -I target/x86_64-unknown-linux-gnu/debian/*.deb` shows correct metadata
- `dpkg -c target/x86_64-unknown-linux-gnu/debian/*.deb` shows expected file layout
- `lintian --suppress-tags bad-distribution-in-changes-file target/x86_64-unknown-linux-gnu/debian/*.deb` reports no errors
- For AppImage: `file paneflow-*.AppImage` reports `ELF` and the binary runs `--version` in isolation

For updater stories (EP-002), additional gate:
- Manual verification: launch binary from `/usr/bin/paneflow` (simulated via symlink), from `/tmp/.mount_paneflow_abc123/paneflow` (simulated), and from `~/.local/paneflow.app/bin/paneflow` (actual install) — verify the correct update strategy is selected in each case

## Epics & User Stories

### EP-001: Phase 1 — Multi-format packaging (x86_64)

Replace the `.run` installer with three x86_64 artifacts (tar.gz + deb + AppImage) produced by the same CI run. Restore the LICENSE file and author AppStream metainfo as prerequisites.

**Definition of Done:** A tag push to `v0.2.0-alpha.1` (or similar) produces three artifacts attached to the GitHub release: `paneflow-<version>-x86_64.tar.gz`, `paneflow-<version>-x86_64.deb`, `paneflow-<version>-x86_64.AppImage` (+ its `.zsync`). The `.run` is no longer produced and its build scripts are removed.

#### US-001: Restore LICENSE file and enrich package metadata

**Description:** As the PaneFlow maintainer, I want a LICENSE file at the repo root and complete package metadata in Cargo.toml so that `cargo-deb`, `cargo-generate-rpm`, AppStream validators, and Linux distro inclusion review criteria all find what they need without manual file assembly.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `LICENSE` file exists at `/home/arthur/dev/paneflow/LICENSE` with the standard MIT license text and copyright line `Copyright (c) 2025 Arthur Jean`
- [ ] Root workspace `Cargo.toml` adds `description`, `repository`, `homepage`, `keywords`, `categories` fields at `[workspace.package]`
- [ ] `src-app/Cargo.toml` inherits the workspace metadata and adds `readme = "../README.md"`
- [ ] Given no LICENSE exists (deleted), when running `cargo deb -p paneflow-app --target x86_64-unknown-linux-gnu`, then the build fails with a clear error mentioning the missing license file (unhappy path: tool fails loudly, not silently with a bad package)
- [ ] `cargo package --list -p paneflow-app` lists `LICENSE` and `README.md` in the crate package

#### US-002: Author AppStream metainfo XML

**Description:** As a Linux desktop user browsing GNOME Software or KDE Discover, I want PaneFlow to show rich metadata (description, screenshots, release notes, content rating) so that I can evaluate it the same way I evaluate any native desktop app.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `assets/io.github.arthurdev44.paneflow.metainfo.xml` exists and validates with `appstreamcli validate` (zero errors)
- [ ] Metainfo includes: `<id>`, `<name>`, `<summary>`, `<description>`, `<url type="homepage">`, `<url type="bugtracker">`, `<launchable type="desktop-id">paneflow.desktop</launchable>`, `<releases>` with entry for current version
- [ ] `<content_rating type="oars-1.1" />` present with no non-default values (PaneFlow has no violence, nudity, gambling, etc.)
- [ ] `.deb` and AppImage both install the metainfo to `/usr/share/metainfo/` (via cargo-deb `assets` and linuxdeploy, respectively)
- [ ] Given an invalid metainfo (e.g., malformed `<releases>`), when CI runs `appstreamcli validate`, then CI fails (unhappy path: validation prevents ship)

#### US-003: Configure `[package.metadata.deb]` and produce .deb artifact

**Description:** As an Ubuntu/Debian/Mint user, I want a `.deb` package that installs cleanly via `sudo apt install ./paneflow-*.deb` with correct desktop integration so that I can use PaneFlow like any native app.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-002

**Acceptance Criteria:**
- [ ] `src-app/Cargo.toml` contains a complete `[package.metadata.deb]` section with `maintainer`, `copyright`, `license-file`, `section = "utils"`, `priority = "optional"`, `extended-description`, `revision = "1"`, and explicit `depends` (never `$auto`): `libc6, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libvulkan1, libfontconfig1, libfreetype6`
- [ ] `assets` list covers the binary (`/usr/bin/paneflow`, mode 755), desktop entry (`/usr/share/applications/paneflow.desktop`, 644), all 6 icon sizes (`/usr/share/icons/hicolor/{16,32,48,128,256,512}x{...}/apps/paneflow.png`, 644), AppStream metainfo (`/usr/share/metainfo/io.github.arthurdev44.paneflow.metainfo.xml`, 644), and LICENSE as `/usr/share/doc/paneflow/copyright`
- [ ] `cargo deb -p paneflow-app --target x86_64-unknown-linux-gnu` produces a `.deb` under `target/x86_64-unknown-linux-gnu/debian/`
- [ ] `lintian target/x86_64-unknown-linux-gnu/debian/*.deb` produces no errors (warnings acceptable if justified)
- [ ] In a fresh Ubuntu 22.04 container, `sudo apt install ./paneflow-*.deb` succeeds; `paneflow --version` prints the version; `sudo apt remove paneflow` removes all installed files cleanly
- [ ] Given `.deb` is installed over a previous version, when running `sudo apt install ./paneflow-newer.deb`, then upgrade completes without conflicts and `paneflow --version` reflects the new version (happy path: in-place upgrade works)
- [ ] Given the `.deb` depends on `libvulkan1` and the host is missing it, when running `sudo apt install ./paneflow-*.deb` on that host, then apt reports the missing dependency rather than installing a broken package (unhappy path: dependency resolution works)

#### US-004: Produce tar.gz artifact with Zed-style layout

**Description:** As a user on Fedora Silverblue / SteamOS / MicroOS (immutable root) or any distro without a native package, I want a `.tar.gz` I can extract to `~/.local/paneflow.app/` and run without root, matching the Zed distribution model.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] `scripts/bundle-tarball.sh` exists and produces `paneflow-<version>-x86_64.tar.gz`
- [ ] Tarball internal layout: `paneflow.app/bin/paneflow`, `paneflow.app/share/applications/paneflow.desktop`, `paneflow.app/share/icons/hicolor/{size}x{size}/apps/paneflow.png`, `paneflow.app/share/metainfo/io.github.arthurdev44.paneflow.metainfo.xml`, `paneflow.app/LICENSE`, `paneflow.app/README.md`, `paneflow.app/install.sh`
- [ ] The included `install.sh` extracts to `$HOME/.local/paneflow.app/`, symlinks `$HOME/.local/bin/paneflow -> $HOME/.local/paneflow.app/bin/paneflow`, copies the desktop file to `$HOME/.local/share/applications/` (with `Exec=` rewritten to the absolute symlinked path), copies icons to `$HOME/.local/share/icons/hicolor/`, and runs `gtk-update-icon-cache` / `update-desktop-database` best-effort
- [ ] Given `~/.local/paneflow.app/` already exists from a previous install, when running `install.sh`, then the existing directory is atomically swapped (move old to `.app.old`, extract new, remove `.app.old` on success)
- [ ] Given extraction fails mid-way (e.g., disk full), when the script exits, then `~/.local/paneflow.app.old` is preserved and the user sees an error pointing at the staging path (unhappy path: partial install is recoverable)
- [ ] Installing from tarball on Fedora Silverblue works — no writes to `/usr`, binary runs, desktop entry appears in GNOME

#### US-005: Produce AppImage artifact with linuxdeploy + zsync update-info

**Description:** As any Linux desktop user, I want a single-file AppImage that runs without installation and supports delta updates via zsync, so that I can try PaneFlow without committing to a package manager install.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-002

**Acceptance Criteria:**
- [ ] `packaging/AppRun` exists, sets `LD_LIBRARY_PATH` to the AppImage's bundled `usr/lib`, and does NOT bundle or set `VK_ICD_FILENAMES` (Vulkan ICDs come from host)
- [ ] CI builds AppImage via `linuxdeploy --appdir PaneFlow.AppDir --executable target/release/paneflow --desktop-file assets/paneflow.desktop --icon-file assets/icons/paneflow-256.png --custom-apprun packaging/AppRun --output appimage`
- [ ] Environment variable `UPDATE_INFORMATION="gh-releases-zsync|ArthurDEV44|paneflow|latest|paneflow-*-x86_64.AppImage.zsync"` is set before invoking `linuxdeploy` so the AppImage contains an embedded update-info string
- [ ] Produced artifacts: `paneflow-<version>-x86_64.AppImage` AND `paneflow-<version>-x86_64.AppImage.zsync` — both uploaded to the GitHub release
- [ ] `file paneflow-*.AppImage` reports `ELF 64-bit LSB executable, x86-64`
- [ ] `./paneflow-*.AppImage --version` prints the version on Ubuntu 22.04 (FUSE 2 available)
- [ ] Given FUSE 2 is missing (simulated by running on Ubuntu 24.04 without `libfuse2`), when the user runs `./paneflow-*.AppImage`, then the binary prints a clear error and a suggestion to use `./paneflow-*.AppImage --appimage-extract-and-run` (unhappy path: FUSE2 breakage has recovery path)
- [ ] AppImage size < 80 MB (bundling only shared deps, no Vulkan ICDs, no bloat)
- [ ] No file matching `libvulkan_*.so` or `nvidia_icd.json` exists in `PaneFlow.AppDir/usr/lib/` after linuxdeploy runs (regression guard: we do NOT ship GPU drivers)

#### US-006: Extend CI release workflow to build all 3 Phase-1 artifacts

**Description:** As the PaneFlow maintainer, I want `.github/workflows/release.yml` to produce all three Phase 1 artifacts from a single tag push so that cutting a release is a single `git tag && git push --tags`.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003, US-004, US-005

**Acceptance Criteria:**
- [ ] `release.yml` job matrix defines one row per target — Phase 1: `x86_64-unknown-linux-gnu` only. The workflow structure supports adding aarch64 later by adding a matrix entry (no structural rewrite required)
- [ ] Workflow installs `cargo-deb` 3.6.3 pinned, `cargo-generate-rpm` is NOT installed yet (Phase 2), `linuxdeploy` and `appimagetool` downloaded from their continuous releases
- [ ] Workflow produces and uploads: `paneflow-<tag>-x86_64.tar.gz`, `paneflow-<tag>-x86_64.deb`, `paneflow-<tag>-x86_64.AppImage`, `paneflow-<tag>-x86_64.AppImage.zsync`
- [ ] Workflow runs all Quality Gates commands before packaging (defense-in-depth against shipping broken builds)
- [ ] Workflow total time measured on `ubuntu-22.04`: <15 minutes end-to-end (from tag push to release assets visible)
- [ ] Given any single artifact build fails (e.g., `cargo deb` fails), when the workflow runs, then the entire job fails and no partial release is published (unhappy path: atomic release, no half-shipped tag)
- [ ] The workflow uses `softprops/action-gh-release@v2` to upload all artifacts to the GitHub release (existing pattern from `release.yml:55`)

#### US-007: Remove `.run` installer — scripts and CI cleanup

**Description:** As a user landing on the download page, I want only the modern artifact formats visible so that I'm not confused by a legacy `.run` option alongside the new packages.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] `scripts/make-installer.sh` is deleted
- [ ] `release.yml` no longer calls `make-installer.sh` and no longer produces `paneflow-*-x86_64-linux.run`
- [ ] `scripts/install-icons.sh` is either deleted (if subsumed by tarball `install.sh`) or kept with a header comment explaining its standalone purpose
- [ ] `scripts/install.sh` (if it exists as a dev-tool) is preserved or deleted with a git log note
- [ ] Existing GitHub releases v0.1.0–v0.1.7 are NOT modified (the `.run` files on those releases stay forever — users of old versions can still download them)
- [ ] README.md installation section is rewritten to reference the three new artifact types with copy-paste commands for each
- [ ] Given a user navigates to the `v0.2.0-alpha.1` release page, when they scroll to assets, then the `.run` file is NOT present and the three new files ARE present (happy path: clean cutover)
- [ ] Given a v0.1.x user still has `~/.local/bin/paneflow` from the old `.run` install (no `paneflow.app/` directory present), when their copy hits the new update-check on startup, then the installed binary logs a warning "legacy .run install detected — see README for migration" and the updater enters `InstallMethod::Unknown` state rather than attempting to download a `.run` that no longer exists (unhappy path: legacy users don't hit a dead URL)

---

### EP-002: Transverse updater refactor

Decouple the in-app updater from the `.run` format. Detect install method at runtime, pick format-specific update strategy, never write to `/usr`, handle FUSE 2 / network / integrity failures gracefully.

**Definition of Done:** After Phase 1 ships, a user who installed via `.deb` sees the in-app update button disabled with a hint; a user who installed via AppImage sees the button trigger a zsync delta update; a user who installed via tar.gz sees the button atomically swap `~/.local/paneflow.app/`. All three code paths are exercised by unit tests. No code path writes to `/usr`.

#### US-008: Runtime detection of install method by binary path

**Description:** As the in-app updater, I want to determine how PaneFlow was installed (deb/rpm/AppImage/tar.gz/unknown) by inspecting my own binary path at startup so that I can pick the right update strategy without reading any config or environment variable.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] New function `install_method::detect() -> InstallMethod` in `src-app/src/install_method.rs`
- [ ] `InstallMethod` enum has variants: `SystemPackage { manager: PackageManager }` (Apt | Dnf inferred from presence of `/etc/debian_version` or `/etc/fedora-release`), `AppImage { mount_point: PathBuf, source_path: PathBuf }`, `TarGz { app_dir: PathBuf }`, `Unknown`
- [ ] Detection rules (applied in order):
  - `/usr/bin/paneflow` or `/usr/local/bin/paneflow` → `SystemPackage`
  - Path matches `/tmp/.mount_*/**/paneflow` (and `APPIMAGE` env var set) → `AppImage` with `source_path` from `$APPIMAGE`
  - Path matches `$HOME/.local/paneflow.app/**/paneflow` → `TarGz`
  - Otherwise → `Unknown`
- [ ] Detection uses `std::env::current_exe()` and canonicalizes the result with `std::fs::canonicalize`
- [ ] Unit test covers each variant with a mocked binary path
- [ ] Given `current_exe()` returns a symlink (e.g., `~/.local/bin/paneflow` pointing at `~/.local/paneflow.app/bin/paneflow`), when detection runs, then the symlink is resolved and `TarGz` is correctly identified (unhappy path: symlinks resolved, not misclassified as Unknown)
- [ ] Given a user launches the AppImage manually without setting `$APPIMAGE` (non-standard invocation), when detection runs, then the `AppImage` variant is still detected via the `/tmp/.mount_*/` path pattern alone

#### US-009: Asset-selection matcher by arch + format suffix

**Description:** As the update checker, I want to pick the correct release asset from the GitHub API response by matching both the current architecture and the install method's format so that users on Fedora aarch64 don't accidentally download an Ubuntu x86_64 `.deb`.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] `update_checker.rs` asset-selection logic replaces the hardcoded `".ends_with(\"-x86_64-linux.run\")"` (current `update_checker.rs:99`) with a function `pick_asset(assets: &[GitHubAsset], arch: &str, method: InstallMethod) -> Option<&GitHubAsset>`
- [ ] The function filters by arch (`x86_64`, `aarch64` — resolved via `std::env::consts::ARCH`) AND by format suffix matching the install method:
  - `SystemPackage { Apt }` → prefer `.deb`
  - `SystemPackage { Dnf }` → prefer `.rpm`
  - `AppImage` → prefer `.AppImage`
  - `TarGz` → prefer `.tar.gz`
  - `Unknown` → prefer `.tar.gz` as safest fallback
- [ ] Asset filename convention documented: `paneflow-<version>-<arch>.<format>` (e.g., `paneflow-v0.2.0-x86_64.deb`)
- [ ] Unit test covers: matching asset exists; matching asset missing (returns None → `UpdateStatus::Available` with `asset_url: None` as today); multiple arches present (correct one picked); case-sensitivity handled
- [ ] Given a release has `.deb` but no `.rpm` and the user is on Fedora, when the matcher runs, then it returns `None` rather than returning the `.deb` — a wrong-format package is NEVER auto-selected (unhappy path: fail clean, don't mislead)
- [ ] `UpdateStatus::Available` struct grows a new field `asset_format: Option<AssetFormat>` so the UI can show the correct action ("Update via apt" vs "Download new AppImage")

#### US-010: AppImage update flow via appimageupdatetool

**Description:** As an AppImage user, when I click the "Update" button in the title bar, I want PaneFlow to run a zsync delta update in-place (downloading only changed blocks, typically 10–30% of file size) so that updates are fast and my AppImage stays in whatever directory I put it.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008, US-009

**Acceptance Criteria:**
- [ ] New function `self_update::appimage::run_update(source_path: &Path) -> Result<PathBuf>`
- [ ] Function resolves `appimageupdatetool` via `which::which`; if missing, downloads it from `https://github.com/AppImage/AppImageUpdate/releases/latest/download/appimageupdatetool-x86_64.AppImage` to a temp location (arch-aware for aarch64 after Phase 3)
- [ ] Function invokes `appimageupdatetool <source_path>` via `std::process::Command`
- [ ] On success, returns the same path as `source_path` (the AppImage is updated in place via zsync)
- [ ] On failure, returns a detailed error: network error, zsync info missing, signature mismatch, disk full
- [ ] After update, `cx.set_restart_path(source_path)` is called so GPUI relaunches the updated AppImage
- [ ] Given `appimageupdatetool` fails to download (no network), when the update is triggered, then an error toast displays with text "Could not download update tool. Try again when online." (unhappy path: explicit failure message, no silent bricking)
- [ ] Given the AppImage file lacks embedded update-information (e.g., an old version), when the updater runs, then it detects the missing info and shows "This AppImage cannot self-update. Download v<new> from the releases page." (unhappy path: ancient AppImage recovery)

#### US-011: tar.gz update flow via atomic swap in `~/.local/paneflow.app/`

**Description:** As a tar.gz user (including on immutable distros), when I click "Update", I want PaneFlow to download the new tar.gz, atomically swap `~/.local/paneflow.app/` to the new version, and relaunch — without requiring root or writing anywhere outside `$HOME`.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008, US-009

**Acceptance Criteria:**
- [ ] New function `self_update::targz::run_update(asset_url: &str) -> Result<PathBuf>`
- [ ] Downloads the tar.gz to `$HOME/.cache/paneflow/update-<pid>.tar.gz` (never `/tmp` — cleared on reboot, interrupts resumable updates)
- [ ] Verifies SHA-256 of the downloaded file against a `sha256` attached to the GitHub release asset (or a `.sha256` companion file)
- [ ] Extracts into `$HOME/.local/paneflow.app.new/`
- [ ] Atomic swap: `rename($HOME/.local/paneflow.app, $HOME/.local/paneflow.app.old)` then `rename($HOME/.local/paneflow.app.new, $HOME/.local/paneflow.app)` then `rm -rf $HOME/.local/paneflow.app.old`
- [ ] Uses `cx.set_restart_path($HOME/.local/paneflow.app/bin/paneflow)` for relaunch
- [ ] NEVER writes to `/usr`, `/opt`, `/bin`, or anywhere outside `$HOME` — asserted by a code comment AND a unit test using `tempdir`
- [ ] Given the download completes but the atomic swap fails (e.g., `paneflow.app.old` exists from a crashed prior update), when the updater runs, then it aborts with a message "Previous update did not clean up. Delete `~/.local/paneflow.app.old` and retry." — it does NOT blindly overwrite
- [ ] Given SHA-256 verification fails, when the updater runs, then the downloaded file is deleted and an error toast appears (unhappy path: integrity mismatch blocks install)
- [ ] Given extraction fails mid-way (disk full), when the updater runs, then `paneflow.app` is untouched (still the old version) and `paneflow.app.new` is cleaned up (unhappy path: failure preserves working state)

#### US-012: deb/rpm update UX — disable in-app button, hint system manager

**Description:** As a user who installed via `.deb` or `.rpm`, I want the in-app "Update" button to clearly redirect me to my system package manager rather than trying to update in place — because updating `/usr/bin/paneflow` from my unprivileged process would fail and also break on immutable distros.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] When `InstallMethod::SystemPackage` is detected, the title-bar update pill shows a different label: `"Update via apt"` for Apt, `"Update via dnf"` for Dnf
- [ ] Clicking the pill opens a toast / tooltip with exact copy-paste-able commands:
  - Apt: `sudo apt update && sudo apt upgrade paneflow`
  - Dnf: `sudo dnf upgrade paneflow`
- [ ] The pill does NOT trigger any download — no HTTP request to the installer URL
- [ ] The pill remains clickable so users get the hint, but visually de-emphasized (opacity 0.7, cursor remains default)
- [ ] Given a user clicks the pill on a deb install, when the toast appears, then it includes the exact tag version they should be upgrading to (e.g., "upgrade paneflow=0.2.0-1") so users with the repo pinned can target the specific version (unhappy path: users with version pins get the right incantation)
- [ ] Given the system's package manager is not standard Apt/Dnf (e.g., `eopkg` on Solus, `xbps` on Void), when `SystemPackage` is detected but neither Apt nor Dnf markers are present, then the toast falls back to "Update PaneFlow via your system's package manager" (unhappy path: graceful degradation for unsupported distros)

#### US-013: Updater error handling — FUSE 2 missing, network failure, integrity mismatch

**Description:** As a user on any format, I want clear, recoverable error messages when something goes wrong during update — not a silent failure or a misleading generic error.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-010, US-011

**Acceptance Criteria:**
- [ ] `SelfUpdateStatus::Errored` variant is enriched with a structured `UpdateError` enum: `Network(String)`, `IntegrityMismatch { expected: String, got: String }`, `Fuse2Missing`, `DiskFull { path: PathBuf }`, `Other(String)`
- [ ] Title bar renders differentiated error toasts based on variant:
  - `Network`: "Update failed: no connection. Retry when online."
  - `IntegrityMismatch`: "Update failed: downloaded file is corrupt or tampered. Retry or download manually."
  - `Fuse2Missing`: "Update requires FUSE 2. Run: `./paneflow-*.AppImage --appimage-extract-and-run` — or install libfuse2."
  - `DiskFull`: "Update failed: disk full at `{path}`. Free space and retry."
  - `Other`: show the raw message
- [ ] Toast includes a "Retry" button that re-invokes the update flow without restarting the app
- [ ] Given three consecutive failures of the same update, when the fourth attempt triggers, then the in-app updater suggests "Download manually from the releases page" with a link (unhappy path: escape hatch after repeated failure)
- [ ] Error variants are unit-tested via mocked IO failures

---

### EP-003: Phase 2 — RPM + hosted APT/RPM repositories

Add `.rpm` x86_64 to the release matrix, stand up hosted APT + RPM repositories on Cloudflare R2 + reprepro + createrepo_c, ship a postinst script that auto-adds `pkg.paneflow.dev` on first install (Cursor pattern — but thoroughly tested against a clean Ubuntu VM to avoid their broken-apt-update incident), and establish GPG signing infrastructure.

**Definition of Done:** After Phase 2 ships, a Fedora user runs `sudo dnf install ./paneflow-0.3.0-1.x86_64.rpm` once and subsequently gets updates via `sudo dnf upgrade paneflow`. Same for Ubuntu with `apt`. The repo metadata is GPG-signed; the signing key is documented and rotatable.

#### US-014: Configure `[package.metadata.generate-rpm]` and produce .rpm artifact

**Description:** As a Fedora/RHEL/Rocky/openSUSE user, I want a `.rpm` package that installs cleanly via `sudo dnf install ./paneflow-*.rpm` so that I can use PaneFlow like any other RPM-based native app.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006

**Acceptance Criteria:**
- [ ] `src-app/Cargo.toml` contains a complete `[package.metadata.generate-rpm]` section with explicit `assets` (binary, desktop entry, icons, AppStream metainfo, LICENSE as `%license`)
- [ ] `[package.metadata.generate-rpm.requires]` declares: `vulkan-loader`, `libxkbcommon`, `libxkbcommon-x11`, `fontconfig`, `freetype`
- [ ] `release.yml` installs `cargo-generate-rpm` 0.20.0 and runs `cargo generate-rpm -p paneflow-app --target x86_64-unknown-linux-gnu` after the binary build
- [ ] Produces `paneflow-<version>-1.x86_64.rpm` in `target/x86_64-unknown-linux-gnu/generate-rpm/`
- [ ] `rpm -qip paneflow-*.rpm` shows correct metadata: name, version, release, summary, license, URL
- [ ] `rpm -qlp paneflow-*.rpm` shows all expected files at correct paths
- [ ] In a Fedora 40 container: `sudo dnf install ./paneflow-*.rpm` succeeds; `paneflow --version` runs; `sudo dnf remove paneflow` removes cleanly
- [ ] Given an RPM-based distro is missing `vulkan-loader`, when install is attempted, then dnf reports the missing dep (unhappy path: dep resolution works)

#### US-015: Host APT + RPM repositories on Cloudflare R2

**Description:** As a user who already installed PaneFlow once, I want `sudo apt upgrade paneflow` or `sudo dnf upgrade paneflow` to pull new versions automatically so that I don't need to manually download packages every release.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** Blocked by US-014, US-017

**Acceptance Criteria:**
- [ ] Cloudflare R2 bucket `paneflow-pkg` exists with public read access and a custom domain `pkg.paneflow.dev`
- [ ] APT repo layout at `pkg.paneflow.dev/apt/`: `dists/stable/main/binary-amd64/Packages{,.gz}`, `dists/stable/Release`, `dists/stable/Release.gpg`, `dists/stable/InRelease`, `pool/main/p/paneflow/paneflow_<version>_amd64.deb`
- [ ] RPM repo layout at `pkg.paneflow.dev/rpm/`: `repodata/{repomd.xml,repomd.xml.asc,primary.xml.gz,filelists.xml.gz,other.xml.gz}`, `Packages/paneflow-<version>-1.x86_64.rpm`
- [ ] CI workflow `repo-publish.yml` runs after a successful release: downloads the `.deb` and `.rpm` from the GitHub release, runs `reprepro includedeb stable <deb>`, runs `createrepo_c --update <rpm-dir>`, signs both with the GPG key stored in CI secrets, uploads to R2 via `rclone`
- [ ] GPG public key is available at `https://pkg.paneflow.dev/gpg` (ASCII-armored) and is referenced in both APT and RPM client instructions
- [ ] Given a user already has `pkg.paneflow.dev` configured, when a new release is published and `repo-publish.yml` completes, then `sudo apt update && sudo apt upgrade paneflow` installs the new version
- [ ] Given CI's GPG signing step fails (e.g., key not loaded), when the workflow runs, then the partial repo upload is aborted and the previous repo state is preserved (unhappy path: signing failure doesn't corrupt the repo)
- [ ] R2 egress under 10 GB/month at current release cadence (monitored via Cloudflare dashboard)

#### US-016: Postinst script auto-adds pkg.paneflow.dev on first install

**Description:** As a first-time `.deb` or `.rpm` installer, I want the package to automatically configure the pkg.paneflow.dev repository on my system so that subsequent `apt upgrade` / `dnf upgrade` pulls PaneFlow updates without manual setup — matching the Cursor pattern but without their broken `apt update` incident.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `packaging/debian/postinst` script is included in the `.deb` via `cargo-deb`'s `maintainer-scripts` config
- [ ] Postinst idempotent: creates `/etc/apt/sources.list.d/paneflow.list` only if absent, fetches `/usr/share/keyrings/paneflow-archive.gpg` from a stable URL only if absent
- [ ] `paneflow.list` contents: `deb [signed-by=/usr/share/keyrings/paneflow-archive.gpg] https://pkg.paneflow.dev/apt stable main` — exact format verified by `apt-get update` in a CI smoke test
- [ ] Equivalent `packaging/rpm/postinst.sh` for RPM, dropping `/etc/yum.repos.d/paneflow.repo` with `[paneflow]` section including `baseurl`, `gpgcheck=1`, `gpgkey=https://pkg.paneflow.dev/gpg`
- [ ] CI smoke test job runs in a clean Ubuntu 22.04 Docker container: `sudo apt install ./paneflow-*.deb && sudo apt update && sudo apt-cache policy paneflow` — MUST show `pkg.paneflow.dev` as a source AND `sudo apt update` must complete without error (regression test for Cursor's incident)
- [ ] Same smoke test for Fedora 40: `sudo dnf install ./paneflow-*.rpm && sudo dnf check-update paneflow` — must succeed
- [ ] Given a user already has `paneflow.list` pointing elsewhere (e.g., a custom mirror), when `apt install paneflow-*.deb` runs, then the postinst does NOT overwrite the existing file (unhappy path: user customization respected)
- [ ] Given the GPG key fetch fails during postinst (network issue), when postinst runs, then the package still installs, `apt update` will fail clearly, and a printed message instructs the user to manually `curl | tee /usr/share/keyrings/paneflow-archive.gpg`

#### US-017: GPG signing infrastructure — packages and repo metadata

**Description:** As a security-conscious user, I want both the individual `.deb`/`.rpm` artifacts AND the repository metadata to be GPG-signed so that I can verify integrity on my machine and `apt`/`dnf` refuse to install tampered packages.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None (can run in parallel with US-015 — its blocker)

**Acceptance Criteria:**
- [ ] Dedicated long-lived GPG key generated: `paneflow-release@paneflow.dev`, 4096-bit RSA, expiration 2 years, stored encrypted in a password manager
- [ ] Public key ASCII-armored, published at `https://pkg.paneflow.dev/gpg` AND committed as `packaging/paneflow-release.asc` for audit trail
- [ ] Private key exported and stored as GitHub Actions secret `GPG_PRIVATE_KEY` (encrypted) + passphrase as secret `GPG_PASSPHRASE`
- [ ] CI workflow imports the key via `gpg --import`, runs `debsigs --sign=origin -k <keyid>` on `.deb` and `rpmsign --addsign` on `.rpm`
- [ ] `dpkg-sig --verify paneflow-*.deb` returns "GOODSIG"; `rpm --checksig paneflow-*.rpm` returns "pgp OK"
- [ ] Key rotation runbook documented at `docs/release-signing.md`: how to generate a new key, publish it alongside the old key during a transition period, migrate users with `apt-key` equivalent instructions (noting that `apt-key` is deprecated — use `/usr/share/keyrings/`)
- [ ] Given CI runs with the `GPG_PRIVATE_KEY` secret missing or malformed, when the signing step runs, then the workflow fails clearly with "GPG signing failed: key not loaded" (unhappy path: no silent-unsigned-release disaster)

---

### EP-004: Phase 3 — ARM64 support

Validate that GPUI commit `0b984b5` builds and runs on `aarch64-unknown-linux-gnu` (spike — Phase 3 dies cleanly if it doesn't). If it does, extend the CI matrix to produce `.deb` + `.rpm` + AppImage for ARM64 via native `ubuntu-22.04-arm` runners. Validate on real aarch64 hardware (Raspberry Pi 5 or Asahi Linux on Apple Silicon).

**Definition of Done:** Every Phase 2+ release has 6 artifacts (3 formats × 2 arches). A user on Asahi Linux can install `paneflow-*.aarch64.deb` and it works.

#### US-018: [SPIKE] Validate GPUI build + runtime on aarch64-unknown-linux-gnu

**Description:** As the PaneFlow maintainer, I want to know whether GPUI commit `0b984b5` can be built and run on `aarch64-unknown-linux-gnu` before committing to expanding the CI matrix and signing infrastructure so that Phase 3 doesn't waste engineering effort on a broken foundation.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Spike document `tasks/spike-aarch64-build.md` records: `cargo build --release --target aarch64-unknown-linux-gnu -p paneflow-app` result (pass/fail with compiler output if fail), total build time, any dependencies requiring aarch64 patches
- [ ] If build passes, binary runs on a real aarch64 machine (Asahi Linux, RPi 5, or a Graviton EC2 instance with a display) — `paneflow --version` AND `paneflow` with a tty that renders a panel for ≥5 seconds without crashing
- [ ] If build fails, root-cause documented (e.g., "GPUI commit references `__builtin_unreachable` macro incompatible with aarch64 Clang 18") AND a recommendation: "bump GPUI to a newer commit that supports aarch64" OR "stay x86_64-only until upstream adds aarch64 CI"
- [ ] Given the spike passes build but the binary crashes at runtime on aarch64 (e.g., Vulkan loader issue), when documented, then the spike result is "BLOCKED" and Phase 3 does not proceed until a fix is found
- [ ] Spike concludes with a clear go/no-go recommendation for stories US-019 and US-020

#### US-019: Extend CI matrix to produce all 3 artifacts for aarch64

**Description:** As an ARM64 Linux user, I want the same three artifacts (`.deb`, `.rpm`, AppImage) available for my architecture so that I can install PaneFlow on my Raspberry Pi 5, Asahi Linux laptop, or Graviton EC2 instance.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** Blocked by US-018 (go decision)

**Acceptance Criteria:**
- [ ] `release.yml` matrix adds the row `{ target: "aarch64-unknown-linux-gnu", arch: "aarch64", runner: "ubuntu-22.04-arm" }`
- [ ] `cargo-deb`, `cargo-generate-rpm`, `linuxdeploy-aarch64.AppImage` all function on the native ARM runner (no `cross`, no QEMU)
- [ ] Produces `paneflow-<version>-aarch64.{tar.gz,deb,rpm,AppImage}` + `.AppImage.zsync`
- [ ] `release.yml` total time with matrix: <25 minutes
- [ ] Asset-selection matcher from US-009 already handles aarch64 (no additional code changes needed) — regression test: on an aarch64 system with install method TarGz, the correct `-aarch64.tar.gz` is picked
- [ ] Given a release workflow run where the aarch64 build fails but x86_64 succeeds, when the workflow completes, then the entire release is marked failed (no half-matrix shipped) — matches US-006 atomic-release criterion

#### US-020: Validate aarch64 artifacts on real hardware

**Description:** As a release maintainer, I want to verify the aarch64 artifacts actually install and run on real ARM64 hardware before declaring Phase 3 shipped, because CI cannot validate GPU-dependent runtime behavior (no GPU on any ARM64 runner).

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-019

**Acceptance Criteria:**
- [ ] Validation runbook `docs/validation-aarch64.md` documents the test procedure: install each of 4 artifact types on an ARM64 machine (Raspberry Pi 5 with Ubuntu 24.04 or Asahi Linux), run `paneflow` in a graphical session, perform smoke actions (open a pane, split, type, close)
- [ ] Runbook completed successfully for at least one aarch64 machine type; test evidence (screenshots or asciicast) attached to the release notes
- [ ] A GitHub issue template `aarch64-bug-report.md` exists for users to report ARM-specific issues with enough context (distro, kernel, GPU)
- [ ] Given validation reveals a runtime bug (e.g., Wayland compositor issue on Asahi), when discovered, then the aarch64 assets are NOT attached to the public release — they go to a pre-release or draft instead (unhappy path: don't ship broken aarch64 without a clear warning)

---

### EP-005: Transverse documentation & release runbook

Document user-facing installation and maintainer release procedures. This epic is small but essential — Phase 1 is not done until users know how to install each format and the maintainer knows how to cut a release.

**Definition of Done:** A new user landing on the repo can install PaneFlow via any Phase 1 format in under 3 minutes. A new maintainer (or Arthur after a 2-month break) can cut a release in under 15 minutes following `docs/release-runbook.md`.

#### US-021: User installation docs + maintainer release runbook

**Description:** As a new user, I want clear per-format installation instructions in the README so that I can pick the right method for my distro without reading 500 lines of build system source; AND as the PaneFlow maintainer, I want a checklist-style release runbook so that cutting v0.3.0 after a month away is not re-learning the CI pipeline.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007 (Phase 1 final shape), US-015 (Phase 2 repo URLs)

**Acceptance Criteria:**
- [ ] `README.md` "Installation" section rewritten with 4 subsections: Ubuntu/Debian (apt + repo), Fedora/RHEL (dnf + repo), AppImage (all distros), Tarball (immutable distros) — each with copy-paste commands
- [ ] Each subsection mentions the expected verification command (`paneflow --version`)
- [ ] README includes a "Troubleshooting" section covering FUSE 2 on Ubuntu 24.04, how to verify GPG signatures, how to uninstall each format
- [ ] `docs/release-runbook.md` created with phased checklist: (1) pre-flight (tests green, CHANGELOG updated, version bumped in all Cargo.toml), (2) tag and push, (3) monitor release.yml, (4) verify artifacts attached, (5) verify repo-publish.yml, (6) smoke-test install in Ubuntu and Fedora containers, (7) announce
- [ ] Runbook estimates time per step and flags steps that require manual human judgment (e.g., "wait for CI green before tagging")
- [ ] Given a maintainer follows the runbook end-to-end, when they reach step 7, then a verifiable release is live AND all three Phase 1 formats (plus Phase 2 `.rpm`) install cleanly in fresh containers — runbook validated by at least one dry-run before being marked as done
- [ ] Given a step in the runbook fails (e.g., CI green wait times out), when the maintainer checks the runbook, then the troubleshooting subsection for that step lists the top 3 common failure modes with recovery actions (unhappy path: runbook covers failure modes, not just happy-path steps)

---

## Functional Requirements

- **FR-01:** The system must produce `.tar.gz`, `.deb`, and `AppImage` artifacts for x86_64 on every release tag push (Phase 1).
- **FR-02:** The system must produce `.rpm` for x86_64 and host a GPG-signed APT + RPM repository at `pkg.paneflow.dev` after Phase 2 is shipped.
- **FR-03:** The system must produce `.deb`, `.rpm`, AppImage, and `.tar.gz` for aarch64 after Phase 3 is shipped.
- **FR-04:** When PaneFlow launches, the system must detect its install method by inspecting its own binary path and expose this as a value read by the update-check subsystem.
- **FR-05:** When the user triggers an update, the system must pick the correct release asset matching both current architecture and install-method format.
- **FR-06:** The in-app updater must NEVER write to `/usr`, `/opt`, `/bin`, or `/etc`. All write paths are within `$HOME/.local` or `$HOME/.cache`.
- **FR-07:** When update is triggered on a system-package install (`.deb`/`.rpm`), the system must NOT download the installer binary. It must display distro-appropriate upgrade commands instead.
- **FR-08:** When update is triggered on an AppImage install, the system must invoke `appimageupdatetool` to perform a zsync delta update.
- **FR-09:** When update is triggered on a tar.gz install, the system must download the new tar.gz, verify SHA-256, and atomically swap `$HOME/.local/paneflow.app/`.
- **FR-10:** Every `.deb` and `.rpm` artifact produced after Phase 2 must be GPG-signed with the `paneflow-release@paneflow.dev` key.
- **FR-11:** The postinst script in every `.deb` and `.rpm` must idempotently configure `pkg.paneflow.dev` as an APT/RPM source on first install.
- **FR-12:** The `.run` installer must NOT be produced from Phase 1 onward. Legacy `.run` assets on v0.1.0–v0.1.7 GitHub releases remain attached but no new ones are created.

## Non-Functional Requirements

- **Performance:** Phase 1 CI release workflow completes in <15 min (x86_64 only); Phase 3 CI with aarch64 matrix completes in <25 min. AppImage size <80 MB. `.deb` / `.rpm` size <30 MB.
- **Reliability:** CI release workflow is atomic — partial success (e.g., `.deb` builds but AppImage fails) results in zero artifacts published; the GitHub release is either complete or not created. Measured: 0 half-shipped releases over Phase 1 + Phase 2 lifespan.
- **Security:** All `.deb` and `.rpm` artifacts from Phase 2 onward are GPG-signed (`dpkg-sig --verify` returns GOODSIG; `rpm --checksig` returns "pgp OK"). GPG key is 4096-bit RSA with 2-year expiration. No unsigned package is ever pushed to `pkg.paneflow.dev`.
- **Integrity:** tar.gz and AppImage artifacts include SHA-256 checksums in the release notes. In-app tar.gz updater verifies SHA-256 before atomic swap.
- **Immutable-distro compatibility:** Tar.gz install + updater verified working on Fedora Silverblue 40, openSUSE MicroOS 2026, and SteamOS 3. AppImage install + updater verified with `--appimage-extract-and-run` on all three.
- **Update delta size:** AppImage zsync update transfers ≤30% of full file size on typical minor-version bumps (measured: compare v0.2.0 vs v0.2.1 via `appimageupdatetool` dry run).
- **Repo availability:** `pkg.paneflow.dev` APT+RPM endpoints return HTTP 200 for `Release` / `repomd.xml` with P95 <500ms globally (Cloudflare CDN default SLA).
- **Reproducibility:** Given the same git commit and same Rust toolchain version, two independent CI runs produce `.deb` files with identical SHA-256 checksums for the binary section (reproducible builds goal — measured, not strictly enforced in Phase 1).

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | First-time AppImage launch on Ubuntu 24.04 | FUSE 2 missing | Binary aborts with recovery hint | "This AppImage needs FUSE 2. Run with `--appimage-extract-and-run`, or install `libfuse2`." |
| 2 | Update downloads over flaky network | Partial download, timeout | Abort, delete partial file, toast | "Update download failed. Check your connection and retry." |
| 3 | SHA-256 mismatch on tar.gz update | Corrupt/tampered archive | Abort, delete file, keep current install | "Downloaded file failed integrity check. Retry or download manually." |
| 4 | Two updates triggered concurrently | User double-clicks button | Second click is no-op while first runs | (UI: button greyed while `Downloading/Installing`) |
| 5 | Update on immutable distro attempts `/usr` write | Should never happen — detection bug | Abort with assertion failure log | (Developer-only: bug report instructions) |
| 6 | Release has `.deb` but no matching `.rpm` | Partial release | Asset-selector returns None, UI shows "Update unavailable for your system" | "No matching installer found for your system. Check the releases page." |
| 7 | postinst GPG key fetch fails | apt install works, apt update later fails | Package installs, clear message printed | "To receive auto-updates, run: `curl -fsSL https://pkg.paneflow.dev/gpg \| sudo tee /usr/share/keyrings/paneflow-archive.gpg`" |
| 8 | User tries to install `.deb` on Fedora | `dpkg` not present | Clean error from `dnf`/`rpm` | (Standard distro error; we add FAQ entry) |
| 9 | Cross-arch download (aarch64 user gets x86_64 .deb) | User ignores download-page architecture selector | `dpkg -i` fails with clear error | "Package architecture `amd64` does not match system architecture `arm64`." (standard dpkg message) |
| 10 | `paneflow.app.old` left over from crashed update | Prior update interrupted | New update aborts with manual-cleanup hint | "Previous update did not finish cleaning up. Delete `~/.local/paneflow.app.old` and try again." |
| 11 | `$HOME` is on a full disk | tar.gz extraction fails | Clean up `paneflow.app.new`, keep `paneflow.app` | "Update failed: not enough space in `~/.local`. Free space and retry." |
| 12 | GPUI spike fails on aarch64 | Phase 3 blocked | Document root cause, cancel EP-004 | (Maintainer-facing: spike report in `tasks/spike-aarch64-build.md`) |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | GPUI commit `0b984b5` does not build on aarch64 | Medium | High (cancels Phase 3) | Phase 3 gated by US-018 spike; Phase 1+2 independent of this |
| 2 | Postinst script breaks `apt update` like Cursor's did | Medium | High (user trust damage) | US-016 has explicit regression test in clean Ubuntu container: `apt install + apt update` must succeed |
| 3 | GPG private key leaks or is lost | Low | Critical (impersonation risk or lockout) | Key stored in password manager + CI secret; US-017 includes rotation runbook; 2-year expiration forces hygiene |
| 4 | Cloudflare R2 egress exceeds free tier | Low | Medium (surprise bill) | Monitored via dashboard; current volume is ~20 GB/month with 50× safety factor before egress cost applies |
| 5 | `cargo-deb` `$auto` dep overcollection was not the only bug | Low | Medium (broken dep resolution on exotic distros) | Explicit `depends` declared; lintian check in CI catches gross errors |
| 6 | AppImage FUSE 2 deprecation accelerates | Medium | Low (fallback exists) | `--appimage-extract-and-run` works today; track AppImage Type 3 when stable |
| 7 | Users on deb/rpm don't realize in-app update button is disabled by design | Medium | Low (UX confusion) | US-012 shows explicit "Update via apt" label rather than greying out silently |
| 8 | Atomic swap on `paneflow.app` fails because cwd is in the directory | Low | Medium (updater crash) | US-011 changes cwd before swap; tested with `tempdir`-based unit test |
| 9 | `ubuntu-22.04-arm` runner removed from GitHub Actions free tier | Low | Medium (Phase 3 CI cost) | Fall back to `cross` + custom sysroot if needed; documented as backup plan |
| 10 | AppStream metainfo validation fails in distro review | Low | Low (deferred inclusion) | US-002 `appstreamcli validate` gate prevents shipping invalid metainfo |

## Non-Goals

Explicit boundaries — what this PRD does NOT include:

- **Flatpak packaging** — Fedora/GNOME push Flatpak as the default GUI format but GPUI's direct Wayland surface access has unverified compatibility with Flatpak's sandbox portal model. Revisit in a separate PRD after Phase 3 ships (or if Flathub inclusion is actively pursued).
- **Snap packaging** — Canonical-only push; AppImage + `.deb` together cover the Ubuntu-family user base without introducing a second confinement model.
- **Homebrew (Linuxbrew) formula** — Viable for developer users who already use brew. Low-effort community contribution path; leave to interested contributors rather than maintaining in-tree.
- **Nix flake** — Growing adoption but niche audience vs Phase 1 target (Ubuntu/Fedora desktop users). Defer. Accept community-maintained flake if one appears.
- **Windows/macOS packaging** — Project is Linux-only per CLAUDE.md. Cross-platform is a separate, much larger conversation tracked in `research_macos_port_feasibility.md`.
- **Distro submissions (Debian/Ubuntu/Fedora official repos)** — Requires long review cycles and ongoing maintenance from a distro packager, not the upstream maintainer. Out of scope. Users getting PaneFlow from `pkg.paneflow.dev` is functionally equivalent for our purposes.
- **Reproducible builds (strict bit-for-bit)** — Stated as a goal in NFRs for `cargo-deb` outputs, but bit-reproducibility of the final `.deb` signed metadata and AppImage runtime layer is not required in Phase 1/2. Revisit if Debian inclusion is pursued.
- **Automatic CVE scanning of dependencies** — Good practice, separate concern. Covered by `cargo audit` in CI eventually — not packaging scope.
- **Delta updates for `.deb`/`.rpm`** — System package manager handles this via its own transport; no custom delta layer needed.
- **macOS-style code signing / notarization** — Linux equivalent is GPG signing (covered by US-017). No SIP, no Gatekeeper, no equivalent notarization concept for `.deb`/`.rpm`.

## Files NOT to Modify

- `src-app/src/main.rs` — central composition root; packaging work doesn't touch UI or event loop. Updater refactor (EP-002) modifies `self_update.rs` + `update_checker.rs` + adds `install_method.rs`; it does NOT rewrite `main.rs` beyond wiring the new update flow into existing action handlers.
- `crates/paneflow-config/` — config crate is independent of packaging. Any `install_method` detection lives in `src-app/`, not this crate.
- `src-app/src/terminal.rs`, `terminal_element.rs`, `split.rs`, `workspace.rs` — terminal and layout code is orthogonal to packaging. Do NOT touch.
- `Cargo.lock` — must be committed but not regenerated casually. Only regenerate if a story explicitly requires a new crate (e.g., `which` for US-010). Regeneration is a PR-level decision, not a story-level one.
- GPUI git deps in root `Cargo.toml` — stays pinned at `0b984b5` throughout this PRD. No rev bumps.

## Technical Considerations

Frame as questions for engineering input:

- **Artifact naming scheme:** Proposed `paneflow-<version>-<arch>.<format>` (e.g., `paneflow-v0.2.0-x86_64.deb`). Debian convention is `paneflow_<version>_<arch>.deb`. Should US-003 follow strict Debian convention (underscore, lowercase arch) or uniform PaneFlow convention? Recommendation: use strict Debian convention for `.deb` and `.rpm` (required by `dpkg`/`rpm` tooling), uniform convention for `.tar.gz` and `.AppImage`.
- **Signing key storage:** Store the GPG private key in GitHub Actions secrets (recommended by US-017) vs an external secret manager (1Password Connect, HashiCorp Vault)? Trade-off: GitHub secrets are simpler and well-integrated; external secret managers add rotation ceremony but better audit trails. Phase 2 recommendation: GitHub secret. Revisit if GitHub compromise becomes plausible.
- **RPM release numbering:** `paneflow-0.2.0-1.x86_64.rpm` follows Fedora convention (`-1` is the RPM release, separate from the version). Should the release number ever bump independent of the version (e.g., `0.2.0-2` for a repackaging fix)? Recommendation: yes, document the policy in `docs/release-runbook.md`.
- **APT dist name:** `stable` vs `main` vs `<distro-codename>`. Cursor uses `stable`. Flatpak uses per-branch. Recommendation: `stable` for simplicity; add `beta` later if pre-releases warrant.
- **tar.gz symlink policy:** The `install.sh` in US-004 creates `~/.local/bin/paneflow` as a symlink to `~/.local/paneflow.app/bin/paneflow`. If the user already has a `paneflow` in `~/.local/bin/` from the old `.run` installer, should install.sh overwrite it? Recommendation: yes, with a visible message — the legacy install path is retired.
- **`install_method` detection: what if the user is running PaneFlow from `cargo run` during development?** `current_exe()` returns `target/debug/paneflow`. Should `Unknown` default silently disable the updater or show a dev-only banner? Recommendation: `Unknown` → updater is disabled with a debug-log message (no UI surface). Developers don't expect self-update.
- **zsync asset URL pattern:** `linuxdeploy` expects the pattern `gh-releases-zsync|<owner>|<repo>|<tag>|<pattern>`. Using `latest` as tag means every AppImage always updates to the latest release — this is what we want. But a user who pinned to v0.2.0 and ran update would be forcibly moved to v0.3.0. Recommendation: accept this — zsync is opt-in; users who want to pin should not run the in-app updater.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Artifacts per release (x86_64) | 1 (`.run` only) | 4 (tar.gz, deb, AppImage, AppImage.zsync) | End of Phase 1 (v0.2.0) | Count GitHub release assets |
| Artifacts per release (all) | 1 | 6+ (x86_64 × 4 + aarch64 × 4 + rpm × 2) | End of Phase 3 | Count GitHub release assets |
| CI release workflow time | ~6 min (single-job `.run`) | <15 min (Phase 1), <25 min (Phase 3 with matrix) | End of each phase | GitHub Actions workflow duration |
| Install-method coverage (automated test) | 0 codepaths exercised | 4/4 (SystemPackage, AppImage, TarGz, Unknown) | End of EP-002 | Unit tests in `install_method.rs` |
| Users on pkg.paneflow.dev (APT + RPM) | 0 | ≥50 active clients | Month-3 after Phase 2 ship | Cloudflare analytics (unique IPs hitting `/apt/dists/stable/Release`) |
| `.run` downloads post-migration | ~all downloads | 0 on new releases (old releases keep their `.run`) | Immediate on v0.2.0 | GitHub release download stats |
| Immutable-distro install success | Untested | 100% success on Fedora Silverblue / openSUSE MicroOS / SteamOS smoke test | End of EP-002 | Manual smoke test checklist in `docs/validation-immutable-distros.md` |
| Open issues tagged `packaging` | 0 (feature doesn't exist) | <5 outstanding | Month-1 after Phase 1 | GitHub issue label filter |

## Open Questions

- **Domain provisioning:** `pkg.paneflow.dev` — who owns the `paneflow.dev` domain today, and is there budget for it? If domain is unavailable, fall back to `paneflow-pkg.arthurdev44.dev` or similar. → Arthur to resolve before US-015.
- **GPG key ownership:** Key email `paneflow-release@paneflow.dev` — does this mailbox need to exist for key revocation workflow? Alternative: use Arthur's personal email as the key identity. → Arthur to decide before US-017.
- **Release cadence during migration:** While Phase 1 is being implemented, should v0.1.8 / v0.1.9 still ship as `.run`? Or do we freeze releases on the current branch until Phase 1 is ready? → Recommendation: allow `.run` releases to continue during EP-001 development; cut over at v0.2.0.
- **Aarch64 test hardware:** Does Arthur have ongoing access to an ARM64 Linux machine for US-020? If not, budget for a Raspberry Pi 5 or a monthly Graviton EC2 allocation. → Resolve before US-018 (spike sets direction).
- **AppImage sandbox:** Should we ship a Flatpak eventually for sandbox-conscious users? Non-goal for this PRD but worth a follow-up PRD after Phase 3. Note in `docs/roadmap.md` after completion.
- **Legacy `.run` user migration:** Users stuck on v0.1.x `.run` installs — do they get a migration notice in the in-app updater? Recommendation: when update_checker finds a new release but the user is on `.run` (detected via `~/.local/bin/paneflow` WITHOUT `~/.local/paneflow.app/`), show a one-time banner explaining the new formats and linking to `README.md#migration`. Track as a follow-up story if not included in Phase 1.

[/PRD]
