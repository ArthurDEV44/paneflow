[PRD]
# PRD: PaneFlow v2 — Full Native Rust Terminal Multiplexer

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-03 | Arthur (via Claude) | Initial draft — full native rewrite replacing Tauri+xterm.js |

## Problem Statement

1. **PaneFlow v1 (Tauri+xterm.js) has an architectural latency ceiling of ~13ms.** The Tauri IPC boundary (JSON serialization), xterm.js `requestAnimationFrame` gate (up to 16.6ms), and base64 encode/decode pipeline create irreducible overhead. cmux achieves ~2-3ms with its portal architecture (Metal above SwiftUI) and zero-IPC keystroke path. PaneFlow v1 cannot match this within the WebView paradigm.

2. **The xterm.js rendering pipeline cannot be bypassed from within Tauri.** Native view overlay on top of Tauri's WebView is an unsolved problem (Tauri Discussion #11944, unresolved as of Aug 2025). WebView2 on Windows doesn't support HTTP streaming via custom protocols. The fundamental constraint is that all terminal data must cross the process boundary and be consumed by JavaScript.

3. **PaneFlow v1 has critical implementation gaps.** The ZerolagInputAddon is loaded but never wired (`addChar`/`removeChar`/`clear` never called). PTY processes are destroyed on every workspace switch (`<Show>` unmounts all TerminalPane components). The `alacritty_terminal` emulator is instantiated but never fed output on the hot path. The TerminalPanel domain object is decoupled from PtyBridge.

**Why now:** The AI agent terminal space is accelerating (cmux, AMUX, Conductor, Claude Code Agent Teams — all Q1 2026). cmux's typing-latency-architecture has been fully documented (5 layers, 12 design principles) providing a complete specification to implement. The Rust native GUI ecosystem reached production maturity in December 2025 (COSMIC desktop shipping on iced 0.14). Zed and Lapce prove that `alacritty_terminal` + WGPU is a viable, performant stack.

## Overview

PaneFlow v2 is a complete rewrite of the rendering and UI layer, replacing Tauri v2 + SolidJS + xterm.js with a full native Rust application using winit + iced 0.14 + WGPU + alacritty_terminal. The core Rust crates (`paneflow-core`, `paneflow-ipc`, `paneflow-config`, `paneflow-cli`) are preserved and adapted. The Tauri app shell (`src-tauri/`) and web frontend (`frontend/`) are replaced entirely.

Key architectural changes:
- **Zero-IPC keystroke path:** winit keyboard event → direct PTY `write_all()` in the same Rust process. No JSON serialization, no WebView boundary, no JavaScript. Matches cmux's "keyboard events never enter the UI framework" principle.
- **GPU terminal rendering:** Custom WGPU renderer reads `alacritty_terminal` grid cells and paints via instanced glyph draws from a bin-packed texture atlas. Dirty-cell tracking ensures only changed cells trigger GPU upload. Matches Zed's `TerminalElement` pattern.
- **iced for UI chrome only:** The sidebar, command palette, tab bars, and notifications are iced widgets. Terminal panes are custom WGPU `Canvas` elements within the iced layout tree. iced never re-renders during typing because terminal rendering is a custom paint operation, not an iced widget tree diff.
- **Demand-driven rendering:** No app-level display link or polling loop. PTY output triggers a render request. WGPU presents at the display's native refresh rate. Matches cmux's `wakeup_cb → scheduleTick → ghostty_app_tick` pattern.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| Typing latency (keystroke to pixel) | < 8ms P95 on 60Hz display | < 5ms P95 on 120Hz display |
| Terminal rendering throughput | 60 FPS sustained during `cat /dev/urandom \| xxd` | 120 FPS on high-refresh displays |
| Cross-platform support | Linux (X11+Wayland) primary, macOS secondary | Linux + macOS + Windows |
| Socket API coverage | 15 core methods (system.*, workspace.*, surface.*) | 40+ methods (cmux V2 subset) |
| Memory per terminal pane | < 15 MB (4000-line scrollback) | < 10 MB optimized |
| Cold start to first terminal ready | < 500ms | < 300ms |

## Target Users

### AI Agent Developer (Primary)
- **Role:** Developer running 2-8 AI coding agents (Claude Code, Codex, OpenCode) in parallel
- **Behaviors:** Uses tmux or multiple terminal windows today; monitors agent output across sessions; sends commands to agents via CLI; needs sub-10ms typing latency to feel "native"
- **Pain points:** cmux requires macOS; Tauri-based PaneFlow v1 has perceptible input lag (~15ms); no cross-platform terminal multiplexer offers both agent IPC and native-level latency
- **Current workaround:** macOS-only cmux, or tmux with manual multi-pane management
- **Success looks like:** Launch PaneFlow on Linux, create 4 workspaces for 4 agents via socket API, type in any terminal with zero perceptible lag, observe all agent activity in sidebar

