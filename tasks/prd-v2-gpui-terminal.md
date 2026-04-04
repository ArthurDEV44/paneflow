[PRD]
# PRD: PaneFlow v2 — GPUI Native Terminal Multiplexer

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-03 | Arthur (via Claude) | Initial draft — GPUI rewrite replacing Tauri+xterm.js, validated by spike |

## Problem Statement

1. **PaneFlow v1 (Tauri+xterm.js) has an irreducible latency ceiling of ~18-25ms per keystroke.** The WebView IPC boundary (JSON serialization ~1-2ms), xterm.js `requestAnimationFrame` gate (0-16.6ms at 60Hz), and base64 encode/decode pipeline create overhead that cannot be eliminated within the WebView paradigm. cmux achieves ~2-3ms. PaneFlow v1 scored 38% (46/120) against cmux's 12 typing-latency design principles.

2. **PaneFlow v1 has 6 critical implementation bugs.** ZerolagInputAddon loaded but never wired. PTY processes destroyed on workspace switch. alacritty_terminal emulator instantiated but never fed output. Exit code hardcoded to 0. `cwd` never passed to spawn. `write_pty` invoke command dead code.

3. **The iced 0.14 + WGPU rewrite (PRD v2) failed.** Building a custom WGPU glyph atlas from scratch (US-004, 5 story points) was too complex and unstable. iced had known performance regressions (issues #2848, #2899).

4. **GPUI eliminates the hard problems.** GPUI handles text rendering natively (cosmic-text on Linux, Core Text on macOS). Zed's terminal implementation (~8500 LOC) provides a complete blueprint for cell rendering, keystroke handling, and PTY integration. A spike (`spike-gpui/`) successfully compiles and runs on Linux Wayland, validating the approach.

**Why now:** The GPUI spike is validated. Okena (GPUI terminal mux, v0.18.0, 883 commits) proves the architecture works at scale. gpui-component v0.5 provides 59 widgets including Sidebar and Resizable. The Zed codebase at `/home/arthur/dev/zed` is available as a complete reference.

## Overview

PaneFlow v2 replaces the Tauri v2 + SolidJS + xterm.js stack with GPUI (Zed's framework) + alacritty_terminal (Zed's fork). The core Rust crates (`paneflow-core`, `paneflow-ipc`, `paneflow-config`, `paneflow-cli`) are preserved. The Tauri app shell (`src-tauri/`) and web frontend (`frontend/`) are replaced entirely.

Key architectural changes:
- **Zero-IPC keystroke path:** GPUI KeyDownEvent -> `try_keystroke()` -> `pty_tx.notify(Cow<[u8]>)` -> PTY fd. Same process, zero serialization. Target: < 100us per keystroke.
- **GPUI native text rendering:** No custom glyph atlas. GPUI's text system (cosmic-text on Linux) handles font rasterization, shaping, and GPU upload. Terminal cells rendered as `BatchedTextRun` objects via the `Element` trait.
- **Zed-compatible themes:** 32 terminal color slots in JSON format, hot-reload via filesystem watcher (100ms debounce). Compatible with Zed's theme ecosystem.
- **Demand-driven rendering:** PTY output triggers `cx.notify()` -> GPUI repaint. No polling loop, no `requestAnimationFrame`.
- **alacritty EventLoop for PTY I/O:** Not portable-pty. Alacritty's native `EventLoop` handles PTY read/write/resize with `FairMutex` and `polling` crate. Battle-tested in Zed.

## Goals

| Goal | Month-1 Target | Month-3 Target |
|------|---------------|----------------|
| Typing latency (keystroke to pixel) | < 8ms P95 on 60Hz | < 5ms P95 on 120Hz |
| Terminal rendering throughput | 60 FPS during `cat /dev/urandom \| xxd` | 120 FPS on high-refresh |
| Cross-platform | Linux (X11+Wayland) primary | Linux + macOS |
| Socket API | 15 core methods | 30+ methods |
| Memory per terminal pane | < 15 MB (5000-line scrollback) | < 10 MB optimized |
| Cold start | < 800ms | < 400ms |
| Theme support | 5 bundled themes | Import Zed/Alacritty themes |

## Target Users

### AI Agent Developer (Primary)
- **Role:** Developer running 2-8 AI coding agents (Claude Code, Codex, OpenCode) in parallel
- **Pain points:** cmux requires macOS; PaneFlow v1 has perceptible input lag; no cross-platform multiplexer offers both agent IPC and native latency
- **Success:** Launch PaneFlow on Linux, create 4 workspaces via socket API, type with zero perceptible lag

### Terminal Power User (Secondary)
- **Role:** Developer using tmux/Zellij daily on Linux wanting GPU-accelerated multiplexing
- **Pain points:** tmux is TUI-only; WezTerm ~26ms latency; no GPU-rendered multiplexer with rich sidebar on Linux
- **Success:** Switch from tmux to PaneFlow with splits, keybindings, sidebar, and CLI

## Research Findings

### Competitive Context
| Product | Latency | Platform | Architecture | Gap |
|---------|---------|----------|-------------|-----|
| cmux | ~2-3ms | macOS only | SwiftUI+AppKit+Ghostty(Metal) | No Linux/Windows |
| Alacritty | ~7ms | Cross-platform | winit+OpenGL, no multiplexing | No splits/workspaces |
| WezTerm | ~26ms | Cross-platform | Custom wgpu, Lua config | High latency |
| Zellij | N/A (TUI) | Linux/macOS | Terminal-in-terminal | No GPU rendering |
| Okena | Unknown | Cross-platform | GPUI+alacritty_terminal | Closest reference, 31 stars |
| **PaneFlow v2** | **< 5ms target** | **Linux+macOS** | **GPUI+alacritty_terminal** | **Agent-oriented IPC** |

### Technical Validation
- **GPUI spike validated:** `spike-gpui/` opens a GPUI window, spawns a PTY, handles keystrokes, renders terminal output on Linux Wayland
- **Zed terminal blueprint:** `terminal.rs` (3500L) + `terminal_element.rs` (2342L) + `terminal_view.rs` (2710L) = 8552 LOC of reference code
- **gpui-component v0.5:** 59 widgets including Sidebar, Resizable, List, VirtualList — covers all UI chrome needs
- **Zed theme system:** 32 terminal color slots in JSON, hot-reload with 100ms debounce, One Dark example available
- **alacritty_terminal EventLoop:** Handles PTY I/O with `FairMutex`, `polling` crate, `Notifier` for writes. Used by Zed in production.

### Best Practices Applied (from cmux typing-latency-architecture.md)
| cmux Principle | PaneFlow v2 Implementation |
|---------------|---------------------------|
| Keyboard events bypass UI framework | GPUI `on_key_down` -> direct `pty_tx.notify()`, no framework event loop |
| Terminal rendering outside UI framework | `Element` trait custom paint, GPUI layout doesn't diff terminal cells |
| Demand-driven rendering | `cx.notify()` on PTY output, no polling |
| Zero-allocation hot path | `Cow<'static, [u8]>` for keystroke bytes, `FairMutex` for term lock |
| Coalesce everything | 4ms sync interval batches PTY output, `BatchedTextRun` for rendering |
| All I/O off main thread | alacritty EventLoop on dedicated OS thread |
| Debug instrumentation = zero in release | `#[cfg(debug_assertions)]` timing probes |

## Assumptions & Constraints

### Assumptions (to validate during implementation)
- GPUI path deps to Zed monorepo will remain stable across Zed releases (medium confidence — pin to a specific commit)
- gpui-component v0.5 is compatible with GPUI 0.2.2 from the Zed monorepo (medium confidence — verify during EP-002)
- alacritty_terminal's `EventLoop` can be used without the full Alacritty app context (high confidence — validated in spike and by Zed)
- `cosmic-text` on Linux provides sub-millisecond glyph shaping for monospace terminal fonts (high confidence — Zed uses this in production)

### Hard Constraints
- Must preserve `paneflow-core`, `paneflow-ipc`, `paneflow-config`, `paneflow-cli` crate APIs
- Must run on Linux (X11 + Wayland) as primary platform
- Typing latency must be < 8ms P95 — the project's raison d'etre
- No WebView for terminal rendering — GPUI native GPU only
- Apache-2.0 license (compatible with GPUI's Apache-2.0 and alacritty_terminal's Apache-2.0)
- Zed monorepo at `/home/arthur/dev/zed` is the reference codebase for all GPUI patterns

## Quality Gates

These commands must pass for every user story:
- `cargo check --workspace` — compilation check across all crates
- `cargo clippy --workspace -- -D warnings` — lint with zero warnings
- `cargo test --workspace` — full test suite
- `cargo build --release` — release build succeeds

For UI stories:
- Launch the app, visually verify the feature works as described
- Verify on at least one Linux environment (X11 or Wayland)

## Epics & User Stories

### EP-001: GPUI App Shell & Window

Establish the GPUI application window with basic layout regions (sidebar + main area), replacing the Tauri WebView shell. This epic evolves the validated spike into a proper app structure.

**Definition of Done:** A native GPUI window opens on Linux showing a sidebar region and a main content area. The terminal rendering is cell-by-cell with colors.

**Zed reference:** `crates/gpui/examples/hello_world.rs` for app bootstrap, `crates/workspace/src/pane_group.rs` for layout patterns.

#### US-001: GPUI Application Bootstrap with Vendored Dependencies
**Description:** As a developer, I want PaneFlow to open as a native GPU-accelerated window using GPUI so that there is no WebView overhead.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Running `cargo run` from the project root opens a native GPUI window (wgpu on Linux, Metal on macOS)
- [ ] Window has title "PaneFlow", default size 1200x800, min size 800x500
- [ ] Window is resizable and responds to platform close/minimize/maximize
- [ ] The layout has two regions: a fixed-width left sidebar (220px) and a flexible main content area, using GPUI `div()` flex layout
- [ ] The main content area shows a placeholder message when no terminal panes exist
- [ ] All GPUI dependencies are vendored via path deps to `/home/arthur/dev/zed` with `[patch.crates-io]` for async-task, calloop
- [ ] `[profile.release]` includes `lto = "thin"`, `codegen-units = 1`, `strip = true`
- [ ] Given a GPU driver that doesn't support wgpu Vulkan, when the app starts, then it logs an actionable error message (not a panic)

**Zed reference:** Explore `crates/gpui/examples/hello_world.rs` and `crates/gpui_platform/src/gpui_platform.rs` at `/home/arthur/dev/zed`

#### US-002: Terminal Cell Renderer with ANSI Colors
**Description:** As a developer, I want terminal text rendered cell-by-cell with full ANSI color support so that command output is readable and colored.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] A `TerminalElement` struct implements GPUI's `Element` trait with `request_layout`, `prepaint`, and `paint` lifecycle methods
- [ ] Terminal cells from `alacritty_terminal::Term::renderable_content()` are rendered as `BatchedTextRun` objects — adjacent cells with identical style are grouped into single text runs for GPU efficiency
- [ ] All 32 terminal color slots are supported: 8 base ANSI + 8 bright + 8 dim + foreground + background + bright_foreground + dim_foreground + ansi_background + cursor
- [ ] 24-bit true color (`Color::Spec(rgb)`) renders the exact RGB value
- [ ] 256-color indexed palette (`Color::Indexed(i)`) maps to the standard xterm-256color palette
- [ ] Cell attributes are rendered: bold (font weight 700), italic, underline, strikethrough, inverse video
- [ ] CJK wide characters render correctly (occupying 2 cell widths)
- [ ] Background colors are rendered as `LayoutRect` quads behind the text
- [ ] Given `echo -e "\e[31mRed\e[32mGreen\e[0mDefault"`, text renders in red, green, then default foreground
- [ ] Given a 200x60 terminal grid, full frame render time is < 2ms (measured via `Instant::now()` in debug builds)
- [ ] Given an empty cell (space or NUL), it is not included in text runs (performance optimization)

