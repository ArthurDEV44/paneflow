[PRD]
# PRD: Zed Terminal Parity — VT Protocol, Input, and Rendering Compliance

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-15 | Claude + Arthur | Initial draft from 52-divergence audit against Zed's terminal implementation |

## Problem Statement

PaneFlow's terminal emulator diverges from Zed's reference implementation in 52 documented areas, causing three categories of breakage:

1. **Common TUI programs are broken** — `less`, `htop`, `man`, and `vim` cannot scroll via mouse wheel because ALTERNATE_SCROLL mode (DECSET 1007) is never checked. Mouse release events in Normal/UTF-8 encoding send wrong button codes. `Ctrl+Backspace` (word-delete) and `Alt+Backspace` (backward-word-kill) are silently swallowed. These are daily-use workflows that fail without visible error.

2. **Terminal protocol compliance is incomplete** — OSC 52 clipboard events are silently discarded (breaks Neovim `"+y`/`"+p` over SSH, tmux clipboard sync). OSC 7 CWD data from shell integration is parsed by VTE but never stored (the `current_cwd` field is always `None`). OSC 10/11/12 color queries get no response (breaks TUI apps that detect light/dark themes). Bell events are silently dropped. No hyperlink support (OSC 8 or regex URL detection).

3. **Input pipeline has gaps** — 13 keystroke mappings are missing (Ctrl+Shift+letter, modifier+F-keys, Alt+Enter, Shift+Enter, etc.). GPUI dispatch context lacks terminal mode flags (`screen`, `DECCKM`, `bracketed_paste`, `mouse_reporting`), making it impossible to scope keybindings by terminal state. IME composition for CJK input is unimplemented. Linux primary selection (middle-click paste) is absent.

**Why now:** PaneFlow's architecture is complete (19 stories delivered across two PRDs, stabilization PRD in progress). The terminal functions — it spawns shells, renders output, handles basic input. But the gap between "functions" and "correctly implements the VT protocol" is what prevents adoption. Users who try PaneFlow with their real workflows hit these issues within minutes. The Zed deep dive document (`ZED_TERMINAL_DEEP_DIVE.md`) provides an exact implementation reference for every divergence.

## Overview

This PRD closes all 52 gaps between PaneFlow's terminal and Zed's terminal, organized into two release phases across eight epics.

**Release 1 (EP-001 through EP-003)** fixes the 6 critical issues and the highest-impact major gaps: complete keystroke mapping, mouse protocol fixes, alternate scroll mode, CWD tracking, OSC 52 clipboard, middle-click paste, image paste forwarding, key context enrichment, environment variables, and process lifecycle. After Release 1, all common TUI programs work correctly.

**Release 2 (EP-004 through EP-008)** adds hyperlink detection (OSC 8 + regex), IME composition, rendering polish (theme-aware colors, background merging, ligature control), search regex, OSC 133 prompt marks, display-only terminal, drag-and-drop, and performance optimizations. After Release 2, PaneFlow matches Zed's terminal feature set.

Key decisions:
- **Follow Zed exactly** — same escape sequences, same AlacEvent handling patterns, same GPUI Element conventions. The deep dive document is the implementation spec.
- **OSC 52 defaults to write-only** — clipboard read requires explicit opt-in (`osc52: "copy-paste"` in config). Write-only prevents clipboard exfiltration by malicious terminal applications.
- **OSC 8 allowlists URL schemes** — only `http`, `https`, `mailto`, and `file` (with localhost validation) are opened. Arbitrary schemes like `x-callback://` are blocked to prevent code execution.
- **IME depends on GPUI's Wayland backend** — if `zwp_text_input_v3` preedit events are already surfaced, implement rendering. If not, file upstream and implement a fallback.

## Goals

| Goal | Month-1 Target | Month-6 Target |
|------|---------------|----------------|
| TUI app compatibility | vim, tmux, less, htop, Claude Code all work correctly | Pass esctest2 automated VT compliance suite |
| Keystroke coverage | 100% of Zed's `to_esc_str()` mappings implemented | Zero silent keystroke drops for any modifier combo |
| Protocol compliance | OSC 7, OSC 52, OSC 10/11/12 all handled | OSC 8, OSC 133, full mouse protocol, IME all working |
| Rendering fidelity | Theme-aware selection/scrollbar, ligatures disabled | Pixel-identical to Zed for all test scripts |

## Target Users

### TUI Power User
- **Role:** Developer running terminal-intensive workflows — neovim, tmux, lazygit, Claude Code, htop
- **Behaviors:** Relies on Ctrl+Backspace for word-delete, Alt+Backspace for backward-kill-word, mouse scroll in `less`/`man`, clipboard sync over SSH via OSC 52
- **Pain points:** These keystrokes and scroll behaviors silently fail in PaneFlow. Forced to use a different terminal for real work.
- **Current workaround:** Uses Alacritty, WezTerm, or Kitty for TUI work; PaneFlow only for basic shell commands
- **Success looks like:** PaneFlow replaces their daily terminal — all TUI apps work identically to Zed/Alacritty

### International User (CJK/Accent Input)
- **Role:** Developer who types in Chinese, Japanese, Korean, or uses accented characters (French, Spanish, German)
- **Behaviors:** Uses system IME (ibus, fcitx5) for CJK composition; expects preedit text rendered inline at cursor
- **Pain points:** IME composition is completely broken — no preedit rendering, no composition events forwarded
- **Current workaround:** Cannot use PaneFlow for any work requiring non-ASCII input
- **Success looks like:** IME preedit renders inline, composition commits to PTY, accent dead keys work

## Research Findings

Key findings that informed this PRD:

### Competitive Context
- **Ghostty/Kitty:** Top-tier VT compliance (VT525). Reference implementations for mouse protocol, OSC 8, Unicode width handling.
- **Alacritty:** Only VT102 compliance — deliberately minimal. Zed's fork adds patches but remains below WezTerm/Kitty.
- **WezTerm:** Solid VT compliance, excellent Lua config, reference IME implementation. Alternate scroll synthesizes 3 arrow keys per wheel tick.
- **Market gap:** No GPU-accelerated terminal multiplexer with Zed-level TUI support on Linux. PaneFlow fills this if protocol compliance is achieved.

