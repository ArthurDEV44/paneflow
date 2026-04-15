[PRD]
# PRD: Ink/TUI Application Rendering Compatibility

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-15 | Claude + Arthur | Initial draft from 4-codebase cross-analysis (Ink, Claude Code, PaneFlow, Ghostty) |

## Problem Statement

PaneFlow's terminal emulator cannot correctly render Ink-based TUI applications (notably Claude Code). A cross-codebase analysis of Ink, Claude Code, PaneFlow, and Ghostty identified five root causes:

1. **No synchronized output (DEC private mode 2026)** — Ink wraps every frame in `\e[?2026h`..`\e[?2026l` to prevent partial renders. The Alacritty fork no-ops both set and unset (`term/mod.rs:1992,2041`), and PaneFlow's `pty_reader_loop` sends `Wakeup` after every `read()` unconditionally (`terminal.rs:1185`). Every intermediate state (erase → cursor move → new content) is rendered as a separate frame, producing intense flickering. Ghostty solves this with a renderer gate (`renderer/generic.zig:1176`) that skips frames entirely during sync.

2. **No mouse event forwarding to the PTY** — Claude Code activates mouse modes 1000/1002/1003/1006 simultaneously (`ink/termio/dec.ts:51-60`). The Alacritty fork tracks these modes correctly in `TermMode` (`term/mod.rs:60-80`), but PaneFlow never checks these flags — zero references to `TermMode::MOUSE_*` in `src-app/src/`. Mouse handlers only perform text selection (`terminal.rs:1009-1078`). Zed has a complete reference implementation at `zed/crates/terminal/src/mappings/mouse.rs:239-318`.

3. **No terminal identification** — PaneFlow sets only `PANEFLOW_*` env vars for the child PTY (`terminal.rs:306-313`). No `TERM_PROGRAM`, `COLORTERM`, or `TERM_PROGRAM_VERSION`. Claude Code checks these at `ink/terminal.ts:70-118` for capability detection and falls back to "unknown terminal" mode — disabling synchronized output, Kitty keyboard protocol, and other optimizations.

4. **No focus event forwarding (DEC 1004)** — Claude Code enables focus tracking via `\e[?1004h` (`ink/termio/dec.ts:41-42`) to receive `CSI I` / `CSI O` events. PaneFlow has zero focus forwarding to the PTY. Claude Code uses these events to throttle rendering when the terminal is unfocused.

5. **Initial PTY size race condition** — The PTY starts at hardcoded 80×24 (`terminal.rs:262-263`). Resize to actual dimensions only happens during the first `prepaint()` cycle (`terminal_element.rs:482-497`). If Claude Code reads `stdout.columns`/`stdout.rows` before `SIGWINCH` arrives, Yoga computes layout for 80×24 regardless of actual pane size.

**Why now:** The stabilization-polish PRD (EP-001 through EP-004) is complete. PaneFlow is architecturally mature with config hot-reload, keybinding customization, session persistence, and terminal power features (search, copy mode, zoom). The remaining gap is TUI application compatibility — and Claude Code is the primary use case. Without these fixes, PaneFlow cannot serve as a daily-driver terminal for AI-assisted development workflows.

## Overview

This PRD addresses PaneFlow's terminal protocol compliance for modern TUI applications, organized into four epics:

**EP-001 (Terminal Identity)** sets the foundation: inject `TERM_PROGRAM`, `COLORTERM`, and `TERM_PROGRAM_VERSION` into the PTY environment, and fix the initial PTY size race by passing actual pane dimensions at spawn time. These are trivial changes (~25 lines total) that unlock application-side capability detection.

**EP-002 (Synchronized Output)** implements DEC private mode 2026 at the PaneFlow level — scanning raw PTY bytes for BSU/ESU sequences in `pty_reader_loop`, gating `Wakeup` events via a shared `AtomicBool`, and adding a 1000ms safety timer to auto-clear on timeout. This eliminates frame tearing for any application that uses synchronized output (Ink, Textual, Ratatui, Charm/Bubbletea).

**EP-003 (Mouse Forwarding)** ports Zed's `mappings/mouse.rs` SGR/X10 mouse encoding into PaneFlow, and rewires the existing mouse handlers to check `TermMode` flags before deciding whether to forward events to the PTY or handle them locally for text selection. This makes Claude Code's mouse-interactive UI functional.

