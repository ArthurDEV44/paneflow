[PRD]
# PRD: Padding Extension & Cross-Platform Foundation

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-07 | Claude + Arthur | Initial draft from deep comparative analysis of Ghostty, cmux, Zed, and PaneFlow rendering pipelines |

## Problem Statement

PaneFlow's terminal rendering has a visible defect that breaks TUI application display, and its dependency architecture prevents cross-platform expansion:

1. **Full-width background highlight bars don't extend to terminal edges** — TUI apps like Codex, Claude Code, and neovim use `CSI K` (Erase in Line) with colored backgrounds to create full-width highlight bars. In PaneFlow, these bars stop at the cell grid boundary, leaving visible strips of the default background color on the left (1 cell-width gutter) and right (fractional pixel gap). This makes every TUI app look broken compared to Ghostty, cmux, or any native terminal.

2. **Zed monorepo coupling blocks portability** — GPUI and alacritty_terminal are local path dependencies pointing to `/home/arthur/dev/zed`. This requires the Zed checkout to exist on every build machine, prevents CI/CD, and makes cross-platform builds impossible without that exact directory structure.

3. **No Windows/macOS PTY abstraction** — `terminal.rs` uses raw POSIX `openpty()` directly. Windows requires ConPTY, macOS has minor differences. Without a cross-platform PTY layer, PaneFlow cannot compile on non-Linux platforms.

**Why now:** PaneFlow's v2 GPUI rewrite is feature-complete. The rendering parity PRD (EP-001–EP-003) fixes text rendering, cursor, and contrast. But the padding extension bug makes every TUI app visually broken, and the dependency architecture must be refactored before any cross-platform release can be attempted. These are the last architectural blockers before public release.

## Overview

This PRD addresses two coupled concerns: a rendering fix and cross-platform foundation work.

**Rendering fix:** Extend edge cells' background colors into the padding/gutter areas of the terminal widget, matching Ghostty's `EXTEND_LEFT/RIGHT/UP/DOWN` behavior. The fix modifies `terminal_element.rs`'s `paint()` method to extend `LayoutRect` bounds for edge cells to the widget boundary. A `neverExtendBg` heuristic prevents extending Powerline-style prompts that intentionally use color transitions at edges.

**Cross-platform foundation:** Migrate from Zed's local path dependencies to pinned git deps (GPUI) and upstream crates.io (alacritty_terminal). Add `portable-pty` for ConPTY support on Windows. Abstract PTY creation behind a trait for platform-agnostic terminal spawning.

Key decisions:
- **CPU quad extension, not GPU shaders** — GPUI doesn't support custom fragment shaders. Extend `LayoutRect` bounds in the `paint()` loop (~20 lines).
- **Upstream alacritty_terminal** — reduce Zed coupling. Spike first to validate API compatibility.
- **portable-pty** — battle-tested by WezTerm, provides ConPTY on Windows out of the box.
- **GPUI stays as the UI framework** — only viable Rust framework with rich UI primitives (Entity/Context, focus chain, actions) AND cross-platform GPU rendering (Vulkan, Metal, DirectX 11).

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| TUI visual fidelity | Full-width bg bars render edge-to-edge in PaneFlow (match Ghostty/cmux) | Zero visual difference from Ghostty for TUI apps |
| Build portability | Builds without local Zed checkout (git deps) | CI/CD pipeline on GitHub Actions |
| Platform support | Linux (Wayland + X11) + macOS compilation validated | Windows compilation validated (when GPUI Windows exits alpha) |

## Target Users

### Developer (TUI power user)
- **Role:** Software engineer using PaneFlow daily with TUI tools (Codex, Claude Code, neovim, lazygit)
- **Behaviors:** Runs multiple TUI apps simultaneously across split panes and workspaces
- **Pain points:** Highlight bars, selection backgrounds, and colored status lines in TUI apps look broken — visible gaps at edges, making PaneFlow feel buggy compared to Ghostty or native terminals
- **Current workaround:** Uses Ghostty for TUI-heavy work, PaneFlow only for basic shell sessions
- **Success looks like:** TUI apps in PaneFlow are visually indistinguishable from Ghostty