### Best Practices Applied
- SGR mouse mode (DECSET 1006) is the mandatory modern standard — Normal mode release must send button code 3
- OSC 52 write-only by default (Alacritty's `OnlyCopy` pattern) — read path is a security risk
- OSC 8 hyperlinks require URL scheme allowlisting — arbitrary schemes enable code execution (iTerm2/Hyper CVE precedent)
- DECSET 1007 (alternate scroll) must synthesize arrow keys in APP_CURSOR mode — `\x1bOA`/`\x1bOB`

*Full research: 25 sources including xterm ctlseqs reference, jeffquast 2025 terminal compliance report, egmontkob OSC 8 spec, WezTerm IME docs.*

## Assumptions & Constraints

### Assumptions (validated)
- ~~Zed's Alacritty fork (rev `9d9640d4`) exposes `AlacEvent::ClipboardStore` and `AlacEvent::ClipboardLoad` variants~~ — **VALIDATED:** Both present at `event.rs:24-30` with `Osc52` config enum
- ~~GPUI's Wayland backend surfaces `zwp_text_input_v3` preedit events via the `InputHandler` trait~~ — **VALIDATED:** Full pipeline in `gpui_linux/wayland/client.rs`, trait at `platform.rs:1272`
- ~~`alacritty_terminal::Term::renderable_content()` includes cell-level `hyperlink()` attribute from OSC 8~~ — **VALIDATED:** `Cell::hyperlink()` at `term/cell.rs:219`, `Hyperlink` struct with `.id()` and `.uri()` accessors, stored in `CellExtra`
- OSC 7 is NOT exposed by the fork — requires a pre-VTE byte scanner in the reader loop (validated, approach documented)

### Hard Constraints
- Must use Zed's Alacritty fork (`rev = "9d9640d4"`) — no switching to upstream or another VTE parser
- GPUI is a local path dependency at `/home/arthur/dev/zed` — no crates.io version available
- Linux-only (Wayland + X11) — no macOS/Windows considerations
- Must not regress any feature from prd-stabilization-polish (search, copy mode, pane zoom, etc.)

## Quality Gates

These commands must pass for every user story:
- `cargo build` - compilation succeeds
- `cargo clippy --workspace -- -D warnings` - zero clippy warnings
- `cargo test --workspace` - all tests pass
- `cargo fmt --check` - formatting correct

For TUI stories, additional gates:
- Verify in a running PaneFlow instance with: vim (cursor/scroll), tmux (mouse mode), less (scroll), htop (mouse clicks), `printf '\e[?1049h\e[?1007h'` (alternate scroll test)
- For keystroke stories: verify with `cat -v` or `showkey -a` that correct bytes are sent
- For OSC stories: verify with test scripts (e.g., `printf '\e]52;c;dGVzdA==\a'` for OSC 52)

---

## Epics & User Stories

---

### EP-001: Input Pipeline Parity

Complete keystroke-to-escape-sequence coverage matching Zed's `to_esc_str()` function. After this epic, every modifier+key combination produces the correct VT escape sequence.

**Definition of Done:** All keystroke mappings from Zed's `keys.rs` (240 lines) are implemented in PaneFlow's `keys.rs`. `cat -v` confirms correct bytes for every mapped combination.

#### US-001: Complete core keystroke mapping
**Description:** As a terminal user, I want Ctrl+Shift+letter, Ctrl+Backspace, Alt+Backspace, Ctrl+Space, Shift+Enter, Alt+Enter, Ctrl+@, and Ctrl+? to produce correct escape sequences so that word-delete, backward-kill-word, set-mark, and line-feed work in shells and editors.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `Ctrl+Shift+A` through `Ctrl+Shift+Z` map to `\x01` through `\x1a` (same as Ctrl+letter without Shift) — remove the `!shift` guard in `keys.rs:23`
- [ ] `Ctrl+Backspace` maps to `\x08` (BS control character)
- [ ] `Alt+Backspace` maps to `\x1b\x7f` (ESC + DEL)
- [ ] `Ctrl+Space` maps to `\x00` (NUL)
- [ ] `Shift+Enter` maps to `\x0a` (LF)
- [ ] `Alt+Enter` maps to `\x1b\x0d` (ESC + CR)
- [ ] `Ctrl+@` maps to `\x00` (NUL)
- [ ] `Ctrl+?` maps to `\x7f` (DEL)
- [ ] Given `cat -v` running in PaneFlow, when pressing each combo above, then the correct control character is displayed
- [ ] Given an unrecognized modifier+key combo, when pressed, then it falls through to printable character handling (no panic, no silent swallow)

#### US-002: Extended keystroke mapping — modifier+function and modifier+navigation
**Description:** As a terminal user, I want Shift/Alt/Ctrl+F1–F12, modifier+Insert/PageUp/PageDown, Shift+Home/End on alt screen, F13–F20, and Alt+Shift+letter to produce correct CSI escape sequences so that function key shortcuts work in vim, tmux, and other TUI apps.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Shift+F1 maps to `\x1b[1;2P`, Alt+F1 to `\x1b[1;3P`, Ctrl+F1 to `\x1b[1;5P` (modifier code N in position 4)
- [ ] Same pattern for F2 (`Q`), F3 (`R`), F4 (`S`), and F5–F12 using `\x1b[{num};{N}~` format
- [ ] Shift+Insert maps to `\x1b[2;2~`, Ctrl+PageUp to `\x1b[5;5~`, Ctrl+PageDown to `\x1b[6;5~`
- [ ] On alt screen: Shift+Home maps to `\x1b[1;2H`, Shift+End to `\x1b[1;2F`, Shift+PageUp to `\x1b[5;2~`, Shift+PageDown to `\x1b[6;2~`
- [ ] F13 through F20 map to `\x1b[25~` through `\x1b[34~` respectively
- [ ] Alt+Shift+letter maps to `\x1b` + uppercase letter (e.g., Alt+Shift+A → `\x1b` + `A`)
- [ ] Given vim running with custom F-key mappings, when pressing Shift+F5, then vim receives the correct `\x1b[15;2~` sequence
- [ ] Given an invalid modifier+key combo (e.g., Ctrl+Shift+F13), when pressed, then it is silently ignored (not sent as garbage bytes)

#### US-003: Alternate scroll mode and mouse release fix
**Description:** As a terminal user, I want scroll wheel events in ALT_SCREEN to send arrow key sequences (when ALTERNATE_SCROLL is active and mouse reporting is off), and I want mouse release events in Normal/UTF-8 encoding to send button code 3 so that scrolling works in less/vim/htop and mouse-aware TUI apps receive correct release events.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] When `TermMode::ALT_SCREEN` and `TermMode::ALTERNATE_SCROLL` are set and `TermMode::MOUSE_MODE` is NOT set, scroll wheel up sends `\x1bOA` per line (APP_CURSOR) or `\x1b[A` (normal), scroll wheel down sends `\x1bOB`/`\x1b[B`
- [ ] Number of arrow key repeats equals number of accumulated scroll lines (matching Zed's behavior)
- [ ] When `MOUSE_MODE` IS set alongside `ALTERNATE_SCROLL`, scroll events are forwarded as mouse button 64/65 reports (mouse mode takes priority)
- [ ] In Normal mouse encoding (not SGR), mouse release events send button code 3 (not the original button code) — fix `terminal.rs` mouse release path
- [ ] In SGR mouse encoding, release continues to use `m` suffix with original button code (already correct, verify no regression)
- [ ] Given `less` running (activates ALT_SCREEN + ALTERNATE_SCROLL), when scrolling mouse wheel down 3 lines, then less scrolls down 3 lines
- [ ] Given `htop` running, when scrolling mouse wheel, then process list scrolls correctly
- [ ] Given a TUI app using Normal mouse mode, when clicking and releasing left button, then the app receives button 0 press and button 3 release

#### US-004: Middle-click paste and Linux primary selection
**Description:** As a Linux user, I want middle-click to paste from the primary selection, and I want text selections to automatically update the primary selection so that the standard X11/Wayland select-to-copy, middle-click-to-paste workflow functions correctly.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] On every text selection completion (mouse up after drag, double-click word select, triple-click line select), `cx.write_to_primary_clipboard()` is called with the selected text
- [ ] Middle mouse button click reads from primary clipboard via `cx.read_from_primary_clipboard()` and pastes the text
- [ ] Middle-click paste respects `TermMode::BRACKETED_PASTE` — wraps content in `\x1b[200~...\x1b[201~` when active, strips ESC characters
- [ ] When mouse mode is active and Shift is not held, middle-click is forwarded to PTY as button 1 mouse event (standard middle button)
- [ ] When mouse mode is active and Shift IS held, middle-click performs primary selection paste (Shift override)
- [ ] Given text selected in terminal A, when middle-clicking in terminal B, then the selected text from A is pasted into B

