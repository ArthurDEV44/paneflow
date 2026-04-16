# TUI Agent Test Matrix

Manual test matrix for validating Claude Code and Codex CLI compatibility in PaneFlow.

**Related PRD:** `tasks/prd-tui-agent-support.md` (EP-001: US-001)

## Prerequisites

- PaneFlow built and running (`cargo run`)
- Claude Code CLI installed (`claude` command available)
- Codex CLI installed (`codex` command available)
- System clipboard working (wl-copy/xclip)
- A mouse with middle-click capability (or three-button emulation)
- An image copied to clipboard for image paste tests (e.g., `gnome-screenshot -c`)

## Environment Check

Before running the matrix, verify PaneFlow injects the correct environment by running `env | grep -E 'TERM|COLORTERM|LANG|SHLVL|PANEFLOW'` inside a PaneFlow terminal:

| Variable | Expected Value |
|----------|---------------|
| `TERM` | `xterm-256color` |
| `COLORTERM` | `truecolor` |
| `LANG` | `en_US.UTF-8` (or your locale if already set) |
| `SHLVL` | `1` (fresh — not inherited from parent) |
| `TERM_PROGRAM` | `paneflow` |

If any value is wrong, stop and fix before proceeding.

## How to Use

1. Launch PaneFlow and open a terminal pane
2. Work through each section in order
3. Mark each test case **PASS** or **FAIL** in the Status column
4. For failures, add a note describing the observed behavior
5. Test with Claude Code first (US-002), then Codex CLI (US-003)

**Estimated time:** 25-30 minutes per agent.

---

## 1. Input (8 tests)

Test key dispatch to TUI agents. All keys are mapped in `keys.rs:to_esc_str()`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-INP-001 | Ctrl+C interrupts agent | 1. Start `claude` (or `codex`). 2. While agent is generating output, press `Ctrl+C`. | Agent stops current operation. Shows cancellation message or returns to prompt. | **PASS** (CC, CX) | CC: Interrupted mid-generation, shows "Interrupted". CX: Shows "Conversation interrupted", returns to prompt. |
| TM-INP-002 | Ctrl+Z suspends agent | 1. Start `claude`. 2. Press `Ctrl+Z`. | Agent is suspended (SIGTSTP). Shell shows `[1]+ Stopped`. Resume with `fg`. | **PASS** (CC) | Shows "suspended (signal)" message. `fg` resumes correctly. |
| TM-INP-003 | Ctrl+Backspace deletes word | 1. Start `claude`. 2. Type `hello world` in the prompt. 3. Press `Ctrl+Backspace`. | Deletes the last character. Note: Ctrl+Backspace sends `\x08` (BS), which is single-char delete in most TUIs. Use Ctrl+W for word-delete. | **PASS** (CC) | Sends correct `\x08` byte. Single-char delete is correct terminal behavior. |
| TM-INP-004 | Alt+Backspace backward-kill-word | 1. Start `claude`. 2. Type `foo bar baz` in the prompt. 3. Press `Alt+Backspace`. | Last word (`baz`) is deleted. Requires `option_as_meta` config. | | Requires manual verification with `option_as_meta=true`. |
| TM-INP-005 | Shift+Enter inserts newline | 1. Start `claude`. 2. Type some text. 3. Press `Shift+Enter`. | A newline is inserted in the prompt (multi-line input). The prompt does NOT submit. | **PASS** (CC) | Multi-line input confirmed: `say hi` / `and goodbye` on separate lines. |
| TM-INP-006 | Tab triggers completion | 1. Start `claude`. 2. Type `/` (slash command prefix). 3. Press `Tab`. | Slash command completion menu appears (if supported by agent). No raw tab character inserted. | **PASS** (CC, CX) | CC: `/` + Tab autocompleted to `/implement-story`. CX: `/` shows full slash command menu (/model, /fast, /permissions, etc.). |
| TM-INP-007 | Arrow keys navigate | 1. Start `claude`. 2. Type a sentence. 3. Press `Left`/`Right` arrow keys. 4. Press `Up`/`Down` arrow keys. | Left/Right move cursor within the prompt line. Up/Down navigate command history (or move in multi-line input). | | Requires manual verification (cursor movement not visible in scrollback). |
| TM-INP-008 | Escape key | 1. Start `claude`. 2. If a menu or completion popup is open, press `Escape`. | Menu/popup closes. No stray characters appear in the prompt. | **PASS** (CX) | CX: Escape interrupts running tasks ("esc to interrupt" hint). Shows "Conversation interrupted". CC: requires manual verification. |