### Terminal Power User (Secondary)
- **Role:** Developer who uses tmux/Zellij daily on Linux and wants a more modern, GPU-accelerated multiplexer
- **Behaviors:** Heavy keyboard user, customizes keybindings, relies on split panes and named sessions
- **Pain points:** tmux is TUI-only; WezTerm has ~26ms latency; Zellij is not agent-oriented; no GPU-rendered multiplexer with rich sidebar on Linux
- **Current workaround:** tmux + tmuxinator, or Zellij with KDL configs
- **Success looks like:** Switch from tmux to PaneFlow with splits, keybindings, and CLI all working; feel the latency improvement immediately

## Research Findings

Key findings from the 8-agent deep audit that informed this PRD:

### Competitive Context
- **cmux (macOS):** ~2-3ms latency via 5-layer architecture (event routing bypass, portal Metal rendering, demand-driven Ghostty, SwiftUI re-render prevention, main thread protection). 150+ socket commands. AGPL-3.0.
- **Zed Editor (cross-platform):** Proves `alacritty_terminal` + GPUI + WGPU works at production scale. Split panes via `PaneGroup` binary tree. 120 FPS target. 4ms PTY event batching.
- **Alacritty (cross-platform):** ~7ms latency. Proves `alacritty_terminal` crate as a standalone VT library. No multiplexer features.
- **WezTerm (cross-platform):** ~26ms latency despite WGPU. Multi-backend overhead. Lua config.
- **Market gap:** No cross-platform terminal multiplexer achieves < 10ms latency AND provides agent-oriented socket IPC AND has a rich GUI sidebar.

### Best Practices Applied (from cmux typing-latency-architecture.md)
- Keyboard events bypass the UI framework entirely (cmux Layer 1: performKeyEquivalent swizzle)
- Terminal rendering is physically separated from UI framework rendering (cmux Layer 2: portal architecture)
- Rendering is demand-driven, not polled (cmux Layer 3: wakeup_cb → scheduleTick)
- Hot path has zero allocations (cmux Layer 3: forceRefresh allocation-free contract)
- All background I/O is off-main and coalesced (cmux Layer 5: SocketFastPathState, PortScanner 200ms coalesce)
- Debug instrumentation compiles to zero in release (cmux Layer 8: `#[cfg(debug_assertions)]` pattern)