**EP-004 (Extended Protocol Support)** adds focus event forwarding (DEC 1004) and XTVERSION response, completing the terminal protocol surface that modern TUI apps expect.

Key decisions:
- **DEC 2026 at PaneFlow level, not Alacritty fork** — Scan raw bytes in `pty_reader_loop` instead of modifying the Alacritty fork. Avoids fork divergence while achieving correct semantics.
- **Mouse encoding ported from Zed** — Zed's `mappings/mouse.rs` already uses the same Alacritty fork's `TermMode` types. Minimal adaptation needed.
- **Safety timer at 1000ms** — Matches Ghostty's proven default. Prevents renderer deadlock from misbehaving programs.
- **No custom terminfo entry** — Use standard `xterm-256color` TERM value. Custom terminfo deferred to future work.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| TUI rendering fidelity | Claude Code renders flicker-free, zero visual artifacts | All major TUI frameworks (Ink, Textual, Ratatui, Bubbletea) render correctly |
| Mouse interactivity | Claude Code mouse clicks, scroll, and drag all functional | Full SGR mouse protocol compliance |
| Terminal identification | Apps detect PaneFlow and enable optimized codepaths | PaneFlow in community terminal detection databases |
| Protocol compliance | DEC 2026 + mouse modes + focus events | XTVERSION + bracketed paste + all DEC private modes in common use |

## Target Users

### Linux Power User (AI Developer)
- **Role:** Software engineer using Claude Code, Cursor, or other AI coding assistants inside PaneFlow daily
- **Behaviors:** Runs Claude Code in a pane alongside editor/git panes, relies on mouse interaction for Claude Code's TUI (clicking suggestions, scrolling output, selecting text)
- **Pain points:** Claude Code renders with intense flickering, mouse clicks don't register, layout sometimes starts at wrong dimensions
- **Current workaround:** Uses Ghostty or another terminal for Claude Code sessions, defeating the purpose of a multiplexer
- **Success looks like:** Launches Claude Code inside PaneFlow and it renders identically to Ghostty — flicker-free, mouse-responsive, correct layout from first frame

### Linux Power User (TUI Developer)
- **Role:** Developer building or using Rust/Python/Go TUI applications (Ratatui, Textual, Bubbletea)
- **Behaviors:** Tests TUI apps in PaneFlow, expects modern terminal protocol support
- **Pain points:** TUI apps that use synchronized output flicker. Mouse-driven TUIs don't respond to clicks.
- **Current workaround:** Tests in Alacritty or Ghostty instead
- **Success looks like:** PaneFlow supports the same terminal protocols as Ghostty and modern Alacritty

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **Ghostty:** Full DEC 2026 with renderer gate + 1000ms safety timer, SGR mouse, focus events, XTVERSION, Kitty keyboard protocol. The gold standard for TUI compatibility.
- **Alacritty (upstream):** DEC 2026 via EventLoop sync suppression. Zed's fork disables this (SyncUpdate → no-op) because Zed manages its own rendering pipeline.
- **WezTerm:** Full DEC 2026, mouse, focus events. Lua-configurable.
- **Kitty:** Full protocol suite including Kitty graphics protocol and keyboard protocol.
- **Market gap:** PaneFlow is the only GPU-rendered Linux terminal multiplexer with native GPUI chrome — but it's also the only one that can't render Claude Code correctly.

### Best Practices Applied
- DEC 2026 (synchronized output) is the de facto standard for flicker-free TUI rendering — adopted by all modern terminals (spec: christianparpart/gist)
- Safety timer (1-2s) is mandatory — prevents renderer deadlock from programs that send BSU without ESU
- Mouse forwarding must be mode-aware: forward to PTY when mouse modes active, handle locally for text selection when not
- `TERM_PROGRAM` is the de facto standard for terminal identification — set by Ghostty, iTerm2, WezTerm, VS Code, Alacritty, Kitty
- XTVERSION (`CSI > 0 q` → `DCS > | name(version) ST`) is the modern async terminal detection protocol