**Zed reference:** Explore `crates/terminal_view/src/terminal_element.rs` (lines 83-450 for `BatchedTextRun` and `layout_cells`, lines 1628-1672 for `convert_color`) at `/home/arthur/dev/zed`

#### US-003: Cursor Rendering
**Description:** As a developer, I want a visible terminal cursor so that I know where input will appear.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] The cursor renders at the `alacritty_terminal` cursor position from `renderable_content().cursor`
- [ ] Three cursor styles supported: block (default), beam, underline — responding to DECSCUSR escape sequences
- [ ] Cursor blinking with configurable interval (default 530ms) using a GPUI timer/subscription
- [ ] Given the terminal loses focus, the cursor renders as a hollow block outline
- [ ] Given no PTY output for 530ms, the cursor blinks (alternates visible/invisible)

**Zed reference:** Explore `crates/terminal_view/src/terminal_element.rs` (search for `CursorLayout`, `cursor` painting) at `/home/arthur/dev/zed`

---

### EP-002: PTY Bridge & Zero-Latency Input

Wire PTY processes to the GPUI terminal renderer with zero IPC overhead. Keystrokes write directly to the PTY fd in the same process via alacritty's `Notifier`.

**Definition of Done:** Typing in a terminal pane has < 8ms P95 latency. PTY output renders at 60 FPS without blocking the UI thread.