### Contributor / Cross-Platform Builder
- **Role:** Developer wanting to build PaneFlow on macOS or contribute from a non-Linux machine
- **Behaviors:** Clones the repo, runs `cargo build`, expects it to work
- **Pain points:** Build fails immediately — requires `/home/arthur/dev/zed` to exist. No way to build on macOS or Windows.
- **Current workaround:** None — must use Arthur's exact machine setup
- **Success looks like:** `cargo build` works on a fresh Linux or macOS checkout with only `git` and `cargo` installed

## Research Findings

Key findings from deep comparative analysis of four terminal codebases:

### Competitive Context
- **Ghostty:** Uses a full-screen `cell_bg` fragment shader (`shaders.metal:455-494`) that covers every pixel. Pixels in padding areas are clamped to the nearest edge cell via `EXTEND_LEFT/RIGHT/UP/DOWN` flags. The `neverExtendBg` heuristic (`row.zig:8`) prevents extending rows containing Powerline glyphs or semantic prompt markers. Cell bg colors for erased cells are stored directly in the cell's `ContentTag` (`page.zig:2011`) — no indirection.
- **cmux:** Uses **libghostty** as its terminal backend (`GhosttyTerminalView.swift:3693`). Inherits Ghostty's exact rendering pipeline including padding extension. macOS-only.
- **Zed terminal:** Same GPUI framework as PaneFlow. Has the **same padding extension bug** — background rects stop at cell grid boundaries. Paint order: fill bounds with default bg → paint non-default cell bg rects → paint text.
- **WezTerm:** Cross-platform multiplexer using `portable-pty` for PTY abstraction (POSIX + ConPTY). Custom `window` crate for windowing. OpenGL renderer with optional wgpu backend.
- **Market gap:** No Rust terminal multiplexer combines full TUI rendering fidelity with true cross-platform support.

### Best Practices Applied
- Edge cell bg extension is standard in modern terminals (Ghostty, iTerm2, Kitty, WezTerm)
- `portable-pty` is the Rust ecosystem standard for cross-platform PTY (used by WezTerm in production)
- Upstream `alacritty_terminal` on crates.io (v0.26) is API-compatible with Zed's fork for core operations

*Research based on direct source code analysis of Ghostty (Zig), cmux (Swift), Zed (Rust), PaneFlow (Rust), and WezTerm (Rust).*

## Assumptions & Constraints

### Assumptions (to validate)
- Upstream `alacritty_terminal` (crates.io) provides the same `display_iter()`, `Cell.bg`, `Cell.zerowidth()`, and `renderable_content()` APIs as Zed's fork (rev 9d9640d4) — **validated by spike US-004**
- `portable-pty` works with PaneFlow's async model (GPUI `cx.spawn()` + smol) — may need adapter
- GPUI can be consumed as a git dependency with `rev = "..."` pinning instead of local path — **must verify Cargo resolver handles GPUI's transitive deps**