#### US-005: Image paste forwarding for TUI agents
**Description:** As a user of Claude Code or Codex CLI running inside PaneFlow, I want image paste (Ctrl+V with image in clipboard) to be forwarded as raw Ctrl+V (byte 0x16) to the PTY so that TUI agents can access the clipboard image directly.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] When clipboard contains an image entry (`ClipboardEntry::Image` with non-empty bytes), paste action sends byte `0x16` (raw Ctrl+V) to PTY instead of pasting text
- [ ] When clipboard contains only text, paste action continues to work normally (bracketed paste when enabled)
- [ ] When clipboard contains both image and text, image takes priority (raw Ctrl+V sent)
- [ ] Given Claude Code running in PaneFlow with an image in clipboard, when pressing Ctrl+Shift+V, then Claude Code receives the Ctrl+V signal and reads the clipboard image itself
- [ ] Given no image in clipboard, when pressing Ctrl+Shift+V, then normal text paste occurs (no regression)

---

### EP-002: Terminal Protocol Parity

Handle all AlacEvent variants that Zed handles, and fix OSC 7 CWD tracking. After this epic, terminal protocol compliance matches Zed for clipboard, color queries, CWD tracking, and dispatch context.

**Definition of Done:** All AlacEvent variants produce the same behavior as Zed's `Terminal::process_event()`. CWD tracking works end-to-end.

#### US-006: Fix OSC 7 CWD tracking
**Description:** As a terminal user, I want PaneFlow to track the current working directory of the shell via OSC 7 so that new panes open in the same directory and tab titles can reflect the CWD.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] An `Osc7Scanner` (byte-level state machine, same pattern as existing `XtversionScanner` at `terminal.rs:1295-1335`) intercepts OSC 7 sequences in `pty_reader_loop` before VTE processing — since the Alacritty fork silently ignores OSC 7
- [ ] The scanner matches `\x1b]7;file://[hostname]/path` terminated by ST (`\x1b\\`) or BEL (`\x07`)
- [ ] OSC 7 parsing handles both `file:///path` (empty hostname) and `file://hostname/path` URI formats
- [ ] Percent-encoded path components are decoded (e.g., `%20` → space)
- [ ] The `current_cwd` field in `TerminalState` is updated via an `AlacEvent::PtyWrite`-like mechanism or a dedicated channel from the reader loop
- [ ] `TerminalEvent::CwdChanged(path)` is emitted when CWD changes
- [ ] `PaneFlowApp::handle_cwd_change()` receives the event and updates the workspace CWD
- [ ] Shell integration scripts (zsh, bash, fish) continue to emit OSC 7 on directory change (verify existing scripts)
- [ ] Given `cd /tmp/test dir` in zsh, when the prompt renders, then `current_cwd` equals `/tmp/test dir`
- [ ] Given OSC 7 is not emitted (e.g., `dash` shell), when checking CWD, then fallback to `/proc/{pid}/cwd` still works

#### US-007: OSC 52 clipboard read/write
**Description:** As a terminal user, I want programs to read from and write to my system clipboard via OSC 52 escape sequences so that Neovim `"+y`/`"+p` works over SSH and tmux clipboard sync functions correctly.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `AlacEvent::ClipboardStore(selection, text)` writes `text` to system clipboard via `cx.write_to_clipboard()`
- [ ] `AlacEvent::ClipboardLoad(selection, format_fn)` reads clipboard text, base64-encodes it, calls `format_fn` to create the OSC 52 response, and writes the response to PTY
- [ ] Default mode is write-only (`OnlyCopy`) — ClipboardLoad is silently ignored unless config enables read
- [ ] Config option `osc52` with values `"disabled"`, `"copy-only"` (default), `"copy-paste"` controls behavior
- [ ] ESC characters in clipboard content are stripped before base64 encoding (prevents escape injection on read)
- [ ] Given Neovim over SSH with `clipboard=unnamedplus`, when yanking text with `"+y`, then the text appears in PaneFlow's system clipboard
- [ ] Given `osc52 = "copy-only"`, when a program sends OSC 52 read query (`\e]52;c;?\a`), then no response is sent to PTY (silent ignore)
- [ ] Given `osc52 = "disabled"`, when a program sends OSC 52 write, then clipboard is NOT updated