### Technical Validation
- **iced 0.14:** Shipped COSMIC desktop (Pop!_OS 24.04 LTS, Dec 2025). WGPU backend. Reactive rendering reduces CPU 60-80% on static UIs. `Canvas` widget supports custom WGPU drawing.
- **alacritty_terminal 0.26.0-rc1:** Pre-release but stable API. Used by Zed (pinned rev) and Lapce (pinned rev). Provides `Term<EventListener>` grid, VTE/ANSI parser, scrollback, selection, reflow.
- **WGPU:** Cross-platform GPU abstraction (Metal/Vulkan/DX12/OpenGL). Used by iced, COSMIC, and WezTerm. Avoids platform-specific shader languages.
- **Tauri IPC ceiling:** `invoke()` ~3-7ms, `emit()` ~1-2ms, `Channel<T>` ~1ms. All JSON-serialized. Cannot go below ~1ms due to process boundary. (Sources: Tauri Discussion #5690, #7146)

*Full research from 8 agent reports available in project documentation.*

## Assumptions & Constraints

### Assumptions (to validate)
- iced 0.14's `Canvas` widget can host a custom WGPU terminal renderer without fighting iced's own WGPU context — based on COSMIC desktop using custom drawing, but terminal-specific use case is unvalidated
- `alacritty_terminal`'s `Term` grid provides sufficient cell-level access for custom rendering — based on Zed's proven usage, high confidence
- `cosmic-text` or `swash` can shape terminal fonts with sub-millisecond latency — based on COSMIC desktop font stack, medium confidence
- `portable-pty` 0.9 handles ConPTY on Windows for cross-platform PTY — based on WezTerm's proven usage, high confidence

### Hard Constraints
- Must preserve `paneflow-core`, `paneflow-ipc`, `paneflow-config`, `paneflow-cli` crate APIs
- Must maintain V2 JSON-RPC socket protocol compatibility with cmux
- Must run on Linux (X11 + Wayland) as primary platform
- Typing latency must be < 8ms P95 — this is the project's raison d'etre
- No Electron, no WebView for terminal rendering — native GPU only
- AGPL-3.0 or MIT license (TBD — must be compatible with alacritty_terminal's Apache-2.0)

## Quality Gates

These commands must pass for every user story:
- `cargo check --workspace` — compilation check across all crates
- `cargo clippy --workspace -- -D warnings` — lint with zero warnings
- `cargo test --workspace` — full test suite
- `cargo build --release` — release build succeeds (catches LTO/optimization issues)

For UI stories:
- Launch the app, visually verify the feature works as described in acceptance criteria
- Verify on at least one Linux environment (X11 or Wayland)

## Epics & User Stories

### EP-001: Native Window & UI Shell

Establish the native application window using winit + iced, replacing the Tauri WebView shell. This epic delivers a visible window with the iced widget tree, basic layout regions (sidebar + main area), and the command palette overlay.

**Definition of Done:** A native window opens on Linux and macOS showing a sidebar region and a main content area. The command palette opens/closes with a keyboard shortcut.

#### US-001: Native Window with iced Application Shell
**Description:** As a developer, I want PaneFlow to open as a native GPU-accelerated window so that there is no WebView overhead in the rendering pipeline.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Running `cargo run` opens a native window (winit) with iced 0.14 rendering on WGPU backend
- [ ] Window has configurable title ("PaneFlow"), default size 1200x800, min size 800x500
- [ ] Window is resizable and responds to platform close/minimize/maximize controls
- [ ] The layout has two regions: a fixed-width left sidebar (220px) and a flexible main content area
- [ ] The main content area shows a placeholder message when no terminal panes exist
- [ ] Given a GPU driver that doesn't support WGPU Vulkan, when the app starts, then it falls back to WGPU OpenGL or displays an actionable error message
- [ ] The binary size is < 30 MB for a release build on Linux

#### US-002: Sidebar Widget with Workspace List
**Description:** As a developer, I want a sidebar showing my workspaces with titles, status indicators, and selection state so that I can navigate between workspaces at a glance.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] The sidebar displays a scrollable list of workspaces, each showing: title (bold), current directory (monospace, muted), and active pane count
- [ ] Clicking a workspace item selects it and updates the main content area
- [ ] The selected workspace is visually highlighted (accent color background)
- [ ] A "+" button at the bottom creates a new workspace with a default shell
- [ ] Keyboard navigation works: arrow keys move selection, Enter selects, Ctrl+Shift+N creates new workspace
- [ ] Given 50 workspaces, when scrolling the sidebar, then rendering stays at 60 FPS with no dropped frames
- [ ] Given zero workspaces, when the app starts, then one default workspace is auto-created

#### US-003: Command Palette Overlay
**Description:** As a developer, I want a fuzzy-search command palette (Ctrl+Shift+P) so that I can quickly access workspace and pane operations without memorizing shortcuts.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002

**Acceptance Criteria:**
- [ ] Ctrl+Shift+P toggles the command palette overlay (centered, 60% window width)
- [ ] The palette lists all available commands (new workspace, split, close, focus, etc.)
- [ ] Typing filters commands using fuzzy matching via the `nucleo` crate
- [ ] Enter executes the selected command, Escape closes the palette
- [ ] Results update within 5ms of each keystroke (no perceptible delay)
- [ ] Given a command that requires a target (e.g., "focus workspace"), when selected, then a secondary picker appears with workspace names
- [ ] Given the palette is open, when the user presses Escape, then focus returns to the previously focused terminal pane

---

### EP-002: GPU Terminal Renderer

Build a custom WGPU-based terminal cell renderer that reads from `alacritty_terminal`'s `Term` grid and paints glyphs via an instanced draw pipeline. This is the core rendering engine replacing xterm.js.

**Definition of Done:** A terminal grid is rendered via WGPU showing colored text, cursor, and selection. The renderer handles all 256 ANSI colors, bold/italic/underline attributes, and Unicode (including CJK wide characters).

#### US-004: WGPU Glyph Atlas and Text Renderer
**Description:** As a developer, I want terminal text rendered via GPU-accelerated glyph instancing so that the terminal achieves < 1ms per frame render time.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] A `TerminalRenderer` struct creates a WGPU render pipeline with a glyph texture atlas
- [ ] Glyphs are rasterized on-demand using `cosmic-text` (or `swash`) and bin-packed into a GPU texture using the `etagere` crate
- [ ] An instanced draw call renders all visible cells in a single GPU submission per frame
- [ ] Rendering supports: 16 base ANSI colors, 256 extended colors, 24-bit true color (SGR 38/48), bold, italic, underline, strikethrough, inverse video
- [ ] CJK wide characters render correctly (occupying 2 cell widths)
- [ ] Given a 200x60 terminal grid, when rendering a full frame, then GPU render time is < 1ms (measured via WGPU timestamp queries)
- [ ] Given a glyph not in the atlas, when it first appears, then it is rasterized and added without dropping the current frame

#### US-005: alacritty_terminal Grid Integration
**Description:** As a developer, I want the WGPU renderer to read cell data from `alacritty_terminal`'s `Term<T>` grid so that all VT100/xterm escape sequences are correctly interpreted and rendered.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-004

**Acceptance Criteria:**
- [ ] A `TerminalState` wrapper owns an `alacritty_terminal::Term<EventListener>` instance
- [ ] The renderer iterates `term.renderable_content()` to extract visible cells with their attributes (fg/bg color, flags, character)
- [ ] Dirty-cell tracking: only cells that changed since last frame trigger GPU texture updates
- [ ] The grid size (cols x rows) is computed from the iced `Canvas` dimensions and the font cell size
- [ ] Given `echo -e "\e[31mRed\e[0m"`, when rendered, then "Red" appears in ANSI red and subsequent text in default foreground
- [ ] Given `cat /usr/share/misc/termcap` (heavy VT output), when scrolling, then rendering maintains 60 FPS
- [ ] Given an unsupported escape sequence, when received, then it is silently ignored (no crash, no visual corruption)