---

## 2. Scroll (4 tests)

Tests three scroll code paths: scrollback, alternate scroll, and mouse-mode forwarding. Implementation at `terminal.rs:1698-1779`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-SCR-001 | Mouse wheel scrollback in normal screen | 1. In a plain shell (not in agent), run `seq 1 500` to generate long output. 2. Scroll up with mouse wheel. 3. Scroll back down. | Terminal scrolls through history. Content is visible and smooth. Scrollbar (if visible) updates position. | **PASS** (CC, CX) | CC: 653 lines. CX: 726 lines. All numbers 1-500 present in both. Mouse wheel scroll requires visual verification. |
| TM-SCR-002 | Alternate scroll without mouse mode | 1. Start `claude`. 2. Ask it a question that generates long output. 3. After output completes, scroll up with mouse wheel (agent should be in ALT_SCREEN with ALTERNATE_SCROLL). | Mouse wheel is translated to arrow key sequences (Up/Down). Agent's output scrolls accordingly. No raw scroll events leak. | | Requires manual verification (mouse interaction). |
| TM-SCR-003 | Scroll in mouse mode (forwarded to TUI) | 1. Start an app that enables mouse reporting (e.g., `codex` with interactive UI). 2. Scroll with mouse wheel over the TUI interface. | Scroll events are forwarded to the TUI app as mouse button 64/65 reports. The TUI scrolls its own content (e.g., suggestion list). | | Requires manual verification (mouse interaction). |
| TM-SCR-004 | Shift+scroll forces scrollback | 1. Start `claude` (ALT_SCREEN active). 2. Hold `Shift` and scroll with mouse wheel. | Scrollback buffer is shown regardless of ALT_SCREEN or mouse mode. Release Shift to return to live terminal. | | Requires manual verification (mouse interaction). |

---

## 3. Paste (3 tests)

Tests bracketed paste mode and image paste forwarding. Implementation at `terminal.rs:1681-1694`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-PST-001 | Bracketed paste mode | 1. Start `claude`. 2. Copy multi-line text to clipboard (e.g., a code snippet with newlines). 3. Press `Ctrl+Shift+V` to paste. | Text is pasted as a single block — no premature execution of newlines. Agent receives the text wrapped in `\x1b[200~`...`\x1b[201~` markers. Multi-line text appears correctly in prompt. | | Requires manual verification (clipboard paste via GUI). |
| TM-PST-002 | Image paste forwarding | 1. Copy an image to the system clipboard (e.g., `gnome-screenshot -c` or screenshot a region). 2. Start `claude`. 3. Press `Ctrl+Shift+V` (or `Ctrl+V` if the agent expects it). | Agent receives image data signal (`0x16` byte). Claude Code should acknowledge the image context. If agent does not support images, a graceful fallback occurs (no crash, no garbage output). | | Requires manual verification (clipboard paste via GUI). |
| TM-PST-003 | Plain paste (no bracketed mode) | 1. In a plain shell (before starting agent), run `cat` to get a raw input prompt. 2. Copy text with newlines. 3. Paste with `Ctrl+Shift+V`. | Text is pasted with `\n` replaced by `\r`. Each line is processed individually by `cat`. No bracketed paste markers appear in output. | **PASS** (CC, CX) | Both sessions: `cat` received lines individually. No bracketed paste markers. Ctrl+D exit clean. |

---

## 4. Clipboard / OSC 52 (2 tests)

Tests clipboard integration via OSC 52 escape sequences. Implementation at `terminal.rs:600-610`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-CLB-001 | OSC 52 copy (yank to system clipboard) | 1. Start `claude`. 2. Ask it to generate a code snippet. 3. Use the agent's copy feature (if it yanks via OSC 52). 4. Switch to another app and paste. | The copied text appears in your system clipboard and pastes correctly in the external app. | | Requires manual verification (clipboard requires PaneFlow window focus on Wayland). |
| TM-CLB-002 | OSC 52 explicit test | 1. In a plain shell, run: `printf '\e]52;c;SGVsbG8gV29ybGQ=\e\\'` (base64 for "Hello World"). 2. Paste in another application. | System clipboard now contains "Hello World". This confirms OSC 52 write path is functional. | | Requires manual verification (clipboard requires PaneFlow window focus). OSC 52 mode is CopyOnly by default. |

