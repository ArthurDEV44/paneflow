[PRD]
# PRD: Stabilization & Polish — cmux Feature Parity for Linux

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-14 | Claude + Arthur | Initial draft from cmux codebase analysis and PaneFlow fragility audit |

## Problem Statement

PaneFlow's v2 GPUI rewrite is architecturally complete (19 user stories delivered across two PRDs), but six fragile points prevent it from being a credible high-quality alternative to cmux on Linux:

1. **Config fields are decorative** — `default_shell` in `paneflow.json` is ignored; `TerminalState::new()` reads `$SHELL` directly. The `shortcuts` schema exists but keybindings are hardcoded at `main.rs:710–734`. Users cannot customize their experience.

2. **Split resize uses a hardcoded pixel estimate** — `split.rs:141` assumes an 800px container width for ratio calculations. On monitors wider or narrower than ~800px, drag-to-resize is inaccurate and feels broken.

3. **No terminal power features** — cmux offers in-buffer search, copy mode, pane zoom, split equalize, and pane swap. PaneFlow has none of these. Power users who rely on these features cannot switch from tmux/Zellij.

4. **Session persistence is incomplete** — PaneFlow saves workspace layout and CWD, but not scrollback history. cmux restores up to 4000 lines of scrollback with ANSI-safe truncation. Losing terminal output on restart is a dealbreaker for long-running sessions.

5. **Config hot-reload uses polling instead of the existing watcher** — `paneflow-config` has a fully implemented `ConfigWatcher` using the `notify` crate with 300ms debounce, but the app ignores it and uses 500ms mtime polling instead. This wastes CPU and has higher latency.

6. **Zero tests in the app crate** — All 39 tests are in `paneflow-config`. No regression protection exists for UI behavior, keybinding dispatch, or session persistence. Any change can silently break core functionality.

**Why now:** The distribution story is validated — PaneFlow's binary requires only 5 system libs (`libxcb`, `libxkbcommon`, `libxkbcommon-x11`, `libc`, `libm`) and runs on any Linux desktop with Vulkan. The technical foundation is solid. What remains is the gap between "it runs" and "it's good enough to replace cmux/tmux" — and that gap is entirely in the 6 points above.

## Overview

This PRD addresses PaneFlow's path from functional prototype to production-quality terminal multiplexer, organized into four epics:

**EP-001 (Stabilization)** fixes the fragile internals: wire the existing config fields (`default_shell`, `shortcuts`), replace the hardcoded split resize estimate with dynamic layout measurement, switch to the already-implemented `ConfigWatcher`, and add graceful GPU error handling at startup.

**EP-002 (Terminal Power Features)** adds the five features that cmux users expect: in-buffer scrollback search with a highlight overlay, keyboard-driven copy mode for text selection, pane zoom to temporarily maximize a single pane, split equalize to distribute panes evenly, and pane swap to rearrange pane positions.

**EP-003 (Session Robustness)** upgrades session persistence to include scrollback history with ANSI-safe truncation, atomic writes to prevent corruption on crash, and undo-close-pane for accidental closures.

**EP-004 (Quality Foundation)** establishes an integration test harness for the app crate, covering keybinding dispatch and session persistence round-trips.