#### US-008: ColorRequest (OSC 10/11/12) theme-aware response
**Description:** As a TUI app developer, I want PaneFlow to respond to OSC 10 (foreground), OSC 11 (background), and OSC 12 (cursor) color queries with the active theme's colors so that apps like bat, delta, and vivid can detect light/dark mode and adjust their color scheme.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `AlacEvent::ColorRequest(index, format_fn)` is handled in `sync()` — looks up the color for the given index from the active `TerminalTheme`, calls `format_fn(color)` to produce the response string, and writes to PTY
- [ ] Index mapping: foreground → `theme.foreground`, background → `theme.ansi_background`, cursor → `theme.cursor`
- [ ] Color format in response matches xterm convention: `rgb:RRRR/GGGG/BBBB` (16-bit per channel)
- [ ] Given `printf '\e]11;?\a'` in PaneFlow, when the theme background is Catppuccin Mocha (#1e1e2e), then the response contains the correct RGB values
- [ ] Given theme changes while a TUI is running, when the TUI re-queries colors, then the new theme colors are returned

#### US-009: Handle Bell, CursorBlinkingChange, and TextAreaSizeRequest events
**Description:** As a terminal user, I want bell events to produce a notification, cursor blink changes from TUI apps to be respected, and text area size requests to be answered so that minor protocol compliance gaps are closed.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `AlacEvent::Bell` emits a `TerminalEvent::Bell` that triggers a visual flash (200ms background color pulse) on the terminal view
- [ ] Bell does NOT produce audio by default — audio bell is a config option for future implementation
- [ ] `AlacEvent::CursorBlinkingChange(blinking)` updates the blink state — when `blinking = false`, cursor stays visible; when `blinking = true`, blink timer activates (if user hasn't disabled blink)
- [ ] `AlacEvent::TextAreaSizeRequest(format_fn)` responds with current terminal dimensions (cols, rows, pixel width, pixel height) via PTY write
- [ ] Given `printf '\a'` in a terminal tab that is NOT focused, when the bell fires, then the tab title or workspace indicator shows a visual alert
- [ ] Given `printf '\a'` in a focused terminal, when the bell fires, then a subtle background flash is visible

#### US-010: Key context enrichment for GPUI dispatch
**Description:** As a keybinding author, I want the terminal to set rich key context flags (`screen`, `DECCKM`, `bracketed_paste`, `mouse_reporting`, `mouse_format`) so that keybindings can be scoped to specific terminal states (e.g., different behavior on alt screen vs normal screen).

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `screen` key context is set to `"alt"` when `TermMode::ALT_SCREEN` is active, `"normal"` otherwise
- [ ] `DECCKM` key context is added when `TermMode::APP_CURSOR` is active
- [ ] `DECPAM` key context is added when `TermMode::APP_KEYPAD` is active
- [ ] `bracketed_paste` key context is added when `TermMode::BRACKETED_PASTE` is active
- [ ] `any_mouse_reporting` key context is added when any `TermMode::MOUSE_MODE` flag is set
- [ ] `mouse_reporting` key context is set to `"click"`, `"drag"`, `"motion"`, or `"off"` based on active mouse mode
- [ ] `mouse_format` key context is set to `"sgr"`, `"utf8"`, or `"normal"` based on active mouse format
- [ ] `report_focus` key context is added when `TermMode::FOCUS_IN_OUT` is active
- [ ] `alternate_scroll` key context is added when `TermMode::ALTERNATE_SCROLL` is active
- [ ] Key context values are read from the locked `Term` mode during `render()` or `prepaint()`, same phase as existing `"Terminal"` context
- [ ] Given vim running (ALT_SCREEN + APP_CURSOR), when inspecting key context, then `screen == "alt"` and `DECCKM` is present
- [ ] Given a keybinding scoped to `"Terminal && screen == alt"`, when on normal screen, then the binding does not fire

---

### EP-003: Environment & Process Lifecycle

Fix environment variable setup and process shutdown to match Zed's behavior. After this epic, shells launch with correct environment and processes terminate cleanly.

**Definition of Done:** Environment variables match Zed's `TerminalBuilder::new()` setup. Process shutdown includes grace period and force kill.

#### US-011: Environment variable parity
**Description:** As a terminal user, I want PaneFlow to set `TERM=xterm-256color` explicitly, remove `SHLVL` from the child environment, and set `LANG=en_US.UTF-8` as fallback so that shells and TUI apps detect capabilities correctly and SHLVL doesn't inflate.

**Priority:** P1
**Size:** XS (1 pt)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `TERM` is explicitly set to `xterm-256color` in the child environment (not relying on portable-pty's default)
- [ ] `SHLVL` is removed from the inherited environment before spawning the child process
- [ ] If `LANG` is not set or is empty in the parent environment, set `LANG=en_US.UTF-8`
- [ ] Given PaneFlow launched from a shell with `SHLVL=2`, when checking `echo $SHLVL` in PaneFlow's terminal, then the value is `1` (not `3`)
- [ ] Given a Docker container with no `LANG` set, when running PaneFlow, then `locale` shows `en_US.UTF-8`

#### US-012: Shutdown grace period with SIGKILL
**Description:** As a terminal user, I want terminal shutdown to include a 100ms grace period followed by forced kill so that processes that ignore SIGHUP don't become orphans.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `Drop` impl for `TerminalState` sends `Msg::Shutdown`, then spawns a background task that waits 100ms and sends SIGKILL to the child process
- [ ] Child PID is stored and used for the SIGKILL (not relying on PTY fd close alone)
- [ ] If the process exits cleanly within the 100ms window, SIGKILL is not sent
- [ ] Given a shell running `sleep 999 &; trap '' HUP; cat` (ignores SIGHUP), when closing the pane, then the process is killed within 200ms
- [ ] Given a normal shell session, when closing the pane, then the shell exits cleanly via SIGHUP (SIGKILL not needed)

#### US-013: ALT_SCREEN cursor override and ligature disable
**Description:** As a TUI user, I want the cursor to always be visible when in ALT_SCREEN mode (no blink-off state), and I want font ligatures to be disabled by default so that monospace alignment is preserved in terminal output.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] When `TermMode::ALT_SCREEN` is active, cursor is always painted regardless of blink timer state
- [ ] When ALT_SCREEN is exited, normal blink behavior resumes
- [ ] Font is created with `FontFeatures::disable_ligatures()` instead of `Default::default()` in `terminal_element.rs`
- [ ] Ligature characters (`fi`, `fl`, `->`, `=>`, `!=`) render as separate monospace characters, not joined glyphs
- [ ] Given vim running (ALT_SCREEN), when the blink timer would hide the cursor, then the cursor remains visible
- [ ] Given code with `->` operator displayed in terminal, when using a ligature-capable font, then `->` renders as two separate characters occupying two cells

---

### EP-004: Hyperlink System

Implement URL detection and OSC 8 hyperlink support matching Zed's hyperlink pipeline. After this epic, URLs are clickable in the terminal.

**Definition of Done:** Cmd/Ctrl+hover over URLs shows underlined link; Cmd/Ctrl+click opens in browser. OSC 8 explicit hyperlinks have priority over regex detection.

#### US-014: OSC 8 hyperlink detection from cell attributes
**Description:** As a terminal user, I want programs that emit OSC 8 hyperlinks to have those links detected and stored per-cell so that explicit hyperlinks from modern CLI tools are recognized.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] During cell iteration in `terminal_element.rs`, read `cell.hyperlink()` attribute from Alacritty cells
- [ ] Store hyperlink URL and `id` parameter per cell in the layout state
- [ ] OSC 8 hyperlinks take priority over regex-detected URLs when both match the same cell range
- [ ] URL scheme is validated against allowlist: `http`, `https`, `mailto`, `file` — others are stored but not openable
- [ ] `file://` URLs validate that hostname matches `localhost` or is empty (security: prevent accessing remote files)
- [ ] Given `printf '\e]8;;https://example.com\e\\Click here\e]8;;\e\\'`, when the text renders, then cells "Click here" carry the hyperlink attribute
- [ ] Given a malicious `terminal://evil` OSC 8 link, when Ctrl+clicking, then nothing happens (scheme blocked)

#### US-015: URL regex detection with box-drawing exclusion
**Description:** As a terminal user, I want bare URLs in terminal output (http://, https://, file://, etc.) to be detected by regex so that links printed without OSC 8 are still clickable.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** US-014

**Acceptance Criteria:**
- [ ] URL regex matches protocols: `https://`, `http://`, `git://`, `ssh:`, `ftp://`, `file://`, `mailto:`, matching Zed's `terminal_hyperlinks.rs`
- [ ] Box-drawing characters (`U+2500`–`U+257F`) are excluded from the regex first-character match to prevent TUI frames from triggering false positives
- [ ] Path hyperlink regexes are configurable via `terminal.path_hyperlink_regexes` setting (optional, empty by default)
- [ ] Regex detection runs on the line text at the mouse cursor position during hover (not on every frame)
- [ ] Given `echo "Visit https://example.com for info"`, when hovering over the URL with Ctrl held, then the URL is detected
- [ ] Given a TUI with box-drawing frame `┌──────┐`, when hovering over the frame, then no hyperlink is detected
- [ ] Given a wrapped URL spanning two lines, when the URL has an OSC 8 `id` parameter, then both line segments are linked

#### US-016: Hyperlink hover rendering and click-to-open
**Description:** As a terminal user, I want URLs to show as underlined blue text when hovering with Ctrl/Cmd held, and I want Ctrl/Cmd+click to open the URL in my default browser.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** US-014, US-015

**Acceptance Criteria:**
- [ ] When Ctrl is held and mouse hovers over a detected hyperlink, the link text renders with underline and `link_text_color` (blue) from theme
- [ ] Mouse cursor changes to pointing hand when over a hyperlink with Ctrl held
- [ ] Ctrl+click on a detected URL opens it via `open::that()` or equivalent system URL opener
- [ ] A tooltip showing the full URL appears near the cursor when hovering over a link
- [ ] When Ctrl is released, link styling reverts to normal terminal colors
- [ ] Given a long URL, when hovering, then the tooltip shows the full URL (not truncated)
- [ ] Given Ctrl+click on a `file:///home/user/code.rs` link, when the file exists, then it opens in the default handler
- [ ] Given Ctrl+click on a `javascript:alert(1)` link, when clicked, then nothing happens (scheme not in allowlist)

---

### EP-005: IME & Agent Interaction

Implement IME composition rendering and terminal input actions for agent interaction. After this epic, CJK input works and agents can send keystrokes to terminals.

**Definition of Done:** IME preedit text renders inline at cursor position. SendText action forwards strings to running terminals.

#### US-017: IME composition rendering
**Description:** As a CJK user, I want IME preedit (in-progress composition) text to render inline at the terminal cursor position with an underline indicator so that I can see what I'm composing before committing to the PTY.

**Priority:** P1
**Size:** L (5 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Register a `TerminalInputHandler` via `window.handle_input()` in `TerminalElement::paint()`
- [ ] `InputHandler::marked_text_range()` returns the preedit text range when IME is composing
- [ ] Preedit text renders at the current cursor position with an underline style indicator
- [ ] A background quad covers the terminal content behind the preedit text (prevents overlap)
- [ ] When composition is confirmed (committed), the final text is written to PTY and preedit overlay disappears
- [ ] When composition is cancelled (Escape), preedit overlay disappears with no PTY write
- [ ] IME popup (candidate window) positions near the cursor based on `cursor_bounds` reported to the input handler
- [ ] When `TermMode::ALT_SCREEN` is active, IME is disabled (keystrokes go directly to PTY) — matching Zed's behavior
- [ ] Given ibus with Pinyin input active, when typing `zhong`, then preedit shows `zhong` with underline at cursor; when selecting `中`, then `中` is written to PTY
- [ ] Given IME composition in progress, when pressing Escape, then composition cancels cleanly with no garbage bytes sent to PTY

#### US-018: SendText and SendKeystroke actions
**Description:** As an AI agent or automation tool, I want to send text and keystrokes to a running terminal session via IPC so that agents can interact with TUI programs.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] New `SendText` action that writes arbitrary text bytes to a terminal's PTY (no bracketed paste wrapping)
- [ ] New `SendKeystroke` action that converts a keystroke description (e.g., `"ctrl-c"`, `"enter"`) to escape sequence via `to_esc_str()` and writes to PTY
- [ ] IPC method `surface.send_text(surface_id, text)` dispatches `SendText` to the correct terminal
- [ ] IPC method `surface.send_keystroke(surface_id, keystroke)` dispatches `SendKeystroke` to the correct terminal
- [ ] Given a running `cat` process in a terminal, when calling `surface.send_text(id, "hello\n")`, then `hello` appears followed by a newline
- [ ] Given an invalid surface_id, when calling `surface.send_text`, then a JSON-RPC error is returned (not a panic)

---

### EP-006: Rendering Polish

Align rendering details with Zed — theme-aware colors, background merging, decorative character ranges. After this epic, rendering matches Zed's visual output.

**Definition of Done:** Selection and scrollbar use theme colors. Background regions merge vertically. Powerline range matches Zed exactly.

#### US-019: Theme-aware selection and scrollbar colors
**Description:** As a terminal user, I want selection highlighting and the scrollbar indicator to use colors from the active theme instead of hardcoded values so that they integrate with any theme.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Selection color reads from `TerminalTheme` (add a `selection` field) or falls back to a computed semi-transparent variant of the theme's accent color
- [ ] Scrollbar thumb color reads from `TerminalTheme` (add a `scrollbar` field) or falls back to `theme.foreground` at 40% opacity
- [ ] Both colors update immediately when the theme changes (no restart needed)
- [ ] Given Catppuccin Mocha theme, when selecting text, then the selection highlight uses theme-consistent color (not hardcoded blue)
- [ ] Given a theme with light background, when scrollbar is visible, then the thumb is visible against the light background

#### US-020: Background region vertical merging and Powerline range fix
**Description:** As a terminal user, I want adjacent same-color background regions to merge vertically (reducing draw calls) and Powerline character ranges to exactly match Zed's 6 sub-ranges so that rendering is efficient and decorative characters are handled correctly.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Background region merging supports both horizontal (same row) and vertical (same column span, adjacent rows) merging — implement a two-pass algorithm matching Zed's `merge_background_regions()`
- [ ] Powerline decorative character ranges updated to Zed's exact 6 sub-ranges: `0xE0B0..=0xE0B7`, `0xE0B8..=0xE0BF`, `0xE0C0..=0xE0CA`, `0xE0CC..=0xE0D1`, `0xE0D2..=0xE0D7` (excluding `0xE0CB`)
- [ ] `is_decorative_character()` and `is_powerline_or_boxdraw()` both updated to use the new ranges
- [ ] Given a TUI app with a solid-color sidebar spanning 20 rows, when rendering, then the sidebar is painted as fewer merged rectangles (not 20 separate row rects)
- [ ] Given Powerline prompt with Nerd Font symbols, when rendering, then decorative chars bypass contrast enforcement correctly

#### US-021: Rounded selection corners and hollow cursor refinement
**Description:** As a terminal user, I want text selections to have optional rounded corners and the hollow (unfocused) cursor to render as a single rounded-rect outline so that visual polish matches Zed's terminal.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Selection highlight rectangles support a configurable corner radius (default: `0.15 * line_height`)
- [ ] Corner radius can be set to 0 for sharp corners (user preference)
- [ ] Hollow cursor renders as a single rounded-rect outline instead of 4 separate edge quads
- [ ] Hollow cursor line thickness is 1.5px, matching current implementation
- [ ] Given an unfocused terminal with a block cursor, when viewing the cursor, then it appears as a single smooth outline (no visible corner gaps from 4 separate quads)
- [ ] Given selected text spanning 3 lines, when viewing the selection, then corners are subtly rounded

---

### EP-007: Advanced Protocol Features

Add OSC 133 prompt marks, search regex, display-only terminal, and drag-and-drop. After this epic, PaneFlow supports all Zed terminal features.

**Definition of Done:** Shell prompt boundaries are detected. Search supports regex. Display-only terminals can render ANSI content without a PTY.

#### US-022: Search regex support
**Description:** As a terminal user, I want to toggle between plain text and regex search modes in the terminal search overlay so that I can find patterns in scrollback history.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None (builds on existing search from stabilization PRD)

**Acceptance Criteria:**
- [ ] Search overlay has a toggle button or keybinding to switch between plain text and regex mode
- [ ] In regex mode, the search query is compiled as a `regex::Regex` and matched against each line
- [ ] Invalid regex shows an inline error indicator (red border or tooltip) without crashing
- [ ] Search results (match positions) are highlighted identically to plain text search
- [ ] Given regex `\d{3,}` in search, when scrollback contains "error code 12345", then "12345" is highlighted as a match
- [ ] Given invalid regex `[unterminated`, when typed, then error indicator appears but no panic

#### US-023: OSC 133 shell prompt marks
**Description:** As a terminal user, I want PaneFlow to detect shell prompt boundaries (OSC 133 A/B/C/D sequences) so that future features like jump-to-prompt and command region detection are possible.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] OSC 133 A (prompt start), B (prompt end/command start), C (command end/output start), D (output end) sequences are detected during VTE processing
- [ ] Prompt boundaries are stored as a list of `PromptMark { line: i32, kind: A|B|C|D }` in the terminal state
- [ ] Shell integration scripts (zsh, bash, fish) are extended to emit OSC 133 sequences at prompt boundaries
- [ ] A `jump_to_previous_prompt` and `jump_to_next_prompt` action scrolls to the nearest prompt mark
- [ ] Given a shell session with 5 commands executed, when pressing the jump-to-previous-prompt keybinding 3 times, then the viewport scrolls to the 3rd-from-last prompt
- [ ] Given a shell that does NOT emit OSC 133 (e.g., dash), when the prompt mark list is empty, then jump actions are no-ops

#### US-024: Display-only terminal mode
**Description:** As a developer building an agent panel, I want to create a terminal instance without a PTY that can render ANSI-formatted content via direct VTE processing so that AI agent output can be displayed with full color/formatting support.

**Priority:** P2
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `TerminalState::new_display_only(rows, cols)` creates a terminal with no PTY, no reader thread, no message loop
- [ ] `write_output(&mut self, bytes: &[u8])` processes bytes through VTE directly — converts `\n` to `\r\n` (no PTY to do CR insertion)
- [ ] Display-only terminal supports full ANSI rendering: colors, bold/italic, underline, cursor movement
- [ ] Display-only terminal has scrollback history (configurable size)
- [ ] Display-only terminal does NOT accept keyboard input (read-only)
- [ ] Given ANSI-colored text written via `write_output`, when rendering, then colors display correctly
- [ ] Given `write_output(b"line1\nline2\n")`, when rendering, then both lines appear on separate rows (CR+LF)

#### US-025: Drag-and-drop file path paste
**Description:** As a terminal user, I want to drag files from my file manager and drop them onto the terminal to paste their paths so that I can quickly reference files in commands.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Dropping a file onto the terminal pastes its absolute path as text input to the PTY
- [ ] Paths containing spaces are automatically quoted (wrapped in single quotes)
- [ ] Dropping multiple files pastes space-separated quoted paths
- [ ] Dropped paths respect bracketed paste mode when active
- [ ] Given dragging `/home/user/my file.txt` from Nautilus onto the terminal, when dropped, then `'/home/user/my file.txt'` is pasted
- [ ] Given mouse mode active in a TUI app, when dropping a file, then the path paste overrides mouse mode (drop always pastes)

---

### EP-008: Performance & Housekeeping

Event batch optimization, viewport culling, terminal actions, and configuration for future platform support.

**Definition of Done:** Event batching matches Zed's `select_biased!` pattern. Viewport culling skips offscreen rows. ClearScrollHistory and ResetTerminal actions work.

#### US-026: Event batch coalescing with select_biased
**Description:** As a terminal user viewing fast output (e.g., `cat large_file.txt`), I want PTY events to be coalesced in a 4ms batch window with a max 100-event cap so that rendering performance matches Zed's approach.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Replace the current `try_recv` drain with Zed's `select_biased!` pattern: first event processed immediately, then batch for 4ms, max 100 events per batch
- [ ] Wakeup events during batching are deduplicated (only one `dirty = true` per batch)
- [ ] After processing a batch, `yield_now().await` is called to let other GPUI tasks run
- [ ] The adaptive 4ms/50ms idle timer from current implementation is preserved as an optimization on top of the batch window
- [ ] Given `cat /dev/urandom | head -c 1000000` running, when observing CPU usage, then rendering batches at ~250Hz (4ms interval) not at PTY byte rate
- [ ] Given a single keystroke echo, when typing, then the response appears within 4ms (first event immediate, no batch delay)

#### US-027: Viewport culling for embedded terminals
**Description:** As a developer embedding terminals in scrollable containers, I want offscreen terminal rows to be skipped during cell processing so that CPU is not wasted on invisible content.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] When terminal element's visible bounds (from content mask) are a subset of the full bounds, only rows within visible bounds are processed
- [ ] Three-path rendering: fully offscreen → empty layout, fully visible → process all cells (fast path), partially clipped → skip non-visible rows
- [ ] Row visibility check uses screen-relative Y position, not grid line numbers
- [ ] Given a terminal embedded in a scrollable agent panel, when the terminal is scrolled mostly offscreen, then only visible rows are processed in `build_layout()`
- [ ] Given a fully visible terminal, when rendering, then no extra computation is done (fast path)