### Diagnosis Sources
- **Ink codebase** (`/home/arthur/dev/ink`): rendering pipeline (ink.tsx, log-update.ts, write-synchronized.ts, cursor-helpers.ts)
- **Claude Code codebase** (`/home/arthur/dev/claude-code`): custom Ink fork (ink/termio/dec.ts, ink/terminal.ts, ink/components/AlternateScreen.tsx)
- **Ghostty codebase** (`/home/arthur/dev/ghostty`): DEC 2026 implementation (renderer/generic.zig, termio/Thread.zig, terminal/modes.zig)
- **Zed codebase** (`/home/arthur/dev/zed`): mouse forwarding reference (crates/terminal/src/mappings/mouse.rs)

*Full cross-codebase analysis conducted on 2026-04-15 using 5 parallel exploration agents.*

## Assumptions & Constraints

### Assumptions (to validate)
- Scanning raw PTY bytes for `\e[?2026h/l` before VTE parsing is sufficient — escape sequences may span buffer boundaries, requiring a small state machine to track partial matches
- Zed's `mappings/mouse.rs` SGR/X10 encoding is directly portable to PaneFlow's architecture (same Alacritty fork, same TermMode types)
- Setting `TERM_PROGRAM=paneflow` will not cause applications to attempt unsupported features that produce worse rendering than the current "unknown terminal" fallback
- The Alacritty fork correctly tracks mouse mode flags in TermMode even though SyncUpdate is a no-op (confirmed by source reading)

### Hard Constraints
- Alacritty fork at git rev `9d9640d` — no modifications to the fork in this PRD. All fixes at PaneFlow level.
- Linux-only (Wayland + X11) — no macOS/Windows considerations
- PTY I/O runs on a dedicated thread, UI on GPUI main thread — cross-thread communication via `Arc<FairMutex<Term>>` and `UnboundedChannel`
- Must not regress existing text selection, copy/paste, or scroll behavior when mouse modes are inactive
- Must pass `cargo clippy --workspace -- -D warnings` and `cargo fmt --check`

## Quality Gates

These commands must pass for every user story:
- `cargo clippy --workspace -- -D warnings` — zero warnings
- `cargo fmt --check` — consistent formatting
- `cargo test --workspace` — all tests pass

For UI stories, additional gates:
- Launch with `cargo run` and verify the feature works with a real TUI application
- For DEC 2026: run Claude Code or an Ink app and confirm flicker-free rendering
- For mouse forwarding: run Claude Code and confirm mouse clicks register
- For terminal identity: run `echo $TERM_PROGRAM` in a PaneFlow terminal and confirm output

## Epics & User Stories

### EP-001: Terminal Identity & Bootstrap

Set the foundation for application-side capability detection and fix the PTY size race. These are small, self-contained changes that unlock optimized codepaths in TUI applications.

**Definition of Done:** Applications can detect PaneFlow via `TERM_PROGRAM`, and the initial PTY size matches the actual pane dimensions.

#### US-001: Set Terminal Identification Environment Variables
**Description:** As a TUI application running inside PaneFlow, I want to detect which terminal emulator I'm running in, so that I can enable optimized rendering codepaths.

**Priority:** P0
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `TerminalState::new()` injects `TERM_PROGRAM=paneflow` into the child PTY environment at `terminal.rs:306-313`
- [ ] `TERM_PROGRAM_VERSION` is set to the crate version via `env!("CARGO_PKG_VERSION")`
- [ ] `COLORTERM=truecolor` is set (PaneFlow supports 24-bit color via GPUI)
- [ ] Given a new terminal pane, when running `echo $TERM_PROGRAM`, then the output is `paneflow`
- [ ] Given a new terminal pane, when running `echo $COLORTERM`, then the output is `truecolor`
- [ ] Existing `PANEFLOW_*` env vars are unchanged
- [ ] Given an application that checks `TERM_PROGRAM` (e.g., `env | grep TERM`), when it runs in PaneFlow, then it sees `TERM_PROGRAM=paneflow` alongside the inherited `TERM` value

#### US-002: Fix Initial PTY Size Race Condition
**Description:** As a TUI application, I want the terminal to report correct dimensions from the first moment, so that my initial layout renders at the right size without waiting for a resize event.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `TerminalState::new()` accepts initial `cols` and `rows` parameters derived from the actual pane bounds, instead of hardcoded 80×24 at `terminal.rs:262-263`
- [ ] The caller (workspace/split code that creates terminals) calculates the initial grid size from the available pixel bounds and cell dimensions
- [ ] If pane bounds are not yet known at spawn time, a reasonable default of 120×40 is used (closer to typical pane size than 80×24)
- [ ] Given a PaneFlow window of 1920×1080 with a single pane, when a new terminal is opened and immediately runs `tput cols; tput lines`, then the values match the pane's actual grid dimensions (not 80×24)
- [ ] Given a terminal spawned via IPC `surface.split`, when the new pane runs `tput cols`, then the value reflects the split pane size, not the full window
- [ ] Given a race condition where `prepaint()` has not yet run, when an application reads terminal size, then it gets the initial estimate rather than 80×24