**Zed reference:** `crates/terminal/src/terminal.rs` for PTY wiring, `crates/terminal/src/mappings/keys.rs` for keystroke translation.

#### US-004: PTY Spawn via alacritty EventLoop
**Description:** As a developer, I want each terminal pane backed by an alacritty EventLoop so that PTY I/O is handled by battle-tested code.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] `TerminalState::new()` creates a PTY via `alacritty_terminal::tty::new()`, initializes a `Term<ZedListener>` with `FairMutex`, and starts an `EventLoop`
- [ ] The `EventLoop` runs on a dedicated OS thread (`event_loop.spawn()`) — not a tokio task
- [ ] The `Notifier` (wrapping `mpsc::Sender<Msg>`) is stored for write access
- [ ] PTY spawns the user's `$SHELL` (or `/bin/bash` fallback) in the configured working directory
- [ ] `Term` is sized to match the GPUI container dimensions (cols x rows computed from pixel size / cell size)
- [ ] Given a shell that exits (e.g., `exit`), when the child process terminates, an `AlacTermEvent::ChildExit` is received and the pane shows "[Process exited with code N]"
- [ ] Given the pane is closed, the `Notifier` sends `Msg::Shutdown` to cleanly terminate the EventLoop thread

**Zed reference:** Explore `crates/terminal/src/terminal.rs` (lines 564-682 for TerminalBuilder, lines 1449-1461 for write_to_pty) at `/home/arthur/dev/zed`