Key decisions:
- **Keybindings use config-driven dispatch** with hardcoded defaults as fallback — the `shortcuts` field in `paneflow.json` already has the schema, it just needs wiring.
- **Search overlay renders inside the terminal view** (like cmux's `SurfaceSearchOverlay`), not as a separate panel.
- **Copy mode reuses alacritty_terminal's selection API** (`Term::selection`, `Term::selection_to_string`) rather than building custom selection logic.
- **Scrollback persistence caps at 4000 lines** per terminal (matching cmux) with ANSI-safe truncation that avoids splitting escape sequences.
- **GPUI rev stays pinned** at `0b984b5` throughout this PRD — no upstream bumps during stabilization.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Config coverage | `default_shell` and `shortcuts` both functional | All config fields wired, zero decorative fields |
| Terminal features | Search + copy mode + pane zoom shipped | Feature parity with cmux's terminal capabilities |
| Session fidelity | Scrollback persisted, crash-safe writes | Zero data loss on crash or restart |
| Test coverage | Integration test harness + 15 tests in app crate | 50+ tests covering all keybindings and session paths |
| Split accuracy | Dynamic measurement, accurate on all resolutions | Pixel-perfect resize on HiDPI and mixed-DPI setups |

## Target Users

### Linux Power User (TUI Developer)
- **Role:** Software engineer using terminal-heavy workflows daily (neovim, lazygit, Claude Code, htop)
- **Behaviors:** Runs 4-8 terminal panes across 2-3 workspaces, switches frequently, relies on keyboard shortcuts
- **Pain points:** PaneFlow lacks search-in-buffer and copy mode — forced to use tmux/Zellij for these features. Cannot remap keybindings without editing source code. Split resize feels broken on ultrawide monitors.
- **Current workaround:** Uses tmux inside PaneFlow for search/copy, or uses a different terminal entirely
- **Success looks like:** PaneFlow replaces tmux/Zellij entirely — all terminal power features accessible via customizable shortcuts

### Linux Desktop User (Casual)
- **Role:** Developer or sysadmin who wants a modern terminal with split panes, but doesn't need tmux-level power
- **Behaviors:** Opens PaneFlow, uses 1-2 panes, expects the configured shell to launch
- **Pain points:** Config says `"default_shell": "/bin/fish"` but zsh launches instead. Confusing.
- **Current workaround:** Manually sets `$SHELL` before launching, or edits `.bashrc`
- **Success looks like:** Config just works — default_shell, theme, and shortcuts all respected

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **cmux:** ~60 configurable keyboard actions, in-buffer search via `SurfaceSearchOverlay`, copy mode, pane zoom, split equalize, 4000-line scrollback restore with ANSI-safe truncation, 8-second autosave, project-level config with directory walk. macOS-only.
- **Zellij:** WASM plugin system, floating panes, session management. TUI-only, no GPU rendering, no native GUI chrome.
- **WezTerm:** Extensive Lua config, GPU-rendered, cross-platform. Maintenance has slowed.
- **Market gap:** No GPU-rendered Linux terminal multiplexer with native GUI chrome AND cmux-level power features.

### Best Practices Applied
- Configurable keybindings are table-stakes — every serious terminal allows remapping
- Scrollback search is universally expected (Ctrl+Shift+F or Cmd+F)
- Atomic config/session writes prevent corruption (write to temp file, then rename)
- cmux's ANSI-safe truncation approach (skip partial escape sequences at boundary) is the correct pattern for scrollback persistence

*Research sources: cmux codebase at /home/arthur/dev/cmux (direct analysis of 68 Swift source files), awesome-gpui community projects, Zellij/WezTerm public documentation.*

## Assumptions & Constraints

### Assumptions (to validate)
- alacritty_terminal's `Term::selection` and `Term::selection_to_string()` API is sufficient for copy mode without building custom selection logic
- GPUI's text input overlay can be positioned inside the terminal view for the search overlay without z-order issues
- 4000 lines of scrollback per terminal is sufficient (matches cmux's default)
- Serializing scrollback as plain text (stripping ANSI) is acceptable for v1 — ANSI-safe replay (cmux's approach) deferred

### Hard Constraints
- GPUI rev pinned at `0b984b5` — no upstream bumps during this PRD
- Linux-only (Wayland + X11) — no macOS/Windows work
- No new external crate dependencies without justification
- Must pass `cargo clippy --workspace -- -D warnings` and `cargo fmt --check`

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --workspace -- -D warnings` — zero warnings
- `cargo fmt --check` — consistent formatting
- `cargo test --workspace` — all tests pass

For UI stories, additional gates:
- Launch with `cargo run` and manually verify the feature works on both Wayland and X11 (if available)
- Test the golden path and at least one edge case

## Epics & User Stories

### EP-001: Stabilization — Fix Fragile Internals

Fix the existing features that are broken or decorative. These are bugs, not new features — they should have worked from the start.

**Definition of Done:** All config fields in `paneflow.json` are functional, split resize is accurate on any resolution, config changes are detected via `notify` watcher, and GPU absence produces a clear error message.

#### US-001: Wire `default_shell` Config to Terminal Spawning
**Description:** As a user, I want my `default_shell` setting in `paneflow.json` to control which shell launches in new terminals, so that I don't have to set `$SHELL` manually.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `TerminalState::new()` reads `default_shell` from `PaneFlowConfig` and uses it as the shell command
- [ ] If `default_shell` is empty or not set, falls back to `$SHELL` environment variable
- [ ] If both are absent, falls back to `/bin/sh`
- [ ] Given `"default_shell": "/bin/fish"` in config, when a new terminal is opened, then fish shell launches
- [ ] Given `"default_shell": "/nonexistent/shell"` in config, when a new terminal is opened, then falls back to `$SHELL` and logs a warning

#### US-002: Fix Split Resize Dynamic Measurement
**Description:** As a user, I want split pane drag-to-resize to work accurately on any monitor resolution, so that I can resize panes precisely without overshoot or undershoot.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `split.rs` reads the actual parent container pixel width from GPUI's layout pass instead of hardcoded 800px
- [ ] Drag-to-resize is accurate on a 1920px wide window
- [ ] Drag-to-resize is accurate on a 3840px ultrawide window
- [ ] Drag-to-resize is accurate on a 1366px laptop display
- [ ] Given a window resized to 600px width, when dragging a vertical split divider, then the ratio updates proportionally to the actual width
- [ ] The hardcoded 800px constant at `split.rs:141` is removed

#### US-003: Wire Shortcuts Config to Action Dispatch
**Description:** As a user, I want to customize keybindings via the `shortcuts` field in `paneflow.json`, so that I can adapt PaneFlow to my muscle memory without editing source code.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The `shortcuts` map in `paneflow.json` maps action names (e.g., `"split_horizontal"`) to key combination strings (e.g., `"ctrl+shift+d"`)
- [ ] User-defined shortcuts override the hardcoded defaults
- [ ] Actions not overridden in config keep their default keybindings
- [ ] All 24 existing actions are addressable by name in the shortcuts config
- [ ] Given `"shortcuts": {"split_horizontal": "ctrl+alt+h"}` in config, when pressing Ctrl+Alt+H, then a horizontal split is created
- [ ] Given an invalid key combination string (e.g., `"shortcuts": {"split_horizontal": "asdfghjkl"}`), when config is loaded, then the invalid entry is ignored with a warning log, and the default keybinding is used
- [ ] Shortcut changes are picked up on config hot-reload without restart

#### US-004: Replace Mtime Polling with ConfigWatcher
**Description:** As a developer, I want the app to use the existing `ConfigWatcher` (notify crate, 300ms debounce) for config hot-reload, so that changes are detected faster with less CPU overhead.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] The 500ms mtime polling loop in `main.rs` is removed
- [ ] `ConfigWatcher` from `paneflow-config` is initialized at app startup and drives config reload
- [ ] Theme changes in `paneflow.json` are reflected within 500ms (300ms debounce + processing)
- [ ] Given config file is saved, when the watcher fires, then the new config is applied without restart
- [ ] Given config file is deleted, when the watcher fires, then the app continues running with the last known config (no crash)

#### US-005: Graceful Vulkan/GPU Error at Startup
**Description:** As a user running PaneFlow on a machine without Vulkan support, I want a clear error message instead of a crash, so that I understand why PaneFlow won't start.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] If wgpu fails to find a Vulkan adapter at startup, PaneFlow prints a human-readable error to stderr: "PaneFlow requires a GPU with Vulkan support. Install mesa-vulkan-drivers (AMD/Intel) or your GPU's proprietary driver."
- [ ] The error message includes a link or command for common distros (apt/dnf)
- [ ] The process exits with code 1, not a panic backtrace
- [ ] Given a working Vulkan setup, when PaneFlow starts, then no error is shown (no regression)

---

### EP-002: Terminal Power Features

Add the five features that cmux power users expect. These transform PaneFlow from a basic split terminal into a serious productivity tool.

**Definition of Done:** Users can search scrollback, select text via keyboard, zoom a pane to fullscreen, equalize split ratios, and swap pane positions — all via keybindings.

#### US-006: In-Buffer Scrollback Search
**Description:** As a user, I want to search for text in my terminal's scrollback buffer with a search overlay, so that I can find previous command output without scrolling manually.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Ctrl+Shift+F opens a search overlay bar at the top of the focused terminal pane
- [ ] Typing in the search bar highlights all matches in the scrollback buffer with a distinct background color
- [ ] The terminal scrolls to the first match
- [ ] Enter/Shift+Enter navigates to next/previous match
- [ ] Escape closes the search overlay and returns focus to the terminal
- [ ] The match count is displayed (e.g., "3/17")
- [ ] Given no matches found, when searching, then the overlay shows "0 results" and no highlighting occurs
- [ ] Given the terminal has 10,000 lines of scrollback, when searching, then results appear within 100ms
- [ ] Search is case-insensitive by default

#### US-007: Copy Mode (Keyboard-Driven Selection)
**Description:** As a user, I want to enter a copy mode where I can navigate and select text with the keyboard, so that I can copy terminal output without reaching for the mouse.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new keybinding (default: Ctrl+Shift+X) enters copy mode
- [ ] In copy mode, arrow keys move a visible cursor through the terminal buffer (including scrollback)
- [ ] Shift+arrow keys extend a selection from the cursor position
- [ ] Enter copies the selection to the system clipboard and exits copy mode
- [ ] Escape exits copy mode without copying
- [ ] The selection is visually highlighted (using the theme's selection color)
- [ ] Given copy mode is active, when pressing 'q', then copy mode exits (vi-style escape)
- [ ] Given copy mode is active, when terminal output arrives, then the view does NOT auto-scroll (selection stays stable)
- [ ] Copy mode uses `alacritty_terminal::Term` selection API — no custom selection logic

#### US-008: Pane Zoom Toggle
**Description:** As a user, I want to temporarily zoom one pane to fill the entire workspace area, so that I can focus on a single terminal without closing other panes.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new keybinding (default: Ctrl+Shift+Z) toggles zoom on the focused pane
- [ ] When zoomed, the pane fills 100% of the workspace content area (sidebar remains visible)
- [ ] Other panes are hidden but remain alive (processes continue running)
- [ ] Pressing the zoom keybinding again restores the original split layout
- [ ] A visual indicator shows that zoom is active (e.g., "[Z]" badge on the workspace tab or a subtle border)
- [ ] Given a zoomed pane is closed, when the pane is removed, then zoom is exited and the remaining layout is shown
- [ ] Given zoom is active, when switching workspaces, then the zoom state is per-workspace (other workspaces are unaffected)

#### US-009: Split Equalize
**Description:** As a user, I want to distribute all panes evenly across the workspace with one action, so that I can reset a messy layout quickly.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new keybinding (default: Ctrl+Shift+E, reassigned — current Ctrl+Shift+E for vertical split moves to Ctrl+Shift+|) triggers equalize
- [ ] All split ratios in the workspace's binary tree are set to 0.5
- [ ] The layout updates immediately with a smooth visual transition
- [ ] Given a workspace with 4 panes in a 2x2 grid, when equalizing, then all 4 panes are the same size
- [ ] Given a workspace with a single pane (no splits), when equalizing, then nothing happens (no error)

#### US-010: Pane Swap
**Description:** As a user, I want to swap the position of two panes, so that I can rearrange my layout without closing and re-splitting.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new keybinding (default: Ctrl+Shift+S) initiates swap mode
- [ ] In swap mode, the next focus navigation key (Alt+Arrow) selects the target pane
- [ ] The focused pane and target pane swap positions in the split tree
- [ ] Both terminals retain their state (scrollback, running process, CWD)
- [ ] Given swap mode is active, when pressing Escape, then swap mode is cancelled
- [ ] Given a workspace with a single pane, when initiating swap, then nothing happens (no crash)

---

### EP-003: Session Robustness

Upgrade session persistence from "saves layout" to "saves everything" — matching cmux's session fidelity.

**Definition of Done:** Terminal scrollback is preserved across restarts, session files cannot be corrupted by crashes, and accidentally closed panes can be recovered.

#### US-011: Scrollback Persistence in Session Save
**Description:** As a user, I want my terminal scrollback history to be saved when PaneFlow exits and restored when it restarts, so that I don't lose previous command output between sessions.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] On session save, each terminal's scrollback is extracted as plain text (stripping ANSI escape sequences) and stored in the session JSON
- [ ] Maximum 4000 lines of scrollback per terminal are saved (configurable in the future)
- [ ] Maximum 400,000 characters per terminal's scrollback (prevents memory explosion from binary output)
- [ ] On session restore, saved scrollback is fed into the new terminal as initial content before the shell prompt
- [ ] Given a terminal with 10,000 lines of scrollback, when saving, then only the most recent 4000 lines are kept
- [ ] Given a terminal with binary garbage output, when saving, then the character limit prevents the session file from growing unbounded

#### US-012: ANSI-Safe Scrollback Truncation
**Description:** As a developer, I want scrollback truncation to never split a partial ANSI escape sequence, so that restored scrollback doesn't display garbled characters.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] When truncating scrollback at the 4000-line boundary, if the truncation point falls within an ANSI escape sequence (CSI, OSC, DCS), the sequence is skipped entirely
- [ ] A `\x1b[0m` (reset) is prepended to the truncated output to clear any dangling style state
- [ ] Given scrollback ending with a partial `\x1b[38;2;` (incomplete RGB color), when truncating, then the incomplete sequence is removed
- [ ] Given scrollback with no escape sequences (plain text), when truncating, then truncation works identically to naive line splitting

#### US-013: Crash-Safe Session Autosave
**Description:** As a user, I want session saves to use atomic writes, so that a crash during save doesn't corrupt my session file.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Session save writes to a temporary file (`session.json.tmp`) first, then atomically renames to `session.json`
- [ ] If the rename fails, the temporary file is cleaned up and a warning is logged
- [ ] Given PaneFlow is killed (SIGKILL) during a save, when restarting, then the previous valid `session.json` is still intact
- [ ] Given the filesystem is full, when saving, then the write fails gracefully with a log warning (no crash, no corruption)

#### US-014: Undo Close Pane
**Description:** As a user, I want to undo accidentally closing a pane, so that I can recover a terminal session I didn't mean to close.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new keybinding (default: Ctrl+Shift+T) reopens the most recently closed pane
- [ ] The reopened pane restores: working directory, scrollback text, and position in the split tree
- [ ] The running process is NOT restored (a new shell is spawned in the same CWD)
- [ ] A stack of up to 5 recently closed panes is maintained (LIFO)
- [ ] Given no panes have been closed, when pressing Ctrl+Shift+T, then nothing happens (no error)
- [ ] Given 6 panes are closed sequentially, when pressing Ctrl+Shift+T, then only the 5 most recent are recoverable

---

### EP-004: Quality & Testing Foundation

Establish regression protection for the app crate. Without tests, every story in EP-001–EP-003 risks breaking existing functionality.

**Definition of Done:** An integration test harness exists for the app crate, with tests covering keybinding dispatch and session persistence.

#### US-015: Integration Test Harness for App Crate
**Description:** As a developer, I want a test harness in `src-app/` that can instantiate GPUI entities without a real GPU, so that I can write automated tests for app logic.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A `tests/` directory exists in `src-app/` with at least one passing test
- [ ] Tests use GPUI's `#[gpui::test]` macro (or equivalent headless mode) to avoid requiring a GPU
- [ ] A test can create a `PaneFlowApp` entity, access its state, and verify properties
- [ ] Given `cargo test -p paneflow-app`, when running, then tests execute without a display server
- [ ] Given a test that accesses workspace state, when the test runs, then it completes in under 1 second

#### US-016: Keybinding Dispatch Integration Tests
**Description:** As a developer, I want tests that verify keybinding dispatch works correctly, so that shortcut changes don't silently break.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015, US-003

**Acceptance Criteria:**
- [ ] At least 5 tests cover keybinding dispatch: split horizontal, split vertical, close pane, next workspace, focus navigation
- [ ] Tests verify that default keybindings dispatch the correct action
- [ ] Tests verify that config-overridden keybindings dispatch the correct action
- [ ] Given a test with `"shortcuts": {"split_horizontal": "ctrl+alt+h"}`, when simulating Ctrl+Alt+H, then the split horizontal action fires
- [ ] Given a test with an invalid shortcut config, when loading, then the default is used (no panic)

#### US-017: Session Persistence Round-Trip Tests
**Description:** As a developer, I want tests that verify session save/restore produces identical state, so that session persistence regressions are caught automatically.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] At least 3 tests cover session round-trips: single workspace, multiple workspaces, workspace with splits
- [ ] Tests verify that workspace titles, split ratios, and CWD survive a save/restore cycle
- [ ] Tests verify that scrollback text (when US-011 is done) survives a save/restore cycle
- [ ] Given a session with 3 workspaces and nested splits, when saved and restored, then the structure is identical
- [ ] Given a corrupted session.json (truncated file), when restoring, then the app starts with a fresh session (no crash)

---

## Functional Requirements

- FR-01: The system must read `default_shell` from config and use it to spawn terminal processes
- FR-02: The system must allow users to override any keybinding via the `shortcuts` config field
- FR-03: The system must use filesystem event notifications (not polling) for config change detection
- FR-04: The system must provide in-buffer text search across the full scrollback history
- FR-05: The system must support keyboard-driven text selection (copy mode) without mouse interaction
- FR-06: The system must support pane zoom (temporarily maximize one pane)
- FR-07: The system must persist scrollback history (up to 4000 lines per terminal) across sessions
- FR-08: The system must use atomic writes for session persistence files
- FR-09: The system must NOT truncate scrollback within an ANSI escape sequence

## Non-Functional Requirements

- **Performance:** Scrollback search returns results in <100ms for 10,000 lines of history
- **Performance:** Config hot-reload applies changes within 500ms of file save
- **Performance:** Session save completes in <200ms for 20 workspaces with scrollback
- **Reliability:** Session file is never corrupted — atomic write + rename pattern
- **Reliability:** GPU absence produces a clean exit (code 1) with actionable error message, not a panic
- **Resource usage:** Session file size <5MB for 20 workspaces with max scrollback (4000 lines × 20 terminals)
- **Compatibility:** All features work on both Wayland and X11 display backends
- **Test coverage:** App crate has >15 integration tests after EP-004

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Shell not found | `default_shell` points to nonexistent binary | Fall back to `$SHELL`, then `/bin/sh`. Log warning. | Warning in terminal: "Configured shell not found, using fallback" |
| 2 | Config file deleted | User deletes `paneflow.json` while app runs | Continue with last known config. Log info. | — |
| 3 | Vulkan unavailable | No GPU driver or VM without passthrough | Print error to stderr, exit code 1 | "PaneFlow requires Vulkan. Install mesa-vulkan-drivers." |
| 4 | Max panes reached | User tries to split beyond 32 panes | Reject the split action. Flash the status bar. | — |
| 5 | Session file corrupted | Truncated JSON from previous crash | Start fresh session. Log warning. Old file moved to `.bak`. | — |
| 6 | Scrollback memory explosion | Terminal outputs 100MB of binary data | Cap at 400,000 chars per terminal in session save | — |
| 7 | Empty search query | User opens search and presses Enter without typing | Close overlay without error | — |
| 8 | Swap with single pane | User tries to swap in a workspace with one pane | Ignore, no error | — |
| 9 | Zoom + close last pane | User closes the zoomed pane (last remaining) | Workspace closes normally | — |
| 10 | Invalid shortcut string | Config contains `"split_horizontal": "🍕"` | Ignore entry, use default, log warning | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | GPUI headless testing not supported | Med | High | US-015 is a spike — if `#[gpui::test]` doesn't work headless, pivot to unit-testing extractable logic without GPUI entities |
| 2 | alacritty_terminal selection API insufficient for copy mode | Low | Med | US-007 validates the API — if insufficient, implement selection tracking manually using terminal grid coordinates |
| 3 | Scrollback persistence makes session files too large | Low | Med | 4000 lines × 400K chars cap per terminal. Monitor file sizes in testing. |
| 4 | Config watcher fires too frequently | Low | Low | Already has 300ms debounce. Add rate limiting if needed. |
| 5 | Keybinding conflicts between user shortcuts and hardcoded GPUI bindings | Med | Med | Document reserved keys. Validate shortcuts against GPUI's internal bindings on load. |
| 6 | 17 stories is too many for one PRD cycle | Med | Med | Phase 1 (EP-001 + EP-002) can ship independently. EP-003 + EP-004 are additive, not blocking. |

## Non-Goals

Explicit boundaries — what this PRD does NOT include:

- **Browser panes** — cmux's embedded WKWebView is macOS-specific. PaneFlow will not embed a browser in v1. (Revisit if Servo spike from `prd-servo-webview-spike.md` proves viable.)
- **Shell integration / injection** — cmux injects into zsh startup to track CWD, git branch, ports. Too invasive for v1. PaneFlow uses OSC 7 for CWD tracking.
- **Command palette / omnibar** — Nice-to-have deferred to a future PRD.
- **Project-level config** — cmux walks directories for `cmux.json`. Deferred — global config is sufficient for v1.
- **Port scanner** — cmux scans TTYs for listening ports. Deferred.
- **AI agent integration** — cmux tracks Claude Code sessions. Out of scope.
- **Notifications** — cmux has system notification support. Deferred.
- **macOS/Windows support** — Linux-only for this PRD.
- **GPUI version bump** — Stay pinned at rev `0b984b5`.

## Files NOT to Modify

- `crates/paneflow-config/src/lib.rs` — Config schema changes only via the config crate's own stories. App crate reads, doesn't modify.
- `Cargo.toml` (workspace root) — No new workspace members or patch entries in this PRD.

## Technical Considerations

Frame as questions for engineering input:

- **Search overlay rendering:** Should the search bar be a GPUI `Render` view overlaid on the terminal, or a custom `Element` painted in `TerminalElement::paint()`? Recommended: separate `Render` view positioned absolutely — simpler to build and maintain.
- **Copy mode cursor:** Should the copy mode cursor be a blinking block rendered by `TerminalElement`, or a GPUI overlay div? Recommended: render in `TerminalElement::paint()` alongside the regular cursor — avoids z-order issues.
- **Shortcut parsing:** Use a simple `"modifier+key"` string format (e.g., `"ctrl+shift+d"`) parsed into GPUI `KeyBinding`? Or use GPUI's native keybinding format? Recommended: simple string format for user friendliness, parsed into GPUI structs internally.
- **Scrollback extraction:** `alacritty_terminal::Term` provides `grid()` access. Should we iterate the grid directly, or use `term.selection_to_string()` with a full-buffer selection? Recommended: iterate the grid — `selection_to_string` is for user selections, not full-buffer extraction.
- **Test harness:** Does GPUI's `#[gpui::test]` work without a GPU/display? If not, extract testable logic into pure functions and test those. Check Zed's own test patterns.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Config fields wired | 2/6 functional (theme, window_decorations) | 6/6 functional | Month-1 | Manual audit of config fields |
| Terminal power features | 0/5 (no search, copy, zoom, equalize, swap) | 5/5 implemented | Month-1 | Feature checklist |
| App crate test count | 0 | 15+ | Month-1 | `cargo test -p paneflow-app \| grep "test result"` |
| Session restore fidelity | Layout + CWD only | Layout + CWD + scrollback | Month-1 | Manual test: exit and restart, verify scrollback present |
| Split resize accuracy | Hardcoded 800px | Accurate on any resolution | Month-1 | Test on 1366px, 1920px, 3840px monitors |

## Open Questions

- Should copy mode support vi-style navigation (h/j/k/l) in addition to arrow keys? Decision needed before US-007 implementation. Depends on target user expectations.
- Should search support regex in v1, or just plain text? Plain text recommended for v1, regex as a future enhancement.
- What should the session autosave interval be? cmux uses 8 seconds. PaneFlow currently saves on exit only. Decision needed before US-013.
- Should `Ctrl+Shift+E` be reassigned from vertical split to equalize? If so, what replaces it for vertical split? Proposed: `Ctrl+Shift+|` for vertical split.
[/PRD]
