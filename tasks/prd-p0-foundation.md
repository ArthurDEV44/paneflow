[PRD]
# PRD: PaneFlow P0 — Foundation (Terminal + Workspaces + Splits + Socket + CLI + Config)

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-01 | Arthur (via Claude) | Initial draft — P0 foundation phase |
| 1.1 | 2026-04-01 | Arthur (via Claude) | Added cmux reference files to every story |

## Problem Statement

1. **cmux is macOS-only.** cmux (7,700 GitHub stars in its first month, Feb 2026) proved massive demand for a terminal multiplexer purpose-built for AI coding agent workflows. But it requires Swift/AppKit/Metal and only runs on macOS — excluding the 70%+ of developers on Linux and Windows.

2. **No cross-platform alternative exists.** tmux is TUI-only and not agent-aware. Zellij has WASM plugins but no agent-oriented IPC. WezTerm has Lua scripting but no structured agent communication protocol. Warp is closed-source and cloud-dependent. No open-source, cross-platform terminal multiplexer offers a JSON-RPC socket API designed for AI agent orchestration.

3. **Developers running AI agents need multi-pane observability.** The bottleneck in AI-assisted development has shifted from "AI capability" to "human can only supervise one agent at a time." Terminal multiplexers that surface multi-agent state are capturing this demand, but the only viable option (cmux) locks users into macOS.

**Why now:** The AI agent terminal space is accelerating (cmux, AMUX, Conductor, Claude Code Agent Teams all emerged in Q1 2026). The window to establish a cross-platform open-source standard is narrow. cmux's codebase has been fully analyzed (111 Swift files, Go daemon, 150+ commands) providing a complete specification to port from.

## Overview

PaneFlow P0 delivers the foundational cross-platform terminal multiplexer that all subsequent phases build on. It is a Rust backend + Tauri v2 frontend application that provides:

- **Terminal emulation** via `vte` + `alacritty_terminal` + `portable_pty` — the same Rust crate stack proven by Alacritty and WezTerm, providing full VT100/xterm compatibility across Linux, Windows (ConPTY), and macOS.
- **Workspace/tab management** — an ordered list of workspaces (each containing a binary-tree of split panes with terminal surfaces), matching cmux's `TabManager → Workspace → Panel` hierarchy.
- **Binary-tree split panes** — horizontal and vertical splits with resizable dividers, matching cmux's Bonsplit layout model.
- **V2 JSON-RPC socket server** — a Unix domain socket (Linux/macOS) and named pipe (Windows) IPC server implementing the cmux V2 protocol subset, enabling AI agents to programmatically create workspaces, split panes, send text, and query state.
- **CLI binary** — a `paneflow` CLI that communicates with the running app via the socket, providing the same command grammar as `cmux` for core operations.
- **JSON configuration** — a cmux-compatible JSON config file with hot-reload via the `notify` crate.

The frontend uses SolidJS + xterm.js (WebGL renderer) inside Tauri v2 WebViews. PTY data streams via Tauri's `Channel<T>` API — purpose-built for ordered byte streaming with measured throughput suitable for terminal workloads.

Key technical decisions:
- **SolidJS over React** for fine-grained reactivity (many independently updating terminal panes)
- **xterm.js WebGL addon** for terminal rendering (up to 900% faster than canvas, used by VS Code)
- **Hand-rolled JSON-RPC** over tokio UnixListener (simpler than jsonrpsee for local IPC)
- **cmux V2 protocol compatibility** so existing cmux CLI scripts and agent integrations work with minimal changes

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Functional terminal multiplexer on Linux | Single-window with splits, sidebar, config | Full cmux P0-P2 feature parity |
| Windows support | PTY spawning + basic terminal | Full split pane support |
| Socket API coverage | system.*, workspace.*, surface.* (15 methods) | 60+ methods (cmux V2 parity) |
| Terminal rendering latency | < 16ms frame time (60fps) via xterm.js WebGL | < 8ms (120fps target) |
| CLI command coverage | 10 core commands | 40+ commands |

## Target Users

### AI Agent Developer (Primary)
- **Role:** Developer running 2-8 AI coding agents (Claude Code, Codex, OpenCode) in parallel
- **Behaviors:** Uses tmux or multiple terminal windows today; monitors agent output across sessions; sends commands to agents via CLI
- **Pain points:** Cannot observe all agents simultaneously; tmux has no structured IPC for agents; cmux requires macOS
- **Current workaround:** tmux with multiple panes, manually switching between sessions; or macOS-only cmux
- **Success looks like:** Launch `paneflow`, create 4 workspaces for 4 agents, each agent auto-creates its workspace via `paneflow new-workspace`, user sees all agent activity in sidebar

### Terminal Power User (Secondary)
- **Role:** Developer who uses tmux/Zellij daily on Linux and wants a more modern multiplexer
- **Behaviors:** Heavy keyboard user, customizes keybindings, uses split panes and named sessions
- **Pain points:** tmux config is arcane; Zellij is slower than tmux; no good GUI-integrated multiplexer on Linux
- **Current workaround:** tmux with TPM plugins, or Zellij with KDL configs
- **Success looks like:** Switch from tmux to PaneFlow with minimal config migration; splits, keybindings, and CLI all work as expected

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **tmux:** Battle-hardened (since 2007) but TUI-only, steep learning curve, no structured agent IPC
- **Zellij:** Modern Rust multiplexer with WASM plugins, but not agent-oriented; ~22MB RAM
- **WezTerm:** Closest to PaneFlow (Rust, binary-tree panes, Lua config), but no JSON-RPC API; higher latency than Alacritty
- **Market gap:** No cross-platform, open-source terminal multiplexer with a JSON-RPC socket API for AI agent orchestration