#### US-005: Zero-Allocation Keystroke Path
**Description:** As a developer, I want keystrokes to reach the PTY with zero serialization so that typing feels instantaneous.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] GPUI `on_key_down` captures `KeyDownEvent` on the terminal view's focus handle
- [ ] `key_char` (printable characters) is sent directly to `pty_tx.notify()` as bytes — no JSON, no base64, no IPC
- [ ] Ctrl+key combinations (Ctrl+C, Ctrl+D, Ctrl+Z) bypass IME and write the control byte directly
- [ ] Special keys (Enter, Tab, Escape, Backspace, arrows, function keys) are mapped to ANSI escape sequences using a dedicated `keys.rs` mapping module
- [ ] The entire path from `KeyDownEvent` to `write()` on the PTY fd involves zero heap allocations for ASCII printable characters (use `Cow::Borrowed` for static byte slices)
- [ ] Given typing at 120 WPM (~10 chars/sec), P99 per-keystroke PTY write latency is < 100 microseconds (measured with `#[cfg(debug_assertions)]` probes)
- [ ] Given a non-terminal-focused UI element (command palette, sidebar), keystrokes route to that element instead of the terminal

**Zed reference:** Explore `crates/terminal/src/terminal.rs` (lines 1573-1590 for `try_keystroke`, lines 1463-1474 for `input`) and `crates/terminal/src/mappings/keys.rs` at `/home/arthur/dev/zed`