#### US-028: ClearScrollHistory, ResetTerminal, and option_as_meta config
**Description:** As a terminal user, I want actions to clear scrollback history and reset the terminal state, plus a config option for Alt-as-Meta behavior so that I have full terminal control and future macOS support is possible.

**Priority:** P2
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] `ClearScrollHistory` action clears the terminal's scrollback buffer (keeps current screen content)
- [ ] `ResetTerminal` action sends a full reset sequence (`\x1bc` — ESC c, RIS) to the PTY and resets internal state
- [ ] `option_as_meta` config option (boolean, default `true` on Linux) controls whether Alt key is treated as Meta (ESC prefix) or passed as-is
- [ ] When `option_as_meta = false`, Alt+key sends the platform-native key event (relevant for future macOS where Option produces Unicode)
- [ ] Given a terminal with 5000 lines of scrollback, when executing ClearScrollHistory, then scrollback is empty but current screen content is preserved
- [ ] Given `option_as_meta = true`, when pressing Alt+x, then `\x1bx` is sent to PTY (current behavior, no regression)

---

## Functional Requirements

- FR-01: The system must map every keystroke combination that Zed's `to_esc_str()` handles to the same escape sequence byte-for-byte
- FR-02: When `TermMode::ALTERNATE_SCROLL` is active and `TermMode::MOUSE_MODE` is not, scroll wheel events must be translated to arrow key escape sequences
- FR-03: Mouse release events in Normal/UTF-8 encoding must send button code 3 (not the original button code)
- FR-04: The system must handle all `AlacEvent` variants that Zed handles: Wakeup, Title, ResetTitle, ClipboardStore, ClipboardLoad, PtyWrite, TextAreaSizeRequest, CursorBlinkingChange, Bell, Exit, ColorRequest, ChildExit, MouseCursorDirty
- FR-05: The system must NOT open URLs with schemes other than http, https, mailto, and file (with localhost validation)
- FR-06: The system must strip ESC characters from OSC 52 clipboard content and bracketed paste content to prevent escape injection
- FR-07: When `TermMode::ALT_SCREEN` is active, cursor must always be visible regardless of blink timer state