### Best Practices Applied
- Binary-tree layout model (industry standard, used by WezTerm, tiling WMs)
- `vte` + `alacritty_terminal` for VT parsing (avoids multi-year custom parser effort)
- xterm.js WebGL renderer for WebView-based terminal (VS Code's choice, 900% faster than canvas)
- Tauri v2 `Channel<T>` for PTY streaming (not events or invoke — purpose-built for ordered byte streams)
- Config hot-reload via OS-native file watching with 200-500ms debounce

*Full research sources available in CMUX_ANALYSIS.md and research agent outputs.*

## Assumptions & Constraints

### Assumptions (to validate)
- xterm.js WebGL in Tauri v2 WebView delivers < 16ms frame time for typical terminal output (based on VS Code benchmarks, but unverified in Tauri context)
- `portable_pty` Windows ConPTY support is stable enough for production use (based on WezTerm's use of the same crate)
- SolidJS + Tauri v2 integration is mature enough for a desktop app (based on TUICommander precedent)
- The cmux V2 JSON-RPC protocol can be implemented without requiring cmux-specific Swift types

### Hard Constraints
- Must run on Linux (x86_64, aarch64), Windows 10+ (ConPTY requires 1809+), macOS 12+
- Must not use Electron (contradicts "lightweight native" positioning)
- Socket protocol must be V2 JSON-RPC compatible with cmux for agent interoperability
- Config format must be JSON, compatible with cmux's `commands` schema
- Rust edition 2021+, Tauri v2 stable

## Quality Gates

These commands must pass for every user story:
- `cargo check --workspace` - Rust compilation succeeds
- `cargo clippy --workspace -- -D warnings` - No clippy warnings
- `cargo test --workspace` - All Rust tests pass
- `cd frontend && bun typecheck` - TypeScript/SolidJS type checking passes
- `cd frontend && bun lint` - ESLint passes
- `cd frontend && bun run build` - Frontend builds successfully

For UI stories, additional gates:
- Verify terminal renders correctly in Tauri window (manual: type commands, see output)
- Verify splits create and resize without visual artifacts

## Epics & User Stories

> **cmux Reference Directive:** The cmux codebase at `/home/arthur/dev/cmux` is the authoritative specification for PaneFlow. Every story below includes a **cmux Reference** section listing the exact Swift/Go files to study before implementation. Although PaneFlow uses Rust/TypeScript (not Swift), the cmux code defines the behavioral contracts, data structures, edge cases, and protocol formats that PaneFlow must replicate. **Always read the referenced cmux files first** to understand the intent, then implement the equivalent in Rust/TS. The full cmux analysis is at `/home/arthur/dev/paneflow/CMUX_ANALYSIS.md`.

### EP-001: Rust Core Domain Model

Build the foundational Rust data structures that model the workspace/tab/panel/split hierarchy. These types are consumed by all other epics.

**Definition of Done:** `Workspace`, `TabManager`, `Panel` trait, and `SplitTree` compile, pass unit tests, and can represent a multi-workspace layout with nested splits programmatically.

#### US-001: Workspace and TabManager Core Structs
**Description:** As a developer, I want core Rust structs for `Workspace` and `TabManager` so that the application has a well-defined domain model for workspace/tab management.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/Workspace.swift` — Core domain model (10K lines). Study: `id`, `title`, `customTitle`, `isPinned`, `customColor`, `currentDirectory`, `panels: [UUID: any Panel]`, `surfaceIdToPanelId`, `focusedPanelId` (computed from bonsplit), `sidebarObservationPublisher` (Combine merge of all @Published). Note: `typealias Tab = Workspace` — workspace and tab are synonymous.
- `Sources/TabManager.swift` — Workspace list manager (5K lines). Study: `tabs: [Workspace]`, `selectedTabId`, `tabHistory` (back/forward navigation, max 50), `addWorkspace()` (inherits CWD + font from selected), `selectTab()`, `closePanelAfterChildExited` (selection stability: same index, or previous if last). Note: `WorkspacePlacementSettings` controls insertion index (top/after current/end).
- `cmuxTests/TabManagerUnitTests.swift` — Test contracts for close behavior, remote workspace demotion.
- `cmuxTests/WorkspaceUnitTests.swift` — Color, shortcut defaults, rename behavior.

**Acceptance Criteria:**
- [ ] `TabManager` struct holds an ordered `Vec<Workspace>` with a `selected_id: Option<Uuid>` field
- [ ] `Workspace` struct holds `id: Uuid`, `title: String`, `custom_title: Option<String>`, `working_directory: PathBuf`, `panels: HashMap<Uuid, Box<dyn Panel>>`
- [ ] `TabManager::add_workspace()` creates a workspace and sets it as selected
- [ ] `TabManager::close_workspace(id)` removes the workspace and selects the adjacent one (same index, or previous if last)
- [ ] `TabManager::select_workspace(id)` updates `selected_id`
- [ ] `TabManager::reorder_workspace(id, new_index)` moves a workspace in the ordered list
- [ ] Closing the last workspace returns an error (not silently empty) — the caller decides whether to create a default or close the window
- [ ] Unit tests cover: add, close (middle, last, only), select, reorder

#### US-002: Panel Trait and TerminalPanel
**Description:** As a developer, I want a `Panel` trait with a `TerminalPanel` implementation so that each pane in the split tree can hold a terminal session.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/Panels/Panel.swift` — The `Panel` protocol (lines 219+). Study: required properties (`id: UUID`, `panelType`, `displayTitle`, `displayIcon`, `isDirty`), required methods (`close()`, `focus()`, `unfocus()`, `triggerFlash()`), focus intent hierarchy (`PanelFocusIntent`: `.panel`, `.terminal(.surface|.findField)`, `.browser(.webView|.addressBar|.findField)`), attention flash system (`WorkspaceAttentionFlashReason`, `FocusFlashPattern` 4-segment keyframe animation).
- `Sources/Panels/TerminalPanel.swift` — TerminalPanel implementation. Study: wraps `TerminalSurface`, properties (`title`, `directory`, `searchState`, `tmuxLayoutReport`, `hostedView`, `viewReattachToken`), focus lifecycle (`focus()` → `surface.setFocus(true)`, `unfocus()` cancels pending retries), `close()` → portal detach → teardownSurface, `sendText`, `hasSelection`, `needsConfirmClose`.
- `Sources/Panels/PanelContentView.swift` — Routes `any Panel` to typed views by `panelType` switch.
- `cmuxTests/BrowserPanelTests.swift` — Panel focus lifecycle contracts, address bar focus suppression.

**Acceptance Criteria:**
- [ ] `Panel` trait defines: `id() -> Uuid`, `panel_type() -> PanelType`, `title() -> &str`, `close()`, `send_text(&str)`, `resize(rows, cols)`
- [ ] `PanelType` enum has variants: `Terminal`, `Browser` (stub), `Markdown` (stub)
- [ ] `TerminalPanel` struct implements `Panel` and holds a PTY handle (as `Option` — actual PTY integration is US-006)
- [ ] `TerminalPanel` tracks `working_directory: PathBuf` and `custom_title: Option<String>`
- [ ] Given a `TerminalPanel` with no PTY handle, calling `send_text()` returns an error (not panic)
- [ ] Unit tests cover: create, title, close, send_text without PTY

#### US-003: Binary-Tree Split Layout Engine
**Description:** As a developer, I want a binary-tree split layout engine so that workspaces can have horizontal and vertical pane splits with configurable ratios.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/CmuxConfig.swift` (lines 127-216) — Layout tree definition: `LayoutNode` (recursive: pane or split), `CmuxSplitDefinition` (direction: horizontal/vertical, split ratio clamped [0.1, 0.9], exactly 2 children), `CmuxPaneDefinition` (surfaces array, >= 1 required). This is the JSON config schema for layouts.
- `Sources/WorkspaceContentView.swift` — How the split tree is rendered: `BonsplitView(controller: workspace.bonsplitController)` maps each Bonsplit tab to a `PanelContentView`. Study the closure that resolves `workspace.panel(for: tab.id)` via `surfaceIdToPanelId`.
- `Sources/Workspace.swift` (lines 5463+) — `bonsplitController: BonsplitController` manages the tree. Study: `newTerminalSurface(inPane:)` creates panel + bonsplit tab + registers mapping, `closePanel()` removes panel + closes bonsplit tab. The `surfaceIdToPanelId: [TabID: UUID]` dict bridges bonsplit tab IDs to panel UUIDs.
- `cmuxTests/CmuxConfigTests.swift` — Validation: split must have 2 children, pane non-empty, ratio clamping, encoding round-trip.
- `Sources/SessionPersistence.swift` — `SessionWorkspaceLayoutSnapshot` (recursive layout tree for serialization).

**Acceptance Criteria:**
- [ ] `SplitTree` enum: `Leaf { pane_id: Uuid }` or `Split { direction: Direction, ratio: f64, first: Box<SplitTree>, second: Box<SplitTree> }`
- [ ] `Direction` enum: `Horizontal`, `Vertical`
- [ ] `ratio` is clamped to `[0.1, 0.9]` with default `0.5`
- [ ] `SplitTree::split(pane_id, direction)` replaces a leaf with a split node, creating a new leaf for the new pane
- [ ] `SplitTree::close(pane_id)` removes a leaf and collapses the parent split (sibling takes its place)
- [ ] `SplitTree::resize(pane_id, new_ratio)` adjusts the split ratio of the parent
- [ ] `SplitTree::find_pane(pane_id) -> Option<&Leaf>` traverses the tree to find a pane
- [ ] `SplitTree::all_panes() -> Vec<Uuid>` returns all leaf pane IDs in order
- [ ] `SplitTree::layout(width, height) -> Vec<(Uuid, Rect)>` computes pixel-precise rectangles for each pane
- [ ] Given a split at ratio 0.5 in a 1000x600 area, horizontal split produces two 500x600 rects
- [ ] Given a deeply nested tree (3 levels), layout computes correct rects for all 4+ leaves
- [ ] Serialization/deserialization to JSON works (for config and session persistence)
- [ ] Given `close()` on the last pane in a split, the tree collapses correctly to the sibling
- [ ] Unit tests cover: split, close, resize, layout computation, serialization round-trip, edge cases (close last leaf, nested splits)

---

### EP-002: Terminal Backend

Implement PTY spawning, VT emulation, and the async bridge that connects PTY output to the Tauri frontend.

**Definition of Done:** A terminal process can be spawned, its output parsed through VT emulation, and the rendered terminal state streamed to the frontend via Tauri Channel.

#### US-004: Cross-Platform PTY Spawning and Management
**Description:** As a developer, I want cross-platform PTY spawning via `portable_pty` so that terminal processes work on Linux, Windows, and macOS.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `ghostty.h` (lines 440-453) — `ghostty_surface_config_s`: how cmux passes `working_directory`, `command`, `env_vars`, `initial_input` to the terminal engine at surface creation. This is the contract PaneFlow's `PtyManager::spawn()` must replicate.
- `Sources/GhosttyTerminalView.swift` (lines 1122-1378) — Ghostty initialization sequence: `ghostty_init()` → `ghostty_config_new()` → `ghostty_app_new()` → per-pane `ghostty_surface_new()`. Study how env vars are set: `CMUX_PORT`, `CMUX_PORT_END`, `CMUX_PORT_RANGE` (lines 3750-3752), `CMUX_TAB_ID`, `CMUX_PANEL_ID`.
- `ghostty.h` (lines 960-988) — `ghostty_runtime_config_s`: callback contract (wakeup_cb from I/O thread, action_cb for ~50 action types, clipboard read/write, close_surface). This shows the terminal engine ↔ host contract that portable_pty replaces.
- `Sources/GhosttyTerminalView.swift` (lines 944-948) — Wakeup coalescing: Ghostty calls `wakeup_cb` from I/O thread, coalesced into a single `ghostty_app_tick()` on main queue. The PaneFlow equivalent is the `spawn_blocking` PTY reader → channel batching pattern.

**Acceptance Criteria:**
- [ ] `PtyManager` struct wraps `portable_pty::NativePtySystem` and provides `spawn(command, cwd, env, size) -> Result<PtySession>`
- [ ] `PtySession` holds the master PTY reader/writer, child process handle, and pane_id
- [ ] Default shell is resolved from `$SHELL` (Unix) or `cmd.exe` (Windows)
- [ ] `PtySession::resize(rows, cols)` resizes the PTY without killing the process
- [ ] `PtySession::write(bytes)` sends input to the PTY
- [ ] `PtySession::kill()` terminates the child process
- [ ] Given a PTY spawned with `bash`, writing `echo hello\n` produces output containing "hello"
- [ ] Given PTY spawn failure (e.g., invalid command), `spawn()` returns a descriptive error (not panic)
- [ ] On Windows, ConPTY flags `PSEUDOCONSOLE_RESIZE_QUIRK` and `PSEUDOCONSOLE_WIN32_INPUT_MODE` are passed
- [ ] Integration test: spawn shell, write command, read output, resize, close — on host platform

#### US-005: VT Emulation with alacritty_terminal
**Description:** As a developer, I want VT sequence parsing via `alacritty_terminal` so that terminal output (colors, cursor movement, alternate screen) is correctly interpreted.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** US-004

**cmux Reference** (`/home/arthur/dev/cmux`):
- `ghostty.h` — In cmux, ALL VT parsing is inside Ghostty's Zig binary. PaneFlow replaces this entirely with `alacritty_terminal`. Study ghostty.h to understand what features are expected: `ghostty_surface_set_size` (resize), `ghostty_surface_draw/refresh` (repaint triggers), `ghostty_surface_key/text/preedit` (input), `ghostty_surface_mouse_*` (mouse). The key insight: Ghostty owns the grid, cursor, scrollback, and rendering — PaneFlow's `TerminalEmulator` wrapper around `alacritty_terminal::Term` must provide the same abstraction boundary.
- `Sources/GhosttyConfig.swift` — Terminal config: `backgroundOpacity`, theme resolution (paired light/dark, builtin aliases), font config. Study what config values flow into the terminal engine.
- `Sources/Find/SurfaceSearchOverlay.swift` — Terminal search: uses `ghostty_surface_binding_action(surface, "navigate_search:next")`. Shows the search feature contract (P1 but informs the `TerminalEmulator` API surface).
- `cmuxTests/TerminalAndGhosttyTests.swift` — Pasteboard string extraction tests (HTML→plain text, image→temp file). Shows expected clipboard behavior for terminal panes.

**Acceptance Criteria:**
- [ ] `TerminalEmulator` struct wraps `alacritty_terminal::Term` and processes raw PTY bytes via the `vte` parser
- [ ] 256-color and true-color (24-bit) ANSI sequences render correct color attributes
- [ ] Cursor movement sequences (CUP, CUU, CUD, CUF, CUB) update cursor position correctly
- [ ] Alternate screen buffer (smcup/rmcup) switches correctly
- [ ] `TerminalEmulator::screen_content() -> TerminalScreen` returns the current grid as a structured type (rows of cells with text + attributes)
- [ ] `TerminalEmulator::scrollback_lines() -> usize` reports scrollback buffer size
- [ ] Given `\x1b[31mred\x1b[0m`, the parsed output contains "red" with foreground color red
- [ ] Given a terminal resize from 80x24 to 120x40, the grid reflows correctly
- [ ] Given an empty terminal (no output yet), `screen_content()` returns an empty grid without error
- [ ] Unit tests: color parsing, cursor movement, alternate screen, resize reflow

#### US-006: Async PTY I/O Bridge with Tauri Channel
**Description:** As a developer, I want an async bridge between PTY output and Tauri's `Channel<T>` so that terminal data streams efficiently to the xterm.js frontend.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** US-004, US-005

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/GhosttyTerminalView.swift` (lines 944-948) — Wakeup coalescing pattern: Ghostty calls `wakeup_cb` thousands of times/second from I/O thread. cmux coalesces into a single `ghostty_app_tick()` dispatch on main queue. This is the same pattern PaneFlow needs: PTY reader on blocking thread → batch at 16ms intervals → send to frontend.
- `Sources/GhosttyTerminalView.swift` (lines 4608-4618) — Metal rendering: `makeBackingLayer()` creates `CAMetalLayer`. Ghostty writes to `IOSurface`-backed contents. PaneFlow replaces this entire GPU pipeline with xterm.js WebGL — but the data flow pattern (backend produces frames → frontend consumes) is the same.
- `Sources/TerminalWindowPortal.swift` — Portal geometry sync: `synchronizeAllEntriesFromExternalGeometryChange()` → `hostedView.reconcileGeometryNow()` → `refreshSurfaceNow()` tells Ghostty about new PTY dimensions. PaneFlow's equivalent: xterm.js `fit` addon → `resize_pty` Tauri command → PTY resize.
- `Sources/Panels/TerminalPanel.swift` (lines 137-156) — Focus/unfocus lifecycle: `focus()` → `surface.setFocus(true)` + `hostedView.ensureFocus()`, `close()` → portal detach → teardownSurface. Shows the cleanup contract.

**Acceptance Criteria:**
- [ ] `PtyBridge` spawns a `tokio::task::spawn_blocking` thread that reads from the PTY in a loop (4KB buffer)
- [ ] PTY output is batched at ~16ms intervals (one frame) before sending to the Tauri Channel to avoid overwhelming the WebView
- [ ] `TerminalEvent` enum: `Data { pane_id: Uuid, bytes: Vec<u8> }`, `Exit { pane_id: Uuid, code: i32 }`, `Resize { pane_id: Uuid, rows: u16, cols: u16 }`
- [ ] Tauri command `attach_pty(pane_id, on_event: Channel<TerminalEvent>)` registers a channel for a pane
- [ ] Tauri command `write_pty(pane_id, bytes: Vec<u8>)` sends input to the PTY
- [ ] Tauri command `resize_pty(pane_id, rows, cols)` resizes the PTY and terminal emulator
- [ ] Given a fast `cat largefile.txt`, the bridge applies backpressure (batches output) without dropping bytes
- [ ] Given a PTY exit, the `Exit` event is sent on the channel and the bridge task terminates cleanly
- [ ] Given `attach_pty` called with a non-existent pane_id, the command returns an error

---

### EP-003: Tauri UI Shell

Build the Tauri v2 application shell with SolidJS frontend, xterm.js terminal rendering, sidebar, and split pane layout.

**Definition of Done:** A Tauri window renders a sidebar with workspace tabs and a main area with xterm.js terminals in split panes. Users can create workspaces, split panes, resize dividers, and type in terminals.

#### US-007: Tauri + SolidJS Project Scaffold
**Description:** As a developer, I want a Tauri v2 + SolidJS project scaffold so that the frontend and backend build and run together.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/cmuxApp.swift` — App entry point. Study: `init()` sets up Ghostty environment, language, appearance; `body` creates `WindowGroup { ContentView }` with `.windowStyle(.hiddenTitleBar)` and injects state objects via `.environmentObject()`. PaneFlow's Tauri `main.rs` setup must mirror this: initialize PTY system, config, socket server, then create window.
- `Sources/ContentView.swift` (lines 1570+) — Root view layout: `[Sidebar] | [WorkspaceContentView(selected)]`. Study the `contentAndSidebarLayout` horizontal split. PaneFlow's SolidJS root component mirrors this: `<Sidebar /> | <WorkspaceView />`.
- `Sources/AppDelegate.swift` (lines 2433+) — `applicationDidFinishLaunching`: starts Sentry, PostHog, UpdateController, WindowToolbarController, installs ObjC swizzles. PaneFlow's Tauri `setup()` hook does the equivalent: start socket server, config watcher.
- `cmux.entitlements` — macOS entitlements (JIT, unsigned memory, camera/mic, AppleScript). Informs what Tauri permissions/capabilities PaneFlow needs.

**Acceptance Criteria:**
- [ ] `cargo tauri dev` starts the application with hot-reload on frontend changes
- [ ] `cargo tauri build` produces a distributable binary for the host platform
- [ ] Frontend uses SolidJS with TypeScript, Vite as bundler, pnpm as package manager
- [ ] Tauri config specifies window title "PaneFlow", minimum size 800x500
- [ ] CSP is configured to allow xterm.js WebGL rendering (no `unsafe-eval` unless required by xterm.js)
- [ ] Given `cargo tauri dev`, the window opens showing a placeholder "PaneFlow" text

#### US-008: xterm.js Terminal Pane with WebGL Rendering
**Description:** As a user, I want terminal panes rendered via xterm.js with WebGL so that terminal output displays with high performance and correct colors.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-006, US-007

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/Panels/TerminalPanelView.swift` — The SwiftUI wrapper for Ghostty's terminal view. Study: passes `terminalSurface`, `isActive`, `isVisibleInUI`, `showsInactiveOverlay` (darkens unfocused panes in splits), `showsUnreadNotificationRing`. PaneFlow's `<TerminalPane>` SolidJS component mirrors this — props include `paneId`, `isFocused`, `isVisible`.
- `Sources/GhosttyTerminalView.swift` (lines 4247+) — Keyboard input flow: `keyDown` → builds `ghostty_input_key_s` → `ghostty_surface_key()`. For IME: `ghostty_surface_preedit()` + `ghostty_surface_ime_point()`. PaneFlow replaces this with xterm.js `onData` callback → `write_pty` Tauri command.
- `Sources/GhosttyTerminalView.swift` (lines 6488-6507) — Mouse input: `ghostty_surface_mouse_pos/button/scroll/pressure`. xterm.js handles mouse natively, but study the Y-flip (`bounds.height - point.y`) pattern.
- `cmuxTests/CJKIMEInputTests.swift` — CJK IME contracts: `setMarkedText`, `insertText`, `unmarkText`, `performKeyEquivalent` returns false during composition. xterm.js handles IME natively but these tests define the expected behavior.
- `cmuxTests/InactivePaneFirstClickFocusTests.swift` — `acceptsFirstMouse` setting: click to focus inactive pane. PaneFlow's split pane click-to-focus must match.

**Acceptance Criteria:**
- [ ] `TerminalPane` SolidJS component creates an xterm.js `Terminal` instance with WebGL addon
- [ ] Component calls `attach_pty` Tauri command on mount and subscribes to `TerminalEvent` via Channel
- [ ] Keyboard input is captured by xterm.js and sent to PTY via `write_pty` Tauri command
- [ ] Terminal resize (component resize) triggers `resize_pty` with new rows/cols computed from xterm.js `fit` addon
- [ ] 256-color and true-color ANSI sequences display correctly
- [ ] Given typing `ls` + Enter in the terminal, file listing appears with correct colors
- [ ] Given a terminal pane resized smaller, the content reflows and no visual artifacts appear
- [ ] Given WebGL addon fails to initialize, falls back to canvas renderer with a console warning

#### US-009: Sidebar with Workspace List and Selection
**Description:** As a user, I want a sidebar showing my workspaces so that I can switch between them by clicking.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-001, US-007

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/SidebarSelectionState.swift` — Trivial `ObservableObject` holding `selection: SidebarSelection` enum (`.tabs`, `.notifications`). Shared via `@EnvironmentObject`. PaneFlow needs a similar store.
- `Sources/ContentView.swift` (lines 344+) — `SidebarState`: `isVisible: Bool`, `persistedWidth: CGFloat`, `toggle()`. Study the `contentAndSidebarLayout` horizontal split, `mountedWorkspaceIds` (keep-alive set for fast workspace switching — not all workspaces render simultaneously).
- `Sources/Workspace.swift` (lines 5604+) — `sidebarObservationPublisher`: lazy Combine publisher merging all sidebar-relevant `@Published` properties into one "something changed" signal. PaneFlow's SolidJS equivalent: a derived signal that triggers sidebar row re-renders.
- `cmuxTests/SidebarOrderingTests.swift` — Sidebar active colors (light: black opacity, dark: white opacity), branch layout settings (vertical default), active tab indicator styles, branch ordering (dedup by name, merge dirty state OR semantics).
- `cmuxTests/SidebarWidthPolicyTests.swift` — Width clamping: allows values below legacy minimum (184px).
- `cmuxTests/WorkspaceUnitTests.swift` — Selected workspace colors: `#0088FF` (light), `#0091FF` (dark), foreground always white.

**Acceptance Criteria:**
- [ ] Sidebar component displays an ordered list of workspace titles with the active workspace highlighted
- [ ] Clicking a workspace in the sidebar selects it and displays its split tree in the main area
- [ ] "+" button at the bottom creates a new workspace with a default terminal
- [ ] Right-click on a workspace shows a context menu with "Close" option
- [ ] Closing a workspace from the sidebar calls `TabManager::close_workspace` and selects the adjacent workspace
- [ ] Sidebar width is fixed at 220px for P0 (resizable deferred to P1)
- [ ] Given 0 workspaces (last one closed), a default workspace is auto-created
- [ ] Given 10 workspaces, the sidebar scrolls vertically

#### US-010: Split Pane Layout with Resizable Dividers
**Description:** As a user, I want to split terminal panes horizontally and vertically and resize them by dragging dividers.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** US-003, US-008

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/WorkspaceContentView.swift` (lines 224+) — `BonsplitView(controller: workspace.bonsplitController)` renders the split tree. The closure maps each Bonsplit tab to `PanelContentView`. Study: `isFocused`, `isSelectedInPane`, `isVisibleInUI` computation, `onFocus` callback routing to `workspace.focusPanel()`.
- `Sources/TerminalWindowPortal.swift` — Portal system for hosting AppKit views above SwiftUI. Study: `SplitDividerOverlayView` paints divider lines on top of portaled views, `WindowTerminalHostView.resetCursorRects` adds resize cursors over dividers, `hitTest` passes through to split dividers. PaneFlow replaces portals with CSS flex/grid in SolidJS, but the divider rendering and hit-testing logic is the same concept.
- `Sources/Workspace.swift` — `newTerminalSurface(inPane:focus:)` (line 7652): creates panel + bonsplit tab + mapping. `focusPanel()` (line 8677): finds bonsplit TabID → focusPane + selectTab + AppKit firstResponder. `closePanel()` (line 8021): removes from panels dict + closes bonsplit tab.
- `cmuxTests/WorkspaceContentViewVisibilityTests.swift` — Panel visibility rules: background workspace stays mounted but invisible (opacity 0.001), `panelVisibleInUI` is false when workspace hidden.
- `cmuxUITests/BonsplitTabDragUITests.swift` — Tab reorder via drag, tab bar positioning, sidebar row layout, minimal mode controls behavior.

**Acceptance Criteria:**
- [ ] Main area renders the active workspace's `SplitTree` as nested panes with dividers
- [ ] Keyboard shortcut `Ctrl+Shift+D` splits the focused pane vertically (right)
- [ ] Keyboard shortcut `Ctrl+Shift+Shift+D` (or configurable) splits horizontally (down)
- [ ] Dividers are draggable and update the split ratio in real-time
- [ ] Divider drag is clamped so neither pane goes below 80px
- [ ] Clicking a pane focuses it (highlighted border or subtle indicator)
- [ ] Closing a terminal (shell exit or Ctrl+D on empty) removes the pane and collapses the split
- [ ] Given a single pane, splitting creates two equal panes with a draggable divider between them
- [ ] Given a 3-deep split tree, all panes render at correct sizes and dividers work independently
- [ ] Given closing the last pane in a workspace, the workspace itself closes (and adjacent workspace is selected)

#### US-011: Keyboard Shortcut Framework
**Description:** As a user, I want configurable keyboard shortcuts for common actions (new workspace, split, close, navigate) so that I can work without touching the mouse.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-007

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/KeyboardShortcutSettings.swift` — Full shortcut registry. Study: `Action` enum (34 named actions across 4 categories: titlebar/UI, navigation, panes/splits, panels), `StoredShortcut` Codable struct (`key`, `command`, `shift`, `option`, `control`), persistence in `UserDefaults`, change notification `"cmux.keyboardShortcutSettingsDidChange"`, numbered digit matching for `selectSurfaceByNumber` / `selectWorkspaceByNumber`.
- `Sources/KeyboardLayout.swift` — Physical key translation for non-Latin IMEs: `UCKeyTranslate` via Carbon TIS API. PaneFlow must handle non-Latin keyboards (xkbcommon on Linux).
- `cmuxTests/ShortcutAndCommandPaletteTests.swift` — `SplitShortcutTransientFocusGuard` (suppresses when first responder is fallback + view is tiny), fullscreen shortcut matching (`Cmd+Ctrl+F`: exact modifiers, CapsLock tolerance, layout-aware), `CommandPaletteKeyboardNavigation`.
- `cmuxTests/WorkspaceUnitTests.swift` — Default shortcut bindings: `Cmd+R` rename tab, `Cmd+Ctrl+W` close window, `Cmd+Shift+R` rename workspace, `Cmd+Shift+M` toggle copy mode. All `defaultsKey` values are unique.
- `Sources/AppDelegate.swift` — `installShortcutMonitor`: intercepts global shortcuts for workspace navigation. Study the ObjC swizzle on `NSWindow.performKeyEquivalent`.

**Acceptance Criteria:**
- [ ] `ShortcutRegistry` Rust struct maps `(modifiers, key)` tuples to `Action` enum values
- [ ] Default shortcuts: `Ctrl+Shift+T` (new workspace), `Ctrl+Shift+W` (close workspace), `Ctrl+Shift+D` (split right), `Ctrl+Tab` / `Ctrl+Shift+Tab` (next/prev workspace), `Ctrl+Shift+H/J/K/L` (focus pane left/down/up/right)
- [ ] Shortcuts are intercepted at the Tauri level before reaching xterm.js (so terminal doesn't consume them)
- [ ] Shortcut registry is loaded from config file; missing keys use defaults
- [ ] Given a user presses `Ctrl+Shift+T`, a new workspace is created and selected
- [ ] Given a user presses a shortcut that conflicts with a terminal application (e.g., Ctrl+C), the terminal receives the input (shortcuts only use Ctrl+Shift or other non-conflicting combos)
- [ ] Given an invalid shortcut definition in config, a warning is logged and the default is used

---

### EP-004: Socket IPC Server

Implement the V2 JSON-RPC socket server that enables CLI and AI agent programmatic control.

**Definition of Done:** A Unix domain socket (or named pipe on Windows) accepts connections, parses JSON-RPC requests, dispatches to handlers, and returns JSON-RPC responses for system, workspace, and surface methods.

#### US-012: Socket Server Framework
**Description:** As a developer, I want a tokio-based socket server that listens on a Unix domain socket (or Windows named pipe) and handles concurrent connections.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/TerminalController.swift` (lines 18-84) — `@MainActor` singleton wrapping POSIX Unix socket server. Study: `accept()` loop on background thread (one `Thread` per connection), backlog 128, accept failure recovery (immediate retry → exponential backoff 10ms base, 5000ms max → rearm/recreate socket).
- `Sources/SocketControlSettings.swift` (lines 423-462) — Socket path resolution priority: tagged debug → env override → stable default (`~/Library/Application Support/cmux/cmux.sock`) → user-scoped fallback → legacy. Per-build variants (production, nightly, staging, debug, tagged dev). Last-socket-path file written at `~/Library/Application Support/cmux/last-socket-path`.
- `Sources/SocketControlSettings.swift` (lines 7-61) — Access control modes: `off`, `cmuxOnly` (0o600 + ppid ancestry via `getsockopt(LOCAL_PEERPID)`), `automation` (0o600, no auth), `password` (0o600 + password file), `allowAll` (0o666). PaneFlow P0 implements `automation` mode equivalent (0o600, same user).
- `cmuxTests/TerminalControllerSocketSecurityTests.swift` — Socket permission tests (`allowAll` → 0666, `cmuxOnly` → 0600), password rejection, focus-allowing vs non-focus command policy.
- `Sources/TerminalController.swift` — `SocketListenerHealth`: tracks `isRunning`, `acceptLoopAlive`, `socketPathMatches`, `socketPathExists`.

**Acceptance Criteria:**
- [ ] Server listens on `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` (Linux), `~/Library/Application Support/paneflow/paneflow.sock` (macOS), `\\.\pipe\paneflow` (Windows)
- [ ] Socket file has permissions `0o600` (owner-only)
- [ ] Server accepts multiple concurrent connections via `tokio::net::UnixListener`
- [ ] Each connection gets its own tokio task; one slow client does not block others
- [ ] Server writes the socket path to `$XDG_RUNTIME_DIR/paneflow/last-socket-path` (for CLI discovery)
- [ ] On startup, if the socket file already exists (stale from crash), it is unlinked before binding
- [ ] Given the server is running, `echo '{}' | socat - UNIX-CONNECT:/path/to/socket` connects successfully
- [ ] Given the server shuts down, the socket file is cleaned up

#### US-013: JSON-RPC Protocol Dispatcher
**Description:** As a developer, I want a JSON-RPC 2.0 dispatcher that routes method calls to handlers and returns structured responses.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-012

**cmux Reference** (`/home/arthur/dev/cmux`):
- `CLI/cmux.swift` (lines 842-1152) — `SocketClient`: raw Unix socket client with `sendV2()` for JSON-RPC. Study: protocol detection (first byte `{` → V2, else V1), request format `{"id":"<uuid>","method":"...","params":{}}`, response format `{"id":"...","ok":true,"result":{}}` or `{"ok":false,"error":{"code":"...","message":"..."}}`.
- `Sources/TerminalController.swift` — V2 dispatcher: reads newline-delimited JSON, routes by `method` string. Study the notification names (`socketListenerDidStart`, `terminalSurfaceDidBecomeReady`, etc.) and the health monitoring (`SocketListenerHealth`).
- `docs/v2-api-migration.md` — V1→V2 migration spec. Key: V1 is space-delimited text (`OK ...` / `ERROR: ...`), V2 is JSON-RPC. Both run on same socket, detected per-connection. PaneFlow only needs V2 for P0.
- `CLI/cmux.swift` (lines 531-654) — `SocketPasswordResolver`: auth layers (flag → env → file → keychain). PaneFlow P0 skips password auth but should plan the extension point.

**Acceptance Criteria:**
- [ ] Requests are newline-delimited JSON: `{"id": "<uuid>", "method": "...", "params": {...}}\n`
- [ ] Responses are newline-delimited JSON: `{"id": "<uuid>", "ok": true, "result": {...}}\n` or `{"id": "<uuid>", "ok": false, "error": {"code": "...", "message": "..."}}\n`
- [ ] Dispatcher routes by `method` string to registered handler functions
- [ ] Unknown methods return error `{"code": "method_not_found", "message": "Unknown method: ..."}`
- [ ] Malformed JSON returns error `{"code": "parse_error", "message": "..."}`
- [ ] Given a valid `system.ping` request, the server returns `{"ok": true, "result": {"pong": true}}`
- [ ] Given two requests on the same connection, both are processed independently
- [ ] Given a request with missing `id` field, the server returns an error with `id: null`

#### US-014: Core Method Handlers (system, workspace, surface)
**Description:** As an AI agent developer, I want core JSON-RPC methods for workspace and surface management so that agents can programmatically control PaneFlow.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** US-001, US-002, US-003, US-013

**cmux Reference** (`/home/arthur/dev/cmux`):
- `CLI/cmux.swift` (lines 1539-2395) — Complete command inventory (80+ commands). For P0, study these V2 method implementations: `system.capabilities`, `system.identify`, `workspace.list`, `workspace.create`, `workspace.select`, `workspace.close`, `workspace.current`, `surface.list` (`pane.surfaces`), `surface.split`, `surface.close`, `surface.send_text` (`send`), `surface.focus`.
- `CMUX_ANALYSIS.md` Section 8 — Socket Protocol Reference: method families, handle format (`window:1`, `workspace:3` — short refs deferred to P1), security modes.
- `docs/socket-focus-steal-audit.todo.md` — No-focus-steal policy: socket commands must NOT activate the app or change user focus except for explicit focus-intent commands. Lists which commands allow focus vs don't. Critical for agent workflows.
- `cmuxTests/TerminalControllerSocketSecurityTests.swift` — `workspace.close` rejects pinned workspaces with `protected` error code including `pinned: true` data. `notification.create` with explicit `surface_id` routes to that surface. Remote status payload omits sensitive SSH fields.
- `daemon/remote/cli.go` (lines 50-78) — Remote CLI relay command table: 22 commands across V1/V2. Shows which methods are essential for remote agent workflows.

**Acceptance Criteria:**
- [ ] `system.ping` returns `{"pong": true}`
- [ ] `system.capabilities` returns `{"version": "0.1.0", "protocol": "v2", "methods": [...]}`
- [ ] `system.identify` returns the focused workspace_id, pane_id, and surface_id
- [ ] `workspace.list` returns all workspaces with id, title, working_directory
- [ ] `workspace.create` creates a new workspace (optional: name, cwd, command) and returns its id
- [ ] `workspace.select` switches to a workspace by id
- [ ] `workspace.close` closes a workspace by id; returns `{"code": "protected"}` error if it's the only workspace
- [ ] `workspace.current` returns the currently selected workspace
- [ ] `surface.list` returns all surfaces in a workspace with id, type, title
- [ ] `surface.split` splits a surface (params: surface_id, direction) and returns the new surface id
- [ ] `surface.close` closes a surface by id
- [ ] `surface.send_text` sends text to a terminal surface (params: surface_id, text)
- [ ] `surface.focus` focuses a surface by id
- [ ] Handle references use UUIDs (short refs deferred to P1)
- [ ] Given `workspace.create` with `{"cwd": "/tmp"}`, the new workspace's terminal starts in `/tmp`
- [ ] Given `surface.send_text` to a non-existent surface_id, returns `{"code": "not_found"}` error

---

### EP-005: CLI Binary

Build the `paneflow` CLI that communicates with the running app via the socket.

**Definition of Done:** `paneflow ping`, `paneflow list-workspaces`, `paneflow new-workspace`, `paneflow send` all work from any terminal.

#### US-015: CLI Scaffold with Socket Discovery
**Description:** As a developer, I want a CLI binary with clap that discovers the PaneFlow socket and sends requests.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** US-012

**cmux Reference** (`/home/arthur/dev/cmux`):
- `CLI/cmux.swift` (lines 700-724) — `CLISocketPathResolver`: socket discovery order (tagged → env → stable → legacy → dynamic discovery of tagged sockets by mtime). Study the priority chain and how each candidate is tested for connectivity before use.
- `CLI/cmux.swift` (top-level) — Global flags: `--socket <path>`, `--json`, `--id-format refs|uuids|both`, `--window <id>`, `--password <value>`, `-v`/`--version`, `-h`/`--help`. PaneFlow P0 needs: `--socket`, `--json`, `--version`, `--help`.
- `CLI/cmux.swift` (lines 842-1152) — `SocketClient`: connect with ownership verification (rejects sockets not owned by current user), 15s receive timeout, 120ms idle timeout for multi-line V1 responses, `DispatchSource`-based readiness watching.
- `CLI/cmux.swift` (lines 19-293) — `CLISocketSentryTelemetry`: optional error reporting for CLI failures. Shows the diagnostic info pattern (bundle ID, version, socket diagnostics). PaneFlow should log similar diagnostics on connection failure.

**Acceptance Criteria:**
- [ ] `paneflow` binary is built as a separate Rust binary crate in the workspace
- [ ] Global flags: `--socket <path>` (override), `--json` (JSON output), `-v`/`--version`, `-h`/`--help`
- [ ] Socket discovery order: `PANEFLOW_SOCKET_PATH` env → `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` → `last-socket-path` file → platform-specific defaults
- [ ] `SocketClient` struct connects to the socket, sends JSON-RPC request, reads response with 15s timeout
- [ ] Given `paneflow --version`, prints version string
- [ ] Given PaneFlow is not running (no socket), `paneflow ping` prints a clear error: "PaneFlow is not running (socket not found at ...)"

#### US-016: Core CLI Commands
**Description:** As a user, I want CLI commands for workspace and surface management so that I can script PaneFlow from any terminal.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-014, US-015

**cmux Reference** (`/home/arthur/dev/cmux`):
- `CLI/cmux.swift` (lines 1539-2395) — Full command implementations. For each P0 command, study the argument parsing, V2 method name, params construction, and output formatting. Key commands: `ping` (simplest, good starter), `list-workspaces` (table formatting with `--json`), `new-workspace` (`--name`, `--cwd`, `--command` flags), `send` (`--surface`, `--text`), `new-split` (`--surface`, `--direction`).
- `CLI/cmux.swift` — Output formatting: plain text (default) shows a human-readable table; `--json` shows raw JSON-RPC response. Study how `list-workspaces` formats columns.
- `skills/cmux/SKILL.md` — Skill docs for cmux CLI: topology control examples (windows, workspaces, panes, surfaces, focus, move, reorder, flash, identify). Shows the UX expectations for CLI commands.
- `tests_v2/` — V2 integration tests using Python client `tests_v2/cmux.py`. Study test patterns: connect to socket, send JSON-RPC, assert response fields. PaneFlow's CLI integration tests should follow similar patterns.

**Acceptance Criteria:**
- [ ] `paneflow ping` sends `system.ping` and prints "pong"
- [ ] `paneflow list-workspaces` sends `workspace.list` and prints a formatted table (id, title, cwd)
- [ ] `paneflow new-workspace [--name NAME] [--cwd DIR] [--command CMD]` sends `workspace.create`
- [ ] `paneflow select-workspace <id>` sends `workspace.select`
- [ ] `paneflow close-workspace <id>` sends `workspace.close`
- [ ] `paneflow send --surface <id> --text "hello"` sends `surface.send_text`
- [ ] `paneflow new-split [--surface <id>] [--direction right|down]` sends `surface.split`
- [ ] `paneflow list-surfaces [--workspace <id>]` sends `surface.list`
- [ ] `paneflow identify` sends `system.identify` and prints focused context
- [ ] All commands support `--json` flag for machine-readable JSON output
- [ ] Given `paneflow new-workspace --name "Agent 1" --cwd /tmp`, a workspace named "Agent 1" is created

---

### EP-006: Configuration System

Implement JSON config file loading with validation and hot-reload.

**Definition of Done:** PaneFlow reads `~/.config/paneflow/paneflow.json` on startup, validates it, applies settings, and auto-reloads on file changes.

#### US-017: JSON Config Loader with Validation
**Description:** As a user, I want PaneFlow to read a JSON config file so that I can customize keyboard shortcuts, default shell, and workspace commands.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/CmuxConfig.swift` (lines 5-257) — Full config schema. Study: `CmuxConfigFile` top-level (`commands` array), `CmuxCommandDefinition` (name, description, keywords, restart behavior, mutually exclusive workspace/command), `WorkspaceDefinition` (name, cwd, color as 6-digit hex, layout), `LayoutNode` (recursive pane/split), `SurfaceDefinition` (type: terminal/browser, name, command, cwd, env, url, focus).
- `Sources/CmuxConfig.swift` (lines 337-400) — Config loading: global path `~/.config/cmux/cmux.json` (always loaded/trusted) + local path (searched upward from CWD). Merge: local commands shadow global by name (first-seen wins). Parse errors swallowed with `NSLog`.
- `Sources/CmuxConfig.swift` (lines 603-617) — `resolveCwd`: nil/empty → base, absolute → passthrough, relative → join to baseCwd, `~` → home dir expansion.
- `Sources/CmuxConfigExecutor.swift` — Runtime execution: workspace commands resolve CWD, create workspace tab, apply title+color, apply custom layout. Shell commands sanitize Unicode direction-override characters (13 bidirectional scalars stripped).
- `Sources/CmuxDirectoryTrust.swift` — Trust model: trust key is git repo root, global config always trusted, trust store at `~/Library/Application Support/cmux/trusted-directories.json`.
- `cmuxTests/CmuxConfigTests.swift` — Comprehensive validation tests: blank names, blank commands, workspace+command co-occurrence, missing both, invalid hex colors, split != 2 children, pane 0 surfaces, split clamping, CWD resolution, encoding round-trip.
- `CMUX_ANALYSIS.md` Section 9 — Config schema reference with full JSON example.

**Acceptance Criteria:**
- [ ] Config path: `$XDG_CONFIG_HOME/paneflow/paneflow.json` (Linux), `~/Library/Application Support/paneflow/paneflow.json` (macOS), `%APPDATA%\paneflow\paneflow.json` (Windows)
- [ ] Config schema supports: `shortcuts` (key-action map), `default_shell` (path), `commands` (array of workspace command definitions, cmux-compatible format)
- [ ] `commands` array follows cmux schema: each entry has `name`, optional `description`, `keywords`, `workspace` (with `name`, `cwd`, `color`, `layout`) or `command` (shell string)
- [ ] Validation: blank names rejected, split must have exactly 2 children, pane must have >= 1 surface, split ratio clamped to [0.1, 0.9]
- [ ] If config file doesn't exist, PaneFlow starts with defaults (no error)
- [ ] If config file has invalid JSON, PaneFlow logs a warning and starts with defaults
- [ ] Given a config with `"default_shell": "/bin/zsh"`, new terminals use zsh
- [ ] Given a config with invalid `"commands"` entry (blank name), the entry is skipped with a warning log

#### US-018: Hot-Reload via File Watcher
**Description:** As a user, I want PaneFlow to automatically reload my config when I save changes so that I don't have to restart the app.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** US-017

**cmux Reference** (`/home/arthur/dev/cmux`):
- `Sources/CmuxConfig.swift` (lines 420-491) — File watcher implementation. Study: `DispatchSource.makeFileSystemObjectSource` on dedicated `com.cmux.config-file-watch` serial queue, event handling (`.write`/`.extend` → reload immediately, `.delete`/`.rename` → stop watcher + reload + schedule re-attach with exponential backoff), directory fallback watcher (if file doesn't exist yet, watch parent dir for creation), re-attach policy (max 5 attempts, 0.5s delay).
- `Sources/CmuxConfig.swift` (lines 294-317) — `wireDirectoryTracking()`: subscribes to active workspace's `currentDirectory` via Combine, updating local config path on workspace switch. Makes local config directory-aware.
- `Sources/Panels/MarkdownPanel.swift` (lines 106-178) — Another file watcher example: opens with `O_EVTONLY`, `DispatchSource` on utility queue, handles `.delete`/`.rename` with `scheduleReattach` (6 retries × 0.5s for atomic saves), `isClosed` flag prevents reattach after close. Same pattern, cleaner implementation.
- `Sources/CmuxConfig.swift` (lines 278-279) — Re-attach constants: `maxReattachAttempts = 5`, `reattachDelay = 0.5`. PaneFlow should use similar values for the `notify` debouncer.

**Acceptance Criteria:**
- [ ] File watcher uses `notify` crate with `RecommendedWatcher` (inotify on Linux, kqueue on macOS, ReadDirectoryChangesW on Windows)
- [ ] File events are debounced at 300ms to handle atomic saves (editor write+rename)
- [ ] On successful reload, a Tauri event `config-reloaded` is emitted to all windows
- [ ] On reload with validation errors, the old config is kept and a warning is logged
- [ ] If the config file is deleted, the watcher continues monitoring the directory for recreation
- [ ] Given editing `paneflow.json` to add a new shortcut and saving, the shortcut is active within 1 second without restarting
- [ ] Given the config file is replaced by an editor (delete + create), the reload still fires correctly

---

## Functional Requirements

- FR-01: The system must spawn terminal processes via platform-native PTY (Unix openpty, Windows ConPTY)
- FR-02: The system must parse VT100/xterm escape sequences and render them via xterm.js WebGL
- FR-03: The system must support N workspaces per window, each with an independent binary-tree split layout
- FR-04: When a user splits a pane, the system must create a new terminal with the same working directory as the source pane
- FR-05: The system must expose a JSON-RPC socket server with methods for workspace and surface CRUD
- FR-06: The system must load configuration from a JSON file and apply changes on file modification without restart
- FR-07: The system must provide a CLI binary that discovers the socket and sends commands
- FR-08: The system must NOT activate the app window or steal focus when processing socket commands (no-focus-steal policy)

## Non-Functional Requirements

- **Performance:** Terminal rendering at 60fps (< 16ms frame time) for typical workloads; PTY-to-frontend latency < 50ms for single-line output
- **Memory:** Base memory usage < 80MB with 1 workspace and 1 terminal; < 15MB per additional terminal pane
- **Startup:** Application window visible in < 2 seconds on a modern machine (SSD, 8GB RAM)
- **Platform:** Linux x86_64 + aarch64, Windows 10 1809+ (x86_64), macOS 12+ (arm64 + x86_64)
- **Binary size:** Distributable binary < 50MB (excluding debug symbols)
- **Socket:** IPC response latency < 10ms for read-only queries (system.ping, workspace.list)
- **Config reload:** Changes applied within 1 second of file save

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | First launch, no config | Config file doesn't exist | Start with defaults, single workspace | — |
| 2 | PTY spawn failure | Invalid shell path, permissions error | Show error in pane area, don't crash | "Failed to start shell: {error}. Check your default_shell config." |
| 3 | Socket bind failure | Another PaneFlow instance running, or stale socket | Try to clean stale socket; if still fails, start without socket | "Socket server disabled: {path} is already in use" |
| 4 | Corrupted config file | Invalid JSON syntax | Keep previous config, log warning | Console log: "Config reload failed: {parse error} — keeping previous config" |
| 5 | All workspaces closed | User closes every workspace | Auto-create a default workspace | — |
| 6 | Terminal resize to tiny | Window resized to minimum | Clamp terminal to minimum 2 cols x 1 row | — |
| 7 | Concurrent socket clients | Multiple CLI commands at once | Each request processed independently, no corruption | — |
| 8 | PTY output flood | `cat /dev/urandom` or large file dump | Backpressure via 16ms batching, no OOM | — |
| 9 | Config file deleted during watch | User deletes config file | Continue with current config, watch directory for recreation | — |
| 10 | Socket client disconnects mid-request | CLI killed with SIGKILL | Server cleans up connection, no resource leak | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | xterm.js WebGL performance insufficient in Tauri WebView | Medium | High | Validate in US-008; fallback to canvas renderer; if both insufficient, evaluate native rendering (P2) |
| 2 | Windows ConPTY flag handling causes artifacts | Medium | Medium | Gate flags on OS version detection in US-004; test on Windows 10 and 11 |
| 3 | Tauri v2 IPC latency on Windows (~200ms for large payloads) | Low | Medium | PTY batching at 4KB chunks mitigates; monitor upstream Tauri fixes |
| 4 | alacritty_terminal API instability (not a public API) | Medium | High | Pin to specific git commit; wrap in an abstraction layer so internals can be swapped |
| 5 | SolidJS + Tauri integration edge cases | Low | Low | TUICommander precedent validates the combo; maintain escape hatch to React |
| 6 | Socket hijacking on shared systems | Low | High | Use $XDG_RUNTIME_DIR (per-user tmpfs on Linux), 0600 permissions, verify SO_PEERCRED |

## Non-Goals

Explicit boundaries for P0 — what this version does NOT include:

- **Browser panels** — No embedded browser/WebView panes (P1 scope)
- **Session persistence** — No save/restore of workspace state across restarts (P1 scope)
- **Notifications** — No OSC bell detection, no notification system (P1 scope)
- **SSH/Remote workspaces** — No remote daemon, no SSH integration (P2 scope)
- **Multi-window** — Single window only; multi-window support deferred to P1
- **Command palette** — No fuzzy-search command palette UI (P1 scope)
- **Copy mode** — No vi-style keyboard scrollback navigation (P1 scope)
- **Find-in-terminal** — No Ctrl+F search in terminal output (P1 scope)
- **Drag-and-drop** — No tab drag reordering (P1 scope)
- **Agent hooks** — No Claude Code/Codex notification hooks (P2 scope)
- **tmux compatibility shim** — No tmux command translation layer (P2 scope)
- **Short refs** — Handle format is UUIDs only; short refs (`workspace:1`) deferred to P1

## Technical Considerations

Frame as questions for engineering input — not mandates:

- **Terminal emulation crate:** Recommended: `alacritty_terminal` (battle-tested, used by Alacritty). Risk: it's not a stable public API — pin to a specific git commit. Alternative: `wezterm-term` (WezTerm's crate). Engineering to evaluate API ergonomics.
- **PTY crate:** Recommended: `portable_pty` from WezTerm. No viable alternative for cross-platform PTY. Engineering to validate ConPTY flag handling on Windows 10 vs 11.
- **Frontend framework:** Recommended: SolidJS. Alternative: React (larger ecosystem but heavier). Engineering to confirm SolidJS + Tauri v2 integration maturity.
- **Async runtime:** tokio is the only viable choice for the socket server + PTY bridge. Question: should the entire Rust backend use tokio, or only the socket/PTY subsystems?
- **JSON-RPC library:** Recommended: hand-rolled dispatcher (simpler for local IPC). Alternative: `jsonrpsee` (more features, but heavier). Engineering to decide based on method count growth projections.
- **Monorepo structure:** Recommended Cargo workspace: `paneflow-core` (domain model), `paneflow-terminal` (PTY + emulation), `paneflow-ipc` (socket server), `paneflow-cli` (CLI binary), `paneflow-app` (Tauri app binary). Engineering to confirm.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Functional terminal with splits on Linux | N/A (new) | Working P0 build | Month-1 | Manual testing: spawn shell, run vim, split panes |
| Socket API methods implemented | 0 | 15 core methods | Month-1 | `paneflow capabilities` reports method list |
| CLI commands working | 0 | 10 commands | Month-1 | `paneflow help` shows all commands, each works |
| Terminal rendering frame time | N/A | < 16ms p95 | Month-1 | xterm.js performance metrics in DevTools |
| Windows build functional | N/A | PTY + basic terminal | Month-2 | Manual testing on Windows 10/11 |
| Config hot-reload working | N/A | < 1s reload latency | Month-1 | Edit config, observe change in < 1s |

## Open Questions

- **alacritty_terminal API stability:** Should we pin to a specific commit or fork the crate? The API is not versioned as public. — Engineering to decide after evaluating the API surface needed for P0.
- **xterm.js WebGL in Tauri:** Has anyone benchmarked xterm.js WebGL specifically inside a Tauri v2 WebView on Linux? — Validate early in US-008.
- **Windows named pipe discovery:** The cmux CLI uses socket file discovery via last-socket-path. On Windows with named pipes, should we use a registry key, a file, or a fixed pipe name? — Engineering to decide in US-012.
- **tokio runtime scope:** Should the Tauri app's main thread use `#[tokio::main]` or should we spawn a separate tokio runtime for the socket server only? — Engineering to decide based on Tauri v2's async model.
[/PRD]