#### US-006: Cursor Rendering
**Description:** As a developer, I want a visible, correctly styled cursor so that I always know where my input will appear.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] The cursor renders at the `alacritty_terminal` cursor position in three styles: block (default), beam, underline
- [ ] Cursor blinking is supported with a configurable blink interval (default 530ms) using iced's `Subscription` timer
- [ ] The cursor style responds to DECSCUSR escape sequences (CSI Ps SP q)
- [ ] Given the terminal loses focus, when the window is blurred, then the cursor renders as a hollow block outline
- [ ] Given IME composition is active, when preedit text exists, then the cursor shows the composing text inline

#### US-007: Selection and Clipboard Support
**Description:** As a developer, I want to select terminal text with the mouse and copy it to the system clipboard so that I can share terminal output.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] Click-and-drag selects terminal text using `alacritty_terminal`'s selection model
- [ ] Selected text is visually highlighted (inverted colors or highlight background)
- [ ] Double-click selects a word; triple-click selects a line
- [ ] Ctrl+Shift+C copies selected text to the system clipboard (via `arboard` or `clipboard` crate)
- [ ] Ctrl+Shift+V pastes clipboard content as PTY input
- [ ] Given a selection spanning multiple lines, when copied, then line breaks are preserved correctly
- [ ] Given no text is selected, when Ctrl+Shift+C is pressed, then nothing happens (no error, no empty clipboard write)

---

### EP-003: PTY Bridge & Zero-Latency Input

Wire PTY processes to terminal state and the renderer with zero IPC overhead. Keystrokes write directly to the PTY fd in the same process. PTY output feeds alacritty_terminal and triggers demand-driven rendering.

**Definition of Done:** Typing in a terminal pane has < 8ms P95 latency from key event to pixel update. PTY output (including `cat /dev/urandom | xxd`) renders at 60 FPS without blocking the UI thread.

#### US-008: PTY Spawn with Per-Pane I/O Threads
**Description:** As a developer, I want each terminal pane to have its own PTY process with dedicated I/O so that panes are independent and a blocked PTY doesn't affect others.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None (can start in parallel with EP-002)

**Acceptance Criteria:**
- [ ] `PtyBridge::spawn()` creates a PTY via `portable-pty`, spawns the user's default shell (`$SHELL` or `/bin/bash`), and returns a `PaneHandle`
- [ ] Each PTY reader runs on a dedicated OS thread (`std::thread::spawn`) with a blocking read loop (4 KiB buffer)
- [ ] Each PTY writer is an `Arc<Mutex<Box<dyn Write + Send>>>` accessible from the winit event thread without async overhead
- [ ] `PtyBridge::write_pane(id, data)` acquires only a `RwLock` read + per-pane `Mutex` (same as v1, proven pattern)
- [ ] PTY resize is supported via `master.resize(PtySize { rows, cols, .. })`
- [ ] Given a shell that exits (e.g., `exit`), when the child process terminates, then the pane shows "[Process exited with code N]" and stops the reader thread
- [ ] Given `PtyBridge::close_pane(id)`, when called, then the PTY master fd is closed, the child process is killed, and the reader thread stops within 100ms

#### US-009: Zero-IPC Keystroke Path
**Description:** As a developer, I want keystrokes to reach the PTY with zero serialization or IPC overhead so that typing feels instantaneous.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-008

