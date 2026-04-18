[PRD]
# PRD: Windows Port — Cross-Platform Expansion to Windows 10/11 x86_64

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-18 | Claude + Arthur | Initial draft — port Linux + macOS v0.1.x baseline to Windows 10 1809+ on x86_64-pc-windows-msvc, signed via Azure Trusted Signing, distributed via .msi + winget. ARM64 deferred to post-v1. |

## Problem Statement

PaneFlow ships on Linux (v0.1.7) and macOS (EP-001..005 livrés 2026-04-17/18) but has zero Windows presence, leaving ~35% of the global developer market (StatCounter 2026 Q1) without access. Three concrete problems block reach:

1. **Zero Windows binaries exist, despite the codebase being ~98% portable.** A deep swarm audit on 2026-04-18 (see `memory/research_windows_port_feasibility.md`) found three historically-feared blockers already neutralized: (a) PaneFlow uses `portable-pty 0.8` (`src-app/src/pty.rs:49`) — `native_pty_system()` dispatches ConPTY automatically on Windows, zero PTY rewrite; (b) `alacritty_terminal 0.26` ships a complete Windows path (`src/tty/windows/conpty.rs`, `windows-sys 0.59`, `miow 0.6`) verified directly in the crates.io registry source; (c) Zed's GPUI framework has been GA on Windows since October 2025 via a DirectX 11 backend with HLSL shaders. The remaining work is ~100 lines of `#[cfg]` guards, one IPC migration, and packaging/signing infrastructure — not a rewrite.

2. **The Windows terminal market has evolved — and PaneFlow is invisible.** Alacritty, WezTerm, and Ghostty all ship signed + winget-distributed Windows binaries. Microsoft's Windows Terminal is the default on Windows 11 but lacks split/workspace multiplexing beyond basic tabs. Developers who want GPU-accelerated rendering + tmux-style splits + modern UX install WezTerm or stay in Windows Terminal + external tmux-under-WSL. PaneFlow's Linux+macOS demo videos draw Windows developer interest in GitHub issues (speculative but plausible market signal), but every month without a Windows build is a month that default slot stays filled by a competitor.

3. **The install-method classifier, asset-format matcher, and IPC transport are all POSIX-locked today.** `src-app/src/ipc.rs:9-10` imports `std::os::unix::net::{UnixListener, UnixStream}` — these are cfg-gated to Unix in stdlib and produce compile errors on `target_os = "windows"`. `src-app/src/install_method.rs:111-206` probes `/usr/bin/paneflow`, `/etc/debian_version`, `/tmp/.mount_*` — none exist on Windows, so `InstallMethod::Unknown` always returns, disabling the in-app update prompt. `src-app/src/terminal.rs:986-1004` uses `libc::kill(pid, SIGKILL)` in `Drop` — `libc::kill` does not exist on Windows. These must land before the first Windows release, not after, or early-adopter Windows users silently freeze on their initial build and have no update flow.

**Why now:** (a) The macOS port (EP-001..005) just shipped with a CI matrix refactor that makes adding a fifth Windows leg materially cheaper than it would have been pre-macOS. (b) GPUI Windows DX11 reached GA in October 2025 — we are now 6 months into stability data, past the initial bug wave. (c) The `interprocess` crate for cross-platform IPC (the single non-trivial migration) has been stable for 2+ years. (d) Azure Trusted Signing ($9.99/mo) has been GA since April 2026 and is accessible to EU-registered businesses (Strivex qualifies), removing the historical EV-cert-with-HSM obstacle. (e) Every month of delay is continued invisibility to Windows developer market and continued drift of Ghostty/WezTerm as default answer.

## Overview

This PRD delivers a production-grade Windows port of PaneFlow built from the current Linux + macOS codebase, targeting **Windows 10 1809 x86_64** as the minimum and **Windows 11 x86_64** as the primary test target. The port ships as a signed `.msi` installer distributed via GitHub Releases (primary) and `winget install paneflow` (secondary) through a submission to `microsoft/winget-pkgs`. ARM64 Windows support is explicitly deferred to a follow-up PRD post-v1.

The work decomposes into 8 epics executed with a critical-path spike first:

**EP-W1 (P0, critical path) — Build unblocking & GPUI Windows enablement.** Identify a Zed commit that includes the stable DirectX 11 Windows backend (post October 2025), bump `[patch.crates-io]` + `gpui` git rev in workspace `Cargo.toml`, validate cross-platform regression (Linux + macOS builds still pass), and provision the `windows-2022` GitHub Actions runner with WiX Toolset + VS Build Tools. **This epic dérisques the single largest unknown — the GPUI pin bump** — before committing to any other work.

**EP-W2 (P0) — Runtime parity.** Patch the ~100 lines of POSIX-locked code: (a) `terminal.rs:986-1004` `libc::kill`/`SIGKILL` → `#[cfg(unix)]` guard + Windows `TerminateProcess` via `windows-sys`; (b) `terminal.rs:368, 946, 975` `PermissionsExt::mode()` / `chmod 0o755` → `#[cfg(unix)]` guards (Windows skips chmod); (c) `terminal.rs:381, 384` shell fallback `/bin/sh` → Windows chain `%ComSpec%` → `cmd.exe` → `powershell.exe`; (d) `install_method.rs:408` `std::os::unix::fs::symlink` → platform-neutral or `std::os::windows::fs::symlink_file`; (e) `workspace.rs:174-280` port scan + `terminal.rs:656-659` `cwd_now` already `#[cfg(target_os = "linux")]`-gated — add functional Windows stubs returning empty/None for v1 (real impl via `GetExtendedTcpTable` / `NtQueryInformationProcess` deferred).

**EP-W3 (P0) — IPC named-pipes migration.** Migrate `src-app/src/ipc.rs` from `std::os::unix::net::{UnixListener, UnixStream}` to `interprocess::local_socket::{LocalSocketListener, LocalSocketStream}`. The `interprocess` crate maps to Unix sockets on Linux/macOS and named pipes on Windows transparently. Add `#[cfg(windows)]` branch to `runtime_paths.rs` returning `\\.\pipe\paneflow`, cfg-guard the inode-based clobber detection (`ipc.rs:134-137`), and remove the chmod 0o600 call on Windows. Estimated diff: ~43 lines. Protocol (newline-delimited JSON-RPC) is byte-stream compatible — no wire format changes.

**EP-W4 (P0) — Install detection + update asset matching.** Add `InstallMethod::WindowsMsi { install_path: PathBuf }` variant detected via `%ProgramFiles%\PaneFlow\paneflow.exe` canonical path. Add `AssetFormat::Msi` variant matching `paneflow-<ver>-x86_64-pc-windows-msvc.msi`. Without these, a Windows user who installs via MSI receives no in-app update prompts.

**EP-W5 (P1) — PowerShell shell integration.** Port the zsh/bash/fish OSC 7 CWD hook (`terminal.rs:151-196`) to PowerShell (pwsh.exe) by injecting a `$PROMPT` override that emits OSC 7. CMD.exe gets a best-effort implementation via `%PROMPT%` env var. This is P1 (not release-blocker) because core terminal functionality works without OSC 7 — only auto-CWD-inheritance on split is affected, and users can manually `cd` as a workaround.

**EP-W6 (P0) — WiX packaging + Azure Trusted Signing.** Initialize `packaging/wix/main.wxs` via `cargo wix init`, add `[package.metadata.wix]` to `src-app/Cargo.toml`, onboard to Azure Trusted Signing (business verification can take 2-6 weeks — start in parallel at story start), create `scripts/sign-windows.ps1` wrapping `signtool.exe` with Azure Trusted Signing endpoint, and wire MSI production + signing into `release.yml`.

**EP-W7 (P0) — CI, distribution, website.** Add a fifth matrix leg to `release.yml` (`x86_64-pc-windows-msvc` on `windows-2022`, `continue-on-error: true` for parity with the Intel Mac leg). Add `windows-check` job to `ci.yml` running fmt + clippy + check + test on `--target x86_64-pc-windows-msvc`. Create `packaging/winget/` with 3 manifest YAMLs (installer, locale, version) and `.github/workflows/update-winget.yml` that opens a PR to `microsoft/winget-pkgs` on release. Fill the Windows column in `paneflow-site/src/components/download/download-view.tsx:88` (`WindowsIcon` already imported).