---

### EP-002: Synchronized Output — DEC Private Mode 2026

Implement output batching so that TUI applications can write complex frame updates atomically, eliminating flicker from intermediate render states.

**Definition of Done:** Applications that wrap output in `\e[?2026h`..`\e[?2026l` render flicker-free in PaneFlow, with a safety timer preventing deadlocks.

#### US-003: Implement DEC 2026 Synchronized Output Batching
**Description:** As a TUI application using synchronized output (Ink, Textual, Ratatui), I want PaneFlow to batch my frame writes and render them as a single atomic update, so that users see complete frames instead of flickering intermediate states.

**Priority:** P0
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A shared `AtomicBool` flag `sync_output_active` is accessible from both the PTY reader thread and the GPUI main thread
- [ ] `pty_reader_loop` scans the raw byte buffer for the escape sequences `\x1b[?2026h` (BSU) and `\x1b[?2026l` (ESU) before feeding bytes to the VTE processor
- [ ] When BSU is detected, `sync_output_active` is set to `true` and subsequent `Wakeup` events are suppressed (not sent to the channel)
- [ ] When ESU is detected, `sync_output_active` is set to `false` and a single `Wakeup` event is sent (triggering one repaint with the final accumulated state)
- [ ] The byte scanner handles escape sequences that span two consecutive `read()` buffer boundaries (partial match carried across reads via a small state buffer)
- [ ] The VTE processor still receives all bytes including the BSU/ESU sequences (the Alacritty fork no-ops them, which is fine — PaneFlow handles the gating)
- [ ] Given an Ink application that wraps output in BSU/ESU, when it writes a multi-step frame (erase + cursor move + new content), then PaneFlow renders only the final state — zero intermediate frames visible
- [ ] Given rapid alternating BSU/ESU pairs (e.g., 60fps Ink render loop), when rendering, then each frame is atomic with no inter-frame flicker
- [ ] Given a program that sends BSU but never ESU (buggy program), when 1000ms elapses, then the safety timer fires, clears `sync_output_active`, and sends a `Wakeup` — rendering resumes with whatever state exists

#### US-004: Safety Timer for Synchronized Output
**Description:** As a terminal user, I want PaneFlow to recover automatically if a program sends BSU without ESU, so that a buggy program cannot freeze my terminal display indefinitely.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** Blocked by US-003