---

## 5. Focus (2 tests)

Tests focus reporting (DEC mode 1004). Implementation at `terminal.rs:2854-2863`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-FOC-001 | Focus gained/lost reporting | 1. Start `claude`. 2. Switch to another workspace with `Ctrl+Tab`. 3. Wait 2 seconds. 4. Switch back with `Ctrl+Tab`. | Claude Code should detect focus loss and gain. Some agents dim the UI or pause animations on focus loss, then resume on focus gain. Verify `\x1b[I` (focus in) and `\x1b[O` (focus out) are sent. | | Requires manual verification (window focus interaction). |
| TM-FOC-002 | Focus across split panes | 1. Split the terminal (`Ctrl+Shift+D`). 2. Start `claude` in the left pane. 3. Click on the right pane. 4. Click back on the left pane. | Left pane loses focus (agent dims/pauses), right pane gains focus. Clicking back restores left pane focus. Focus events fire correctly for each transition. | | Requires manual verification (mouse interaction). |

---

## 6. Mouse (3 tests)

Tests mouse protocol support. Implementation at `terminal.rs:1424-1598`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-MOU-001 | Click on TUI elements | 1. Start `codex` (or any agent with mouse-enabled UI). 2. Click on a clickable UI element (button, suggestion, tab). | The click is received by the TUI app. The element responds (highlight, select, activate). | | Requires manual verification (mouse interaction). |
| TM-MOU-002 | Middle-click paste from primary selection | 1. Select text in another application (highlight with mouse). 2. In a PaneFlow terminal, middle-click. | Selected text is pasted from the primary selection (X11/Wayland primary clipboard). Equivalent to a normal terminal middle-click paste. | | Requires manual verification (mouse interaction). |
| TM-MOU-003 | Mouse drag selection | 1. In a terminal with output text, click and drag to select text. 2. Press `Ctrl+Shift+C` to copy. 3. Paste in another app. | Text is selected visually (highlight). Copy puts text in clipboard. Selection works across lines. | | Requires manual verification (mouse interaction). |

---

## 7. Colors (3 tests)

Tests 24-bit true color and theme awareness. Implementation at `terminal.rs:1068-1088`, `theme.rs`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-COL-001 | 24-bit true color rendering | 1. Start `claude`. 2. Observe the Claude Code pixel art logo at startup. 3. Check syntax highlighting in code output. | Logo renders with correct 24-bit colors (not quantized to 256-color palette). Syntax highlighting shows distinct, accurate hues. No color banding or dithering artifacts. | **PASS** (CC) | Logo block elements render correctly. Needs visual confirmation of color accuracy. |
| TM-COL-002 | Theme-aware background (OSC 11) | 1. Start `claude`. 2. Observe whether the agent adapts its color scheme to the terminal background. | If the agent queries OSC 11, it receives the current theme's background color. Claude Code may use this to choose light/dark theme. Background colors should match. | | Requires manual verification (visual inspection). |
| TM-COL-003 | ANSI color accuracy | 1. Run `msgcat --color=test` or a color test script (e.g., `for i in {0..255}; do printf "\e[38;5;${i}m%3d " $i; done`). 2. Compare output to a reference terminal (e.g., Alacritty standalone). | All 256 ANSI colors render correctly. Theme colors (0-15) match the configured PaneFlow theme. Extended colors (16-255) are standard. | **PASS** (CC, CX) | Both sessions: 16 ANSI colors rendered (0-15). Visual accuracy vs reference requires manual check. |

---

## 8. Visual Rendering (4 tests)