#### US-006: Demand-Driven Output Pipeline
**Description:** As a developer, I want terminal rendering triggered by PTY output so that there is no wasted CPU when idle and minimal latency when active.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004, US-002

**Acceptance Criteria:**
- [ ] A sync task runs on a GPUI-spawned async task, polling the terminal `Term` state at a 4ms interval
- [ ] `terminal.sync()` calls `term.lock().renderable_content()` and caches the cell grid, cursor, and mode
- [ ] After sync, `cx.notify()` is called to request a GPUI repaint — only if dirty cells exist
- [ ] There is no app-level fixed timer or `requestAnimationFrame` — rendering is driven by PTY output via the sync task
- [ ] Given the terminal is idle (no PTY output), CPU usage is < 1%
- [ ] Given `seq 1 1000000` (bulk output), frames are presented at the display refresh rate without blocking keystroke processing
- [ ] Given PTY output produces 100KB/s, the sync task coalesces all pending output before triggering one repaint (not one repaint per read)

**Zed reference:** Explore `crates/terminal/src/terminal.rs` (lines 1621-1660 for `sync` and `make_content`) at `/home/arthur/dev/zed`

---

### EP-003: Split Pane Tiling Engine

Build a binary-tree tiling layout using `paneflow-core::SplitTree` and GPUI flex layout, with resizable dividers.

**Definition of Done:** Terminal panes can be split horizontally/vertically, resized by dragging, closed, and zoomed.

#### US-007: Binary Tree Split Layout
**Description:** As a developer, I want to split terminal panes horizontally and vertically so I can view multiple terminals side-by-side.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001, US-004

**Acceptance Criteria:**
- [ ] `paneflow-core::SplitTree` drives the layout: leaf nodes are terminal panes, branch nodes are split containers with direction and ratio
- [ ] Ctrl+Shift+D splits the focused pane horizontally; Ctrl+Shift+E splits vertically
- [ ] Each split spawns a new PTY in the new pane
- [ ] Closing a pane (Ctrl+Shift+W or shell exit) collapses its parent split, expanding the sibling
- [ ] The layout is rendered using GPUI `div()` with `flex()`, `flex_row()`/`flex_col()`, and `flex_basis()` for ratios
- [ ] Minimum pane size is 80px in both dimensions
- [ ] Given 8 recursive splits (16 panes), all panes are visible and rendering correctly
- [ ] Given the last pane in a workspace is closed, the workspace shows the empty state placeholder

**Zed reference:** Explore `crates/workspace/src/pane_group.rs` (lines 30-53 for `Member` enum, full file for layout rendering) at `/home/arthur/dev/zed`

#### US-008: Drag-to-Resize Split Dividers
**Description:** As a developer, I want to drag dividers between panes to resize them.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] A 4px divider renders between split children using a GPUI `div()` with appropriate cursor style
- [ ] Mouse drag on the divider updates the split ratio in real-time via `paneflow-core::SplitTree`
- [ ] The ratio is clamped so neither child falls below 80px
- [ ] During drag, terminal panes do not receive mouse events (pointer-events disabled)
- [ ] Given a fast drag past the minimum boundary, the ratio clamps correctly (no panic)

#### US-009: Pane Focus Navigation
**Description:** As a developer, I want to navigate between panes with keyboard shortcuts.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007

**Acceptance Criteria:**
- [ ] Alt+Arrow (Up/Down/Left/Right) moves focus to the adjacent pane in that direction
- [ ] The focused pane has a visible border highlight (2px accent color)
- [ ] Focus state is tracked per-workspace and preserved across workspace switches
- [ ] Given only one pane exists, Alt+Arrow is a no-op (no error)

---