## Non-Functional Requirements

- **Performance:** Event batch processing at 4ms intervals (250Hz max render rate). Single keystroke echo latency < 8ms (4ms batch + 4ms render).
- **Compatibility:** 100% of Zed's `to_esc_str()` keystroke mappings covered. All AlacEvent variants handled. OSC 7, 8, 10, 11, 12, 52, 133 sequences processed.
- **Security:** OSC 52 clipboard read disabled by default. OSC 8 URL scheme allowlist enforced. Bracketed paste ESC stripping maintained.
- **Memory:** Display-only terminal scrollback capped at configurable line count (default 10,000). No memory leak from event channel accumulation.

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior | User Message |
|---|----------|---------|-------------------|--------------|
| 1 | Invalid regex in search | User types `[unclosed` in regex mode | Show red border on search input, no crash | "Invalid regex" |
| 2 | OSC 52 with empty clipboard | Program queries clipboard when it's empty | Respond with empty base64 (`""`) if read enabled, silent ignore if write-only | — |
| 3 | OSC 8 with extremely long URL | Malicious program sends 100KB URL | Truncate URL at 2048 bytes, store truncated version | URL truncated in tooltip |
| 4 | SIGKILL race condition | Process exits during 100ms grace period | Check if PID still exists before sending SIGKILL | — |
| 5 | IME composition interrupted | User switches focus during IME preedit | Cancel composition, clear preedit overlay, no garbage bytes to PTY | — |
| 6 | Theme change during color query | TUI queries OSC 11, theme changes before response | Return the color that was active at query time (read under lock) | — |
| 7 | CWD with non-UTF-8 path | OSC 7 contains percent-encoded non-UTF-8 bytes | Decode as lossy UTF-8, store best-effort path | — |
| 8 | Mouse release without prior press | Network/input glitch causes orphaned release event | Silently ignore (send button 3 release anyway, apps handle gracefully) | — |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | ~~Alacritty fork lacks OSC 52/OSC 8 cell attributes~~ | ~~Medium~~ | ~~High~~ | **RESOLVED:** Fork at rev `9d9640d4` has `ClipboardStore`, `ClipboardLoad`, and `Cell::hyperlink()`. Config `Osc52` enum has `Disabled/OnlyCopy/OnlyPaste/CopyPaste`. No blocker. |
| 2 | ~~GPUI doesn't expose Wayland IME preedit events~~ | ~~Medium~~ | ~~High~~ | **RESOLVED:** GPUI fully implements `zwp_text_input_v3` in `gpui_linux/wayland/client.rs`. `InputHandler` trait has `replace_and_mark_text_in_range()` for preedit. Zed's `TerminalInputHandler` at `terminal_element.rs:1417` is the reference. No spike needed. |
| 3 | Key mapping changes break existing shortcuts | Low | Medium | Test all 24 current keybindings after each keys.rs change. Existing Ctrl+Shift+C/V must keep working. |
| 4 | Performance regression from enriched key context | Low | Low | Key context reads Term mode under existing lock (no new lock acquisition). Profile with 10+ split panes. |
| 5 | OSC 52 clipboard exfiltration | Low | High | Default to write-only. Read requires explicit config opt-in. Document security implications in config schema. |