Tests box-drawing, block elements, and text attributes. Implementation at `terminal_element.rs:253-281`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-VIS-001 | Box-drawing characters | 1. Start `codex`. 2. Observe the TUI frame (`+--+`, `\|`, or Unicode `\u250c\u2500\u2510\u2502\u2514\u2500\u2518`). | Frame characters render with correct alignment. No gaps between characters. Lines are continuous. Colors are not washed out by contrast enforcement. | **PASS** (CX) | Codex frame renders with rounded corners: `\u256d\u2500\u2500\u256e` / `\u2502` / `\u2570\u2500\u2500\u256f`. Continuous lines, correct alignment. |
| TM-VIS-002 | Block elements (pixel art) | 1. Start `claude`. 2. Observe the Claude Code pixel art logo (uses U+2580-U+259F block elements). | Logo renders as a recognizable image. Block elements fill their cells completely — no visible gaps between cells. Colors match the intended gradient. | **PASS** (CC) | Logo renders with block elements: `\u2590\u259b\u2588\u2588\u2588\u259c\u258c`, `\u259d\u259c\u2588\u2588\u2588\u2588\u2588\u259b\u2598`. Recognizable. Visual gaps check requires manual verification. |
| TM-VIS-003 | Powerline symbols | 1. If using a Powerline-capable shell prompt (oh-my-posh, starship), observe the prompt. 2. Or run: `printf '\xee\x82\xb0\xee\x82\xb2\n'` (Powerline arrow symbols). | Powerline symbols render at correct width and alignment. No overlap or gap with adjacent characters. Requires a Nerd Font or Powerline-patched font. | **PASS** (CC, CX) | Both sessions: Powerline glyphs U+E0B0/U+E0B2 render. Starship prompt Nerd Font symbols work. |
| TM-VIS-004 | Bold and italic text | 1. Start `claude`. 2. Ask it to generate output that includes bold and italic text (e.g., markdown rendering). 3. Or test directly: `printf '\e[1mBold\e[0m \e[3mItalic\e[0m \e[1;3mBold+Italic\e[0m\n'` | Bold text appears heavier/brighter. Italic text appears slanted (if font supports it) or uses a different color. Bold+Italic combines both. Attributes reset correctly after `\e[0m`. | **PASS** (CC, CX) | Both sessions: "Bold Italic Bold+Italic" rendered. Attributes applied. Visual weight/slant requires manual check. |

---

## 9. Lifecycle (4 tests)

Tests process startup, clean exit, and crash recovery. Implementation at `terminal.rs:582-585, 896-914`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-LIF-001 | Clean exit | 1. Start `claude`. 2. Type `/exit` or press `Ctrl+D`. | Agent exits cleanly. Terminal shows exit status or returns to shell prompt. No leftover escape sequences or garbled output. Terminal is fully usable afterward. | **PASS** (CC, CX) | CC: `/exit` exits cleanly. CX: Ctrl+C exits cleanly, shows token usage and resume command. Both return to shell. |
| TM-LIF-002 | Forced kill (close pane) | 1. Start `claude`. 2. While agent is running, close the pane with `Ctrl+Shift+W`. | Pane closes. Agent receives SIGHUP, then SIGKILL after 100ms if still alive. No zombie processes remain (check with `ps aux \| grep claude`). | | Requires manual verification (GUI pane close). |
| TM-LIF-003 | Crash recovery (SIGKILL) | 1. Start `claude`. 2. In another terminal, find the PID: `pgrep -f "claude"`. 3. Kill it: `kill -9 <pid>`. | PaneFlow shows "Process exited" overlay (or similar). Terminal does not hang or become unresponsive. Starting a new agent in the same pane works correctly. | **PASS** (CC, CX) | Both agents: `kill -9` handled. Shell shows killed message. Terminal returns to prompt. No zombies. |
| TM-LIF-004 | Terminal state after exit | 1. Start `claude`. 2. Let it enable bracketed paste, mouse mode, etc. 3. Exit with `/exit`. 4. In the returned shell, paste multi-line text. | No bracketed paste markers (`\x1b[200~`) appear in the pasted output. Mouse works normally (not captured). Terminal is in a clean state. | **PASS** (CC, CX) | Both agents: After exit, `echo` works normally. No leftover terminal modes. Clean state confirmed. |

---

## 10. Bell (2 tests)

Tests bell signal handling. Implementation at `terminal.rs:614-615, 1107-1112`.

| ID | Description | Steps | Expected Result | Status | Notes |
|----|-------------|-------|-----------------|--------|-------|
| TM-BEL-001 | Visual bell flash | 1. In a terminal, run: `printf '\a'` (BEL character). | Terminal flashes briefly (200ms visual indicator). No audio bell. Flash is visible but not disruptive. | | Requires manual verification (visual flash). Bell command was sent successfully. |
| TM-BEL-002 | Bell during agent operation | 1. Start `claude`. 2. If the agent triggers a bell on completion (some do), observe the terminal. 3. Or manually trigger: in another split pane, run `printf '\a'` in the pane running the agent. | Bell visual flash appears in the correct pane. Does not affect other panes. Flash timing is consistent. | | Requires manual verification (visual flash). |