**Acceptance Criteria:**
- [ ] A timer is armed when `sync_output_active` transitions from `false` to `true`
- [ ] The timer fires after 1000ms (matching Ghostty's default) if `sync_output_active` is still `true`
- [ ] On timer expiry: `sync_output_active` is set to `false`, a `Wakeup` event is sent, and a debug log message is emitted (`"DEC 2026 safety timer expired — forced sync clear"`)
- [ ] The timer is canceled if ESU is received before expiry (no spurious wakeup)
- [ ] Given a program that sends `\e[?2026h` and then exits without sending `\e[?2026l`, when 1000ms passes, then the terminal display unfreezes and shows the final PTY state
- [ ] Given a program that sends BSU, writes for 500ms, then sends ESU, when ESU arrives, then the timer is canceled and the frame renders immediately — no 1000ms delay
- [ ] A terminal resize unconditionally clears `sync_output_active` (matching Ghostty's behavior at `Termio.zig:507-508`) to prevent deadlock during resize

---

### EP-003: Mouse Event Forwarding

Enable mouse-driven TUI applications by forwarding mouse events to the PTY when the terminal has mouse reporting modes active, while preserving existing text selection behavior when mouse modes are off.

**Definition of Done:** Applications that activate mouse modes (1000/1002/1003/1006) receive properly encoded mouse events. Text selection continues to work when no mouse mode is active.

#### US-005: Create Mouse Event Encoding Module
**Description:** As a developer, I want a reusable module that encodes GPUI mouse events into terminal escape sequences (SGR and X10 formats), so that mouse forwarding has a clean, testable encoding layer.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] A new `src-app/src/mouse.rs` module provides encoding functions for mouse events
- [ ] `sgr_mouse_report(point, button, pressed) -> String` generates SGR format: `\e[<{button};{col+1};{row+1}{M|m}` (M for press, m for release)
- [ ] `normal_mouse_report(point, button, utf8) -> Option<Vec<u8>>` generates X10 format: `\e[M{button+32}{col+33}{row+33}` with UTF-8 encoding for positions >95
- [ ] `mouse_button_code(button, modifiers) -> u8` maps GPUI `MouseButton` + modifier keys to the terminal mouse button byte (0=left, 1=middle, 2=right, +4=shift, +8=alt, +16=ctrl)
- [ ] `scroll_button_code(direction, modifiers) -> u8` maps scroll up (64) and scroll down (65) with modifier bits
- [ ] `MouseFormat` enum with `Sgr` and `Normal(utf8: bool)` variants, with `from_mode(TermMode) -> MouseFormat` constructor that selects format based on `TermMode::SGR_MOUSE`
- [ ] Positions exceeding X10 limits (>223 without UTF-8, >2015 with UTF-8) return `None` from `normal_mouse_report`
- [ ] Unit tests cover: left/middle/right button encoding, modifier combinations, SGR press/release format, X10 encoding at boundary values (column 94, 95, 96), scroll encoding
- [ ] Given button=Left, point=(10,5), pressed=true, SGR format, then output is `\e[<0;11;6M`
- [ ] Given button=Left, point=(10,5), pressed=false, SGR format, then output is `\e[<0;11;6m`

#### US-006: Wire Mouse Handlers to Forward Events to PTY
**Description:** As a Claude Code user, I want my mouse clicks, drags, and scroll to be forwarded to the TUI application, so that I can interact with Claude Code's mouse-driven interface.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** Blocked by US-005

**Acceptance Criteria:**
- [ ] `handle_mouse_down` checks `term.mode()` for `TermMode::MOUSE_REPORT_CLICK` — if set, encodes the event via `mouse.rs` and writes to PTY instead of starting text selection
- [ ] `handle_mouse_move` checks for `TermMode::MOUSE_MOTION` (mode 1003) or `TermMode::MOUSE_DRAG` (mode 1002, only during button press) — if set, forwards motion as mouse report
- [ ] `handle_mouse_up` forwards release events when any mouse mode is active
- [ ] `handle_scroll_wheel` forwards scroll events as button 64/65 mouse reports when mouse mode is active, falls back to existing scrollback behavior when not
- [ ] Right-click and middle-click are forwarded when mouse mode is active (not just left-click)
- [ ] Modifier keys (Shift, Alt, Ctrl) are included in mouse reports per protocol spec
- [ ] Given Claude Code running with mouse modes 1000+1006 active, when clicking on a UI element, then Claude Code receives the click and responds
- [ ] Given Claude Code running with mode 1003 active, when moving the mouse, then motion events are sent to the PTY
- [ ] Given no mouse mode active (plain shell prompt), when clicking and dragging, then text selection works exactly as before — no regression
- [ ] Given mouse mode active, when pressing Shift+Click, then the event is forwarded with Shift modifier bit set (not intercepted for selection override)

---

### EP-004: Extended Terminal Protocol Support

Add focus event reporting and XTVERSION response to complete the terminal protocol surface expected by modern TUI applications.

**Definition of Done:** Applications that request focus events (DEC 1004) receive them. XTVERSION queries are answered with PaneFlow's identity.

#### US-007: Forward Focus Events (DEC 1004)
**Description:** As a TUI application, I want to know when the terminal pane gains or loses focus, so that I can optimize rendering or show focus indicators.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] When the `TerminalView`'s `FocusHandle` gains focus and `TermMode::FOCUS_IN_OUT` is set (DEC 1004), write `\x1b[I` to the PTY
- [ ] When the `TerminalView`'s `FocusHandle` loses focus and `TermMode::FOCUS_IN_OUT` is set, write `\x1b[O` to the PTY
- [ ] Focus events are only sent when the mode is active — programs that don't request DEC 1004 receive nothing
- [ ] Given Claude Code running with DEC 1004 enabled, when switching to another pane with Alt+Arrow, then Claude Code receives `CSI O` (focus out) and the newly focused pane's application receives `CSI I` (focus in)
- [ ] Given a plain shell (no DEC 1004), when switching panes, then no focus events are written to the PTY