**Acceptance Criteria:**
- [ ] winit `WindowEvent::KeyboardInput` events are intercepted BEFORE iced processes them for terminal-focused panes
- [ ] The keystroke is translated to a byte sequence (using `alacritty_terminal`'s key binding / input encoding) and written directly to the PTY fd via `PtyBridge::write_pane()`
- [ ] The entire path from `winit::event::KeyboardInput` to `write_all()` on the PTY fd involves zero heap allocations for ASCII printable characters
- [ ] Ctrl+key combinations (Ctrl+C, Ctrl+D, Ctrl+Z) bypass any input method processing and write the control byte directly
- [ ] The write path is synchronous on the winit event thread — no `async`, no channel, no `spawn_blocking`
- [ ] Given typing at 120 WPM (~10 chars/sec), when measuring per-keystroke PTY write latency, then P99 is < 100 microseconds
- [ ] Given iced has the command palette open, when a keystroke arrives, then it routes to iced (not the terminal) — focus routing is correct

#### US-010: Demand-Driven Rendering Pipeline
**Description:** As a developer, I want terminal rendering to be triggered by PTY output (not by a fixed timer) so that there is no wasted CPU when idle and minimal latency when active.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** Blocked by US-008, US-005

**Acceptance Criteria:**
- [ ] PTY reader thread sends output bytes through a bounded channel (`mpsc::channel(64)`) to a coalescing task
- [ ] The coalescing task feeds bytes to `alacritty_terminal::Term::process_bytes()`, which updates the grid state
- [ ] After processing, the coalescing task requests an iced redraw (`iced::window::request_redraw`) only if dirty cells exist
- [ ] There is no app-level timer, `requestAnimationFrame`, or fixed-interval polling for terminal rendering
- [ ] A `_tickScheduled` atomic bool (matching cmux's pattern) ensures only one redraw request is queued per batch of PTY output
- [ ] Given the terminal is idle (no PTY output), when measuring CPU usage, then the process uses < 1% CPU
- [ ] Given `seq 1 1000000` (bulk output), when rendering, then frames are presented at the display refresh rate without blocking the main thread
- [ ] Given PTY output arrives mid-frame, when the next vsync occurs, then the updated cells are visible (no extra frame delay)

#### US-011: Coalesced Output Pipeline with Tick Batching
**Description:** As a developer, I want PTY output to be batched and coalesced so that rapid output (e.g., `cat` of a large file) doesn't overwhelm the renderer.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-010

**Acceptance Criteria:**
- [ ] The coalescing task drains all pending chunks from the channel in a `try_recv()` loop before processing (matching cmux's tick coalescing)
- [ ] A `MAX_BATCH_BYTES` cap (32 KiB) limits how many bytes are processed per tick, yielding back to the event loop for responsiveness
- [ ] After processing a batch, if more data is pending, another tick is immediately scheduled
- [ ] The coalescing task runs on a Tokio task (not a dedicated OS thread) to share the async executor
- [ ] Given `cat /dev/urandom | xxd` running at maximum throughput, when measuring main-thread occupancy, then terminal processing uses < 30% of available frame budget (< 5ms of a 16ms frame)
- [ ] Given a PTY producing 100 MB/s of output, when the coalescing task falls behind, then the bounded channel applies backpressure (the OS thread's `blocking_send` blocks, slowing the PTY read rate)
- [ ] Given the user types while heavy output is streaming, when measuring keystroke latency, then P95 remains < 8ms (typing is not delayed by output processing)

---

### EP-004: Split Pane Tiling Engine

Build a binary-tree tiling layout that supports horizontal and vertical splits with resizable dividers, matching cmux's Bonsplit model.

**Definition of Done:** The main content area displays terminal panes in a binary-tree split layout. Panes can be split, closed, zoomed, and resized via drag.

#### US-012: Binary Tree Split Layout
**Description:** As a developer, I want to split terminal panes horizontally and vertically so that I can view multiple terminals side-by-side.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] The `SplitTree` from `paneflow-core` drives the layout: leaf nodes are terminal panes, branch nodes are split containers with direction (horizontal/vertical) and ratio (0.0-1.0)
- [ ] Ctrl+Shift+D splits the focused pane horizontally; Ctrl+Shift+E splits vertically
- [ ] Each split spawns a new PTY (via US-008) in the new pane
- [ ] Closing a pane (Ctrl+Shift+W or shell exit) collapses its parent split, expanding the sibling to fill the space
- [ ] The minimum pane size is 80px in both dimensions
- [ ] Given 8 recursive splits (16 panes), when rendering, then all panes are visible and the layout is correct
- [ ] Given the last pane in a workspace is closed, when the pane closes, then the workspace shows the empty state placeholder (not a crash)

#### US-013: Pane Zoom and Equalize
**Description:** As a developer, I want to zoom a single pane to fill the workspace and equalize all pane sizes so that I can focus on one task or distribute space evenly.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] Ctrl+Shift+Z toggles zoom: the focused pane fills the entire workspace area, hiding all siblings
- [ ] Pressing Ctrl+Shift+Z again restores the previous split layout exactly
- [ ] Ctrl+Shift+= equalizes all split ratios in the current workspace to 0.5 (even distribution)
- [ ] Given a pane is zoomed, when its shell exits, then zoom is cancelled and the pane is closed normally
- [ ] Given a pane is zoomed, when the user splits via Ctrl+Shift+D, then zoom is cancelled and the split is performed

#### US-014: Drag-to-Resize Split Dividers
**Description:** As a developer, I want to drag split dividers with the mouse to resize panes so that I can allocate more space to the terminal I'm actively working in.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-012

**Acceptance Criteria:**
- [ ] A 4px divider is rendered between split children, visually distinct from terminal content
- [ ] The divider changes cursor to `col-resize` or `row-resize` on hover
- [ ] Clicking and dragging the divider updates the split ratio in real-time
- [ ] The split ratio is clamped so that neither child falls below 80px minimum
- [ ] Given a drag is in progress, when the mouse enters a terminal pane, then the terminal does not receive mouse events (pointer-events disabled during drag)
- [ ] Given a fast drag past the minimum boundary, when released, then the ratio clamps correctly (no negative sizes or panics)

---

### EP-005: Workspace & State Management

Multi-workspace support with tab switching, session persistence, and notification system.

**Definition of Done:** Users can create, switch, close, and rename workspaces. Session layout is persisted across app restarts. Terminal bell events produce sidebar notification badges.

#### US-015: Multi-Workspace Model with Tab Switching
**Description:** As a developer, I want multiple workspaces (each with its own split tree and PTY processes) so that I can organize work by project or task.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-002, US-012

**Acceptance Criteria:**
- [ ] `TabManager` from `paneflow-core` manages an ordered list of workspaces with a selected index
- [ ] Ctrl+1 through Ctrl+9 switch to workspace 1-9; Ctrl+Tab cycles forward; Ctrl+Shift+Tab cycles backward
- [ ] Workspace switching preserves all PTY processes in background workspaces (no kill/respawn)
- [ ] The sidebar reflects the selected workspace with visual highlighting
- [ ] Creating a new workspace (Ctrl+Shift+N) appends it to the list and selects it
- [ ] Closing a workspace (Ctrl+Shift+Q) terminates all its PTY processes and removes it from the list
- [ ] Given the last workspace is closed, when closing, then a confirmation dialog appears (prevent accidental app exit)
- [ ] Given 20 workspaces exist, when switching via Ctrl+5, then the switch completes in < 1ms (no re-creation of panes)

#### US-016: Session Persistence
**Description:** As a developer, I want my workspace layout saved automatically and restored on restart so that I don't lose my arrangement after closing the app.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] Every 8 seconds (matching cmux), the app serializes the workspace layout (workspace list, split trees, working directories, window size) to a JSON file at `$XDG_DATA_HOME/paneflow/session.json`
- [ ] Autosave is deferred during active typing (check `last_typing_activity` timestamp, require 2s quiet period — matching cmux's pattern)
- [ ] On launch, if `session.json` exists, the previous layout is restored: workspaces are recreated, splits are rebuilt, PTYs are spawned in the saved working directories
- [ ] Session save uses atomic file write (write to temp file, then rename) to prevent corruption on crash
- [ ] Given a crash during autosave, when the app restarts, then the previous valid session is loaded (not a partial write)
- [ ] Given the user launches with `--no-restore` flag, when starting, then session restore is skipped

#### US-017: Notification System (Bell Detection and Badges)
**Description:** As a developer, I want terminal bell events (BEL, OSC 9/99/777) to produce sidebar badges so that I notice when an agent needs attention.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `alacritty_terminal` bell events are captured via the `EventListener` trait implementation
- [ ] A bell event in a non-focused workspace increments an unread counter displayed as a badge on the sidebar workspace item
- [ ] Switching to a workspace with unread notifications clears the badge (with a 200ms grace period matching cmux)
- [ ] A desktop notification is sent via `notify-rust` for bell events in non-focused workspaces (if the window is not focused)
- [ ] The unread count is displayed next to the workspace title in the sidebar (e.g., "workspace-1 (3)")
- [ ] Given 100 rapid bell events, when processing, then only one desktop notification is sent (coalesce within 500ms window)
- [ ] Given the focused pane produces a bell, when the pane already has focus, then no notification or badge is shown

---

### EP-006: IPC, CLI & Config

Port the socket server, CLI, and config system to work with the new native app. These crates are mostly preserved from v1.

**Definition of Done:** The V2 JSON-RPC socket server runs alongside the native app, the CLI can create/list/control workspaces, and JSON config with hot-reload works.

#### US-018: Unix Socket Server (V2 JSON-RPC)
**Description:** As a developer, I want the V2 JSON-RPC socket server running in the native app so that AI agents can programmatically control PaneFlow.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-015

**Acceptance Criteria:**
- [ ] `paneflow-ipc::SocketServer` starts on app launch, listening at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock` (Linux) with permissions 0o600
- [ ] The server handles: `system.ping`, `system.capabilities`, `workspace.list`, `workspace.create`, `workspace.select`, `workspace.close`, `surface.list`, `surface.send_text`, `surface.send_key`, `pane.split`, `pane.focus`, `pane.close` (15 methods minimum)
- [ ] Socket commands that mutate UI state dispatch to the main thread via a channel (not `DispatchQueue.main.sync` — Rust equivalent)
- [ ] Read-only queries (list, ping) respond without blocking the main thread
- [ ] Given two agents connecting simultaneously, when both send commands, then commands are serialized correctly (no race conditions)
- [ ] Given an invalid JSON-RPC message, when received, then an error response is returned (not a crash or hang)
- [ ] Given the app is closing, when the socket server receives a command, then it returns an error and shuts down cleanly

#### US-019: CLI Binary
**Description:** As a developer, I want a `paneflow` CLI that communicates with the running app via the socket so that I can script workspace management.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-018

**Acceptance Criteria:**
- [ ] `paneflow-cli` binary connects to the socket and sends V2 JSON-RPC commands
- [ ] Supported commands: `paneflow ping`, `paneflow list-workspaces`, `paneflow new-workspace [--name NAME]`, `paneflow select-workspace N`, `paneflow send-text PANE_ID TEXT`, `paneflow split [--direction horizontal|vertical]`
- [ ] Output is JSON by default with `--format json|text` flag
- [ ] Given the app is not running, when the CLI attempts to connect, then it prints "PaneFlow is not running" and exits with code 1
- [ ] Given `paneflow send-text` with a non-existent pane ID, when sent, then the CLI prints the error from the server and exits with code 1

#### US-020: JSON Config with Hot-Reload
**Description:** As a developer, I want a JSON config file with hot-reload so that I can customize PaneFlow without restarting.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** Blocked by US-001

**Acceptance Criteria:**
- [ ] Config is loaded from `~/.config/paneflow/paneflow.json` (global) and `./paneflow.json` (local, overrides global)
- [ ] Config schema covers: font family, font size, color theme (16 ANSI colors + foreground/background), scrollback lines (default 4000), keybindings, shell command
- [ ] The `notify` crate watches config files and triggers hot-reload on change
- [ ] Hot-reload applies font/color changes without restarting PTY processes
- [ ] Given an invalid JSON config, when saved, then a warning is logged and the previous valid config is retained
- [ ] Given no config file exists, when starting, then sensible defaults are used (system monospace font, 14px, dark theme, $SHELL)

---

## Functional Requirements

- FR-01: The system must render terminal cells via WGPU with < 1ms per-frame GPU render time for a 200x60 grid
- FR-02: The system must write keystrokes to the PTY fd within 100 microseconds of the winit key event (zero-IPC path)
- FR-03: The system must coalesce PTY output reads and process them in batches of up to 32 KiB before requesting a redraw
- FR-04: The system must not allocate heap memory on the keystroke hot path for ASCII printable characters
- FR-05: The system must support at least 16 simultaneous terminal panes with independent PTY processes
- FR-06: The system must preserve all PTY processes when switching workspaces (no kill/respawn)
- FR-07: The system must defer session autosave during active typing (2-second quiet period)
- FR-08: The system must respond to V2 JSON-RPC socket commands within 10ms for read-only queries
- FR-09: The system must NOT use any WebView, JavaScript, or HTML for terminal rendering

## Non-Functional Requirements

- **Performance:** P95 keystroke-to-pixel latency < 8ms on a 60Hz display. GPU frame render time < 1ms for 200x60 grid. Idle CPU usage < 1%. Cold start < 500ms.
- **Memory:** < 15 MB per terminal pane (4000-line scrollback). < 80 MB total for 4 panes with sidebar.
- **Reliability:** No crash on GPU context loss (fallback to software rendering). Atomic session save (no corruption on crash). Graceful shutdown (all PTYs terminated within 1 second).
- **Scalability:** Support 100 workspaces and 16 panes per workspace without degradation. Support 100 MB/s PTY output throughput without dropping frames.
- **Cross-Platform:** Primary: Linux (X11 + Wayland via winit). Secondary: macOS. Tertiary: Windows (ConPTY via portable-pty). All three must compile and run.
- **Accessibility:** Full keyboard navigation (no mouse required). High-contrast theme option. Screen reader support deferred to v3.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Empty state | First launch, no config, no session | Auto-create 1 workspace with 1 pane running $SHELL | — |
| 2 | PTY spawn failure | Invalid $SHELL, permission denied | Show error in pane area with retry button | "Failed to spawn shell: {error}. Click to retry." |
| 3 | GPU driver failure | Vulkan/OpenGL not available | Fall back to WGPU software adapter (tiny-skia) or exit with clear message | "GPU not available. Install Vulkan/OpenGL drivers." |
| 4 | Session corruption | Crash during autosave | Load previous valid session from backup file | "Restored from backup session." |
| 5 | 50+ panes | User splits excessively | Cap at 16 panes per workspace; disable split shortcut at limit | "Maximum panes reached (16)." |
| 6 | Socket command race | Two agents send conflicting commands | Serialize via main-thread channel; last-writer-wins for state mutations | JSON-RPC error response for conflicts |
| 7 | Config syntax error | User saves invalid JSON | Log warning, retain previous valid config | — (logged to stderr) |
| 8 | Display disconnect | Monitor unplugged/reconnected | Detect via winit `ScaleFactorChanged`, resize all panes | — |
| 9 | Workspace close with running processes | Ctrl+Shift+Q with active commands | Confirmation dialog: "Close N terminals?" | "This workspace has N active terminals." |
| 10 | Memory pressure | 100 workspaces × 4000-line scrollback | Scrollback is capped; older lines are dropped per alacritty_terminal defaults | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | iced `Canvas` + custom WGPU renderer integration is harder than expected | Medium | High | US-004 is a spike — validate in first week. Fallback: use raw winit+wgpu without iced for the terminal area. |
| 2 | `alacritty_terminal` 0.26.0-rc1 has breaking changes before stable release | Low | Medium | Pin exact version in Cargo.lock. Monitor Alacritty releases. Test suite catches regressions. |
| 3 | Font shaping latency exceeds budget on complex Unicode | Low | Medium | Benchmark cosmic-text vs swash early. Cache shaped runs aggressively. Fall back to monospace-only if needed. |
| 4 | Windows ConPTY via portable-pty has subtle behavior differences | Medium | Medium | Defer Windows to P1. Test ConPTY-specific edge cases (resize, signal handling) in dedicated CI job. |
| 5 | 20 stories exceeds single-phase capacity | Medium | Low | Split EP-005 and EP-006 into Phase B if EP-001 through EP-004 take longer than expected. Core rendering is the priority. |
| 6 | WGPU Wayland support has driver-specific bugs (NVIDIA) | Medium | Medium | Test on Intel/AMD first. NVIDIA Wayland is still maturing. Document known driver requirements. |

## Non-Goals

Explicit boundaries — what PaneFlow v2 does NOT include:

- **Browser panels (WKWebView/WebView2 equivalent):** Deferred to v3. Will use `wry` crate when added. v2 focuses on terminal-only rendering.
- **SSH/Remote workspaces:** Deferred to v3. cmux's `cmuxd-remote` Go daemon can be reused as-is when the time comes.
- **tmux compatibility shim:** Not a goal. PaneFlow has its own command grammar and V2 JSON-RPC protocol.
- **Auto-update system:** Deferred. Users install via package manager or binary download.
- **Telemetry/analytics:** Not included. No PostHog, no phone-home.
- **Plugin system (WASM or Lua):** Deferred to v4+. Socket API is the extension mechanism for now.
- **IME input (CJK languages):** Deferred to v2.1. Requires winit IME integration which is platform-dependent.
- **Markdown viewer panel:** Deferred to v3 (alongside browser panels).
- **Sixel/image protocol rendering:** Deferred. Terminal is text-only in v2.

## Files NOT to Modify

These existing crates from PaneFlow v1 are preserved and adapted, not rewritten:

- `crates/paneflow-core/` — Domain model (Workspace, Panel, SplitTree, TabManager). Adapt trait implementations but preserve the core types.
- `crates/paneflow-ipc/` — Socket server and JSON-RPC dispatcher. Wire into the new app binary but preserve the protocol handling.
- `crates/paneflow-config/` — Config schema, loader, and file watcher. Extend schema for new settings but preserve the architecture.
- `crates/paneflow-cli/` — CLI client binary. No changes needed — it talks to the socket, not the app directly.

## Technical Considerations

Frame as questions for engineering input — not mandates:

- **Architecture:** iced 0.14 `Canvas` widget for terminal rendering vs. raw winit+wgpu alongside iced — recommended: `Canvas` widget to stay within iced's layout system. Engineering to validate in US-004 spike.
- **Font Stack:** `cosmic-text` (used by COSMIC desktop, proven with iced) vs. `swash` (lower-level, more control) vs. `ab_glyph` (simpler, no shaping). Recommended: `cosmic-text` for maximum compatibility. Benchmark during US-004.
- **Event Thread Model:** winit event loop as the main thread, with iced running on the same thread. PTY readers on dedicated OS threads (std::thread). Coalescing task on Tokio runtime. Engineering to confirm this matches iced's `Application::run()` model.
- **Binary Layout:** Single binary `paneflow-app` (replaces `src-tauri/`). Contains the native window app. `paneflow-cli` remains a separate binary. Both share workspace crates.
- **Migration:** `src-tauri/` and `frontend/` directories will be removed. No backward compatibility with v1 — this is a clean break. Users of the socket API are unaffected (protocol preserved).

## Success Metrics

| Metric | Baseline (PaneFlow v1) | Target (v2) | Timeframe | How Measured |
|--------|----------------------|-------------|-----------|-------------|
| P95 keystroke-to-pixel latency | ~15ms (estimated) | < 8ms | Month-1 | Custom timing probe: winit key event timestamp → next GPU present |
| GPU frame render time (200x60) | ~0.7ms (xterm.js WebGL) | < 1ms | Month-1 | WGPU timestamp queries in debug builds |
| Idle CPU usage | ~2% (Tauri+WebView) | < 1% | Month-1 | `top` measurement with terminal idle |
| Memory per pane (4000 scrollback) | ~20 MB (xterm.js + WebView overhead) | < 15 MB | Month-1 | `/proc/self/smaps` measurement |
| Cold start to first terminal | ~1.5s (Tauri+Vite HMR) | < 500ms | Month-1 | Wall clock from process start to first winit Redraw |
| Socket API response time (ping) | ~5ms (Tauri JSON round-trip) | < 2ms | Month-1 | Measured via CLI `time paneflow ping` |
| Cross-platform builds passing | 1 (Linux only, Tauri) | 3 (Linux, macOS, Windows) | Month-3 | CI matrix |

## Open Questions

- **iced Canvas + WGPU context sharing:** Does iced 0.14 expose the WGPU `Device` and `Queue` to `Canvas` paint callbacks, or do we need to create a separate WGPU context? This determines whether terminal rendering shares iced's GPU context or manages its own. → Answer needed from US-004 spike, blocks renderer architecture.
- **winit keyboard event interception:** Can we intercept winit `KeyboardInput` events before iced processes them (for the zero-IPC keystroke path), or do we need to use iced's `Subscription` for keyboard events? → Engineering to validate during US-009.
- **Workspace-specific Tokio runtimes:** Should each workspace have its own Tokio runtime (isolation) or share a global runtime (simpler)? cmux uses per-connection threads for the socket but a global main queue. → Engineering decision during US-008.
- **License:** MIT vs AGPL-3.0? `alacritty_terminal` is Apache-2.0 (compatible with both). `iced` is MIT. cmux is AGPL-3.0. → Arthur to decide before public release.
[/PRD]