**EP-W8 (P0) — QA + smoke tests + known risks docs.** Smoke test on Windows 10 1809 (ConPTY minimum) and Windows 11 (DX11 rendering, HiDPI, multi-monitor). Document known upstream limitations in `docs/WINDOWS.md`: IME CJK panic (zed#12563), ConPTY Ctrl-C signal propagation (alacritty#3075), RDP initialization broken (zed#26692), devcontainer freeze (zed#49072), older GPU drivers `NoSupportedDeviceFound` (zed#28683).

Key decisions:
- **Windows 10 1809 minimum, not Windows 11.** Windows 10 1809 is the ConPTY API minimum (`CreatePseudoConsole` introduced there). Supporting older Windows 10 would require the deprecated `winpty` path — rejected as technical debt from day one. ~98% of active Windows installs per StatCounter 2026 are 1809+.
- **x86_64 only for v1. ARM64 deferred.** Windows ARM64 (Surface Pro X, Snapdragon X Elite laptops) is <2% of active Windows installs per 2026 data and GPUI's DX11 backend has documented ARM64 driver-reliability issues (zed#36798). Ship x86_64 first, reassess ARM64 in 6 months.
- **MSI via cargo-wix, not NSIS or InstallShield.** MSI is the Microsoft-canonical installer format, integrates with Group Policy, is supported first-class by winget, and `cargo-wix` is the idiomatic Rust workflow. NSIS is simpler but lacks Group Policy integration and winget prefers MSI.
- **winget as primary discovery channel, GitHub Releases as .msi direct download.** winget (`winget install paneflow`) is Microsoft's official package manager, pre-installed on Windows 11. `.msi` direct download covers Windows 10 users who haven't installed winget and enterprise users with offline installers.
- **Azure Trusted Signing over EV cert.** Since March 2024, EV certs no longer grant instant SmartScreen trust — both build reputation organically. Azure Trusted Signing ($9.99/mo) is materially cheaper than an EV cert ($300-$580/yr), avoids HSM hardware token requirements (FIPS 140-2 Level 2), and integrates cleanly with GitHub Actions via a service principal. Strivex (EU-registered business) qualifies for the business-only onboarding path.
- **No Microsoft Store distribution.** Store sandboxing (AppContainer) blocks arbitrary shell execution, PTY spawning, and named pipe access outside the package — all foundational to a terminal multiplexer. Direct MSI + winget is the only viable channel, matching every Rust terminal competitor.
- **`continue-on-error: true` on the Windows matrix leg (initial releases).** Parity with the Intel Mac leg. Allows shipping Linux + aarch64 macOS + x86_64 macOS even if the Windows build is flaky in the first 2-3 releases. Flip to `false` after 3 consecutive green Windows releases.
- **ARM64 + MSIX + Microsoft Store defer to follow-up PRD.** Scope discipline — this PRD delivers a working Windows x86_64 MSI with signed installer + winget + update flow. Everything else is future work.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Windows binaries shipped | 1 (`paneflow-<ver>-x86_64-pc-windows-msvc.msi` signed) | 1 x86_64 MSI + ARM64 MSI (deferred follow-up PRD) |
| Windows install UX | `winget install paneflow` works OR `.msi` double-click install works | `winget install paneflow` works; SmartScreen "Unknown Publisher" no longer shown (reputation built) |
| SmartScreen pass rate | First-run dialog says "signed by Strivex" (Azure Trusted Signing) — no "Unknown Publisher" block | 100% silent install (reputation-based bypass) |
| Update flow coverage | Windows MSI installs receive in-app update prompt for new `.msi` releases | Same; winget users update via `winget upgrade paneflow` |
| CI build time (full release) | <30 min for Linux + macOS (both arches) + Windows x86_64 | <35 min same, including smoke tests |
| Minimum Windows supported | 10 1809 x86_64 | 10 1809 x86_64 (held stable) |
| Keybinding platform conformance | All app-global bindings use `secondary` modifier (Ctrl on Windows) — unchanged from Linux | Same; plus audit of any cmd-hardcoded macOS bindings |
| Known upstream risks documented | `docs/WINDOWS.md` lists 5 known risks with GitHub issue links | Same; plus GitHub issues filed on PaneFlow side tracking each upstream |

## Target Users

### Windows 11 Developer (Primary persona)
- **Role:** Full-stack or systems developer on Windows 11, often with WSL2 installed but preferring native Windows tools for speed
- **Behaviors:** Uses Windows Terminal by default. Installs CLI tools via `winget` (`winget install`) or Scoop. Has Git for Windows. Often uses PowerShell 7 (pwsh.exe) instead of the legacy Windows PowerShell 5.1.
- **Pain points:** Windows Terminal has no first-class split panes beyond basic tabs. WezTerm on Windows feels non-native (custom titlebar, slightly off fonts). Ghostty is Swift on macOS only. Wants the PaneFlow UX seen in Linux demos but on Windows 11.
- **Current workaround:** Windows Terminal + separate browser tabs for "workspaces" OR WezTerm OR VS Code integrated terminal.
- **Success looks like:** `winget install paneflow`, first launch shows "Signed by Strivex" SmartScreen dialog, opens with `Ctrl+Shift+N` for new workspace, `Ctrl+Shift+D/E` for splits. Zero configuration to reach feature parity with Linux.

### Windows 10 Enterprise Developer
- **Role:** Developer on a locked-down Windows 10 22H2 machine in an enterprise context. Has no admin rights by default but can install `.msi` signed packages via deployment tools or user-scope install.
- **Behaviors:** Cannot use winget (often blocked by Group Policy). Downloads `.msi` from vendor sites. Uses Windows PowerShell 5.1 (pwsh 7 may not be pre-installed).
- **Pain points:** Unsigned or untrusted installers are blocked outright. EV-certed installers from small vendors still trigger SmartScreen. Wants a trusted, signed, installer-based install flow.
- **Current workaround:** Uses whatever IT has pre-approved — typically Windows Terminal or VS Code integrated terminal. Installs external tools rarely.
- **Success looks like:** `.msi` direct download from GitHub Releases, double-click install, SmartScreen says "Signed by Strivex", installs to `%ProgramFiles%\PaneFlow\` with Start Menu entry. Post-install, `paneflow.exe` on PATH.

### Cross-Platform Developer (Linux + macOS + Windows)
- **Role:** Developer who daily-drives Linux or macOS for work but uses Windows for personal/gaming/occasional work.
- **Behaviors:** Already installed PaneFlow on Linux or macOS. Has muscle memory for `Ctrl+Shift+D` (Linux) or `Cmd+Shift+D` (macOS). On Windows, expects `Ctrl+Shift+D` to match Linux.
- **Pain points:** Configuration drift between platforms. Wants identical `paneflow.json` schema to work across all three OSes.
- **Current workaround:** Maintains two config files, or uses a sync-git-repo approach.
- **Success looks like:** Same `paneflow.json` schema works on all three platforms (Windows config path: `%APPDATA%\paneflow\paneflow.json`). `Ctrl+Shift+D` works on Windows identically to Linux. No platform-specific config quirks.

### Winget-First Windows User
- **Role:** Developer who installs everything via `winget` — no direct-download mentality. Similar persona to the Homebrew-first macOS user.
- **Behaviors:** Scripts machine setup with a `winget import` manifest. Expects `winget upgrade` to handle all tools.
- **Pain points:** Manual `.msi` downloads force opening a browser, consent dialogs, UAC prompts — friction that scripts avoid.
- **Current workaround:** Adds missing tools via `scoop install` or accepts manual install for that one tool.
- **Success looks like:** `winget install paneflow`, `winget upgrade --all` upgrades PaneFlow along with everything else.

## Research Findings

Key findings that informed this PRD (full research in `memory/research_windows_port_feasibility.md`, generated by 2026-04-18 meta-code swarm audit):

### Competitive Context
- **WezTerm (Windows):** Ships signed `.msi` + winget package. Uses portable-pty for ConPTY. Custom titlebar works but renders slightly differently from native Windows 11 chrome.
- **Alacritty (Windows):** Ships signed `.msi` + winget. Dynamically loads `conpty.dll` from Windows Terminal if present. No split panes (core limitation).
- **Ghostty (Windows):** Not available — Swift-only codebase. Community requests exist but no port planned.
- **Zed Editor (Windows):** GA since October 2025 via DirectX 11 GPUI backend. Serves as the canonical reference for "GPUI app on Windows." Ships signed `.msi` + winget.
- **Microsoft Windows Terminal:** Native, pre-installed on Windows 11, but no split panes (tabs only), no GPU rendering beyond DirectWrite glyph rasterization. Not a direct competitor — different product category.
- **Market gap:** No Windows terminal combines (a) GPU-accelerated rendering via DirectX, (b) first-class splits + workspace multiplexing (tmux-style), (c) modern UX polish (rounded corners, dark theme coherence). WezTerm is closest but lacks workspace multiplexing. PaneFlow's Linux v0.1.7 + macOS v0.1.x already delivers all three; extending to Windows captures unserved developer share.

### Best Practices Applied
- **`portable-pty 0.8`'s `native_pty_system()`** — the canonical Rust crate for cross-platform PTY. Dispatches to `UnixPtySystem` on Linux/macOS and `ConPtySystem` (wraps `CreatePseudoConsole`) on Windows 10 1809+. PaneFlow already uses this (`src-app/src/pty.rs:49`). Zero PTY rewrite needed.
- **`interprocess` crate's `local_socket` module** — the modern Rust idiom for cross-platform IPC. `LocalSocketListener` / `LocalSocketStream` present a single API that resolves to Unix domain sockets on POSIX and named pipes (`\\.\pipe\...`) on Windows. Used by rust-analyzer and tauri-apps IPC. Protocol-agnostic (newline-delimited JSON-RPC works identically over named pipes).
- **`windows-sys` crate with per-feature imports** — the canonical way to call Win32 APIs from Rust. Features requested per API (e.g. `Win32_System_Threading` for `TerminateProcess`) to minimize compile time and binary size.
- **`cargo-wix` for MSI generation** — the idiomatic Rust→MSI workflow. Used by Lapce, Helix, and Alacritty. Generates `wix/main.wxs` + integrates `signtool` in the build step. MSI format is Microsoft-canonical and winget-preferred over NSIS.
- **Azure Trusted Signing over EV certs** — since the March 2024 SmartScreen change, both OV and EV certs build reputation organically. Azure Trusted Signing ($9.99/mo Basic) provides identity anchored to a verified business, integrates with GitHub Actions via a service principal (no HSM hardware token required), and requires `signtool` ≥ 10.0.26100. Referenced workflow: Tauri v2 Windows signing docs.
- **GPUI DirectX 11 backend** — Zed's official Windows GPU backend, GA October 2025. HLSL shaders (3× implementation count vs Metal+Vulkan). Confirmed stable in weekly Zed releases. Adopted by bumping PaneFlow's GPUI pin from `0b984b5` (pre-DX11) to a post-October-2025 commit.
- **winget submission via PR to microsoft/winget-pkgs** — the canonical distribution channel. Requires 3 YAML manifests (installer metadata, locale strings, version anchor), a versioned MSI URL, SHA-256 hash, and a signed binary. Unsigned binaries trigger Defender/SmartScreen friction but winget itself does not require EV certs.

*Full research sources: `memory/research_windows_port_feasibility.md` (2026-04-18 meta-code swarm — 7 parallel agents), direct source reads of `/home/arthur/.cargo/registry/src/index.crates.io-*/alacritty_terminal-0.26.0/`, Zed Windows progress blog, Azure Trusted Signing docs, Tauri v2 Windows signing guide.*

## Assumptions & Constraints

### Assumptions (to validate)
- **A Zed commit exists post October 2025 that pins a stable DirectX 11 GPUI backend** — validated indirectly (Zed itself ships weekly Windows releases from HEAD per their release cadence) but not pinpointed to a specific rev. Validation spike is US-001.
- **GPUI pin bump does not regress Linux or macOS builds** — Zed itself maintains all three platforms from HEAD, so this should hold, but two required `[patch.crates-io]` patches (`async-task` fork, `calloop` fork) may have changed since `0b984b5`. Validation in US-002.
- **Azure Trusted Signing business verification approves Strivex within 6 weeks** — Microsoft docs advertise "days to months" depending on identity documentation quality. Arthur's Strivex France SAS registration should satisfy standard KYC. If verification exceeds 6 weeks, fall back to OV cert ($150/yr) with documented reputation-build period.
- **`windows-2022` GitHub Actions runner is sufficient for MSI build + signtool signing** — GitHub's docs confirm `windows-2022` includes Visual Studio Build Tools and Windows SDK; `signtool.exe` is at `C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe`. Fallback to a self-hosted runner if CI time exceeds 30 min.
- **`interprocess` crate's `LocalSocketStream::try_clone` is supported on both Unix and Windows** — required because `ipc.rs:140` currently calls `stream.try_clone()`. Version 2.x exposes this on both platforms per docs.rs, but a 5-minute spike should confirm before committing.
- **winget submission PR review SLA is <2 weeks** — not documented, but community reports suggest 3-10 days for signed installers from verified publishers. Acceptable latency for release-plus-one-week distribution.
- **Windows users accept that `workspace.rs` port detection is non-functional in v1** — the services sidebar will show empty on Windows for the first release, then gain a real `GetExtendedTcpTable`-based implementation in a follow-up PRD. Documented in `docs/WINDOWS.md`.

### Hard Constraints
- **No Microsoft Store / MSIX distribution for v1.** App Store sandboxing (AppContainer) blocks arbitrary PTY spawning, shell execution outside the package, and named pipe access — all foundational to a terminal multiplexer.
- **No WinPTY fallback for pre-1809 Windows 10.** ConPTY-only. Deprecated WinPTY is technical debt from day one.
- **Must not regress Linux or macOS behavior.** Every existing keybinding, config file, IPC socket path, and PTY spawn must continue to work identically on Linux and macOS after the cross-platform refactor. Zero behavioral changes visible to existing users.
- **Single codebase.** No forking `src-app/src/main_windows.rs`. All platform differences behind `#[cfg]` attributes or runtime dispatch inside shared functions.
- **No Win32 C/C++ code in PaneFlow's own source.** All Win32 API calls go through `windows-sys` Rust bindings.
- **GPUI pin is shared across all platforms.** If the post-DX11 rev requires a regression fix on Linux or macOS, that fix is in-scope for this PRD (not a separate PRD). The bump is pan-platform.
- **No ARM64 Windows in v1.** `x86_64-pc-windows-msvc` only. Follow-up PRD after v1 stabilizes.

## Quality Gates

These commands must pass for every user story:
- `cargo fmt --check` — formatting
- `cargo clippy --workspace -- -D warnings` — lint (on the host OS of the PR)
- `cargo test --workspace` — all tests (config crate, no UI tests in app crate)
- `cargo check --target x86_64-pc-windows-msvc` — cross-platform compile verification (runs on `windows-2022` runner in CI, or locally on a Windows machine; skipped on pure-Linux/macOS local dev)
- `cargo check --target aarch64-apple-darwin` — macOS regression check (runs on `macos-14` runner in CI)
- `cargo check` (host Linux) — Linux regression check

For UI stories, additional gate:
- Manual launch on Windows 10 1809 AND Windows 11: verify the specific UI behavior — screenshot attached to PR.

For packaging/CI stories, additional gate:
- CI run green on `windows-2022` runner.
- Resulting `.msi` passes `signtool verify /pa paneflow-<ver>-x86_64-pc-windows-msvc.msi` with valid signature.
- SmartScreen dialog on a clean Windows VM shows "Signed by Strivex" (not "Unknown Publisher").

## Epics & User Stories

### EP-W1: Build Unblocking & GPUI Windows Enablement

Critical-path epic. Identify and adopt a GPUI commit that includes the stable DirectX 11 Windows backend, validate cross-platform regression on Linux + macOS, and provision the Windows CI runner. Without this epic, no Windows code even compiles.

**Definition of Done:** `cargo build --release --target x86_64-pc-windows-msvc` succeeds on a `windows-2022` GitHub Actions runner at the new GPUI rev, producing an executable under `target/x86_64-pc-windows-msvc/release/paneflow.exe`. Linux and macOS builds still pass with zero regressions.

#### US-001: Spike — Identify Zed commit with stable DirectX 11 Windows backend
**Description:** As the PaneFlow maintainer, I want to identify a specific Zed commit post October 2025 that includes the stable DirectX 11 GPUI backend so that I can confidently bump the PaneFlow pin without guesswork.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Given the Zed GitHub repo, when the commit log is searched for "DirectX" or "d3d11" from October 2025 onwards, then a specific commit SHA is identified that introduces the DX11 backend as the default Windows renderer.
- [ ] Given the identified commit, when `tasks/spike-windows-build.md` is created, then it documents the commit SHA, the date, the PR link if available, and any associated `[patch.crates-io]` changes versus `0b984b5`.
- [ ] Given the identified commit, when `git log <0b984b5>..<new-sha> -- crates/gpui/src/platform/windows/` is run on a local Zed checkout, then a non-empty changeset proves DirectX 11 Windows work landed between the two revs.
- [ ] Given the spike document, when the maintainer reviews it, then the recommended new rev is either explicitly approved for US-002 or the spike recommends waiting with a documented reason.

#### US-002: Bump GPUI pin + validate Linux/macOS regression
**Description:** As the PaneFlow maintainer, I want the `gpui`, `gpui_platform`, and `collections` git revs in workspace `Cargo.toml` bumped to the US-001 commit so that Windows compilation becomes possible, without breaking Linux or macOS builds.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Given `Cargo.toml` at the workspace root, when the three `rev = "0b984b5..."` entries are updated to the US-001 SHA, then `cargo update -p gpui -p gpui_platform -p collections` completes with a refreshed `Cargo.lock`.
- [ ] Given the `[patch.crates-io]` block for `async-task` and `calloop`, when the new GPUI rev requires different patch revs, then the patches are updated per the Zed monorepo `Cargo.toml` at the new SHA.
- [ ] Given `cargo build --release` on Linux, when run after the bump, then the build completes with zero errors and the resulting binary launches to the sidebar (manual smoke test).
- [ ] Given `cargo build --release --target aarch64-apple-darwin` on the CI `macos-14` runner, when run after the bump, then the build completes with zero errors.
- [ ] Given the Linux regression test, when any existing keybinding is fired in the post-bump binary, then the binding behaves identically to pre-bump (no behavior change).
- [ ] Given a regression surfaces (compile error or runtime panic) in Linux or macOS, when triaged, then it is either (a) fixed in-PRD as a new story, (b) reverted with the spike reopened, or (c) documented as known-and-acceptable. No silent regressions.

#### US-003: Provision windows-2022 CI runner with WiX + VS Build Tools
**Description:** As the PaneFlow maintainer, I want a GitHub Actions job configuration that runs on `windows-2022`, pre-provisioned with Visual Studio Build Tools, Windows SDK, and WiX Toolset, so that all downstream Windows stories have a working CI environment.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Given `.github/workflows/ci.yml`, when the `windows-check` job is defined, then it specifies `runs-on: windows-2022` and sets up `actions/checkout@v4` + `dtolnay/rust-toolchain@stable` with `targets: x86_64-pc-windows-msvc`.
- [ ] Given the `windows-2022` runner, when the job runs, then `signtool.exe`, `cl.exe`, and `link.exe` are all on PATH (verified via a `where` command step).
- [ ] Given the WiX Toolset requirement for later stories, when the job installs WiX via `dotnet tool install --global wix` or `cargo install cargo-wix --locked`, then the install step completes in <5 min and caches across subsequent runs.
- [ ] Given `cargo check --target x86_64-pc-windows-msvc --workspace` is the job's smoke step, when it runs after US-002 is merged, then the cross-platform compile check surfaces any remaining POSIX-locked code for EP-W2 to fix.
- [ ] Given the job completes, when CI time is measured, then it is <15 min on first run, <8 min on cached subsequent runs. If slower, document a follow-up optimization story.

---

### EP-W2: Runtime Parity

Close the functional gaps between Linux/macOS and Windows for process lifecycle, file permissions, shell invocation, symlinks, and platform-specific stubs. Without this epic, `cargo check --target x86_64-pc-windows-msvc` fails with compile errors from `std::os::unix` imports and `libc` calls that don't exist on Windows.

**Definition of Done:** `cargo check --target x86_64-pc-windows-msvc --workspace` passes with zero warnings (with `-D warnings`) on the `windows-2022` runner. Linux and macOS builds unchanged.

#### US-004: Cfg-guard POSIX process kill + add Windows TerminateProcess path
**Description:** As a Windows user closing a terminal pane, I want the child shell process to be killed cleanly so that zombie processes do not accumulate, matching the Linux `SIGKILL` behavior.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given `src-app/src/terminal.rs:986-1004` (the `libc::kill(pid, 0)` / `libc::kill(pid, SIGKILL)` block inside `Drop`), when it is refactored, then the existing code is wrapped in `#[cfg(unix)]` and a new `#[cfg(windows)]` branch is added using `windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_TERMINATE}`.
- [ ] Given a running PTY child on Windows, when `TerminalState::drop()` is invoked, then `OpenProcess(PROCESS_TERMINATE, FALSE, pid)` returns a valid handle, `TerminateProcess(handle, 1)` returns non-zero, and `WaitForSingleObject(handle, 5000)` returns `WAIT_OBJECT_0`.
- [ ] Given the child has already exited, when `OpenProcess` returns `NULL` (`ERROR_INVALID_PARAMETER`), then the function returns early without panic or error log above `debug` level.
- [ ] Given the `windows_sys = "0.59"` dep is not yet declared, when added to `[target.'cfg(windows)'.dependencies]` in `src-app/Cargo.toml`, then it enables only the minimum feature set: `["Win32_System_Threading", "Win32_Foundation"]`.
- [ ] Given Linux regression, when the same code runs on Linux, then `libc::kill` is still used with no behavior change (gated by `#[cfg(unix)]`).
- [ ] Given macOS regression, when the code runs on macOS, then `libc::kill` is still used (macOS matches `cfg(unix)`).
- [ ] Given a Windows manual test where the main window is closed while a pane has a running `sleep 60` child, when observed in Task Manager, then the child `sleep` process terminates within 5 seconds.

#### US-005: Cfg-guard PermissionsExt chmod calls
**Description:** As the PaneFlow maintainer, I want the 3 `std::os::unix::fs::PermissionsExt` usages in `terminal.rs` wrapped in `#[cfg(unix)]` guards so that the Windows build compiles without spurious chmod attempts.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given `src-app/src/terminal.rs:368` (inline `PermissionsExt::mode()` check for shell executability), when the call is wrapped in `#[cfg(unix)]`, then the Windows branch uses `Path::exists()` + `metadata().is_ok()` as the equivalent executable-presence check.
- [ ] Given `src-app/src/terminal.rs:946` and `:975` (wrapper script `chmod 0o755`), when both are wrapped in `#[cfg(unix)]`, then Windows skips the chmod entirely (Windows permissions work via ACLs, not POSIX mode bits — no-op is correct).
- [ ] Given `src-app/src/self_update/mod.rs:27` and related files, when `use std::os::unix::fs::PermissionsExt` is encountered, then it is either wrapped in `#[cfg(unix)]` or the entire self-update module section is cfg-guarded if Windows self-update takes a different code path.
- [ ] Given Linux regression, when the app builds on Linux, then all chmod calls still execute as before (gated by `#[cfg(unix)]`).
- [ ] Given `cargo check --target x86_64-pc-windows-msvc`, when run after this story, then zero "cannot find `PermissionsExt`" errors remain.

#### US-006: Windows shell fallback chain
**Description:** As a Windows user launching PaneFlow, I want the default shell to resolve to a working Windows shell (PowerShell or cmd) so that the terminal opens a usable command prompt instead of failing to spawn.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given `src-app/src/terminal.rs:381-384` (`std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())`), when the logic is refactored into a cfg-gated helper function, then the Windows branch resolves via the chain: (1) config `default_shell`, (2) `%ComSpec%` env var, (3) `C:\Windows\System32\cmd.exe`, (4) `powershell.exe` on PATH, (5) error with clear log message.
- [ ] Given a Windows user with PowerShell 7 (pwsh.exe) installed, when they set `"default_shell": "pwsh.exe"` in `paneflow.json`, then the config override resolves first and pwsh.exe is spawned as the shell.
- [ ] Given `src-app/src/terminal.rs:151-196` (`setup_shell_integration` which rsplits the shell path on `/` to get the basename), when it runs on Windows with shell path `C:\Windows\System32\cmd.exe`, then the basename extraction uses `Path::file_name().map(|s| s.to_string_lossy())` (platform-neutral) and correctly identifies `cmd.exe`.
- [ ] Given the path-separator agnostic refactor, when it runs on Linux with shell `/bin/zsh`, then the basename is still `zsh` (no regression).
- [ ] Given Windows integration tests aren't present today, when this story ships, then a manual smoke test on Windows 11 confirms the default shell opens correctly with no env vars set.

#### US-007: Platform-neutral symlink in install_method.rs
**Description:** As the PaneFlow maintainer, I want `install_method.rs:408`'s `std::os::unix::fs::symlink` call replaced with a cross-platform equivalent so that the Windows build compiles.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given `src-app/src/install_method.rs:408`, when the symlink call is refactored, then it uses `#[cfg(unix)] std::os::unix::fs::symlink(src, dst)` and `#[cfg(windows)] std::os::windows::fs::symlink_file(src, dst)` (Windows requires `SeCreateSymbolicLinkPrivilege` — document this in a comment).
- [ ] Given Windows users without admin rights (default), when `symlink_file` fails with `ERROR_PRIVILEGE_NOT_HELD`, then the error is caught and the code falls back to `std::fs::copy(src, dst)` with a `warn!` log message.
- [ ] Given Linux regression, when the code runs on Linux, then `unix::fs::symlink` is used and behavior is unchanged.
- [ ] Given the MSI installer (US-017) configures the `paneflow.exe` location in `%ProgramFiles%\PaneFlow\`, when PaneFlow's self-update tries to create a symlink for version promotion, then either the symlink-or-copy fallback works OR the self-update module skips symlink creation on Windows entirely (documented design choice in the PR).

#### US-008: Windows stubs for workspace port scan + cwd_now
**Description:** As a Windows user, I want the services sidebar and split CWD inheritance to not crash on Windows, accepting that port detection returns empty and CWD inheritance falls back to shell-reported OSC 7 only (no `/proc` equivalent v1).

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given `src-app/src/workspace.rs:425-428` (already `#[cfg(not(any(target_os = "linux", target_os = "macos")))]` stub returning `vec![]` for `detect_ports`), when verified, then the stub is confirmed to cover `target_os = "windows"` and no additional code is needed — the services sidebar shows empty on Windows.
- [ ] Given `src-app/src/terminal.rs:738-741` (similar `cfg(not(any))]` stub returning `None` for `cwd_now`), when verified, then the stub covers Windows and CWD inheritance falls back to OSC 7 shell-integration data when available.
- [ ] Given `docs/WINDOWS.md` (new file to be created in US-022), when this story's behavior is documented, then users are told: "Services sidebar is empty on Windows in v1 — functional `GetExtendedTcpTable` implementation deferred to post-v1." and "Split pane CWD inheritance on Windows relies on PowerShell OSC 7 shell integration (US-013) or manual `cd` after split."
- [ ] Given Linux regression, when the Linux build runs, then `/proc`-based port scan and CWD detection are unchanged.

---

### EP-W3: IPC Named-Pipes Migration

Migrate the Unix-domain-socket JSON-RPC IPC server to the `interprocess` crate so that the same code path works on both POSIX (Unix sockets) and Windows (named pipes). Without this epic, `cargo check --target x86_64-pc-windows-msvc` fails with "cannot find `UnixListener`" errors.

**Definition of Done:** `ipc.rs` uses `interprocess::local_socket::*` types, compiles on all three platforms, and a `system.ping` JSON-RPC round-trip succeeds on Windows via `\\.\pipe\paneflow` with no protocol changes visible to existing clients.

#### US-009: Migrate ipc.rs to interprocess crate + runtime_paths Windows branch
**Description:** As a Windows user, I want the IPC server to bind successfully so that any future CLI tools or shell-integration scripts can communicate with the running PaneFlow instance.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given `src-app/Cargo.toml`, when the `interprocess = "2"` dep is added, then workspace resolver accepts it without version conflicts.
- [ ] Given `src-app/src/ipc.rs:8-10`, when the `use std::os::unix::net::{UnixListener, UnixStream}` import is replaced with `use interprocess::local_socket::{ListenerOptions, Listener, Stream, prelude::*}`, then compile errors for these types disappear.
- [ ] Given `src-app/src/ipc.rs:117` (the `UnixListener::bind(socket_path)` call), when it is replaced with `ListenerOptions::new().name(name).create_sync()?`, then the listener binds successfully on both Linux (to the Unix socket path) and Windows (to `\\.\pipe\paneflow`).
- [ ] Given `src-app/src/ipc.rs:128` (`set_permissions(_, Permissions::from_mode(0o600))`), when the call is wrapped in `#[cfg(unix)]`, then Windows skips the chmod (named pipes use Windows ACLs, not POSIX mode bits).
- [ ] Given `src-app/src/ipc.rs:134-137` (`socket_inode()` via `MetadataExt::ino()`), when the entire clobber-detection block is wrapped in `#[cfg(unix)]`, then the 5-second re-bind loop only runs on Unix (named pipes don't persist stale files the same way).
- [ ] Given `src-app/src/runtime_paths.rs:37-50`, when a `#[cfg(windows)]` branch is added, then `socket_path()` returns `r"\\.\pipe\paneflow"` on Windows and the existing XDG/TMPDIR chain is preserved on Unix via `#[cfg(unix)]`.
- [ ] Given `src-app/src/ipc.rs:140` (`stream.try_clone()`), when it runs on `interprocess::local_socket::Stream`, then `try_clone()` succeeds on both Unix and Windows per the crate's API (verify in docs before merge — if not available, refactor to use a `BufReader + BufWriter` split approach).
- [ ] Given the complete migration, when a minimal JSON-RPC client on Windows sends `{"jsonrpc":"2.0","id":1,"method":"system.ping"}\n` over `\\.\pipe\paneflow`, then the server responds with `{"jsonrpc":"2.0","id":1,"result":"pong"}\n` within 100ms.
- [ ] Given Linux regression, when the same code runs on Linux with `$XDG_RUNTIME_DIR` set, then the socket binds at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` with mode 0o600 and the clobber-detection loop fires as before.
- [ ] Given macOS regression, when the code runs on macOS with `$TMPDIR` set, then the socket binds under `$TMPDIR/paneflow/paneflow.sock` with no behavior change from EP-002 US-004.

---

### EP-W4: Install Detection + Update Asset Matching

Extend the install-method classifier and update-asset matcher to recognize Windows MSI installs. Without this epic, Windows MSI users receive no in-app update prompts and silently miss releases.

**Definition of Done:** A Windows user who installs `paneflow-<ver>-x86_64-pc-windows-msvc.msi` sees `InstallMethod::WindowsMsi` detected on startup, and the update checker correctly matches new `.msi` assets from GitHub Releases for x86_64 Windows.

#### US-010: InstallMethod::WindowsMsi variant + detection logic
**Description:** As a Windows user who installed PaneFlow via MSI to `%ProgramFiles%\PaneFlow\`, I want the in-app update checker to correctly identify my install method so that the update flow works.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Given `src-app/src/install_method.rs` `InstallMethod` enum, when extended, then a new `WindowsMsi { install_path: PathBuf }` variant is added with a clear doc comment explaining the detection rule.
- [ ] Given the `classify()` function, when it runs on Windows and the running executable resides under `%ProgramFiles%\PaneFlow\paneflow.exe` OR `%LocalAppData%\Programs\PaneFlow\paneflow.exe` (per-user MSI), then it returns `InstallMethod::WindowsMsi { install_path }`.
- [ ] Given `classify()` on Windows, when the executable is outside any standard install location (e.g., run from `target/release/paneflow.exe` during dev), then it returns `InstallMethod::Unknown`.
- [ ] Given Linux regression, when `classify()` runs on Linux, then all existing branches (`SystemPackage`, `AppImage`, `TarGz`, `Unknown`) behave identically.
- [ ] Given macOS regression, when `classify()` runs on macOS, then the `AppBundle` variant from macOS EP-002 US-007 behaves identically.
- [ ] Given a Rust unit test, when `paneflow.exe` paths under `C:\Program Files\PaneFlow\` and `C:\Users\<name>\AppData\Local\Programs\PaneFlow\` are fed to the classifier (mock-based, no actual FS), then both return `WindowsMsi` with correct `install_path`.

#### US-011: AssetFormat::Msi variant + pick_asset matching
**Description:** As a Windows user running PaneFlow, I want the update checker to match the correct `.msi` asset from GitHub Releases for my architecture so that I receive the update prompt.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] Given `src-app/src/update_checker.rs` `AssetFormat` enum, when extended, then a `Msi` variant is added with the pattern `.msi` and suffix matching `x86_64-pc-windows-msvc.msi`.
- [ ] Given `pick_asset(release, target_arch)` on Windows x86_64, when the release contains `paneflow-0.2.0-x86_64-pc-windows-msvc.msi`, then the function returns that asset URL.
- [ ] Given a release with no matching Windows asset (e.g., a Linux-only hotfix), when `pick_asset` runs on Windows, then it returns `None` and the update prompt defers silently.
- [ ] Given the `InstallMethod::WindowsMsi` classifier from US-010, when paired with `AssetFormat::Msi`, then the update flow identifies: "Windows MSI install → fetch .msi → show update prompt with download link."
- [ ] Given Linux + macOS regression, when `pick_asset` runs on those platforms, then deb/rpm/AppImage/TarGz/Dmg matching is unchanged.
- [ ] Given a unit test in `update_checker`, when a mock release with a `.msi` asset is passed to `pick_asset` with `target_arch = "x86_64-pc-windows-msvc"`, then the correct URL is returned.

---

### EP-W5: PowerShell Shell Integration (P1)

Port the zsh/bash/fish OSC 7 CWD reporter to PowerShell so that split-pane CWD inheritance works on Windows. This is P1 (not release-blocker) because core terminal functionality works without OSC 7 — only the convenience of auto-CWD-inheritance on split is affected.

**Definition of Done:** When a user splits a pane on Windows running PowerShell 7, the new pane inherits the parent's current directory via OSC 7, matching Linux/macOS behavior.

#### US-012: PowerShell setup_shell_integration branch
**Description:** As a Windows user using PowerShell 7, I want my current directory to be inherited when I split a pane, so that the new pane opens in the same directory without me manually `cd`'ing.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-006, US-009

**Acceptance Criteria:**
- [ ] Given `src-app/src/terminal.rs:151-196` (`setup_shell_integration` which handles `zsh`, `bash`, `fish`), when extended, then a new branch for `powershell.exe` / `pwsh.exe` injects a `$PROMPT` function override (or prepends to an existing one) that emits the OSC 7 escape sequence with the current `$PWD`.
- [ ] Given the OSC 7 sequence (`ESC ] 7 ; file://<hostname><path> ESC \`), when emitted from PowerShell after each prompt, then PaneFlow's OSC 7 parser receives it and updates the pane's tracked CWD.
- [ ] Given a user splits the pane, when the new pane spawns a child shell, then it launches with the parent's tracked CWD as `cwd` (matching Linux/macOS behavior).
- [ ] Given a user has a custom `$PROFILE` in PowerShell, when PaneFlow injects its OSC 7 hook, then the injection uses `function prompt { <user-prompt> + <osc7-emit> }` wrapper pattern (non-destructive — user's prompt still renders).
- [ ] Given `cmd.exe` as the shell (no scripting hook equivalent to `$PROMPT`), when the user is on `cmd.exe`, then OSC 7 integration is skipped with a `info!` log message; CWD inheritance falls back to none (documented in `docs/WINDOWS.md`).
- [ ] Given Linux + macOS regression, when `setup_shell_integration` runs for zsh/bash/fish, then behavior is unchanged.
- [ ] Given a manual Windows 11 + PowerShell 7 test, when splitting a pane in a non-home directory, then the new pane opens in that same directory.

---

### EP-W6: WiX Packaging + Azure Trusted Signing

Produce a signed `.msi` installer via `cargo-wix` + `signtool` with Azure Trusted Signing. Without this epic, the Windows binary exists but cannot be distributed trustworthily (SmartScreen blocks unsigned installers aggressively).

**Definition of Done:** A signed `paneflow-<ver>-x86_64-pc-windows-msvc.msi` is produced by the CI `release.yml` Windows matrix leg, passes `signtool verify /pa`, and on a clean Windows 11 VM shows "Signed by Strivex" in the SmartScreen dialog (not "Unknown Publisher").

#### US-013: Initialize WiX config (packaging/wix + Cargo.toml metadata)
**Description:** As the PaneFlow maintainer, I want a `packaging/wix/main.wxs` authored and `[package.metadata.wix]` configured in `src-app/Cargo.toml` so that `cargo wix build` produces a valid MSI installer with Start Menu entries and a clean uninstaller.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] Given the workspace root, when `cargo install cargo-wix --locked` is run, then `cargo-wix` is installed and `cargo wix init --package paneflow-app` generates `packaging/wix/main.wxs` (rename from default `wix/` to `packaging/wix/` for consistency with `packaging/homebrew/` and `packaging/apt/` conventions).
- [ ] Given `packaging/wix/main.wxs`, when the template is customized, then: (a) install directory is `%ProgramFiles%\PaneFlow\`, (b) a Start Menu shortcut to `paneflow.exe` is created under "PaneFlow", (c) an uninstaller entry is registered in `Add or Remove Programs` with the correct publisher name "Strivex" and icon.
- [ ] Given `src-app/Cargo.toml`, when `[package.metadata.wix]` is added, then it declares `upgrade-guid` and `path-guid` (generate both once with `cargo wix init`; treat as stable forever — changing them breaks upgrade paths for existing users).
- [ ] Given `cargo wix build --package paneflow-app --target x86_64-pc-windows-msvc` on the `windows-2022` runner, when run, then a `target/wix/paneflow-<ver>-x86_64.msi` file is produced of size <50 MB and opens correctly via double-click on Windows 11.
- [ ] Given the MSI is installed, when the user launches `paneflow.exe` from the Start Menu, then it opens with zero config errors (first-launch config file creation works under `%APPDATA%\paneflow\`).
- [ ] Given the MSI is uninstalled via `Add or Remove Programs`, when completed, then `%ProgramFiles%\PaneFlow\` is removed cleanly (but user config at `%APPDATA%\paneflow\` is preserved — matching idiomatic Windows uninstall behavior).

#### US-014: Azure Trusted Signing onboarding + secret provisioning
**Description:** As the PaneFlow maintainer, I want a Strivex-verified Azure Trusted Signing account with a service principal whose credentials are in GitHub Secrets, so that CI can sign MSI files without human interaction.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None (can run in parallel with US-001..US-013; business verification latency is 2-6 weeks)

**Acceptance Criteria:**
- [ ] Given the Azure Portal, when an Azure Trusted Signing account is created under the Strivex subscription, then it completes business identity verification using Strivex's France SAS registration documents within 6 weeks (SLA variable — start early).
- [ ] Given the verified account, when a Certificate Profile "PaneFlow-Release" is created with type "Public Trust" (for SmartScreen), then it provisions an ACME-rotated certificate chained to a CA Microsoft trusts.
- [ ] Given a service principal in Azure AD, when it is granted the `Trusted Signing Certificate Profile Signer` role on the Certificate Profile, then `az account get-access-token` with this service principal returns a valid token.
- [ ] Given GitHub Secrets for the repo, when the following secrets are populated, then subsequent CI runs can authenticate: `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` (service principal), `AZURE_TRUSTED_SIGNING_ENDPOINT`, `AZURE_TRUSTED_SIGNING_ACCOUNT`, `AZURE_TRUSTED_SIGNING_CERT_PROFILE`.
- [ ] Given the onboarding completes, when documented in `memory/project_windows_signing.md` (new memory file), then the approach, cost ($9.99/mo Basic tier), certificate rotation policy, and failure-mode playbook are all captured for future maintenance.
- [ ] Given onboarding fails or exceeds 6 weeks, when the maintainer decides to fall back, then the story is closed with `resolution: fallback-to-OV-cert` and a new story is created to procure a Sectigo OV cert (~$150/yr) — US-015/016 adapt accordingly.

#### US-015: scripts/sign-windows.ps1 signtool wrapper
**Description:** As the release pipeline, I want a `scripts/sign-windows.ps1` that wraps `signtool.exe` with Azure Trusted Signing parameters so that CI can invoke it uniformly per artifact.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-013, US-014

**Acceptance Criteria:**
- [ ] Given `scripts/sign-windows.ps1` (new file), when invoked as `scripts\sign-windows.ps1 -InputFile "<path>\paneflow-x.y.z.msi"`, then it calls `signtool.exe sign /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib <AzureDlibPath> /dmdf <metadata.json> "<InputFile>"` where `<metadata.json>` contains the Azure Trusted Signing endpoint, account, and certificate profile from env vars.
- [ ] Given the env vars `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TRUSTED_SIGNING_*` are set, when the script runs, then it authenticates silently and signs the MSI with no interactive prompt.
- [ ] Given the signed MSI, when `signtool verify /pa /v <path>.msi` is run, then the output shows "Successfully verified" with the Strivex certificate chain.
- [ ] Given a timestamp server failure (rare but possible), when `signtool` returns a non-zero exit code, then the script exits with a clear error message and CI marks the step failed.
- [ ] Given missing Azure env vars, when the script runs, then it exits early with a clear error listing the required vars.
- [ ] Given the signing completes, when the MSI is inspected with `Get-AuthenticodeSignature`, then `Status: Valid` and `SignerCertificate.Subject` contains "Strivex".

#### US-016: MSI build + sign steps in release.yml
**Description:** As the release pipeline, I want MSI production + signing steps integrated into the `release.yml` Windows matrix leg so that every tagged release auto-produces a signed `.msi` asset.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-013, US-015, US-017

**Acceptance Criteria:**
- [ ] Given the `release.yml` Windows matrix leg from US-017, when the build completes, then a new step "Produce MSI" runs `cargo wix build --package paneflow-app --target x86_64-pc-windows-msvc --install-version ${TAG#v}` gated by `if: runner.os == 'Windows'`.
- [ ] Given the MSI is produced, when the next step "Sign MSI" runs, then it invokes `scripts/sign-windows.ps1 -InputFile <msi-path>` with Azure secrets injected as env vars.
- [ ] Given the signed MSI, when the "Stage Windows assets" step runs, then it copies the MSI to `release-assets/paneflow-${TAG#v}-x86_64-pc-windows-msvc.msi` and computes a SHA-256 sidecar `paneflow-${TAG#v}-x86_64-pc-windows-msvc.msi.sha256`.
- [ ] Given the Windows matrix leg has `continue-on-error: true` (parity with Intel Mac), when signing fails (e.g., Azure Trusted Signing outage), then the Linux + macOS legs still succeed and the release is published without the Windows asset.
- [ ] Given a successful end-to-end run on a tagged release, when `release-assets-x86_64-Windows` artifact is downloaded, then it contains both the signed MSI and its SHA-256.
- [ ] Given a manual verification on a clean Windows 11 VM, when the MSI is downloaded and double-clicked, then the SmartScreen dialog says "Signed by Strivex" (not "Unknown Publisher") and install completes with no errors.

---

### EP-W7: CI, Distribution, Website

Add the Windows matrix leg to release.yml, the Windows check job to ci.yml, winget manifests, the winget auto-PR workflow, and the website download column. Without this epic, Windows builds are manual and users cannot discover the product via `winget install`.

**Definition of Done:** Every push to `main` triggers a green `windows-check` CI job. Every tagged release auto-produces a signed MSI, auto-opens a PR to `microsoft/winget-pkgs`, and the website download page shows a working Windows download link.

#### US-017: release.yml 5th matrix leg (windows-2022)
**Description:** As the release pipeline, I want a `windows-2022` matrix leg added to `release.yml` so that tagged releases auto-build a Windows x86_64 binary alongside Linux and macOS.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002, US-003

**Acceptance Criteria:**
- [ ] Given `.github/workflows/release.yml` `build.strategy.matrix.include`, when a 5th entry is added: `{ target: x86_64-pc-windows-msvc, arch: x86_64, runs-on: windows-2022, continue-on-error: true }`, then the matrix expands to 5 legs (Linux x64, Linux ARM64, macOS ARM64, macOS Intel best-effort, Windows x64 best-effort).
- [ ] Given the existing 11 Linux-only steps gated by `if: runner.os == 'Linux'` and the macOS-only steps gated by `if: runner.os == 'macOS'`, when a new Windows-only gate `if: runner.os == 'Windows'` wraps the MSI production + signing steps (from US-016), then each platform runs only its own steps with no cross-platform interference.
- [ ] Given the matrix leg's `continue-on-error: true` at the matrix entry level, when a Windows build fails, then the `build` job for Windows is marked failed-but-non-blocking; the `release` job still runs as long as at least Linux x64 succeeds.
- [ ] Given a tagged release that produces artifacts from all 5 matrix legs, when the `release` job aggregates `release-assets-*` artifacts, then all five contribute to the final GitHub Release body.
- [ ] Given `cargo build --release --target x86_64-pc-windows-msvc` on the Windows matrix leg, when run, then it completes in <20 min (cached) / <35 min (cold), and produces a valid `target/x86_64-pc-windows-msvc/release/paneflow.exe`.
- [ ] Given a dry-run on a test tag, when the full Windows matrix leg executes, then the resulting `release-assets-x86_64-Windows` artifact contains the signed MSI + SHA-256 sidecar and passes `signtool verify`.

#### US-018: ci.yml windows-check job
**Description:** As a PR author touching Windows-relevant code, I want a `windows-check` CI job to run fmt + clippy + test + check on `windows-2022` so that Windows regressions are caught in PR review before merge.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002, US-003

**Acceptance Criteria:**
- [ ] Given `.github/workflows/ci.yml`, when a new `windows-check` job is defined, then it runs on `windows-2022`, uses `dtolnay/rust-toolchain@stable` with `components: rustfmt, clippy` and `targets: x86_64-pc-windows-msvc`.
- [ ] Given the job's steps, when they execute, then they run (in order): `cargo fmt --all --check`, `cargo clippy --workspace --target x86_64-pc-windows-msvc -- -D warnings`, `cargo test --workspace --target x86_64-pc-windows-msvc`, and `cargo check --workspace --target x86_64-pc-windows-msvc`.
- [ ] Given the `changes` path filter, when a PR touches only docs or non-Rust files, then the `windows-check` job is skipped (matching the existing `macos-check` filter behavior).
- [ ] Given the `rust-cache` action, when the Windows cache key is keyed on `x86_64-pc-windows-msvc`, then cached subsequent runs complete in <5 min.
- [ ] Given an intentional POSIX-only import added to a file in a test PR, when CI runs, then `windows-check` fails with a clear error citing the problematic file:line (proves the gate catches regressions).
- [ ] Given Linux + macOS regression, when `ubuntu-latest` `check` and `macos-check` jobs run, then they are unchanged.

#### US-019: Winget manifest files + update-winget.yml bot
**Description:** As a Windows 11 user, I want to `winget install paneflow` to work so that discovery via Microsoft's official package manager is first-class — matching Homebrew for macOS.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] Given `packaging/winget/` (new directory), when 3 YAML manifest templates are authored, then they match the winget-pkgs schema: `ArthurDev44.PaneFlow.installer.yaml` (InstallerType: wix, Installers with URL + InstallerSha256 placeholders), `ArthurDev44.PaneFlow.locale.en-US.yaml` (PackageName: PaneFlow, Publisher: Strivex, LicenseUrl, Description), `ArthurDev44.PaneFlow.yaml` (DefaultLocale: en-US, ManifestType: version, PackageVersion placeholder).
- [ ] Given the templates, when committed to `packaging/winget/`, then they are authored as sed-substitutable (similar to `packaging/homebrew/paneflow.rb` pattern) with `__VERSION__`, `__URL__`, `__SHA256__` placeholders.
- [ ] Given `.github/workflows/update-winget.yml` (new file), when triggered by `release: [published]` + `workflow_dispatch`, then it: (a) resolves and validates the tag, (b) downloads the `paneflow-<ver>-x86_64-pc-windows-msvc.msi` asset, (c) computes its SHA-256, (d) renders the three manifests with substitutions, (e) uses `winget-create` or `gh pr create` to open a PR to `microsoft/winget-pkgs` with the rendered manifests under `manifests/a/ArthurDev44/PaneFlow/<ver>/`.
- [ ] Given the PR is opened, when winget-pkgs maintainers review it (3-10 days typical), then the signed MSI passes their automated validation and is merged.
- [ ] Given the merge, when a Windows 11 user runs `winget install ArthurDev44.PaneFlow`, then the correct signed MSI is fetched and installed.
- [ ] Given the workflow fails for any reason (missing GH token, network, winget-pkgs schema change), when diagnosed, then a clear error message is logged and the maintainer is notified via a GitHub Actions annotation.

#### US-020: Website download page Windows column
**Description:** As a visitor to the PaneFlow website download page, I want a Windows column showing the MSI download link and `winget install` command so that I can choose my install method.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] Given `paneflow-site/src/components/download/download-view.tsx:88` (the empty `items={[]}` placeholder for Windows), when replaced with a `windowsItems(entry.version)` function, then it returns 2 items: `{ label: "Windows x86_64 MSI", url: "<gh-release>/paneflow-<ver>-x86_64-pc-windows-msvc.msi", icon: WindowsIcon }` and `{ label: "winget install", copyText: "winget install ArthurDev44.PaneFlow", icon: TerminalIcon }`.
- [ ] Given the site is built via `bun run build`, when run, then no TypeScript errors and the Windows column renders with the 2 items.
- [ ] Given a visitor clicks the MSI link, when the download starts, then it downloads the correct signed MSI from the latest GitHub Release.
- [ ] Given a visitor clicks the `winget install` copy button, when triggered, then the command is copied to clipboard and a toast confirmation appears.
- [ ] Given mobile browser responsive layout, when the page renders on a <600px viewport, then all 3 OS columns stack vertically with no overflow.

---

### EP-W8: QA, Smoke Tests, Known-Risks Documentation

Manual smoke test on Windows 10 1809 and Windows 11 + document known upstream risks. Without this epic, the release may ship with silent regressions and users have no expectation-setting about known limitations.

**Definition of Done:** The v1 Windows release is smoke-tested on Windows 10 1809 AND Windows 11, the 5 known upstream risks are documented in `docs/WINDOWS.md` with GitHub issue links, and at least 1 happy-path + 1 edge-case scenario are verified per OS.

#### US-021: Windows 10 1809 + Windows 11 smoke test suite
**Description:** As the release manager, I want a documented smoke test checklist run on Windows 10 1809 and Windows 11 VMs before every Windows release so that critical regressions are caught before user download.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-016

**Acceptance Criteria:**
- [ ] Given `docs/WINDOWS-SMOKE-TEST.md` (new file), when authored, then it lists 10 scenarios: (1) fresh install via double-click MSI, (2) SmartScreen shows "Signed by Strivex", (3) launch from Start Menu, (4) `Ctrl+Shift+N` creates a new workspace, (5) `Ctrl+Shift+D` horizontal split works, (6) PowerShell 7 shell spawns correctly, (7) typing 100 chars + resizing window does not crash ConPTY, (8) `Ctrl+C` at shell prompt interrupts (documented limitation: may propagate unexpectedly per alacritty#3075 — still testable), (9) close window + reopen preserves config, (10) uninstall via `Add or Remove Programs` leaves `%APPDATA%\paneflow\` intact.
- [ ] Given a Windows 10 1809 VM (can use Microsoft's free evaluation VHD), when the checklist is run, then all 10 scenarios pass or are explicitly marked "known limitation" with an issue link.
- [ ] Given a Windows 11 VM, when the same checklist is run, then all 10 scenarios pass.
- [ ] Given any failure, when triaged, then it is either: (a) fixed in-PRD by reopening a prior story, (b) logged as a known-v1 issue in `docs/WINDOWS.md`, or (c) blocks release if severity is critical (user-visible crash, data loss).
- [ ] Given the checklist is run, when complete, then a screenshot gallery (one per scenario) is attached to the release PR for asynchronous reviewer verification.

#### US-022: docs/WINDOWS.md — known upstream risks + user guide
**Description:** As a Windows user evaluating PaneFlow, I want a clear doc explaining what works, what doesn't (v1 limitations), and where to report Windows-specific issues so that I know what to expect before installing.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-021

**Acceptance Criteria:**
- [ ] Given `docs/WINDOWS.md` (new file), when authored, then it includes 5 sections: (1) Supported Windows versions (10 1809+, 11), (2) Installation (MSI + winget), (3) Known limitations (services sidebar empty, cmd.exe has no OSC 7, CJK IME caution), (4) Known upstream risks with GitHub links, (5) Reporting issues.
- [ ] Given the known-risks section, when each risk is listed, then it includes: title, brief description, upstream GitHub issue link, severity (cosmetic / functional / blocker), and v1 workaround if any. Minimum 5 risks: IME panic CJK (zed#12563), ConPTY Ctrl-C signal propagation (alacritty#3075), RDP initialization broken (zed#26692), devcontainer freeze (zed#49072), older GPU drivers `NoSupportedDeviceFound` (zed#28683).
- [ ] Given the README contribution section was added in recent macOS work (commit 31a0f2b per memory), when `README.md` is checked, then a "Windows" subsection is added with a one-line description and a link to `docs/WINDOWS.md`.
- [ ] Given the `paneflow-site` has a docs area or FAQ, when the maintainer decides the in-repo `docs/WINDOWS.md` suffices for v1, then the site link simply points to `https://github.com/.../docs/WINDOWS.md` (no duplication).
- [ ] Given a user reports a Windows-specific issue in a GitHub issue, when the issue template is updated, then a "Windows version + build" field is added to the issue template (`.github/ISSUE_TEMPLATE/bug_report.yml`).

## Success Metrics

| Metric | Baseline (pre-PRD) | Target (Month 1 post-release) | Target (Month 6) |
|--------|-------------------|-------------------------------|-------------------|
| Windows binaries shipped per release | 0 | 1 (`x86_64-pc-windows-msvc.msi`) | 1 (same; ARM64 in separate PRD) |
| `winget install paneflow` works | N/A | Yes (within 2 weeks of first release, gated on winget-pkgs review SLA) | Yes, stable |
| Windows CI green rate | N/A | 70% (first month, flaky-by-design with `continue-on-error: true`) | 95%+ (flip to `continue-on-error: false`) |
| SmartScreen "Unknown Publisher" rate | N/A (no binary) | 0% ("Signed by Strivex" on first-run) | 0% + silent install (reputation-based) |
| Time-to-install (user action count) | N/A | ≤3 (winget install OR MSI double-click + UAC accept + Start Menu launch) | Same |
| CI total build time (all platforms) | ~20 min (Linux + macOS) | <35 min (all 5 matrix legs) | <30 min (optimized caches) |
| User-reported Windows crashes in first month | N/A | ≤3 (acceptable initial bug wave) | ≤1/month |

## Out of Scope (v1 — Follow-up PRDs)

- **ARM64 Windows** (`aarch64-pc-windows-msvc`) — separate PRD after Intel Mac stabilizes on Windows matrix
- **Microsoft Store / MSIX distribution** — requires sandbox compatibility work (defer indefinitely; MSI + winget is the canonical path)
- **Portable `.zip` distribution** — low-demand, adds release asset complexity
- **`workspace.rs` functional port scan via `GetExtendedTcpTable`** — stub returns empty in v1 (documented); real impl in follow-up PRD
- **`cwd_now()` functional impl via `NtQueryInformationProcess`** — stub returns `None` in v1 (fallback to OSC 7); real impl in follow-up PRD
- **Cross-compilation from Linux** (via `cargo-xwin` or similar) — nice-to-have; `windows-2022` GH runner is sufficient for v1
- **WSL integration** — PaneFlow running on Windows can launch WSL shells as a regular shell choice, but first-class WSL integration (path translation, WSLg window chrome) is deferred
- **CJK IME polish** — known GPUI upstream risk; manual QA confirms "does not crash" but full CJK input polish is upstream-dependent
- **PowerShell prompt hook polish** — v1 uses a basic `$PROMPT` override; richer integration (PSReadLine aware, oh-my-posh compatible) deferred
- **Self-update auto-download on Windows** — v1 shows an update prompt with a download link; click-to-install automation deferred

## Risks & Mitigations

| Risk | Severity | Probability | Mitigation |
|------|----------|-------------|------------|
| GPUI pin bump breaks Linux or macOS builds | High | Medium | US-001 spike validates bump on all 3 platforms before merge; US-002 explicitly tests Linux+macOS CI runs post-bump |
| Azure Trusted Signing business verification exceeds 6 weeks | Medium | Medium | US-014 starts in parallel at PRD kickoff; fallback to Sectigo OV cert ($150/yr) documented with reputation-build expectation |
| winget-pkgs PR review SLA exceeds 2 weeks | Low | Low | `.msi` direct download via GitHub Releases remains the primary channel; winget is secondary |
| ConPTY Ctrl-C propagation issue (alacritty#3075) surfaces user-visible bug | Medium | High | Documented as known limitation in `docs/WINDOWS.md`; monitor alacritty upstream for fix |
| IME CJK panic (zed#12563) on Windows users | Medium | Low | Issue marked closed upstream but marked as fragile; US-021 tests with CJK input, US-022 documents caution |
| `interprocess` 2.x API surface change during development | Low | Low | Pin to specific minor version in Cargo.toml; check release notes before version bumps |
| Windows CI runner outages / GitHub Actions rate limits | Low | Medium | `continue-on-error: true` on matrix leg; maintainer can manually rebuild missing MSI via `workflow_dispatch` |
| Post-install first-launch fails due to path or permission issues | High | Low | US-021 smoke tests exercise the full install → launch → uninstall cycle on both Windows 10 and 11 |
| User config paths conflict between %APPDATA% (Roaming) and %LOCALAPPDATA% | Low | Low | `dirs::config_dir()` resolves to `%APPDATA%\Roaming` — consistent with Zed and most Rust Windows tools |

## Glossary

- **ConPTY** — Windows Pseudo Console API introduced in Windows 10 1809 (`CreatePseudoConsole`, `ClosePseudoConsole`, `ResizePseudoConsole`). The Windows equivalent of POSIX openpty/forkpty.
- **winget** — Windows Package Manager, Microsoft's official CLI installer, pre-installed on Windows 11. Analogous to Homebrew on macOS or apt on Debian.
- **WiX** — Windows Installer XML Toolset. Open-source tool for authoring MSI installers from XML descriptors.
- **MSI** — Microsoft Installer, the standard Windows installer format. Supports Group Policy, silent install, major/minor upgrade semantics.
- **Azure Trusted Signing** — Microsoft's cloud-based code-signing service (formerly "Azure Code Signing Certificate"). $9.99/mo Basic tier. Provides Public Trust certificates for SmartScreen.
- **SmartScreen** — Windows Defender SmartScreen, the reputation-based protection layer that shows "Unknown Publisher" warnings for untrusted binaries.
- **HLSL** — High-Level Shading Language, DirectX's shader language (analog to Metal Shading Language on macOS).
- **Named Pipe** — Windows IPC primitive under `\\.\pipe\...` namespace. Windows equivalent of Unix domain sockets.
- **UAC** — User Account Control, the Windows elevation prompt shown when installers need admin rights.
- **PowerShell 7 (pwsh.exe)** — Modern, cross-platform PowerShell (distinct from legacy Windows PowerShell 5.1 pre-installed as `powershell.exe`).

[/PRD]