### EP-004: Workspace & Sidebar

Multi-workspace support with a sidebar, workspace switching, and session management.

**Definition of Done:** Users can create, switch, rename, and close workspaces. Sidebar shows workspace list with status.

#### US-010: Sidebar with Workspace List
**Description:** As a developer, I want a sidebar showing my workspaces so I can navigate between them.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Sidebar renders a scrollable list of workspaces using gpui-component's `Sidebar` widget (or GPUI `div()` if incompatible)
- [ ] Each workspace item shows: title (bold), current directory (muted), active pane count
- [ ] Clicking a workspace selects it and updates the main content area
- [ ] The selected workspace is visually highlighted (accent color background)
- [ ] A "+" button at the bottom creates a new workspace with a default shell
- [ ] Ctrl+1-9 switches to workspace 1-9; Ctrl+Tab cycles forward
- [ ] Given 50 workspaces, scrolling stays smooth (virtual list if needed)
- [ ] Given zero workspaces at startup, one default workspace is auto-created

#### US-011: Workspace Lifecycle (Create/Close/Rename)
**Description:** As a developer, I want to create, close, and rename workspaces.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] Ctrl+Shift+N creates a new workspace, appends to list, selects it
- [ ] Ctrl+Shift+Q closes the current workspace and all its PTY processes
- [ ] Double-click on workspace title in sidebar enables inline rename
- [ ] Closing a workspace does NOT destroy PTYs in other workspaces
- [ ] Given the last workspace is closed, a confirmation dialog appears
- [ ] Workspace switching preserves all PTY processes in background workspaces (no kill/respawn — this was a v1 bug)

#### US-012: Terminal Resize on Layout Change
**Description:** As a developer, I want terminal dimensions to update when panes are resized so that shell output wraps correctly.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-007, US-004

**Acceptance Criteria:**
- [ ] When a pane's pixel dimensions change (split resize, window resize), new cols/rows are computed from pixel size / cell size
- [ ] `Msg::Resize(WindowSize)` is sent to the alacritty EventLoop to update the PTY's SIGWINCH
- [ ] `term.lock().resize()` updates the alacritty grid dimensions
- [ ] Resize is debounced — rapid resize events are coalesced (one resize per frame)
- [ ] Given a split divider drag, the terminal reflows text correctly after release

---

### EP-005: Theming & Configuration

JSON-based theming compatible with Zed's theme format, hot-reload via `paneflow-config` watcher.

**Definition of Done:** Users can switch between bundled themes and load custom themes from `~/.config/paneflow/themes/`.