### Hard Constraints
- Must use GPUI's `paint_quad()` for background rendering — no custom GPU shaders
- `alacritty_terminal` is the terminal emulation layer — not replacing with termwiz or custom
- `portable-pty` must support at minimum: Linux (openpty), macOS (openpty), Windows (ConPTY)
- No breaking changes to PaneFlow's existing terminal behavior — extension is purely additive
- GPUI's `[patch]` requirements (`async-task` and `calloop` forks) must be preserved in any dep migration

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --workspace -- -D warnings` - Zero clippy warnings
- `cargo fmt --check` - Formatting compliance
- `cargo test --workspace` - All existing tests pass (39 tests in paneflow-config)
- `cargo build` - Successful compilation

For rendering stories (EP-001), additional visual verification:
- Run Codex (`codex`) in PaneFlow and verify highlight bars extend edge-to-edge
- Run `printf '\e[48;2;80;80;80m\e[2K\e[0m'` and verify the colored line spans full terminal width
- Compare side-by-side with Ghostty for the same TUI app

## Epics & User Stories

### EP-001: Padding Extension

Fix the terminal rendering so that cell background colors extend into padding/gutter areas, making full-width TUI highlight bars render edge-to-edge like Ghostty and cmux.

**Definition of Done:** Running Codex or Claude Code in PaneFlow produces full-width highlight bars that extend from left edge to right edge of the terminal widget, with no visible gaps. Powerline prompts are not distorted.

#### US-001: Horizontal padding extension (left/right)
**Description:** As a developer using Codex in PaneFlow, I want full-width highlight bars to extend from the left edge to the right edge of the terminal so that TUI apps look correct instead of having visible background-color gaps at the edges.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] In `paint()`, for each `LayoutRect` where `rect.col == 0`: the painted quad's left edge starts at `bounds.origin.x` instead of `origin.x` (absorbing the left gutter)
- [ ] In `paint()`, for each `LayoutRect` where `rect.col + rect.num_cols >= desired_cols`: the painted quad's right edge extends to `bounds.origin.x + bounds.size.width`
- [ ] `desired_cols` is stored in `LayoutState` so `paint()` can access it for the right-edge check
- [ ] Running `printf '\e[48;2;80;80;80m\e[2K\e[0m'` produces a grey bar that spans the full terminal width with no visible gaps at left or right edges
- [ ] Running Codex in PaneFlow shows highlight bars identical in width to Codex in Ghostty
- [ ] Given a terminal whose `bounds.size.width - gutter` is exactly divisible by `cell_width` (no fractional gap), the extension produces no visual change (no-op for right edge)
- [ ] Given a terminal with 1 column (extreme resize), the single cell's background fills the full widget width without overflow

#### US-002: Vertical padding extension (top/bottom)
**Description:** As a developer, I want the first and last visible rows' background colors to extend into any vertical padding so that TUI apps with colored top/bottom bars have no gaps above or below.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001 (same paint loop, shared `LayoutState` fields)

**Acceptance Criteria:**
- [ ] `LayoutState` stores `desired_rows` alongside `desired_cols`
- [ ] In `paint()`, for each `LayoutRect` on the first visible line (`rect.line == 0`): the painted quad's top edge extends to `bounds.origin.y`
- [ ] In `paint()`, for each `LayoutRect` on the last visible line (`rect.line == desired_rows - 1`): the painted quad's bottom edge extends to `bounds.origin.y + bounds.size.height`
- [ ] Running a TUI app with a colored status bar at the bottom shows no gap between the status bar and the terminal widget's bottom edge
- [ ] Given a terminal with only 1 row, the background fills from top to bottom of the widget

#### US-003: neverExtendBg heuristic
**Description:** As a developer using Powerline/Starship prompts, I want the padding extension to NOT distort my prompt's intentional color transitions at the edges so that Powerline arrow glyphs and segment boundaries remain visually correct.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] A function `should_extend_row(cells: &[(Point, char, ...)], line: i32) -> (bool, bool)` returns `(extend_left, extend_right)` booleans
- [ ] Returns `(false, _)` for `extend_left` if column 0 of the row contains a Powerline glyph (U+E0B0–U+E0D4 range) or a box-drawing character (U+2500–U+257F)
- [ ] Returns `(_, false)` for `extend_right` if the last non-default-bg cell of the row contains a Powerline glyph or box-drawing character
- [ ] The heuristic data is computed in `build_layout()` and stored per-line in `LayoutState` (not recomputed in `paint()`)
- [ ] Given a Starship prompt with a Powerline arrow at column 0, the left edge is NOT extended (default bg shows in gutter)
- [ ] Given a normal highlighted line (no Powerline glyphs), both edges ARE extended
- [ ] Given a row with ONLY default-bg cells, no extension occurs (no LayoutRects exist to extend)

---

### EP-002: Dependency Decoupling

Migrate from local path dependencies on the Zed monorepo to reproducible, portable dependency references. This unblocks builds on any machine and prepares for CI/CD.

**Definition of Done:** `cargo build` succeeds after a fresh `git clone` of PaneFlow without any local Zed checkout. All existing functionality preserved.

#### US-004: Spike — Upstream alacritty_terminal API compatibility
**Description:** As a developer, I want to validate that upstream `alacritty_terminal` (crates.io) is API-compatible with Zed's fork so that we can migrate without breaking terminal emulation.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Document every `alacritty_terminal` API used by PaneFlow (`terminal.rs` and `terminal_element.rs`): `Term`, `Grid`, `display_iter()`, `renderable_content()`, `Cell.c`, `Cell.fg`, `Cell.bg`, `Cell.flags`, `Cell.zerowidth()`, `CellFlags`, `EventLoop::spawn()`, `Msg`, `Notifier`, `FairMutex`, `WindowSize`, `SizeInfo`, `Event`, `EventListener`
- [ ] For each API, check presence and signature in upstream `alacritty_terminal` v0.26 (latest on crates.io)
- [ ] Document all API differences in a `tasks/spike-alacritty-upstream.md` file with migration path for each
- [ ] If `Cell.zerowidth()` is missing from upstream (Zed-specific addition), document the workaround (direct `CellExtra` access or feature-gated method)
- [ ] If `FairMutex` is Zed-specific, document the replacement (`parking_lot::FairMutex` or `std::sync::Mutex`)
- [ ] Verdict: GO (migrate) or NO-GO (stay on fork with documented reasons)

#### US-005: Migrate to upstream alacritty_terminal
**Description:** As a contributor, I want PaneFlow to use `alacritty_terminal` from crates.io so that building PaneFlow doesn't require a local Zed monorepo checkout.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-004 (spike must return GO verdict)

**Acceptance Criteria:**
- [ ] `src-app/Cargo.toml` replaces `alacritty_terminal = { path = "..." }` with `alacritty_terminal = "0.26"` (or latest compatible version)
- [ ] All compilation errors from API differences are resolved using the migration paths documented in US-004
- [ ] If `FairMutex` is not available upstream, replaced with `parking_lot::FairMutex` or equivalent
- [ ] If `Cell.zerowidth()` is not available upstream, implemented via direct cell extra access
- [ ] `cargo test --workspace` passes — all 39 existing tests pass
- [ ] Running PaneFlow with the upstream crate produces identical terminal behavior (verified by visual comparison with Ghostty)
- [ ] Given an `alacritty_terminal` API that changed between Zed fork and upstream, the migration is documented in a code comment explaining the adaptation

#### US-006: GPUI git dependency with pinned rev
**Description:** As a contributor, I want GPUI consumed as a git dependency with a pinned commit rev so that builds are reproducible without requiring a local Zed checkout at a specific path.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None (can run in parallel with US-004/US-005)

**Acceptance Criteria:**
- [ ] `src-app/Cargo.toml` replaces `gpui = { path = "/home/arthur/dev/zed/crates/gpui" }` with `gpui = { git = "https://github.com/zed-industries/zed", rev = "{current_rev}" }`
- [ ] Same treatment for `gpui_platform`, `collections`, and any other Zed path deps
- [ ] The `[patch.crates-io]` section is updated to reference the same git rev for `async-task` and `calloop` forks
- [ ] `cargo build` succeeds from a fresh checkout (no `/home/arthur/dev/zed` needed)
- [ ] `cargo build` time does not increase by more than 30% compared to path deps (git checkout caching)
- [ ] Given a future GPUI update, the process to bump the rev is documented in a comment in Cargo.toml
- [ ] Given a network-offline build (after initial fetch), `cargo build` succeeds from cargo's git cache

---

### EP-003: Cross-Platform PTY Foundation

Add a cross-platform PTY abstraction layer that works on Linux, macOS, and Windows (ConPTY), preparing PaneFlow for multi-platform releases.

**Definition of Done:** PaneFlow's terminal spawning uses `portable-pty` instead of raw POSIX `openpty()`. The crate compiles on macOS. Windows compilation is gated behind a feature flag for future activation.

#### US-007: Integrate portable-pty
**Description:** As a developer building PaneFlow on macOS, I want terminal spawning to use a cross-platform PTY library so that `cargo build` succeeds on macOS without POSIX-specific code errors.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None (can run in parallel with EP-001 and EP-002)

**Acceptance Criteria:**
- [ ] `portable-pty` added to `src-app/Cargo.toml` dependencies
- [ ] `terminal.rs`: `TerminalState::new()` uses `portable_pty::native_pty_system()` to create a PTY pair instead of raw `openpty()`
- [ ] The shell command is built from `$SHELL` env var (existing behavior preserved)
- [ ] PTY size is set from `desired_cols` / `desired_rows` using portable-pty's `PtySize` struct
- [ ] Reading from the PTY reader and writing to the PTY writer integrates with the existing `alacritty_terminal::EventLoop` (or equivalent async loop)
- [ ] Given `$SHELL` is unset, falls back to `/bin/sh` on Unix or `cmd.exe` on Windows
- [ ] Given a PTY creation failure (e.g., permission denied), the error is logged and the terminal shows an error message instead of crashing

#### US-008: PTY trait abstraction
**Description:** As a developer, I want PTY creation abstracted behind a trait so that platform-specific PTY implementations can be swapped without changing terminal logic.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] A `PtyBackend` trait (or equivalent) is defined with methods: `spawn(command, size) -> Result<(Reader, Writer, ChildProcess)>`
- [ ] `PortablePtyBackend` implements the trait using `portable-pty`
- [ ] `TerminalState::new()` accepts a `Box<dyn PtyBackend>` (or generic parameter) instead of directly calling PTY creation
- [ ] Given a mock `PtyBackend` implementation, a `TerminalState` can be created in tests without spawning a real shell
- [ ] The trait is in its own module (`pty.rs` or `pty/mod.rs`) with no GPUI imports (pure Rust, testable independently)

#### US-009: macOS build validation
**Description:** As a developer, I want to validate that PaneFlow compiles on macOS after the dependency migrations so that macOS is confirmed as a viable target platform.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** Blocked by US-005, US-006, US-007

**Acceptance Criteria:**
- [ ] `cargo build` succeeds on macOS (Apple Silicon or Intel) with the migrated dependencies
- [ ] If GPUI requires platform-specific features, they are correctly feature-gated in `Cargo.toml`
- [ ] Any Linux-specific code in PaneFlow is gated behind `#[cfg(target_os = "linux")]`
- [ ] A terminal window opens and a shell prompt appears (basic functionality)
- [ ] Build failures and their resolutions are documented in `tasks/spike-macos-build.md`
- [ ] Given a macOS system without Vulkan support, GPUI falls back to Metal rendering (GPUI's default on macOS)

## Functional Requirements

- FR-01: The terminal renderer must extend edge cells' background colors to the widget boundary (left, right, top, bottom)
- FR-02: The padding extension must NOT apply to rows containing Powerline glyphs at the edge positions
- FR-03: The terminal must spawn shells using a cross-platform PTY library (not raw POSIX `openpty()`)
- FR-04: The project must build from a fresh git clone without any local Zed monorepo checkout
- FR-05: The terminal must preserve all existing rendering behavior for cells that are not at edges

## Non-Functional Requirements

- **Performance:** Padding extension must add < 0.1ms to the paint cycle (no per-pixel computation — only rect bound adjustments per LayoutRect)
- **Performance:** `portable-pty` overhead vs raw `openpty()` must be < 5% on keystroke-to-echo latency
- **Compatibility:** All 39 existing paneflow-config tests must pass after migrations
- **Build time:** Fresh `cargo build` from git clone must complete in < 10 minutes on a 4-core machine
- **Platform coverage:** Code must compile on `x86_64-unknown-linux-gnu` and `aarch64-apple-darwin` targets

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Terminal width exactly divisible by cell_width | Window resize to exact multiple | No right-edge extension needed (no-op), no visual artifact | — |
| 2 | Terminal with 1 column | Extreme resize or split | Single cell's bg fills entire widget width | — |
| 3 | Powerline arrow at column 0 | Starship/Powerline prompt | Left padding NOT extended (intentional color transition) | — |
| 4 | All cells on a row have default bg | Empty shell line | No LayoutRects exist, no extension, default bg fills row | — |
| 5 | alacritty_terminal upstream API incompatibility | US-004 spike returns NO-GO | Stay on Zed fork, document gap, revisit on next upstream release | — |
| 6 | portable-pty fails to create PTY | Permission denied, resource exhaustion | Terminal shows error text: "Failed to create terminal: {error}" instead of crashing | "Failed to create terminal: {error}" |
| 7 | Resize race: bounds change during paint | Rapid window resize | `desired_cols` snapshot from `build_layout` is stable — paint uses snapshotted value | — |
| 8 | GPUI git dep fetch fails (offline) | No network after initial build | Cargo uses cached git checkout — build succeeds | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Upstream alacritty_terminal API differs from Zed fork | Medium | High | US-004 spike validates compatibility before migration. NO-GO path stays on fork. |
| 2 | GPUI git dep breaks due to transitive dependency resolution | Medium | Medium | Pin exact rev. Document `[patch]` requirements. Test on fresh checkout before merging. |
| 3 | portable-pty ConPTY issues on Windows | Low | Low | Windows build is P2/future. WezTerm validates portable-pty in production. |
| 4 | Padding extension causes visual regression for non-TUI usage | Low | Medium | neverExtendBg heuristic (US-003) + visual testing with shell prompts |
| 5 | GPUI Windows never exits alpha | Medium | High | Architecture allows future migration to wgpu+winit. PTY trait (US-008) isolates platform concerns. |

## Non-Goals

Explicit boundaries — what this version does NOT include:

- **Custom GPU shaders for cell rendering** — Ghostty's approach is superior but incompatible with GPUI's quad-based API. We use CPU rect extension instead.
- **Windows build support** — GPUI's Windows backend is alpha. macOS validation only (US-009). Windows deferred until GPUI stabilizes.
- **WezTerm-style SSH multiplexing** — portable-pty is for local PTY only. Remote multiplexing is a separate feature.
- **Replacing GPUI with another framework** — Evaluated wgpu+winit, GTK4, and Ghostty embedding. GPUI remains the best option for a rich UI multiplexer.
- **Full test suite for rendering** — Terminal rendering is visually verified. Automated pixel-comparison tests are out of scope.
- **Config-driven padding extension behavior** — Unlike Ghostty's `window-padding-color` config option, PaneFlow always extends. Configurable in a future PRD.

## Files NOT to Modify

- `crates/paneflow-config/` — Config crate is stable and unrelated to rendering or deps. Its 39 tests are the only automated quality gate.
- `src-app/src/ipc.rs` — IPC system is orthogonal to rendering and deps.
- `src-app/src/split.rs` — Split system is unaffected by rendering changes.
- `src-app/src/title_bar.rs` — Title bar is unaffected.
- `src-app/src/workspace.rs` — Workspace management is unaffected.

## Technical Considerations

- **Architecture:** Padding extension modifies only the `paint()` loop in `terminal_element.rs`. No structural changes to the rendering pipeline. Engineering to confirm `LayoutState` can carry `desired_cols`/`desired_rows` without lifetime issues.
- **Data Model:** No data model changes. `alacritty_terminal`'s `Cell.bg` already stores the correct background color for erased cells (verified in `clear_line()` at `term/mod.rs:1635`).
- **Dependencies:** Adding `portable-pty` (~50KB, no transitive heavy deps). Removing local path deps for GPUI (replaced with git+rev). Potentially adding `parking_lot` if `FairMutex` is not in upstream alacritty.
- **Migration:** Dependency migration (US-005, US-006) is a Cargo.toml change + API adaptation. No data migration. Rollback: revert Cargo.toml to path deps.
- **Rendering pipeline unchanged:** Paint order stays: 1) full bg fill → 2) cell bg rects (now extended) → 3) text → 4) cursor. No new passes.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| TUI highlight bar visual parity | Visible gaps at left/right edges | Zero visible gaps (edge-to-edge) | Month-1 | Side-by-side screenshot comparison with Ghostty |
| Build portability | Requires local Zed checkout at exact path | Builds from fresh git clone | Month-1 | `git clone && cargo build` on clean machine |
| Platform compilation | Linux only | Linux + macOS compile | Month-1 | `cargo build --target aarch64-apple-darwin` |
| Dependency count (Zed path deps) | 3 path deps (gpui, gpui_platform, collections) | 0 path deps | Month-1 | `grep 'path = "/home' Cargo.toml` returns 0 |

## Open Questions

- Should `portable-pty` be integrated alongside `alacritty_terminal::EventLoop`, or should we replace EventLoop with a custom read loop? Engineering to evaluate compatibility during US-007.
- What exact GPUI git rev should we pin to? Latest `main` at implementation time, or the rev that matches the current `/home/arthur/dev/zed` checkout? Recommend: current checkout rev for safety, bump later.
- Should the `neverExtendBg` heuristic also check for semantic shell integration markers (OSC 133) like Ghostty does? Defer to future iteration unless shell integration is already wired.
[/PRD]