## Non-Goals

Explicit boundaries — what this PRD does NOT include:

- **Sixel/Kitty/iTerm2 inline image protocols** — No image rendering in the terminal grid. Image paste forwarding (US-005) sends raw Ctrl+V, not image data.
- **Kitty keyboard protocol** — PaneFlow continues to use traditional VT key encoding. The Kitty protocol is a future consideration but out of scope.
- **macOS or Windows support** — All implementation is Linux-only (Wayland + X11). `option_as_meta` config (US-028) prepares for future macOS but no macOS code is written.
- **Configurable contrast algorithm** — Zed uses APCA with a configurable `minimum_contrast` setting. PaneFlow keeps WCAG 2.0 AA with hardcoded 4.5 threshold. A future PRD may address this.
- **Vi mode for scrollback navigation** — The stabilization PRD's copy mode covers basic keyboard selection. Full vim-style scrollback navigation (motions, search, marks) is deferred.

## Files NOT to Modify

- `crates/paneflow-config/` — Config crate has its own PRD scope. Only add new config fields (osc52, option_as_meta) via the existing schema pattern.
- `src-app/src/ipc.rs` — Only add new IPC methods (`surface.send_text`, `surface.send_keystroke`). Do not restructure existing IPC architecture.
- `src-app/src/workspace.rs` — Workspace management is not in scope. No changes to workspace create/select/close.
- `src-app/src/split.rs` — Split tree is not in scope. No changes to split layout or resize logic.