#### US-008: Respond to XTVERSION Query
**Description:** As a TUI application, I want to query the terminal's identity via the XTVERSION protocol, so that I can detect capabilities even when `TERM_PROGRAM` is not available (e.g., over SSH).

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] When the VTE processor receives `CSI > 0 q` (XTVERSION query/Primary Device Attributes variant), PaneFlow responds with `DCS > | paneflow({version}) ST` written to the PTY input
- [ ] The version string uses the crate version from `env!("CARGO_PKG_VERSION")`
- [ ] The response is written via the existing PTY write path (not directly to the reader)
- [ ] Given a program that sends `\e[>0q` to detect the terminal, when PaneFlow receives it, then it responds with `\eP>|paneflow(0.1.0)\e\\` (DCS format with ST terminator)
- [ ] Given the query is sent over an SSH session where `TERM_PROGRAM` is not forwarded, when the application probes via XTVERSION, then it successfully identifies PaneFlow

---

## Functional Requirements

- FR-01: PaneFlow must gate rendering updates during DEC 2026 synchronized output, rendering only when ESU is received or the safety timer expires
- FR-02: PaneFlow must forward mouse events to the PTY as SGR or X10 escape sequences when the terminal has mouse reporting modes active
- FR-03: PaneFlow must not forward mouse events when no mouse mode is active — text selection must work identically to current behavior
- FR-04: PaneFlow must set `TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, and `COLORTERM` in the child PTY environment
- FR-05: PaneFlow must report correct terminal dimensions from the first frame, not after a lazy resize
- FR-06: PaneFlow must forward focus in/out events when DEC 1004 is active
- FR-07: PaneFlow must respond to XTVERSION queries with its identity

## Non-Functional Requirements

- **Rendering latency:** DEC 2026 ESU → pixel must add <2ms beyond the existing keystroke→pixel pipeline (4ms poll + sync + paint)
- **Safety timer precision:** 1000ms ±100ms tolerance on the DEC 2026 safety timer
- **Mouse report latency:** Mouse event → PTY write must complete within 1ms (no buffering or batching of mouse reports)
- **Memory overhead:** DEC 2026 state machine adds <64 bytes per terminal (one `AtomicBool` + small state buffer for cross-boundary matching)
- **Thread safety:** All cross-thread state (`sync_output_active`) uses `std::sync::atomic::Ordering::SeqCst` for correctness over performance
- **Backward compatibility:** Zero regression in existing terminal behavior when interacting with programs that don't use DEC 2026, mouse modes, or focus events

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | BSU without ESU | Buggy program sends `\e[?2026h` then exits | Safety timer fires at 1000ms, clears sync flag, renders final state | (none — transparent recovery) |
| 2 | BSU/ESU split across read boundaries | `\e[?2026` in buffer 1, `h` in buffer 2 | State machine carries partial match, completes on next read | (none) |
| 3 | Rapid BSU/ESU cycling | 60fps render loop: BSU→content→ESU every 16ms | Each ESU triggers exactly one Wakeup, timer reset each cycle | (none) |
| 4 | Mouse position overflow (X10) | Click at column 250 in X10 mode (no SGR) | `normal_mouse_report` returns `None`, event silently dropped | (none) |
| 5 | Mouse mode change mid-drag | App disables mouse mode while user is dragging | PaneFlow checks mode on each event — switches to selection on next move | (none — seamless transition) |
| 6 | Resize during sync | Window resized while `sync_output_active` is true | Resize clears sync flag unconditionally, sends Wakeup | (none) |
| 7 | Multiple nested BSU | Program sends `\e[?2026h` twice without ESU | Second BSU is idempotent — flag stays true, timer not reset | (none) |
| 8 | Process exit during sync | Shell exits while `sync_output_active` is true | Reader loop exits → child wait → sync flag becomes irrelevant (terminal is dead) | (none) |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Raw byte scanning for DEC 2026 sequences may miss matches at buffer boundaries | Medium | High | Implement a small state machine (≤8 bytes) that carries partial escape sequence matches across `read()` calls. Test with synthetic buffers that split the sequence at every possible byte position. |
| 2 | Setting `TERM_PROGRAM=paneflow` causes apps to try unsupported features | Low | Medium | PaneFlow already supports 24-bit color, alternate screen, bracketed paste, and standard mouse modes. Any remaining unsupported features (e.g., Kitty graphics) will be silently ignored by the Alacritty fork's VTE parser. |
| 3 | Mouse forwarding breaks text selection UX for users who rely on it | Medium | High | Mode-aware switching: only forward when TermMode mouse flags are set. Add Shift+Click override for selection in mouse mode (standard terminal convention). |
| 4 | AtomicBool for sync flag introduces ordering bugs between reader thread and poll loop | Low | High | Use `Ordering::SeqCst` (strictest ordering). The flag is only toggled by the reader thread and read by the poll thread — no ABA problem possible. |
| 5 | Safety timer implementation complexity on reader thread (no async runtime) | Medium | Medium | Use `std::time::Instant` tracking in the reader loop. On each `read()`, check if sync has been active longer than 1000ms. No separate timer thread needed. |

## Non-Goals

Explicit boundaries — what this version does NOT include:

- **Kitty graphics protocol** — Image rendering in terminals. Out of scope; requires significant Alacritty fork changes.
- **Kitty keyboard protocol** — Enhanced keyboard reporting. Claude Code only enables this for allowlisted terminals; PaneFlow can be added later.
- **Custom terminfo entry** — A `paneflow` terminfo database entry. Would require user installation. Deferred to packaging/distribution phase.
- **Modifying the Alacritty fork** — All fixes are at the PaneFlow level. The fork stays at rev `9d9640d`.
- **OSC 8 hyperlink rendering** — Clickable URLs. Requires hyperlink detection + GPUI click handling. Separate feature.
- **Mouse hover cursor changes** — Changing cursor shape based on TUI hover state. Cosmetic, not functional.

## Files NOT to Modify

- `crates/paneflow-config/src/schema.rs` — Config schema; this PRD doesn't add config options
- `crates/paneflow-config/src/loader.rs` — Config loader; no config changes needed
- `src-app/src/ipc.rs` — IPC server; no new IPC methods in this PRD
- `src-app/src/title_bar.rs` — Title bar; unrelated to terminal protocol
- `src-app/src/theme.rs` — Theme system; unrelated
- `src-app/src/settings_window.rs` — Settings UI; unrelated

## Technical Considerations

- **DEC 2026 scanning approach:** Recommended: scan raw bytes before VTE processor in `pty_reader_loop`. Alternative: modify Alacritty fork to emit an event on SyncUpdate. Trade-off: raw scanning adds ~20 lines of state machine but avoids fork divergence. Engineering to confirm: can partial escape sequences realistically span buffers with the 4096-byte read buffer?

- **Mouse encoding module placement:** Recommended: `src-app/src/mouse.rs` as a new module (matching Zed's `mappings/mouse.rs` pattern). Alternative: inline in `terminal.rs`. Trade-off: separate module enables unit testing of encoding logic without GPUI context.

- **Safety timer threading:** Recommended: track `Instant::now()` at BSU detection, check elapsed time on each subsequent `read()` iteration. Alternative: spawn a dedicated timer thread. Trade-off: in-loop checking adds negligible overhead (<1μs per read) and avoids thread management. The 4096-byte buffer means reads complete frequently enough for 1000ms granularity.

- **Cross-thread sync flag:** Recommended: `Arc<AtomicBool>` shared between `pty_reader_loop` thread and the `sync()` method on the main thread. Alternative: send sync state changes via the existing `UnboundedChannel`. Trade-off: AtomicBool is simpler and avoids channel congestion during high-throughput output.

- **Mouse mode check cost:** Reading `term.mode()` requires locking the `FairMutex`. This already happens in `handle_mouse_down` (for selection). Recommended: read mode once at handler entry, branch on mouse flag. No additional lock acquisition needed.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Claude Code visual artifacts | Persistent flickering on every frame | Zero flicker in normal operation | Month-1 | Manual testing: run Claude Code, observe rendering during active conversation |
| Claude Code mouse responsiveness | 0% mouse events reach application | 100% of mouse events forwarded when mouse mode active | Month-1 | Manual testing: click UI elements in Claude Code |
| Initial terminal size accuracy | 80×24 until first repaint (~4ms) | Correct from first frame | Month-1 | Run `tput cols; tput lines` immediately in a new terminal |
| Terminal identification | Not detected (`TERM_PROGRAM` absent) | Detected as `paneflow` by Claude Code, neofetch, etc. | Month-1 | Run `echo $TERM_PROGRAM` in PaneFlow terminal |
| TUI framework compatibility | Only basic VT100 apps render correctly | Ink, Textual, Ratatui, Bubbletea all render correctly | Month-6 | Test suite of representative TUI apps |

## Open Questions

- **Shift+Click override in mouse mode:** Standard terminals allow Shift+Click to bypass mouse mode and perform text selection. Should US-006 implement this convention? Engineering to decide during implementation — it's a UX nicety, not a protocol requirement.
- **XTVERSION exact format:** The XTVERSION spec has variations. Ghostty uses `DCS > | ghostty(version) ST`. xterm uses `DCS > | xterm(version) ST`. Should PaneFlow follow the same `name(version)` format? Recommended: yes, for consistency.
- **DEC 2026 nested/re-entrant behavior:** If a program sends BSU twice without ESU, should the second BSU reset the safety timer? Ghostty does NOT reset it. Recommended: match Ghostty's behavior — timer starts at first BSU, not reset by subsequent BSUs.

## Appendix: Cross-Codebase Diagnosis Reference

This section preserves the full diagnosis that informed this PRD. File:line references verified on 2026-04-15.

### Ink Rendering Pipeline
```
React reconciliation → Yoga layout (width = terminal columns)
→ Output grid (StyledChar[][] cells) → Grid serialization (multi-line string)
→ Frame diff (standard: full erase+rewrite, incremental: line-level patch)
→ Synchronized output wrapping (BSU/ESU when TTY && interactive)
→ Terminal write
```

Key files: `ink/ink.tsx:373-388` (BSU/ESU wrapping), `ink/log-update.ts:97` (eraseLines), `ink/write-synchronized.ts:4-5` (BSU/ESU constants), `ink/output.ts:305-318` (grid serialization).

### Claude Code Terminal Features
```
Alternate screen buffer: DEC 1049 (default fullscreen mode)
Mouse tracking: modes 1000 + 1002 + 1003 + 1006 (all simultaneously)
Synchronized output: DEC 2026 (gated by TERM_PROGRAM detection)
Focus events: DEC 1004
Bracketed paste: DEC 2004
Kitty keyboard: CSI >1u (allowlisted terminals only)
XTVERSION probe: CSI >0q (async detection)
```

Key files: `ink/termio/dec.ts:37-60` (all DEC mode constants), `ink/terminal.ts:70-118` (capability detection by TERM_PROGRAM), `ink/terminal.ts:156-163` (Kitty keyboard allowlist: iTerm, kitty, WezTerm, ghostty, tmux, windows-terminal).

### PaneFlow Gaps (confirmed by source)
```
DEC 2026:    SyncUpdate => ()     (alacritty fork term/mod.rs:1992,2041)
             Wakeup unconditional  (terminal.rs:1185)
Mouse:       TermMode::MOUSE_* never checked (zero references in src-app/)
             Handlers only do selection (terminal.rs:1009-1078)
Identity:    No TERM_PROGRAM set  (terminal.rs:306-313)
Focus:       No DEC 1004 forwarding (zero FocusIn/FocusOut references)
PTY size:    Hardcoded 80×24      (terminal.rs:262-263)
```

### Ghostty Reference Implementation
```
DEC 2026:    Renderer gate skips frame (renderer/generic.zig:1176-1179)
             Safety timer 1000ms  (Thread.zig:37)
             Resize clears sync   (Termio.zig:507-508)
Mouse:       Full SGR + X10       (all modes in modes.zig:270-283)
Identity:    TERM_PROGRAM=ghostty, TERM=xterm-ghostty
Focus:       DEC 1004 in modes.zig:277
XTVERSION:   DCS > | ghostty(version) ST
```

### Zed Terminal Mouse Reference
```
File: zed/crates/terminal/src/mappings/mouse.rs
- mouse_report(point, button, pressed, modifiers, format) -> Option<Vec<u8>>   :239
- sgr_mouse_report(point, button, pressed) -> String                            :307
- normal_mouse_report(point, button, utf8) -> Option<Vec<u8>>                   :275
- MouseFormat::from_mode(TermMode) selects SGR vs Normal encoding
```

[/PRD]