#### US-013: Theme Engine with 32 Terminal Color Slots
**Description:** As a developer, I want terminal colors driven by a theme so that I can customize the appearance.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] A `TerminalTheme` struct holds 32 color slots matching Zed's schema: `terminal_background`, `terminal_foreground`, `terminal_bright_foreground`, `terminal_dim_foreground`, `terminal_ansi_background`, plus 8 hues x 3 intensities (normal/bright/dim)
- [ ] `convert_color()` maps alacritty `Color` values to theme colors (same logic as Zed's `terminal_element.rs:1628`)
- [ ] 5 bundled themes: One Dark, Catppuccin Mocha, Dracula, Gruvbox Dark, Solarized Dark
- [ ] Theme JSON files follow Zed's format with `terminal.*` keys and hex RGBA values
- [ ] Given `echo -e "\e[31mRed"`, the text renders in the theme's `terminal.ansi.red` color
- [ ] Given a theme with no `terminal.ansi.background` key, the fallback is `terminal.background`

**Zed reference:** Explore `crates/theme/src/styles/colors.rs` (lines 244-303 for ThemeColors) and `assets/themes/one/one.json` (lines 71-98 for terminal color section) at `/home/arthur/dev/zed`

#### US-014: Theme Hot-Reload from Config
**Description:** As a developer, I want themes to reload when I edit the config file.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-013

**Acceptance Criteria:**
- [ ] User themes are loaded from `~/.config/paneflow/themes/*.json`
- [ ] Active theme is set in `~/.config/paneflow/config.json` via `"theme": "One Dark"`
- [ ] `paneflow-config` watcher (already implemented) detects config changes and triggers theme reload
- [ ] Theme reload debounced at 100ms (matching Zed's pattern)
- [ ] Given a user edits a theme JSON file, within 200ms the terminal colors update without restart
- [ ] Given an invalid theme JSON, the current theme is preserved and an error is logged

---

### EP-006: IPC, CLI & Debug Instrumentation

Wire the existing socket server and CLI into the GPUI app. Add typing latency measurement.

**Definition of Done:** The JSON-RPC socket server runs alongside the GPUI app. CLI can control workspaces. Debug builds measure typing latency.

#### US-015: Socket Server Integration
**Description:** As a developer, I want the JSON-RPC socket server running in the GPUI app so that AI agents can control PaneFlow.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-011

**Acceptance Criteria:**
- [ ] `paneflow-ipc::SocketServer` starts on app launch at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`
- [ ] The server handles 15 methods: `system.ping`, `system.capabilities`, `system.identify`, `workspace.list`, `workspace.create`, `workspace.select`, `workspace.close`, `workspace.current`, `surface.list`, `surface.split`, `surface.close`, `surface.send_text`, `surface.send_key`, `surface.focus`, `surface.resize`
- [ ] Socket commands that mutate UI state dispatch to the GPUI main thread via `cx.update()`
- [ ] Read-only queries respond without blocking the main thread
- [ ] `surface.send_text` writes directly to the PTY via `Notifier` — no IPC overhead within the process
- [ ] Given two agents connecting simultaneously, commands are serialized correctly
- [ ] Given the app is closing, the server returns errors and shuts down cleanly

#### US-016: CLI Binary
**Description:** As a developer, I want a `paneflow` CLI that controls the running app via the socket.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `paneflow-cli` binary connects to the socket and sends JSON-RPC commands
- [ ] Supports: `paneflow workspace list`, `paneflow workspace create [name]`, `paneflow workspace select [id]`, `paneflow send-text [pane-id] [text]`, `paneflow split [direction]`
- [ ] Exit code 0 on success, 1 on error with stderr message
- [ ] Given no running PaneFlow instance, the CLI reports "PaneFlow is not running" and exits with code 1

#### US-017: Debug Typing Latency Probes
**Description:** As a developer, I want typing latency measured in debug builds so that I can verify the < 8ms P95 target.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] `#[cfg(debug_assertions)]` timing probes measure: (1) KeyDownEvent to PTY write, (2) PTY output to render completion, (3) total keystroke-to-pixel
- [ ] Probes use `std::time::Instant` (zero overhead in release builds)
- [ ] Threshold logging: only log if any phase exceeds 1ms or total exceeds 8ms
- [ ] Activation via `PANEFLOW_LATENCY_PROBE=1` environment variable
- [ ] Given normal typing, no probe output is emitted (below threshold)
- [ ] Given a latency spike > 8ms, a log line is emitted with per-phase breakdown

---

### EP-007: Selection, Clipboard & Scroll (P1)

Mouse selection, clipboard integration, and scrollback for terminal power users.

**Definition of Done:** Users can select text with the mouse, copy to clipboard, paste, and scroll through history.

#### US-018: Mouse Text Selection
**Description:** As a developer, I want to select terminal text with the mouse and copy it.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Click-and-drag selects text using `alacritty_terminal`'s `Selection` model
- [ ] Selected text is visually highlighted (inverted colors or highlight background)
- [ ] Double-click selects a word; triple-click selects a line
- [ ] Ctrl+Shift+C copies selected text to system clipboard (via GPUI's `cx.write_to_clipboard()`)
- [ ] Ctrl+Shift+V pastes clipboard content as PTY input (with bracketed paste mode support)
- [ ] Given a selection spanning multiple lines, line breaks are preserved in the clipboard
- [ ] Given no text selected, Ctrl+Shift+C is a no-op

**Zed reference:** Explore `crates/terminal/src/terminal.rs` (search for `Selection`, `SelectionType`, `mouse_button_down`) at `/home/arthur/dev/zed`

#### US-019: Scrollback Navigation
**Description:** As a developer, I want to scroll through terminal history.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Scroll wheel scrolls through the terminal scrollback buffer (5000 lines default)
- [ ] Shift+PageUp/PageDown scrolls one page at a time
- [ ] Scrolling uses `alacritty_terminal`'s `Scroll` enum (`Delta`, `PageUp`, `PageDown`, `Top`, `Bottom`)
- [ ] The scroll position is displayed via a scrollbar indicator (thin, overlay style)
- [ ] Given new PTY output while scrolled up, the terminal does NOT auto-scroll to bottom (unless the user was already at the bottom)

---

## Crates Preserved from PaneFlow v1

| Crate | Status | Notes |
|-------|--------|-------|
| `paneflow-core` | **Preserved** | SplitTree, TabManager, Workspace, Panel — pure domain model |
| `paneflow-ipc` | **Preserved** | SocketServer, Dispatcher, Handlers — pure tokio |
| `paneflow-config` | **Preserved** | Loader, Schema, Watcher — pure notify/serde |
| `paneflow-cli` | **Preserved** | CLI client — pure clap/libc |
| `paneflow-terminal` | **Replaced** | PtyManager/bridge replaced by alacritty EventLoop integration |
| `src-tauri/` | **Removed** | Replaced by GPUI app shell |
| `frontend/` | **Removed** | Replaced by GPUI rendering |

## Files NOT to Modify

These v1 crate files should remain untouched unless their API needs adaptation for GPUI integration:

- `crates/paneflow-core/src/workspace.rs` — domain model
- `crates/paneflow-core/src/tab_manager.rs` — workspace list management
- `crates/paneflow-core/src/split_tree.rs` — binary tree layout engine
- `crates/paneflow-core/src/panel.rs` — panel trait
- `crates/paneflow-ipc/src/server.rs` — socket server loop
- `crates/paneflow-ipc/src/dispatcher.rs` — method routing
- `crates/paneflow-ipc/src/protocol.rs` — JSON-RPC types
- `crates/paneflow-config/src/loader.rs` — config file loading
- `crates/paneflow-config/src/schema.rs` — config schema
- `crates/paneflow-config/src/watcher.rs` — filesystem watcher
- `crates/paneflow-cli/src/main.rs` — CLI entrypoint
- `crates/paneflow-cli/src/client.rs` — socket client

## Dependency Architecture

```toml
# New workspace Cargo.toml
[workspace]
members = ["crates/*", "src-app"]

[workspace.dependencies]
# GPUI from Zed monorepo
gpui = { path = "/home/arthur/dev/zed/crates/gpui" }
gpui_platform = { path = "/home/arthur/dev/zed/crates/gpui_platform", features = ["wayland", "x11"] }

# Alacritty terminal — Zed's fork
alacritty_terminal = { git = "https://github.com/zed-industries/alacritty", rev = "9d9640d4" }

# Preserved crates
paneflow-core = { path = "crates/paneflow-core" }
paneflow-ipc = { path = "crates/paneflow-ipc" }
paneflow-config = { path = "crates/paneflow-config" }

[patch.crates-io]
async-task = { git = "https://github.com/smol-rs/async-task.git", rev = "b4486cd" }
calloop = { git = "https://github.com/zed-industries/calloop" }

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

## Success Metrics

| Metric | Baseline (v1) | Target (v2) | Timeframe |
|--------|--------------|-------------|-----------|
| Keystroke-to-pixel latency P95 | ~18ms | < 8ms | Month 1 |
| CPU at idle (no typing) | ~2% | < 1% | Month 1 |
| Memory per pane (5K scrollback) | ~20MB | < 15MB | Month 1 |
| Cold start to first terminal | ~1.5s | < 800ms | Month 1 |
| Story points delivered | 0 | 19 stories (34 pts) | 6 weeks |

[/PRD]