## Technical Considerations

- **OSC 7 implementation path:** **RESOLVED** — The Alacritty fork silently ignores OSC 7 (no match arm in `vte/src/ansi.rs` OSC dispatch). Implementation must intercept OSC 7 in the PTY reader loop before VTE processing, using a byte-level scanner identical to the existing `XtversionScanner` pattern at `terminal.rs:1295-1335`. Parse `\x1b]7;file://[hostname]/path\x1b\\` or `\x1b]7;file://[hostname]/path\x07`, percent-decode the path, update `current_cwd`, emit `CwdChanged`.
- **OSC 52 event availability:** **RESOLVED** — `Event::ClipboardStore(ClipboardType, String)` and `Event::ClipboardLoad(ClipboardType, Arc<dyn Fn>)` are present in the fork at `event.rs:24-30`. Already arriving in `events_rx` but discarded by the `_ => {}` catch-all in `sync()`. Just add match arms.
- **GPUI InputHandler for IME:** **RESOLVED** — GPUI fully implements `zwp_text_input_v3` in the Wayland backend. The `InputHandler` trait at `gpui/src/platform.rs:1272` has all needed methods: `replace_and_mark_text_in_range` (preedit), `replace_text_in_range` (commit), `unmark_text` (cancel), `bounds_for_range` (popup positioning). Zed's `TerminalInputHandler` at `terminal_element.rs:1417-1533` is the exact reference to port.
- **Hyperlink regex engine:** Use the `regex` crate (already a dependency). Compile URL regex once at terminal creation, not per-hover.
- **Key context performance:** Reading `TermMode` flags requires locking `Term`. This happens during `render()` which already locks for other purposes — combine into a single lock acquisition.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| Keystroke mapping coverage | ~65% of Zed's mappings | 100% | Release 1 | Diff PaneFlow keys.rs against Zed keys.rs |
| AlacEvent handling coverage | 5 of 12 variants handled | 12 of 12 | Release 1 | Code audit — every match arm has behavior |
| TUI app compatibility | vim/less scroll broken | vim, tmux, less, htop, Claude Code all working | Release 1 | Manual test matrix |
| OSC sequence support | OSC 7 (broken), no others | OSC 7, 8, 10, 11, 12, 52, 133 | Release 2 | Test scripts for each OSC |
| Hyperlink detection | None | URLs clickable with Ctrl+click | Release 2 | Manual test with URL-heavy output |

## Open Questions (Resolved)

All questions have been investigated and resolved:

- **GPUI InputHandler preedit support** — **RESOLVED: Yes.** GPUI's Wayland backend fully implements `zwp_text_input_v3`. The `InputHandler` trait at `gpui/src/platform.rs:1272` exposes preedit via `replace_and_mark_text_in_range()`. Zed's `TerminalInputHandler` at `terminal_element.rs:1417-1533` is the reference. No spike needed for US-017.
- **Alacritty fork OSC 52 events** — **RESOLVED: Yes.** Both `Event::ClipboardStore(ClipboardType, String)` and `Event::ClipboardLoad(ClipboardType, Arc<dyn Fn>)` are present at `event.rs:24-30`. Config has `Osc52` enum with `Disabled/OnlyCopy/OnlyPaste/CopyPaste`. Events already arrive in `events_rx` but are discarded in `sync()`.
- **OSC 7 capture path** — **RESOLVED: Not exposed.** The fork silently ignores OSC 7 (falls through to `unhandled()` in `vte/src/ansi.rs:1523`). Implementation must use a pre-VTE byte scanner in the reader loop (same pattern as existing `XtversionScanner`). Parse `\x1b]7;file://[hostname]/path{ST|BEL}`, percent-decode, update `current_cwd`.
- **Bell visual flash implementation** — Decision: simple timer-based approach. 200ms background color pulse via `cx.notify()` with a `bell_flash_until: Option<Instant>` field on `TerminalView`.
[/PRD]