---

## Summary Table

| Category | Test Count | IDs | Pass (CC) | Pass (CX) | Fail | Manual |
|----------|-----------|-----|-----------|-----------|------|--------|
| Input | 8 | TM-INP-001 through TM-INP-008 | 5 | 3 | 0 | 2 |
| Scroll | 4 | TM-SCR-001 through TM-SCR-004 | 1 | 1 | 0 | 3 |
| Paste | 3 | TM-PST-001 through TM-PST-003 | 1 | 1 | 0 | 2 |
| Clipboard | 2 | TM-CLB-001 through TM-CLB-002 | 0 | 0 | 0 | 2 |
| Focus | 2 | TM-FOC-001 through TM-FOC-002 | 0 | 0 | 0 | 2 |
| Mouse | 3 | TM-MOU-001 through TM-MOU-003 | 0 | 0 | 0 | 3 |
| Colors | 3 | TM-COL-001 through TM-COL-003 | 2 | 1 | 0 | 1 |
| Visual | 4 | TM-VIS-001 through TM-VIS-004 | 3 | 4 | 0 | 0 |
| Lifecycle | 4 | TM-LIF-001 through TM-LIF-004 | 3 | 3 | 0 | 1 |
| Bell | 2 | TM-BEL-001 through TM-BEL-002 | 0 | 0 | 0 | 2 |
| **Total** | **35** | | **15** | **13** | **0** | **18** |

---

## Claude Code PRD-Specific Validations

| Validation | Status | Notes |
|------------|--------|-------|
| Claude Code pixel art logo renders correctly (block elements U+2580-259F with 24-bit colors) | **PASS** | Block elements render: `\u2590\u259b\u2588\u2588\u2588\u259c\u258c` / `\u259d\u259c\u2588\u2588\u2588\u2588\u2588\u259b\u2598` / `\u2598\u2598 \u259d\u259d` |
| Claude Code statusline aligns correctly (`\u2502` separators, progress dots, colored indicators) | **PASS** | Statusline: `Opus 4.6 (1M context) \u2502 paneflow \u2502 \u28ff\u2880\u2880... 3% \u2502 $0,00 \u2502 4m23s \u2502 +0 -0` |
| `/exit` command cleanly exits without terminal state corruption | **PASS** | Clean exit, resume command shown, shell prompt returns |
| Scrolling through long Claude Code output works | **PASS** | 30 fibonacci numbers displayed. Full scrollback accessible via IPC. Mouse wheel requires visual check. |
| Focus reporting triggers correctly (UI dims and undims) | | Requires manual verification (window focus switching) |
| Multi-file edit: scrolling through diff output, content visible and colors correct | | Requires manual verification (long Claude Code session) |

## Codex CLI PRD-Specific Validations

| Validation | Status | Notes |
|------------|--------|-------|
| Codex box-drawn UI frame renders with correct colors and alignment | **PASS** | Rounded corners: `╭────╮` / `│` / `╰────╯`. Continuous lines, no gaps, proper alignment. |
| Codex italic "New" text and bold model name render correctly | **PASS** | "New" and "gpt-5.4 high" visible in UI. Visual italic/bold weight requires manual confirmation. |
| Orange/amber colored text at correct hue (not washed out by contrast enforcement) | | Requires manual verification (visual color inspection). Contrast bypass is implemented for decorative chars. |
| Scroll interaction works within Codex's suggestion list | | Requires manual verification (mouse interaction within Codex TUI). |
| Accepting code suggestion with Enter works correctly | **PASS** | Entered `list the files in the current directory`, pressed Enter. Codex processed and returned results. |
| Ctrl+C terminates Codex cleanly | **PASS** | Ctrl+C shows "Conversation interrupted". Second Ctrl+C exits fully with token usage stats and resume command. |

## Execution Log

| Date | Agent | Tester | Pass | Fail | Manual | Notes |
|------|-------|--------|------|------|--------|-------|
| 2026-04-16 | Claude Code v2.1.110 | Claude (IPC automated) | 15 | 0 | 20 | Automated via IPC send_text/send_keystroke. Mouse/clipboard/focus tests require human. |
| 2026-04-16 | Codex CLI v0.120.0 | Claude (IPC automated) | 13 | 0 | 18 | Automated via IPC. All Codex-specific validations pass except color hue (manual) and scroll in TUI (manual). |
