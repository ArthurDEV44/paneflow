[PRD]
# PRD: macOS Port — Cross-Platform Expansion to Apple Silicon + Intel Darwin

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-17 | Claude + Arthur | Initial draft — port Linux v0.1.7 baseline to macOS 13+ on aarch64-apple-darwin (P0) and x86_64-apple-darwin (best-effort), signed + notarized, distributed via .dmg + Homebrew cask |

## Problem Statement

PaneFlow shipped Linux v0.1.7 with a mature feature set (19 GPUI stories, 12 title-bar stories, full multi-format packaging migration) but remains inaccessible to macOS users — the single largest segment of developer-focused terminal-multiplexer users. Three concrete problems block reach:

1. **Zero macOS binaries exist, despite the codebase being 95% portable.** A deep swarm audit on 2026-04-17 (see `memory/research_macos_port_feasibility.md`) found only 6 code locations and 6 UI adaptations blocking a working macOS build — the rest already compiles via existing `#[cfg(target_os = ...)]` gates, POSIX APIs, GPUI's cross-platform abstractions, and the spike at `tasks/spike-macos-build.md` which already landed the `gpui_platform` Cargo.toml split. The port is close to ready; what's missing is execution, UI polish, packaging, code signing, and CI validation — not a rewrite.

2. **The competitive Rust-terminal landscape on macOS is saturated and PaneFlow is absent.** WezTerm (`.dmg` signed + Homebrew cask, Apple Silicon native), Alacritty (same), Ghostty (Swift+AppKit), and Zed itself (upstream of PaneFlow's UI framework) all have macOS-first or macOS-equal distribution. Developers who would be a natural fit for PaneFlow (tmux users wanting GPU rendering, former iTerm2 users wanting splits + GPU) install one of those competitors by default today. Every month PaneFlow stays Linux-only is a month the default for "GPU-accelerated terminal multiplexer" on macOS continues to be something else.

3. **The `.run`-era update flow and install classifier are hardcoded to Linux paths and Linux asset formats.** `src-app/src/install_method.rs:80-110` probes `/usr/bin/paneflow`, `/etc/debian_version`, `$HOME/.local/paneflow.app/` — none of which exist on macOS, so `InstallMethod::Unknown` always returns, disabling the in-app update prompt entirely. `src-app/src/update_checker.rs` `AssetFormat` enum has only `Deb`/`Rpm`/`AppImage`/`TarGz` — zero macOS asset matches. A macOS user who somehow got the binary installed would never be told a new version exists. This has to land before the first macOS release, not after, or early-adopter macOS users silently freeze on their initial build.

**Why now:** Linux v0.1.7 shipped with the Phase 1 packaging migration complete (commit `62e5dcc`) — the CI infrastructure, asset-selector refactor, and install-method classifier are all freshly rewritten and battle-tested on Linux. Extending them with a macOS leg while the code is hot in memory is materially cheaper than resurfacing the same files 6 months later. Additionally, the `tasks/spike-macos-build.md` spike already de-risked the `gpui_platform` build-time blocker on 2026-04-14 — the longest-lead technical unknown is already closed. The cost of further delay is pure market-share erosion to Ghostty and WezTerm, both of which are improving their macOS experience every release.

## Overview

This PRD delivers a production-grade macOS port of PaneFlow built from the existing Linux v0.1.7 codebase, targeting Apple Silicon (aarch64-apple-darwin) as the first-class build and Intel Darwin (x86_64-apple-darwin) as a best-effort universal fallback. The port ships as a signed + notarized `.app` bundle distributed via `.dmg` direct download from GitHub Releases (primary) and a self-hosted Homebrew cask tap at `ArthurDEV44/homebrew-paneflow` (secondary). Minimum supported macOS version is **13.0 Ventura** — matching Zed and covering ~95% of active macOS installs per StatCounter 2026 data.

The work decomposes into 5 epics executed sequentially with opportunistic parallelism:

**EP-001 (P0) — Build unblocking & compile parity.** Validate that `cargo check --target aarch64-apple-darwin` passes on a macOS runner at the pinned GPUI rev `0b984b5`. The `gpui_platform` target-split blocker is already resolved in `spike-macos-build.md`. Remaining work: confirm the `core-graphics 0.23/0.24` transitive conflict does not cause link failures (if it does, apply `[patch.crates-io]` override), and install Xcode Command Line Tools in the CI image.

**EP-002 (P0) — Runtime parity.** Patch the 6 code locations audited: IPC socket path fallback (3 sites using `XDG_RUNTIME_DIR`), `cwd_now()` stub via `proc_pidinfo`, `detect_ports()` stub via `lsof`/`libproc`, `InstallMethod::AppBundle` variant, `AssetFormat::Dmg` variant, and optional `load_mono_fonts()` macOS branch via GPUI `TextSystem`. Without EP-002, the binary launches but IPC (shell integration, external orchestration) is silently disabled and the services sidebar is empty.

**EP-003 (P0) — macOS-native UI.** Migrate all 44 keybindings from hardcoded `ctrl` to a dual-binding model: global actions use GPUI's platform-neutral `secondary` modifier (auto-resolves `cmd` on macOS, `ctrl` elsewhere), terminal-context copy/paste gains `cmd-c`/`cmd-v` on macOS IN ADDITION to the existing `ctrl-shift-c`/`v` (both work cross-platform; no regression for Linux). Add `TitlebarOptions::traffic_light_position` and a `#[cfg(target_os = "macos")] pl(px(80.))` padding guard on the title bar brand div to prevent collision with macOS native window controls. Wire a minimal `cx.set_menus(...)` menu bar (File/Edit/Window/Help) — without it the app appears visually broken on macOS and `Cmd+Q` / Edit-menu clipboard operations silently fail.

**EP-004 (P1) — Packaging & distribution.** Assemble the `.app` bundle layout (`PaneFlow.app/Contents/{MacOS/paneflow, Info.plist, Resources/PaneFlow.icns}`), generate the multi-resolution `.icns` from existing PNG assets (upscale 512→1024 or regenerate from SVG), build the `.dmg` via `create-dmg` in CI, and ship a self-hosted Homebrew cask at `ArthurDEV44/homebrew-paneflow` with auto-upgrade-via-cask support. The cask formula generation auto-updates on every release via a GitHub Actions step that bumps the SHA256 + version.

**EP-005 (P0 — gated release) — CI, code signing, notarization.** Add a `macos-14` runner matrix leg to `release.yml` producing the aarch64 artifact; add an `macos-13` leg for x86_64 best-effort; wire the full signing chain: `codesign --deep --options runtime --sign "Developer ID Application: Arthur Jean (TEAMID)"` → `xcrun notarytool submit --wait --keychain-profile AC_PASSWORD` → `xcrun stapler staple`. Store the developer certificate (`.p12`), Apple ID, and app-specific password in GitHub Secrets. Add a `macos-check` job to `ci.yml` running clippy + build on every PR touching platform-gated code paths.

Key decisions:
- **Apple Silicon P0, Intel best-effort.** Per StatCounter 2026, Apple Silicon is ~78% of active macOS installs, Intel ~22% and declining. A universal binary doubles artifact size and CI time for a shrinking audience; per-arch separate `.dmg` artifacts match Zed and WezTerm's current strategy.
- **`.dmg` over `.app.zip`.** macOS users expect the drag-to-Applications UX. `hdiutil create` or `create-dmg` both produce this in CI with <30s overhead per build.
- **Self-hosted tap, not homebrew-cask core.** Submitting to `homebrew/homebrew-cask` core requires a stable release cadence, upstream review (~1-2 week cycles), and specific license + homepage conventions. A self-hosted tap at `ArthurDEV44/homebrew-paneflow` enables day-one `brew install --cask arthurdev44/paneflow/paneflow` and same-release auto-updates, with migration to the core tap possible in a future release once release cadence stabilizes.
- **Code signing + notarization is P0, not P1.** macOS Sequoia (15.0+) removed the right-click-Open bypass; unsigned binaries require users to manually `xattr -cr` from the command line, which is a non-starter for any user who would download a terminal emulator to get AWAY from command-line friction. Notarization takes <5 min for most submissions in 2026; worst-case Apple SLA is 48h.
- **Minimum macOS 13.0 Ventura.** Matches Zed's minimum. Older macOS 12 users represent <3% of active installs and are disproportionately likely to be on EOL hardware where GPU driver bugs compound.
- **No App Store.** The App Store sandbox blocks PTY spawning via `posix_spawn`, shell command execution outside the bundle, and arbitrary Unix socket paths — all foundational to a terminal multiplexer. Direct distribution + Gatekeeper-approved signing is the only viable channel.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| macOS binaries shipped | 1 (aarch64 `.dmg` signed + notarized, Apple Silicon) | 2 (aarch64 + x86_64 `.dmg` both signed + notarized) |
| macOS install UX | `brew install --cask arthurdev44/paneflow/paneflow` works | Cask passes `brew audit --cask --strict`; direct `.dmg` drag-to-Applications documented on download page |
| Gatekeeper pass rate | 100% — first-launch shows "PaneFlow was notarized" dialog, no `xattr` required | 100% maintained across all releases via CI-blocking signing step |
| Update flow coverage | aarch64 macOS users receive in-app update prompt for new `.dmg` releases | Both arches receive prompt; cask users update via `brew upgrade --cask paneflow` |
| CI build time | <20 min for full release (Linux + aarch64 macOS) | <25 min for Linux + aarch64 + x86_64 macOS + smoke tests |
| Minimum macOS supported | 13.0 Ventura | 13.0 Ventura (held stable) |
| Keybinding platform conformance | All 44 bindings use `secondary` or platform-native modifier, no hardcoded `ctrl` for app-global actions on macOS | Same, plus `Cmd+C/V` natively wired in terminal context |

## Target Users

### Apple Silicon Developer (M1/M2/M3/M4 MacBook)
- **Role:** Full-stack or systems developer on modern Apple Silicon hardware, likely also has a Linux workstation or VM
- **Behaviors:** Installs CLI tools via Homebrew (`brew install` or `brew install --cask`). Uses iTerm2, kitty, WezTerm, Ghostty, or Terminal.app. Has Xcode Command Line Tools installed. Uses `Cmd+T` to open new tab, `Cmd+Shift+D` to split in iTerm2.
- **Pain points:** No GPU-accelerated terminal with GPUI-quality rendering AND first-class split panes AND a native macOS feel exists except Ghostty (which has a different UX model). Wants the PaneFlow UX seen in Linux demo videos but on macOS.
- **Current workaround:** Uses WezTerm or kitty for splits, Ghostty for speed, or tmux-inside-iTerm2 for old-school multiplexing. Each workaround sacrifices one of: GPU rendering, split ergonomics, or native macOS integration.
- **Success looks like:** `brew install --cask arthurdev44/paneflow/paneflow` (or `.dmg` drag), first launch shows notarized dialog + opens with `Cmd+N` for new workspace, all keybindings feel macOS-native, traffic lights in the right place, native menu bar has expected items. Zero configuration needed to reach feature parity with the Linux build.

### Intel Mac Developer (2018-2020 MacBook Pro on macOS Monterey/Ventura)
- **Role:** Developer on an x86_64 MacBook Pro, often in an enterprise context where hardware refresh cycles are long. Represents ~22% of active macOS installs per StatCounter 2026.
- **Behaviors:** Same as Apple Silicon developer but on aging hardware. Sensitive to GPU-intensive apps because Intel iGPUs and discrete Radeons run hot; wants a terminal that doesn't spike CPU at idle.
- **Pain points:** Same as above, plus: many new macOS tools ship Apple-Silicon-only `.dmg` files (WezTerm has universal, kitty has universal, Ghostty has universal) and Intel users either Rosetta-emulate or skip.
- **Current workaround:** iTerm2 is Intel-native and fast; most developers on Intel Macs stay there for terminals specifically.
- **Success looks like:** `x86_64-apple-darwin` `.dmg` available on GitHub Releases, signed + notarized, works without Rosetta, GPU usage at idle is <5% (terminal should not heat the laptop when nothing is happening). Acceptable to lag 1-2 releases behind aarch64 in rare cases of arch-specific issues.

### Cross-Platform Developer (Linux + macOS)
- **Role:** Developer who daily-drives Linux for work but uses macOS for personal or client-meeting scenarios. Wants identical UX across both.
- **Behaviors:** Already installed PaneFlow on Linux. Has muscle memory for `Ctrl+Shift+D` to split, `Ctrl+Tab` to switch workspaces. Uses the same config file (`~/.config/paneflow/paneflow.json` on Linux, `~/Library/Application Support/paneflow/paneflow.json` on macOS) — or at least wants to.
- **Pain points:** Context-switching between `Ctrl+Shift+D` on Linux and `Cmd+Shift+D` on macOS is muscle-memory friction. Wants BOTH bindings to work on both platforms.
- **Current workaround:** Memorizes two keybinding schemes. Or uses a karabiner-style OS-wide remap.
- **Success looks like:** `Ctrl+Shift+D` AND `Cmd+Shift+D` both work on macOS (dual-binding). Config schema is identical across platforms — the `shortcuts` field accepts either modifier and the keybinding loader resolves both.

### Homebrew-First macOS User
- **Role:** Developer who installs everything via Homebrew — no direct-download mentality. Represents a large share of macOS developer users per Homebrew analytics.
- **Behaviors:** Scripts their machine setup with Brewfile. Expects `brew upgrade` to handle all tools. Never visits vendor download pages.
- **Pain points:** `.dmg`-only distribution means manual download, manual drag-to-Applications, manual first-launch consent. No `brew upgrade` integration.
- **Current workaround:** Adds the vendor's tap if they offer one, otherwise falls back to manual install for that one tool.
- **Success looks like:** `brew tap arthurdev44/paneflow && brew install --cask paneflow`, then `brew upgrade --cask` works for every future release.

## Research Findings

Key findings that informed this PRD (full research in `memory/research_macos_port_feasibility.md`):

### Competitive Context
- **WezTerm:** single Rust codebase, ships signed + notarized `.dmg` + official Homebrew cask (`wezterm`), Apple Silicon native + Intel universal binary. Uses native macOS titlebar (no custom CSD on macOS — gives up pixel-perfect UX for zero-friction port).
- **Alacritty:** cross-platform Rust, ships `.dmg` + cask, no custom titlebar, minimal macOS-specific code.
- **Ghostty:** Swift+AppKit on macOS / C+GTK on Linux — two codebases. Avoids the cross-platform complexity but doubles maintenance burden. Not a viable model for PaneFlow (GPUI commits us to a single codebase).
- **Zed:** upstream of GPUI, the canonical reference for "GPUI app on macOS." Ships signed + notarized `.dmg`, auto-update via Sparkle framework, official Homebrew cask (`zed`), custom titlebar with traffic lights repositioned via `TitlebarOptions::traffic_light_position`.
- **Market gap:** No current macOS terminal combines (a) GPU-accelerated rendering, (b) first-class splits + workspace multiplexing (tmux-style), and (c) a Zed-quality visual polish. PaneFlow's Linux v0.1.7 already delivers this on Linux; extending to macOS captures an unserved segment.

### Best Practices Applied
- **GPUI's `secondary` modifier** (from `zed/crates/gpui/src/platform/keystroke.rs:143-159`) — the canonical cross-platform binding pattern. Resolves to `cmd` on macOS, `ctrl` on Linux/Windows. Adopted for all app-global PaneFlow actions.
- **`TitlebarOptions::traffic_light_position`** (from `zed/crates/gpui/src/platform.rs:1549-1561`) — the canonical way to preserve native macOS traffic lights while using a custom titlebar. Zed itself uses this. Adopted.
- **Apple notarization via `notarytool`** — `altool` is deprecated as of Xcode 14. All signing automation uses `xcrun notarytool submit --wait --keychain-profile` with an Apple ID + app-specific password stored in GitHub Secrets.
- **`create-dmg` over manual `hdiutil`** — the NPM `create-dmg` tool handles background image, custom icon positioning, and retina-aware layouts in 3-5 lines of shell script; manual `hdiutil` workflows require 20+ lines and error-prone mount/unmount dances.
- **Self-hosted Homebrew cask tap before homebrew-cask core submission.** Homebrew-cask core requires the project to be mature (stable releases for 6+ months, clear license, homepage matching specific conventions). A self-hosted tap at `ArthurDEV44/homebrew-paneflow` bypasses the review gate for initial distribution; migration to core tap is a future-work item.

*Full research sources: `memory/research_macos_port_feasibility.md` (2026-04-17 swarm audit), `tasks/spike-macos-build.md` (2026-04-14 build spike), direct source reads of Zed monorepo at rev `0b984b5`.*

## Assumptions & Constraints

### Assumptions (to validate)
- **GPUI commit `0b984b5` builds on `aarch64-apple-darwin`** — validated indirectly (Zed itself ships macOS binaries from this rev per their release cadence), but not validated directly against PaneFlow's dependency closure. Validation spike is US-001.
- **The `core-graphics 0.23.2 + 0.24.0` coexistence in `Cargo.lock` does not cause a link failure on macOS.** Cargo allows semver-incompatible versions to coexist, and the two crates produce distinct symbol names. If this assumption fails, a `[patch.crates-io]` override pinning to `0.24` resolves it (see EP-001).
- **Apple Developer account obtainment ($99/yr) is not a blocker.** Arthur's existing developer identity is presumed available, or can be created within 24h of starting US-015.
- **GitHub Actions `macos-14` runners (Apple Silicon, M1) are sufficient** for production `aarch64-apple-darwin` builds. GitHub's documentation confirms macos-14 is M1-based. Fallback to larger runners (macos-14-xlarge, 6 vCPU) if build time exceeds 20 min.
- **Homebrew cask auto-upgrade works without SHA256 divergence between GitHub Releases and tap metadata.** Verified via the WezTerm + Zed cask patterns which use identical automation.
- **macOS Keychain Access + GitHub Secrets combination is sufficient for CI signing** — the `Developer ID Application` certificate `.p12` can be imported into the runner's keychain via `security import` without an interactive prompt.

### Hard Constraints
- **No App Store distribution.** App Store sandboxing blocks `posix_spawn` for arbitrary binaries, `UnixListener::bind` outside the app container, and shell command execution — all foundational.
- **Minimum macOS 13.0 Ventura.** Matches Zed. Older versions are not supported.
- **Must not regress Linux behavior.** Every Linux keybinding, config file, and IPC socket path must continue to work identically on Linux after the cross-platform refactor. Zero behavioral changes visible to existing Linux users.
- **Single codebase.** No forking `src-app/src/main_macos.rs`. All platform differences behind `#[cfg]` attributes or runtime dispatch inside shared functions.
- **No Swift, no Objective-C in PaneFlow's own source.** GPUI internalizes all Obj-C bridging; PaneFlow stays pure Rust.
- **Stable commit anchor: GPUI rev `0b984b5` must not be bumped during this PRD.** If a bump is required for a macOS-specific fix, it becomes a separate PRD with full Linux regression testing. (The spike pre-validates this rev on Linux.)

## Quality Gates

These commands must pass for every user story:
- `cargo fmt --check` — formatting
- `cargo clippy --workspace -- -D warnings` — lint (on the host OS of the PR)
- `cargo test --workspace` — all tests (config crate, no UI tests in app crate)
- `cargo check --target aarch64-apple-darwin` — cross-platform compile verification (runs on macos-14 runner in CI, or locally on a Mac; skipped on pure-Linux local dev)

For UI stories, additional gate:
- Manual launch on a macOS 13+ device: verify the specific UI behavior (menu bar present, traffic lights aligned, keybindings fire, etc.) — screenshot attached to the PR.

For packaging/CI stories, additional gate:
- CI run green on `macos-14` runner.
- Resulting `.dmg` passes `spctl --assess --type exec --verbose` with "accepted" verdict (signed + notarized check).

## Epics & User Stories

### EP-001: Build Unblocking & Compile Parity

Validate that the Linux codebase compiles and links cleanly on macOS at the pinned GPUI rev, resolving any transitive dependency conflicts discovered during the first build attempt. Pre-work has already been done in `tasks/spike-macos-build.md` (2026-04-14) — the `gpui_platform` Cargo.toml split is already in place.

**Definition of Done:** `cargo build --release --target aarch64-apple-darwin` succeeds on a macos-14 GitHub Actions runner using the currently pinned GPUI rev `0b984b5`, producing an executable under `target/aarch64-apple-darwin/release/paneflow`.

#### US-001: Validate GPUI rev 0b984b5 builds on aarch64-apple-darwin
**Description:** As the PaneFlow maintainer, I want a CI smoke build on a macos-14 runner so that I can confirm the pinned GPUI revision compiles on Apple Silicon and catch any transitive dependency conflicts before the rest of the work proceeds.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given a macos-14 GitHub Actions runner with Xcode Command Line Tools installed, when `cargo check --target aarch64-apple-darwin` is run, then the command exits 0 with no linker errors.
- [ ] Given the runner, when `cargo build --release --target aarch64-apple-darwin` is run, then a `target/aarch64-apple-darwin/release/paneflow` binary is produced.
- [ ] Given the resulting binary, when `file target/aarch64-apple-darwin/release/paneflow` is run, then the output reports `Mach-O 64-bit executable arm64`.
- [ ] Given build failure, when the logs are inspected, then the specific failing crate and error are captured in the spike document for remediation in US-002.
- [ ] Given a build time exceeding 20 min, when observed, then fall back to `macos-14-xlarge` runner and document the decision in the PR description.

#### US-002: Resolve core-graphics transitive version conflict if it manifests
**Description:** As the PaneFlow maintainer, I want the `core-graphics 0.23.2 / 0.24.0` dual-version coexistence resolved so that linking succeeds on macOS without symbol duplication errors.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given the US-001 build, when linking errors reference duplicate `core-graphics` symbols, then a `[patch.crates-io]` block is added to the workspace `Cargo.toml` pinning `core-graphics = "0.24"`.
- [ ] Given the patch in place, when `cargo build --release --target aarch64-apple-darwin` is re-run, then linking completes with no symbol errors.
- [ ] Given the patch in place, when `cargo build --release` is run on Linux (without `--target`), then the Linux build is unchanged (no regression).
- [ ] Given US-001 passes without any core-graphics conflict (no patch needed), when this story is encountered, then it is marked `CANCELLED` in status.json with the finding documented.

#### US-003: Install Xcode Command Line Tools + verify gpui_macos deps compile
**Description:** As the PaneFlow maintainer, I want the CI macos-14 runner provisioned with Xcode Command Line Tools and Metal framework headers so that `gpui_macos` internal Objective-C bridging and Metal shader compilation succeed.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `.github/workflows/release.yml`, when the `macos-14` matrix leg is defined, then `xcode-select --install || true` or an equivalent pre-step ensures CLT is present (GitHub runners ship with Xcode by default — verify via `xcode-select -p`).
- [ ] Given `cargo build`, when it runs on the macOS runner, then all `.metal` / `.m` source files inside `gpui_macos` compile without header-not-found errors.
- [ ] Given a missing Metal framework, when detected, then a clear CI log message identifies the failure and the runner configuration is adjusted.

---

### EP-002: Runtime Parity

Close the runtime functional gaps between Linux and macOS for IPC, CWD tracking, port detection, and the update install-method classifier. Without this epic, the app launches on macOS but shell integration (IPC) silently disables, the services sidebar is permanently empty, and the updater never fires.

**Definition of Done:** PaneFlow on macOS launches, the IPC socket at `$TMPDIR/paneflow/paneflow.sock` accepts connections, `cwd_now()` returns the actual shell CWD for split inheritance, `detect_ports()` returns the real listening port set for the services sidebar, and the update checker correctly identifies the install method and asset format.

#### US-004: IPC socket path fallback via $TMPDIR on macOS
**Description:** As a macOS user, I want the PaneFlow IPC socket to bind correctly so that shell integration (OSC 7 CWD, external orchestration via JSON-RPC) works identically to Linux.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/ipc.rs:234-237`, when the `XDG_RUNTIME_DIR` env var is unset and `dirs::runtime_dir()` returns `None`, then the socket path falls back to `std::env::var("TMPDIR").map(PathBuf::from).or_else(|_| dirs::cache_dir().map(|d| d.join("run"))).map(|d| d.join("paneflow/paneflow.sock"))`.
- [ ] Given `src-app/src/terminal.rs:854-857` and `:869-872`, when the same fallback is applied, then PTY wrapper scripts receive a valid `PANEFLOW_SOCKET_PATH` env var on macOS.
- [ ] Given a macOS system where `$TMPDIR` resolves to a path under `/var/folders/xx/...`, when the full socket path is constructed, then its byte length does not exceed 104 (macOS `sun_path` limit).
- [ ] Given the socket created at the fallback path, when the IPC server binds and a client invokes `system.ping`, then the ping response is received with no `ECONNREFUSED`.
- [ ] Given Linux regression test, when the same code runs on Linux with `XDG_RUNTIME_DIR` set, then the socket path resolves to `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` (no behavior change for Linux users).
- [ ] Given a pathological `$TMPDIR` exceeding 104 bytes (rare, but possible on custom configurations), when the fallback chain is exhausted, then `log::warn!` fires with a clear message and IPC gracefully disables.

#### US-005: cwd_now() macOS implementation via proc_pidinfo
**Description:** As a macOS user, I want the shell's current working directory to be detected by PaneFlow so that when I split a pane, the new pane inherits the CWD of its parent.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/terminal.rs:664`, when the non-Linux stub is replaced with a macOS implementation, then it calls `proc_pidinfo(pid, PROC_PIDVNODEPATHINFO, 0, &buf, size)` via the `libproc` crate (add as a macOS-only dep in the target-specific Cargo.toml block).
- [ ] Given a running PTY child shell in some directory, when `cwd_now(pid)` is called, then it returns `Some(PathBuf)` matching the shell's actual CWD.
- [ ] Given a shell that exits between the call and the kernel lookup, when `cwd_now` runs, then it returns `None` gracefully with no panic.
- [ ] Given the OSC 7 shell-integration path (which is the authoritative CWD source when available), when both `cwd_now` and OSC 7 data are present, then OSC 7 takes precedence (behavior unchanged from Linux).
- [ ] Given Linux regression, when the same code runs on Linux, then the `/proc/<pid>/cwd` path is still used with no behavior change.
- [ ] Given unknown failure modes (permission denied, SIP-restricted process), when they arise, then the error is logged at `warn` level and `None` is returned without killing the terminal.

#### US-006: detect_ports() macOS implementation via libproc
**Description:** As a macOS user, I want the services sidebar to show listening ports of my dev servers so that I can click to open `localhost:3000` directly.

**Priority:** P2
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/workspace.rs:283`, when the non-Linux stub is replaced, then it uses `proc_listpids(PROC_ALL_PIDS)` + `proc_pidinfo(PROC_PIDFDSOCKETINFO)` via `libproc` to enumerate sockets per PID.
- [ ] Given a TCP socket listening on localhost, when `detect_ports` is called, then the port appears in the returned vector with the owning process PID.
- [ ] Given ~200 processes and ~50 listening sockets, when `detect_ports` runs, then the full scan completes in <100ms.
- [ ] Given SIP-protected processes that `proc_pidinfo` cannot inspect, when encountered, then they are skipped silently without panic.
- [ ] Given Linux regression, when the same code runs on Linux, then `/proc/net/tcp` parsing is still used with no behavior change.
- [ ] Given the story is ranked P2 and compresses the release schedule, when the maintainer decides to defer, then this story can move to a follow-up PRD and the services sidebar ships macOS-empty for v0.2.0 with a clear disabled-state message.

#### US-007: InstallMethod::AppBundle variant + detection logic
**Description:** As a macOS user who installed PaneFlow to `/Applications/`, I want the in-app update checker to correctly identify my install method so that the update flow can work.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/install_method.rs`, when the `InstallMethod` enum is extended, then a new `AppBundle { bundle_path: PathBuf }` variant is added with clear Rust doc.
- [ ] Given the `detect()` function, when it runs on macOS and the running executable resides inside `/Applications/PaneFlow.app/Contents/MacOS/` or `$HOME/Applications/PaneFlow.app/Contents/MacOS/`, then it returns `InstallMethod::AppBundle { bundle_path }`.
- [ ] Given the `detect()` function, when it runs on macOS and the executable is outside any `.app` bundle (e.g., ad-hoc copied to `~/bin/`), then it returns `InstallMethod::Unknown` with a clear log message.
- [ ] Given Linux regression, when `detect()` runs on Linux, then all existing branches (SystemPackage, AppImage, TarGz, Unknown) behave identically to pre-change.
- [ ] Given a unit test in `paneflow-config` or a new test module, when an `.app` path is fed to the detector, then it returns the expected `AppBundle` variant.

#### US-008: AssetFormat::Dmg variant + pick_asset matching
**Description:** As a macOS user running PaneFlow, I want the update checker to match the correct `.dmg` asset from GitHub Releases for my architecture so that I receive the update prompt instead of silently missing releases.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Given `src-app/src/update_checker.rs` `AssetFormat` enum, when extended, then a `Dmg` variant is added with suffix pattern `.dmg` (e.g., `paneflow-0.2.0-aarch64-apple-darwin.dmg`).
- [ ] Given `pick_asset(release, target_arch)` on macOS aarch64, when the release contains `paneflow-0.2.0-aarch64-apple-darwin.dmg`, then the function returns that asset URL.
- [ ] Given `pick_asset` on macOS x86_64, when the release contains `paneflow-0.2.0-x86_64-apple-darwin.dmg`, then the function returns that asset URL.
- [ ] Given a release with no matching macOS asset (e.g., a hotfix release that only shipped Linux artifacts), when `pick_asset` runs on macOS, then it returns `None` and the update prompt silently defers.
- [ ] Given Linux regression, when `pick_asset` runs on Linux, then deb/rpm/AppImage/TarGz matching is unchanged.
- [ ] Given the `InstallMethod::AppBundle` classifier from US-007, when paired with `AssetFormat::Dmg`, then the update flow correctly identifies "macOS install → fetch .dmg → show update prompt with download link."

---

### EP-003: macOS-Native UI

Adapt the Linux-first UI to feel native on macOS: platform-conforming keybindings, traffic-light-aware titlebar layout, and a minimal native menu bar. Without this epic, the app launches but feels alien to macOS users: Ctrl-based shortcuts don't fire, the custom titlebar collides with native window controls, and `Cmd+Q` doesn't quit.

**Definition of Done:** On macOS, all app-global shortcuts use the `cmd` modifier (resolved via GPUI's `secondary`), terminal copy/paste accepts both `cmd-c/v` and the existing `ctrl-shift-c/v`, the custom title bar leaves ~80px of left padding for native traffic lights (repositioned via `TitlebarOptions::traffic_light_position`), and a minimal native menu bar (File/Edit/Window/Help) is visible.

#### US-009: Migrate app-global keybindings to `secondary` modifier
**Description:** As a macOS user, I want PaneFlow's split/workspace/window shortcuts to use `Cmd` instead of `Ctrl` so that they don't conflict with the macOS convention (`Cmd` for app shortcuts) and don't collide with `Ctrl+C`-to-SIGINT in the terminal.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/keybindings.rs` `DEFAULTS` table, when app-global bindings (`split_horizontally`, `split_vertically`, `close_pane`, `new_workspace`, `close_workspace`, `next_workspace`, `select_workspace_N`) are updated, then they use `secondary-shift-*` or `secondary-*` (GPUI resolves `secondary` to `cmd` on macOS, `ctrl` on Linux/Windows).
- [ ] Given the keybinding loader, when it registers bindings on Linux, then `secondary-shift-d` still fires on `Ctrl+Shift+D` (no regression for Linux users).
- [ ] Given the keybinding loader, when it registers bindings on macOS, then `secondary-shift-d` fires on `Cmd+Shift+D`.
- [ ] Given terminal-context bindings (`terminal_copy` = `ctrl-shift-c`, `terminal_paste` = `ctrl-shift-v`), when they remain unchanged, then Linux users continue to use the terminal-standard `Ctrl+Shift+C/V` (copy/paste-safe vs `Ctrl+C` SIGINT).
- [ ] Given the config file `shortcuts` override, when a user sets a custom binding like `"split_horizontally": "cmd-shift-d"`, then the override works on both Linux (unusual but valid if they have a cmd key) and macOS.
- [ ] Given `keybindings.rs` `format_keystroke()`, when it runs on macOS, then it displays `⌘⇧D` or similar platform-native rendering for the menu bar items.

#### US-010: Add Cmd+C / Cmd+V terminal copy/paste bindings on macOS
**Description:** As a macOS user in the terminal, I want `Cmd+C` to copy selected text and `Cmd+V` to paste, matching every other macOS terminal (iTerm2, Terminal.app, WezTerm), so that my muscle memory works.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-009

**Acceptance Criteria:**
- [ ] Given the keybinding defaults, when running on macOS, then `cmd-c` additionally fires `terminal_copy` in the `Terminal` context (in addition to the existing `ctrl-shift-c`).
- [ ] Given the keybinding defaults, when running on macOS, then `cmd-v` additionally fires `terminal_paste` in the `Terminal` context.
- [ ] Given the user has text selected in the terminal, when they press `Cmd+C` on macOS, then the selection is copied to the system clipboard (verified via `pbpaste`).
- [ ] Given `Ctrl+C` is pressed on macOS with no selection, then it continues to send SIGINT to the running process (no hijack).
- [ ] Given Linux regression, when running on Linux, then only `ctrl-shift-c/v` is registered for terminal copy/paste (no `cmd-c/v` duplicate — Linux keyboards don't have a cmd key by default, so the binding is harmless but we omit it for cleanliness).

#### US-011: TitlebarOptions traffic_light_position + macOS padding guard
**Description:** As a macOS user, I want native traffic lights (red/yellow/green close/minimize/maximize) to be visible and functional without the custom "PaneFlow" brand text colliding with them.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/main.rs:4993-5005` `WindowOptions`, when built for macOS, then `TitlebarOptions` includes `traffic_light_position: Some(point(px(12.), px(12.)))` or equivalent, positioning the native traffic lights in the custom titlebar's left region.
- [ ] Given `src-app/src/title_bar.rs:132-146`, when running on macOS, then the brand container receives a `#[cfg(target_os = "macos")] pl(px(80.))` left padding guard so the "PaneFlow" text starts after the traffic lights.
- [ ] Given the running app on macOS, when the window is rendered, then traffic lights are visible at x≈12-78px and the "PaneFlow" brand text is visible at x≥80px with no visual collision.
- [ ] Given click on the red traffic light, when handled by macOS natively, then the window closes (app quits if it was the last window).
- [ ] Given Linux regression, when running on Linux, then the brand container has `pl_3()` (12px) as before — no padding change for Linux users.
- [ ] Given the screenshot in the PR, when a reviewer opens it, then the traffic lights are unambiguously visible and the brand text does not overlap them.

#### US-012: Minimal native menu bar via cx.set_menus
**Description:** As a macOS user, I want a native menu bar with at least `PaneFlow > Quit` and `Edit > Copy / Paste / Select All` so that Cmd+Q works and the app doesn't look visually broken (macOS apps without a menu bar appear unusable).

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `src-app/src/main.rs` App startup, when running on macOS, then `cx.set_menus(vec![...])` is invoked with at minimum:
  - `Menu::new("PaneFlow").items([MenuItem::action("About PaneFlow", About), MenuItem::separator(), MenuItem::action("Quit PaneFlow", Quit)])`
  - `Menu::new("Edit").items([MenuItem::action("Copy", Copy), MenuItem::action("Paste", Paste), MenuItem::separator(), MenuItem::action("Select All", SelectAll)])`
  - `Menu::new("Window").items([MenuItem::action("New Workspace", NewWorkspace), MenuItem::action("Close Workspace", CloseWorkspace), MenuItem::separator(), MenuItem::action("Next Workspace", NextWorkspace)])`
- [ ] Given the running app on macOS, when the menu bar is visible at the top of the screen, then each top-level menu opens on click and each item shows the correct keyboard shortcut (e.g., `⌘Q` next to Quit).
- [ ] Given `Cmd+Q`, when pressed on macOS, then the app quits via the Quit action.
- [ ] Given `Cmd+Shift+N`, when pressed on macOS, then the New Workspace action fires (matches menu item shortcut via US-009's binding).
- [ ] Given Linux regression, when running on Linux, then `cx.set_menus` is either a no-op or skipped (GPUI handles gracefully) — no Linux UI change.
- [ ] Given the menu bar, when inspected visually, then "PaneFlow" appears as the application name in the menu bar (not "paneflow" lowercase), matching the `CFBundleName` from `Info.plist`.

---

### EP-004: Packaging & Distribution

Produce a signed + notarized `.app` bundle packaged as `.dmg` for GitHub Releases distribution and publish a Homebrew cask formula for `brew install --cask`. Without this epic, technical users could clone + build but no macOS user who expects a binary download has a viable install path.

**Definition of Done:** A tagged release produces `paneflow-<ver>-aarch64-apple-darwin.dmg` (+ x86_64 variant best-effort) uploaded to GitHub Releases, both signed with `Developer ID Application` and notarized via `notarytool`, passing `spctl --assess` as "accepted." A corresponding Homebrew cask formula is published to the tap `ArthurDEV44/homebrew-paneflow` and installs cleanly via `brew install --cask arthurdev44/paneflow/paneflow`.

#### US-013: .app bundle layout + bundle-macos.sh script
**Description:** As the PaneFlow release pipeline, I want a deterministic `.app` bundle assembly script so that every macOS release produces a structurally-valid macOS application bundle.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `scripts/bundle-macos.sh`, when invoked with `--version 0.2.0 --arch aarch64 --target-dir target/aarch64-apple-darwin/release`, then it produces `dist/PaneFlow.app/Contents/` with sub-paths: `MacOS/paneflow` (executable), `Info.plist`, `Resources/PaneFlow.icns`, `Resources/` for any future bundled assets.
- [ ] Given the bundle, when `codesign --verify --deep --strict dist/PaneFlow.app` is run (before signing), then the directory structure is valid even if signatures are absent.
- [ ] Given `assets/Info.plist`, when inspected, then it contains at minimum: `CFBundleIdentifier=io.github.arthurdev44.paneflow`, `CFBundleExecutable=paneflow`, `CFBundleIconFile=PaneFlow`, `CFBundleName=PaneFlow`, `CFBundleVersion` + `CFBundleShortVersionString` (both set from the `--version` argument), `LSMinimumSystemVersion=13.0`, `NSHighResolutionCapable=true`, `LSApplicationCategoryType=public.app-category.developer-tools`.
- [ ] Given the bundle on a macOS machine, when double-clicked in Finder, then the app launches (pre-signing, possibly blocked by Gatekeeper — that's US-015's concern).
- [ ] Given the script, when invoked with `--arch x86_64`, then it produces an x86_64 bundle identically (arch-parameterized).
- [ ] Given a missing input file, when detected, then the script exits non-zero with a clear error message pointing to the missing path.

#### US-014: Generate PaneFlow.icns from existing PNG assets
**Description:** As the PaneFlow release pipeline, I want a multi-resolution `.icns` icon file generated from the existing `assets/icons/paneflow-*.png` sources so that the `.app` bundle displays a proper high-DPI icon.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given `scripts/generate-icns.sh` (new), when invoked, then it generates `assets/PaneFlow.icns` from the PNG sources at `assets/icons/paneflow-{16,32,128,256,512}.png` plus a newly-generated `paneflow-1024.png` (upscale from 512 via `sips -Z 1024 ...` or regenerate from SVG source if one exists).
- [ ] Given the script on a macOS runner, when invoked, then it uses `iconutil -c icns` from a temporary `.iconset/` directory with correctly-named files (`icon_16x16.png`, `icon_16x16@2x.png`, ..., `icon_512x512@2x.png`).
- [ ] Given the script on a Linux runner (for local preview), when invoked, then it falls back to `png2icns` or `icnsutil` and documents the limitation that `iconutil` is macOS-only.
- [ ] Given the resulting `.icns` file on macOS, when Finder displays the `.app` bundle, then the icon renders crisply at 16, 32, 64, 128, and 512px display sizes.
- [ ] Given the 1024px source, when inspected, then it is either freshly generated from a vector source OR a high-quality upscale from the 512px via `sips`; a visual audit confirms no obvious upscaling artifacts.

#### US-015: Code signing + notarization pipeline in release.yml
**Description:** As the PaneFlow release pipeline, I want every macOS release artifact signed with the Developer ID certificate and notarized by Apple so that Gatekeeper accepts the app on first launch without user intervention.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] Given GitHub Secrets, when the following are present, then signing proceeds: `APPLE_DEVELOPER_CERT_P12` (base64-encoded `.p12` file), `APPLE_DEVELOPER_CERT_PASSWORD`, `APPLE_ID`, `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`.
- [ ] Given `.github/workflows/release.yml` macos-14 leg, when the signing step runs, then it executes:
  1. `security create-keychain -p "$KEYCHAIN_PASSWORD" build.keychain`
  2. `security import cert.p12 -k build.keychain -P "$APPLE_DEVELOPER_CERT_PASSWORD" -T /usr/bin/codesign`
  3. `codesign --deep --force --options runtime --timestamp --sign "Developer ID Application: <name> ($APPLE_TEAM_ID)" dist/PaneFlow.app`
- [ ] Given the signed bundle, when `xcrun notarytool submit dist/PaneFlow.zip --apple-id $APPLE_ID --password $APPLE_APP_SPECIFIC_PASSWORD --team-id $APPLE_TEAM_ID --wait` is invoked, then the submission completes with status "Accepted" within 30 min (Apple SLA: 48h max).
- [ ] Given the notarization ticket, when `xcrun stapler staple dist/PaneFlow.app` runs, then the ticket is attached to the bundle.
- [ ] Given the final stapled bundle, when `spctl --assess --type exec --verbose dist/PaneFlow.app` is run, then the output reports "accepted, source=Notarized Developer ID."
- [ ] Given notarization failure, when the Apple log is retrieved (`xcrun notarytool log <submission-id>`), then the error is captured in the CI log for diagnosis.
- [ ] Given the signing step on a Linux PR (not a release), when it is skipped conditionally (`if: runner.os == 'macOS'`), then the Linux release pipeline is unaffected.

#### US-016: .dmg creation + upload to GitHub Releases
**Description:** As a macOS user downloading PaneFlow from GitHub Releases, I want a `.dmg` file that mounts to a familiar drag-to-Applications UX so that installation feels like every other macOS app.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] Given `scripts/create-dmg.sh`, when invoked with a signed `.app` path and `--version`, then it produces `dist/paneflow-<version>-<arch>-apple-darwin.dmg` via `create-dmg` (npm) or a hand-rolled `hdiutil create`.
- [ ] Given the resulting DMG, when mounted on macOS, then the Finder window shows `PaneFlow.app` on the left and a symlink alias to `/Applications` on the right, with a background image placing them at standard positions.
- [ ] Given the DMG, when `codesign --verify --deep --strict` is run, then the enclosed `.app` bundle retains its signature (DMG container preserves Mach-O signing).
- [ ] Given the release.yml artifact-upload step, when the tag push triggers the release job, then the `.dmg` is attached to the GitHub Release alongside the existing Linux artifacts (tar.gz, deb, rpm, AppImage).
- [ ] Given a user downloading the `.dmg` and opening it, when they drag `PaneFlow.app` to `/Applications/`, then the app appears in Launchpad and launches successfully on first click.
- [ ] Given the `.dmg` filename, when parsed by `update_checker.rs::pick_asset` (US-008), then the `-aarch64-apple-darwin.dmg` suffix matches the new `AssetFormat::Dmg` variant.

#### US-017: Homebrew cask formula + self-hosted tap
**Description:** As a macOS Homebrew user, I want to install PaneFlow via `brew install --cask` so that my machine setup scripts pick it up with zero friction.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] Given a new GitHub repo `ArthurDEV44/homebrew-paneflow`, when created, then it follows the Homebrew tap layout (`Casks/paneflow.rb`).
- [ ] Given `Casks/paneflow.rb`, when inspected, then it contains: `version "0.2.0"`, `sha256 "..."` (SHA256 of the aarch64 .dmg), `url "https://github.com/ArthurDEV44/paneflow/releases/download/v#{version}/paneflow-#{version}-aarch64-apple-darwin.dmg"`, `app "PaneFlow.app"`, appropriate `depends_on macos: ">= :ventura"`, and `zap trash:` cleanup paths (`~/Library/Application Support/paneflow`, `~/Library/Caches/paneflow`).
- [ ] Given `brew tap arthurdev44/paneflow && brew install --cask paneflow`, when run on a clean macOS, then the cask downloads, verifies SHA256, mounts the DMG, copies `PaneFlow.app` to `/Applications/`, and unmounts.
- [ ] Given a new release triggering the release.yml workflow, when a post-release step runs (`.github/workflows/update-cask.yml` or equivalent in the release.yml), then it bumps the version + SHA256 in `Casks/paneflow.rb` via a PR or direct commit to the tap repo.
- [ ] Given `brew upgrade --cask paneflow`, when a new version is published to the tap, then the existing install is upgraded in place.
- [ ] Given `brew uninstall --cask paneflow`, when run, then `/Applications/PaneFlow.app` is removed and the `zap trash:` paths are cleaned.

---

### EP-005: CI/CD & Quality Infrastructure

Extend the existing CI/CD infrastructure (`.github/workflows/ci.yml`, `release.yml`) to cover macOS targets on every PR and every tag push, ensuring no regression lands silently. Without this epic, macOS breakage is only caught at release time by Arthur manually running builds.

**Definition of Done:** Every PR that touches platform-gated code (or any Rust file in src-app/) runs `cargo check` on aarch64-apple-darwin via a macos-check CI job. Every tag push produces signed + notarized macOS `.dmg` artifacts alongside Linux artifacts. A macOS smoke test launches the `.app` and verifies basic render.

#### US-018: macOS CI check job in ci.yml
**Description:** As the PaneFlow maintainer, I want every PR that touches Rust code to have its `aarch64-apple-darwin` build verified in CI so that macOS-breaking changes are caught before they merge, not at release time.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `.github/workflows/ci.yml`, when a PR is opened that modifies `src-app/**/*.rs`, `crates/**/*.rs`, or any `Cargo.toml`/`Cargo.lock`, then a `macos-check` job runs on `macos-14`.
- [ ] Given the `macos-check` job, when it runs, then it executes `cargo fmt --check` + `cargo clippy --workspace -- -D warnings` + `cargo check --release --target aarch64-apple-darwin` + `cargo test --workspace` (test of config crate only — app crate has no tests).
- [ ] Given the job completing in <15 min with cache hits, when subsequent PRs run, then the cargo registry + target directory are cached via `Swatinem/rust-cache@v2`.
- [ ] Given the job failing on a PR, when a reviewer checks the CI run, then the specific failure (compile error, clippy warning, test failure) is clear from the log.
- [ ] Given a PR that touches only documentation (`*.md`) or CI config (`.github/**`), when CI runs, then the `macos-check` job is skipped via `paths:` filter.

#### US-019: Release.yml macos-14 matrix leg for aarch64 .dmg
**Description:** As the PaneFlow release pipeline, I want the release workflow to produce a signed + notarized aarch64 `.dmg` on every tag push alongside the existing Linux artifacts so that every release is cross-platform.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015, US-016

**Acceptance Criteria:**
- [ ] Given `.github/workflows/release.yml`, when the matrix is extended with `{os: macos-14, target: aarch64-apple-darwin, arch: aarch64}`, then the job runs on every tag push matching `v*.*.*`.
- [ ] Given the macOS leg, when it runs, then it executes: checkout → Xcode CLT verify → `cargo build --release --target aarch64-apple-darwin` → `scripts/bundle-macos.sh` (US-013) → `scripts/generate-icns.sh` (US-014) → sign + notarize (US-015) → `scripts/create-dmg.sh` (US-016) → upload to GitHub Release.
- [ ] Given the matrix end-to-end, when the full release runs for a tag, then both Linux artifacts and the macOS `.dmg` are attached to the same GitHub Release with consistent naming (`paneflow-<ver>-<arch>-<os>.{tar.gz|deb|rpm|AppImage|dmg}`).
- [ ] Given total release time, when measured on a clean cache, then it completes in <30 min for Linux + macOS aarch64 combined (individual legs run in parallel in the matrix).
- [ ] Given a best-effort x86_64 Intel leg on `macos-13`, when added with `continue-on-error: true`, then x86_64 failures do not block the release but produce the x86_64 `.dmg` when they succeed.

---

## Functional Requirements

- FR-01: The system must build, launch, and render a terminal on macOS 13.0 Ventura or later, on both aarch64-apple-darwin (Apple Silicon) and x86_64-apple-darwin (Intel).
- FR-02: When a user presses `Cmd+Shift+D` on macOS, the system must split the active pane horizontally — equivalent behavior to `Ctrl+Shift+D` on Linux.
- FR-03: When a user presses `Cmd+C` on macOS with a terminal text selection active, the system must copy the selection to the system clipboard; `Ctrl+C` with no selection must still send SIGINT to the running process (no hijack).
- FR-04: The system must render the custom title bar without visually colliding with macOS native traffic lights — at least 80px of left padding for the brand region, traffic lights repositioned via `TitlebarOptions::traffic_light_position`.
- FR-05: The system must provide a native macOS menu bar with at least `PaneFlow > Quit`, `Edit > Copy/Paste/Select All`, `Window > New Workspace / Close Workspace`.
- FR-06: The IPC socket must bind successfully on macOS using a fallback path rooted at `$TMPDIR` when `XDG_RUNTIME_DIR` is unset (i.e., always on macOS).
- FR-07: The update checker must identify `InstallMethod::AppBundle` when running from `/Applications/PaneFlow.app/` or `~/Applications/PaneFlow.app/`, and must match GitHub Release assets with suffix `.dmg` via `AssetFormat::Dmg`.
- FR-08: The release pipeline must produce a `.dmg` that is signed with `Developer ID Application` and notarized by Apple, passing `spctl --assess --type exec` as "accepted."
- FR-09: The system must NOT execute any Linux-only shell command (`fc-list`, `xdg-open`, `notify-send`) at runtime on macOS. Any such call must be behind `#[cfg(target_os = "linux")]` or degrade gracefully via an `Err` match.
- FR-10: The system must NOT regress any existing Linux behavior — all Linux keybindings, config paths, IPC paths, and UI layouts continue to function identically after the macOS work lands.

## Non-Functional Requirements

- **Performance:** First-launch cold start <2.0s on M1 MacBook Air. Per-keystroke latency <16ms (single frame at 60Hz). Idle CPU usage <2% on both Apple Silicon and Intel.
- **Security:** Signed with `Developer ID Application`, hardened runtime enabled (`--options runtime`), notarized via `notarytool`. No entitlements requesting App Sandbox or elevated privileges. No ad-hoc signing.
- **Reliability:** IPC socket fallback must succeed on any system with a valid `$TMPDIR` (99.9% of macOS installs). `.app` bundle passes `spctl --assess` on 100% of releases.
- **Compatibility:** Supports macOS 13.0 Ventura, 14.0 Sonoma, 15.0 Sequoia. Older versions (12.0 Monterey and below) are not supported; launching on older macOS produces a clean error dialog via `LSMinimumSystemVersion`.
- **Binary size:** `.dmg` file size <40MB per arch (baseline: Linux `.tar.gz` is ~25MB; macOS overhead from Metal shaders + `.app` bundle scaffold adds ~10MB).
- **CI time:** macOS `macos-check` PR job completes in <15 min with cache hits. Full release (Linux + macOS aarch64) completes in <30 min.
- **Gatekeeper SLA:** Apple notarization completes in <30 min on median, <48h at worst-case (Apple's published SLA). CI workflow timeout for the notarize step is 2h.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | First launch of unsigned build | Dev-only build, not notarized | macOS shows "cannot be opened" dialog; user must right-click → Open OR run `xattr -cr PaneFlow.app` | "PaneFlow cannot be opened because the developer cannot be verified. Right-click → Open to bypass, or run xattr -cr. (Release builds are signed — this is a dev-only issue.)" |
| 2 | Running on macOS 12 Monterey | OS < `LSMinimumSystemVersion` | macOS Launch Services refuses launch, shows OS-level dialog | (OS-provided) "PaneFlow requires macOS 13.0 or later." |
| 3 | Apple Silicon binary on Intel Mac | User downloads wrong DMG | macOS refuses with clear error | (OS-provided) "PaneFlow cannot be opened on this Mac." User guided to download the x86_64 variant via update-checker. |
| 4 | Rosetta-run Apple Silicon binary | Intel Mac attempts to run arm64 binary through Rosetta | Runs but GPUI/Metal may perform degraded | App logs warning on startup; no blocking dialog. |
| 5 | `$TMPDIR` unset (rare custom config) | User's shell config unsets TMPDIR | Falls back to `dirs::cache_dir()/run/paneflow.sock` | Log warn: "TMPDIR unset, using cache_dir fallback for IPC socket" |
| 6 | `$TMPDIR` path exceeds 104 bytes (`sun_path` limit) | Unusual user setup | IPC silently disables with clear log | Log warn: "IPC socket path exceeds macOS sun_path limit (104 bytes), IPC disabled" |
| 7 | `cwd_now()` returns None for SIP-protected shell | User runs bash as root or in a sandboxed process | Split inheritance falls back to `~` or last known CWD | No visible error; log at debug level. |
| 8 | Port detection fails (SIP restriction on `proc_pidinfo`) | Sandboxed CI environment or unusual security posture | `detect_ports()` returns partial or empty vec | Services sidebar shows "No services detected" empty state. |
| 9 | User installs from `.dmg`, later `brew install --cask` on top | Conflict between two install paths | Homebrew cask detects existing `/Applications/PaneFlow.app`, prompts to overwrite | Homebrew-provided prompt; user accepts → overwrite succeeds. |
| 10 | Update flow triggers during first launch (notarization stapled but asset URL offline) | GitHub Releases API temporarily unreachable | Silent skip, retry on next launch | Log info: "update check deferred, GitHub Releases unreachable" |
| 11 | User's `Info.plist` corrupted after update (shouldn't happen but belt-and-braces) | Disk corruption or interrupted update | App fails to launch; `spctl` rejects modified bundle | (OS-provided) "PaneFlow is damaged and can't be opened." User reinstalls. |
| 12 | Homebrew cask `sha256` mismatch (GitHub Release re-uploaded artifact) | Hash drift between tap and release | Cask install fails with clear Homebrew error | (Homebrew-provided) "SHA256 mismatch — please report." |
| 13 | App Translocation triggered (macOS quarantine on first launch from DMG without dragging to /Applications) | User launches PaneFlow.app directly from the mounted DMG | macOS copies binary to random `/private/var/folders/.../AppTranslocation/`, which breaks relative paths (none should be relative, but Info.plist references could) | App launches normally since all paths are absolute or bundle-relative. No user-visible issue. |
| 14 | Notarization fails in CI (transient Apple API error) | Apple infrastructure hiccup | Release pipeline fails the macOS leg; maintainer re-runs | CI log: Apple notarytool output with submission-id; maintainer reviews + retries. |
| 15 | Xcode Command Line Tools missing on dev machine | Contributor without CLT tries local macOS build | `cargo build` fails with clear `ld` error pointing to missing SDK | README contributing section documents `xcode-select --install` prerequisite. |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | GPUI rev `0b984b5` fails to build on aarch64-apple-darwin despite Zed shipping macOS from same commit | Low | High (blocks entire PRD) | US-001 is the first story — dedicated spike runs before any other work. If it fails, the PRD pauses and either bumps GPUI rev (via separate PRD) or forks GPUI for a macOS-compatible patch. |
| 2 | `core-graphics 0.23.2 + 0.24.0` dual-version coexistence causes link failures | Medium | Medium (1-2 days to fix) | US-002 documents the `[patch.crates-io]` override remediation. Additive mitigation — does not affect Linux. |
| 3 | Apple notarization latency exceeds 30 min or enters the 48h tail | Low | Medium (release day delay) | CI workflow timeout set to 2h; manual re-submission path documented; release can be held in draft state and published once stapling completes asynchronously. |
| 4 | Code signing certificate expires mid-PRD (certs typically 1-3 year validity) | Low | High (signing pipeline halts) | Certificate expiration date checked into MEMORY.md; renewal reminder 60 days before expiry; backup maintainer has Developer ID access. |
| 5 | Homebrew cask SHA256 automation drifts (release auto-update PR to tap repo fails) | Medium | Low (cask stays 1 release behind until manual fix) | Release.yml includes retry logic on the cask-update step; if it fails, the release still ships via GitHub Releases; maintainer fixes the cask PR within 24h. |
| 6 | `libproc` crate (used for `cwd_now` + `detect_ports` on macOS) has unsafe FFI surface that introduces UB | Low | Medium (memory safety concern) | US-005 and US-006 use only documented `libproc` safe wrappers; all unsafe blocks (if any) reviewed in PR; fallback to shell-out `lsof` if `libproc` unreliable. |
| 7 | macOS-specific UI regression breaks Linux (keybinding change, titlebar change) | Medium | High (Linux is the current production platform) | Every UI story has an explicit "Linux regression" acceptance criterion. Manual smoke test on Linux required before merge. Roll back the specific commit if regression detected. |
| 8 | Intel x86_64 build times out or produces broken binary (GitHub macos-13 runner quirks) | Medium | Low (Intel is best-effort P1, not blocking) | US-019's x86_64 leg uses `continue-on-error: true`. Intel users are explicitly told in Target Users that Intel may lag 1-2 releases. |
| 9 | Apple Developer account setup blocked by identity verification delay (up to 48h for new accounts) | Low | High for first release | Acquire account in parallel with US-001 — by the time US-015 starts, the account is ready. Budget +3 days buffer in release plan. |
| 10 | PaneFlow's `$XDG_RUNTIME_DIR`-derived wrapper-script path at `terminal.rs:854-872` breaks PTY shell integration on macOS if missed (only 2 of the 3 sites are fixed) | Medium | High (shell integration silently broken) | US-004 explicitly covers all 3 sites (ipc.rs:234-237 AND terminal.rs:854-857 AND terminal.rs:869-872); a grep-based integration test in the PR verifies zero remaining ungated uses of `XDG_RUNTIME_DIR`. |
| 11 | GPUI menu bar API (`cx.set_menus`) differs from assumed signature at rev `0b984b5` | Low | Low (1h of adjustment) | US-012 has an explicit verify step reading the actual API from the local Zed checkout before writing the code. |
| 12 | Homebrew-cask community rejects the formula in audit (style, URL format, zap paths) | Medium | Low (cask still works via tap, just not accepted to core) | Self-hosted tap from day one; `brew audit --cask --strict` run locally before pushing. Migration to `homebrew/cask` core is explicit future-work, not P0. |
| 13 | `TMPDIR` on macOS resolves to `/var/folders/xx/.../T/` — combined with nested `paneflow/paneflow.sock` the full path approaches the 104-byte `sun_path` limit | Low | Medium (IPC disables on some user configurations) | US-004 explicitly asserts the 104-byte check; fallback to shorter path in `cache_dir` if exceeded; manual verification on 5+ real macOS systems. |

## Non-Goals

- **Windows port.** PaneFlow remains Linux + macOS only after this PRD. Windows support is explicitly deferred — GPUI Windows backend is less mature, and terminal emulation on Windows (ConPTY) has different API surface that warrants a separate PRD.
- **iOS / iPadOS.** Not viable — PTY spawning is blocked by iOS sandboxing.
- **App Store distribution.** Explicit non-goal — sandbox constraints prevent PTY, shell command execution, and arbitrary socket paths. Direct distribution is the only viable channel.
- **Universal binary `.app`.** We ship two separate per-arch `.dmg` files, not a single universal `.app`. Universal binaries double artifact size and add `lipo` complexity for ~20% Intel audience.
- **Sparkle framework auto-update.** The existing in-app update checker (`update_checker.rs`) is extended to handle `.dmg` downloads — we don't adopt Sparkle. Sparkle integration is a future-work stretch goal.
- **Submission to homebrew-cask core tap.** Self-hosted tap at `ArthurDEV44/homebrew-paneflow` is the v0.2.0 target. Migration to core tap is deferred to v0.3+ once release cadence stabilizes.
- **x86_64 Intel as CI-blocking.** Intel is best-effort P1; `continue-on-error: true` in release.yml. Intel builds may lag aarch64 by up to 2 releases in rare cases of arch-specific issues.
- **GTK / Cocoa / AppKit direct integration in PaneFlow source.** All platform-native UI goes through GPUI's abstractions. No raw Obj-C in PaneFlow.
- **Touchbar support.** 2016-2022 MacBook Pro touchbar is not supported — fewer than 15% of active Macs have a touchbar and Apple removed it from all new hardware.
- **macOS <= 12 Monterey.** Supporting older macOS adds GPU driver variance risk for <3% of users.

## Files NOT to Modify

- `Cargo.lock` — let Cargo manage it. Do NOT hand-edit versions. The `core-graphics` patch goes in `Cargo.toml`'s `[patch.crates-io]` section, not the lock file directly.
- `crates/paneflow-config/**/*.rs` — the config crate is already fully portable (uses `dirs` crate correctly). No config-crate changes should be needed for macOS support.
- `src-app/src/terminal_element.rs:52-94` — the `default_font_family()` function already correctly returns `"Menlo"` on macOS. Do NOT rewrite; extending `load_mono_fonts()` (US-006 scope) is the only acceptable change in this area.
- `tasks/spike-macos-build.md` — the completed spike document; update only to add a post-validation note, don't restructure.
- `assets/icons/paneflow-*.png` — the existing PNG sources are the authoritative icon masters. The `.icns` generation reads from them but does NOT modify them.
- `.github/workflows/ci.yml` Linux matrix legs — extend with a new `macos-check` job, do NOT modify the existing `ubuntu-latest` job.
- `.github/workflows/release.yml` Linux matrix legs — extend the matrix with a new `macos-14` entry, do NOT modify the existing `ubuntu-22.04` + `ubuntu-22.04-arm` entries.

## Technical Considerations

Frame as questions for engineering input — not mandates:

- **Architecture:** The audit identified 6 code sites + 6 UI sites + 7 new files as the full surface. Recommended approach is epic-by-epic sequential merge: land EP-001 + EP-002 first (compile + runtime parity), then EP-003 (UI), then EP-004 + EP-005 in parallel (packaging + CI). Engineering to confirm sequencing feasibility.
- **Data Model:** No schema changes. Config format is identical across platforms. The `shortcuts` config map accepts both `cmd-*` and `ctrl-*` modifier strings — GPUI's parser handles both.
- **API Design:** No new IPC methods. Existing Unix socket JSON-RPC schema works identically on macOS — only the socket path construction changes (US-004).
- **Dependencies:** New deps: `libproc` (macOS-only, target-gated) for `proc_pidinfo`. Confirmed: `portable-pty` already supports macOS; `alacritty_terminal 0.26` already supports macOS (verified `unix.rs` has Darwin branch); `dirs 5` and `dirs 6` both portable.
- **Migration:** Zero user migration — existing Linux users see no changes. New macOS users install fresh. No config migration logic needed.
- **Code signing secrets:** The `.p12` certificate, Apple ID, and app-specific password are stored in GitHub Secrets. The `.p12` should be ≤ 4KB when base64-encoded to fit GitHub Secrets limits. Engineering to confirm fallback if size exceeds limit (split into multiple secrets and concat in CI).
- **Notarization cache:** Apple caches notarization per-binary-SHA. Re-notarizing the same binary content is fast (<5 min). Engineering to confirm that `cargo`'s deterministic build output produces identical binaries across runs (minor differences in mach-o LC_UUID are fine — notarytool handles it).

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| macOS binaries shipped per release | 0 | 1 (aarch64 `.dmg`) | v0.2.0 (Month-1) | GitHub Releases artifact list |
| macOS binary pass rate through Gatekeeper | N/A | 100% | v0.2.0 onward | CI `spctl --assess` exit code, monitored per-release |
| Homebrew cask install success rate | N/A | 100% on clean macOS 13+ | Month-1 | Manual smoke test on 3 macOS versions (13/14/15); user reports via GitHub Issues |
| Intel x86_64 build parity lag | N/A | Never more than 2 releases behind aarch64 | Month-6 | GitHub Releases comparison aarch64 vs x86_64 presence |
| macOS download conversion (DMG downloads / Release-page views) | 0% | 25% of macOS visitors download (matches Linux baseline 30%) | Month-3 | GitHub Release asset download analytics |
| Keybinding regression reports on Linux post-EP-003 merge | N/A | Zero | Month-1 | GitHub Issues labeled `linux-regression` filed within 14 days of merge |
| Time from tag push → signed + notarized `.dmg` on GitHub Releases | N/A | <30 min (median), <2h (p99) | v0.2.0 onward | CI workflow duration per release |
| Self-hosted Homebrew cask auto-update success rate | N/A | 100% | Month-3 | Casks/paneflow.rb version bumps align with GitHub Releases; `brew audit --cask --strict` passes on every release |

## Open Questions

- **Apple Developer account ownership and team setup:** is the `APPLE_TEAM_ID` an individual or organization account? Who is the backup maintainer with signing access? Arthur to confirm before US-015 starts, latest by Week 2.
- **Notarization keychain profile name:** should we use `notarytool store-credentials` with a profile name like `AC_PASSWORD` or pass credentials inline every time? Engineering preference; stored-profile approach is slightly cleaner but adds a one-time manual setup on each dev machine.
- **Homebrew tap vs. fork:** should the tap be a brand-new `ArthurDEV44/homebrew-paneflow` or added as `homebrew-cask-arthurdev44` alongside existing repos? Naming decision — maintainer preference. Default to `ArthurDEV44/homebrew-paneflow` unless overridden.
- **Intel x86_64 release cadence:** ship x86_64 only for .0 releases (v0.2.0, v0.3.0, …) or for every patch (v0.2.1, v0.2.2, …)? Recommend every release with `continue-on-error: true` — costs nothing if it succeeds, degrades gracefully if it fails.
- **Future Sparkle adoption:** do we migrate to Sparkle framework for macOS-native auto-update in a later PRD, or extend our existing custom update checker? Defer decision to post-v0.2.0 real-world telemetry (how often does the custom checker fail vs. succeed on macOS?).
- **Menu bar items (US-012 scope):** minimum set is clear (Quit, Copy, Paste, SelectAll, New Workspace, Close Workspace). Should we add `Help > About PaneFlow` pointing to the website and `PaneFlow > Preferences...` (Cmd+,) even if Preferences isn't wired yet? Recommend minimum set for v0.2.0, preferences in a follow-up story.
[/PRD]
