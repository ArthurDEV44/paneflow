[PRD]
# PRD: TUI Agent Support — Claude Code & Codex CLI Compatibility

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-16 | Claude + Arthur | Initial draft from Zed terminal deep dive + Paneflow gap analysis |

## Problem Statement

PaneFlow targets developers who use Claude Code and Codex CLI as daily workflow tools. While a prior audit identified 15 protocol gaps, a fresh codebase exploration (April 16, 2026) confirms **all 15 critical items are already implemented** — AlacEvent handling, focus reporting, alternate scroll, image paste forwarding, key mappings, mouse protocol, environment setup, key dispatch context, middle-click paste, and process lifecycle.

However, three categories of work remain:

1. **No formal verification exists.** The implementations have never been validated against real Claude Code and Codex CLI sessions. Untested code is unreliable code — subtle bugs in escape sequence timing, mode interaction, or edge cases could cause silent breakage that only surfaces during real use.

2. **Hyperlink support is missing.** Claude Code and Codex CLI output URLs (to docs, PRs, files) that should be clickable. OSC 8 cell-level hyperlinks and regex URL detection are not implemented, meaning users must manually copy-paste URLs from terminal output.

3. **Protocol gaps for future TUI evolution.** The Kitty keyboard protocol (`CSI > 1u`) is now the standard for modifier key disambiguation — Crossterm (the TUI framework Claude Code uses) enables it by default when available. Bracketed paste mode cleanup on child exit (Claude Code bug #3134) can leave the terminal in a broken state. Synchronized output prevents tearing during rapid redraws.

**Why now:** PaneFlow's terminal has reached feature completeness for basic VT protocol support. The gap between "features implemented" and "verified working with real TUI agents" is what prevents confident adoption. Users must trust that Claude Code sessions won't hit subtle input bugs or leave the terminal in a broken state.

## Overview

This PRD has three epics across two releases:

**Release 1 (EP-001):** Validate all existing implementations against real Claude Code and Codex CLI sessions. Create a reproducible test matrix. Fix any bugs discovered during validation. After Release 1, users can confidently run Claude Code and Codex in PaneFlow.

**Release 2 (EP-002 through EP-003):** Add hyperlink support (OSC 8 + regex URL detection + click-to-open) and advanced protocol features (bracketed paste cleanup, synchronized output awareness). After Release 2, PaneFlow matches Zed's terminal feature set for TUI agent workflows.

Key decisions:
- **Validation-first approach** — verify existing implementations before adding new features
- **OSC 8 URL scheme allowlist** — only `http`, `https`, `mailto`, and `file` (with localhost validation) are openable
- **Bracketed paste cleanup is defensive** — reset mode on child exit regardless of what the child sent
- **Kitty keyboard protocol deferred** — marked as Won't Have for v1 (requires alacritty_terminal changes upstream)

## Goals

| Goal | Release 1 Target | Release 2 Target |
|------|------------------|-------------------|
| TUI agent compatibility | Claude Code + Codex CLI validated working: input, scroll, paste, clipboard, focus, mouse | Hyperlinks clickable, no terminal state leaks after agent exit |
| Test coverage | Reproducible manual test matrix with 20+ test cases | Automated regression tests for critical paths |
| Protocol compliance | All existing implementations verified correct | OSC 8, bracketed paste cleanup, synchronized output added |

## Target Users

### AI-First Developer
- **Role:** Developer using Claude Code or Codex CLI as primary coding workflow inside PaneFlow
- **Behaviors:** Runs multi-hour Claude Code sessions, pastes images for context, scrolls through long outputs, uses Ctrl+C/Ctrl+Z for session control, copies URLs from output
- **Pain points:** Uncertainty about whether terminal features work correctly. URLs in output are not clickable. Occasional terminal state corruption after agent crashes.
- **Success looks like:** Claude Code and Codex CLI are indistinguishable from running in Alacritty/WezTerm. Zero terminal state leaks.

## Research Findings

### Validated Implementations (April 16, 2026 code audit)

All 15 items from the prior gap audit are confirmed implemented with line-number evidence:

| Feature | File:Line | Status |
|---------|-----------|--------|
| AlacEvent: ClipboardStore/Load | terminal.rs:586-591 | Implemented |
| AlacEvent: ColorRequest | terminal.rs:597 | Implemented |
| AlacEvent: Bell | terminal.rs:600 | Implemented |
| AlacEvent: CursorBlinkingChange | terminal.rs:603 | Implemented |
| AlacEvent: TextAreaSizeRequest | terminal.rs:607 | Implemented |
| Focus reporting (\x1b[I/O) | terminal.rs:2764-2774 | Implemented |
| Alternate scroll (ALT_SCREEN) | terminal.rs:1662-1690 | Implemented |
| Image paste forwarding (0x16) | terminal.rs:1595-1598 | Implemented |
| Key mappings (Ctrl+BS, Alt+BS, etc.) | keys.rs:24-208 | Implemented |
| Mouse release button 3 (Normal) | terminal.rs:1387 | Implemented |
| SHLVL removal | pty.rs:67 | Implemented |
| LANG fallback | terminal.rs:401-403 | Implemented |
| Key dispatch context enrichment | terminal.rs:2701-2757 | Implemented |
| Middle-click primary selection | terminal.rs:1547-1553 | Implemented |
| Shutdown grace period + SIGKILL | terminal.rs:883-901 | Implemented |

### Remaining Gaps

| Gap | Impact | Source |
|-----|--------|--------|
| No OSC 8 hyperlink detection | URLs in Claude Code output not clickable | Codebase audit |
| No regex URL detection | Bare URLs not highlighted on hover | Codebase audit |
| No bracketed paste cleanup on child exit | Terminal state corruption after Claude Code crash (bug #3134) | Web research |
| No synchronized output awareness | Potential TUI tearing during rapid redraws | Web research |
| No formal TUI agent test matrix | Risk of regression, no confidence baseline | Process gap |

### Competitive Context

- **Zed:** Full OSC 8 + regex URL detection. Bracketed paste cleanup via PTY HUP. No Kitty keyboard protocol.
- **Alacritty:** OSC 8 support. Auto-resets modes on PTY close. No synchronized output.
- **WezTerm:** Full OSC 8. Synchronized output. Kitty keyboard protocol. OSC 52 read gated by config.
- **Ghostty:** Full VT525 compliance. Synchronized output. Kitty keyboard. Best-in-class TUI support.

*Sources: Zed terminal deep dive (ZED_TERMINAL_DEEP_DIVE.md), tmuxai.dev terminal compatibility matrix, Claude Code issue #3134.*

## Assumptions & Constraints

### Assumptions (validated)
- Alacritty fork exposes `Cell::hyperlink()` for OSC 8 — **VALIDATED** at `term/cell.rs:219`
- Existing `is_decorative_character()` bypass prevents box-drawing contrast issues — **VALIDATED** at `terminal_element.rs:253`
- GPUI's `cx.open_url()` or equivalent can open URLs in default browser — **VALIDATED** via `open::that()` crate

### Hard Constraints
- Must use Zed's Alacritty fork (`rev = "9d9640d4"`) — no switching VTE backend
- Linux-only (Wayland + X11)
- Must not regress any existing feature (copy mode, search, split panes, session persistence)
- Kitty keyboard protocol requires upstream alacritty_terminal changes — deferred to future PRD

## Quality Gates

These commands must pass for every user story:
- `cargo build` — compilation succeeds
- `cargo clippy --workspace -- -D warnings` — zero clippy warnings
- `cargo test --workspace` — all tests pass
- `cargo fmt --check` — formatting correct

For TUI agent stories, additional gates:
- Verify in a running PaneFlow instance with Claude Code (`claude`) and Codex CLI (`codex`)
- Run the test matrix defined in US-001

---

## Epics & User Stories

---

### EP-001: Validation & Bug Fixes

Validate all existing TUI agent support implementations against real Claude Code and Codex CLI sessions. Create a reproducible test matrix. Fix any bugs discovered.

**Definition of Done:** Every test case in the TUI agent test matrix passes with both Claude Code and Codex CLI. Any bugs found during validation are fixed and documented.

#### US-001: Create TUI agent test matrix
**Description:** As a developer, I want a reproducible test matrix covering all terminal features used by Claude Code and Codex CLI so that I can verify correct behavior and detect regressions.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Test matrix document created at `tasks/tui-agent-test-matrix.md`
- [ ] Matrix covers these categories with specific test steps and expected results:
  - Input: Ctrl+C, Ctrl+Z, Ctrl+Backspace (word-delete), Alt+Backspace (backward-kill-word), Shift+Enter (newline in prompt), Tab (completion), arrow keys, Escape
  - Scroll: mouse wheel scroll in output history, scroll in ALT_SCREEN without mouse mode (alternate scroll), scroll in mouse mode (forwarded to TUI)
  - Paste: text paste with bracketed paste mode, image paste forwarding (Ctrl+V with image in clipboard)
  - Clipboard: OSC 52 copy (yank to system clipboard from Claude Code), OSC 52 read (if enabled)
  - Focus: terminal focus/unfocus (Ctrl+Tab to switch workspace, then back) — Claude Code should dim/undim
  - Mouse: click on Claude Code UI elements (if mouse mode enabled), middle-click paste from primary selection
  - Colors: 24-bit true color rendering (logo, syntax highlighting), theme-aware background (OSC 11 query)
  - Visual: box-drawing characters (frames), block elements (pixel art logo), Powerline symbols, bold/italic text
  - Lifecycle: clean exit (Ctrl+D or `/exit`), forced kill (close pane while running), crash recovery
  - Bell: audible/visual bell when Claude Code signals completion
- [ ] Each test case has: unique ID, description, steps to reproduce, expected result, pass/fail status
- [ ] Matrix is executable by any developer in under 30 minutes
- [ ] Given the test matrix, when a new developer runs it, then they can verify all features without prior knowledge

#### US-002: Execute validation pass with Claude Code
**Description:** As a developer, I want to run the full test matrix against Claude Code CLI inside PaneFlow so that I can confirm all features work correctly and document any bugs found.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-001

**Acceptance Criteria:**
- [ ] Full test matrix executed with Claude Code v2.x running inside PaneFlow
- [ ] Every test case marked pass or fail with notes
- [ ] Any failing test cases have a linked bug report (GitHub issue or inline description)
- [ ] Specific validation: Claude Code pixel art logo renders correctly (block elements U+2580-259F with 24-bit colors)
- [ ] Specific validation: Claude Code statusline (bottom bar with `│` separators, progress dots, colored indicators) aligns correctly
- [ ] Specific validation: `/exit` command cleanly exits without terminal state corruption
- [ ] Specific validation: scrolling through long Claude Code output works (both mouse wheel and keyboard)
- [ ] Given Claude Code running a multi-file edit, when scrolling through the diff output, then all content is visible and colors are correct
- [ ] Given Claude Code running, when switching to another workspace and back, then focus reporting triggers correctly (UI dims and undims)
- [ ] If any test case fails, a follow-up bug fix story (US-004) is created with the specific fix

#### US-003: Execute validation pass with Codex CLI
**Description:** As a developer, I want to run the full test matrix against Codex CLI inside PaneFlow so that I can confirm all features work correctly and document any bugs found.

**Priority:** P0
**Size:** M (3 pts)
**Dependencies:** US-001

**Acceptance Criteria:**
- [ ] Full test matrix executed with Codex CLI (latest version) running inside PaneFlow
- [ ] Every test case marked pass or fail with notes
- [ ] Any failing test cases have a linked bug report
- [ ] Specific validation: Codex box-drawn UI frame (`┌─┐│└─┘`) renders with correct colors and alignment
- [ ] Specific validation: Codex italic "New" text and bold model name render correctly
- [ ] Specific validation: orange/amber colored text appears at correct hue (not washed out by contrast enforcement)
- [ ] Specific validation: scroll interaction works within Codex's suggestion list
- [ ] Given Codex running with a code suggestion, when accepting with Enter, then the action is correctly received
- [ ] Given Codex running, when pressing Ctrl+C, then the session terminates cleanly
- [ ] If any test case fails, a follow-up bug fix story (US-004) is created with the specific fix

#### US-004: Fix validation-discovered bugs
**Description:** As a developer, I want bugs discovered during US-002 and US-003 validation to be fixed so that both TUI agents work correctly in PaneFlow.

**Priority:** P0
**Size:** L (5 pts) — size is estimated; actual size depends on bugs found
**Dependencies:** US-002, US-003

**Acceptance Criteria:**
- [ ] Every bug discovered during US-002 and US-003 is fixed or explicitly deferred with justification
- [ ] Each fix includes a regression test (if feasible — at minimum, a test matrix entry)
- [ ] After fixes, the full test matrix passes for both Claude Code and Codex CLI
- [ ] No fix introduces a regression in existing features (copy mode, search, split panes, session persistence)
- [ ] If no bugs are found during validation, this story is marked as "N/A — no bugs discovered"

---

### EP-002: Hyperlink Support

Implement URL detection and click-to-open for terminal output. After this epic, URLs in Claude Code and Codex CLI output are clickable.

**Definition of Done:** Ctrl+click on URLs in terminal output opens them in the default browser. OSC 8 explicit hyperlinks are detected from cell attributes. Regex-detected bare URLs are highlighted on hover.

#### US-005: OSC 8 hyperlink detection from cell attributes
**Description:** As a terminal user, I want programs that emit OSC 8 hyperlinks to have those links detected and rendered so that explicit hyperlinks from modern CLI tools are recognized.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] During cell iteration in `terminal_element.rs`, read `cell.hyperlink()` attribute from Alacritty cells
- [ ] Store hyperlink URL and `id` parameter per cell range in layout state as `HyperlinkZone` structs
- [ ] OSC 8 hyperlinks take priority over regex-detected URLs when both match the same cell range
- [ ] URL scheme validated against allowlist: `http`, `https`, `mailto`, `file` — others are stored but not openable
- [ ] `file://` URLs validate that hostname matches `localhost` or is empty
- [ ] Given `printf '\e]8;;https://example.com\e\\Click here\e]8;;\e\\'`, when the text renders, then cells "Click here" carry the hyperlink attribute
- [ ] Given a `javascript:alert(1)` OSC 8 link, when Ctrl+clicking, then nothing happens (scheme blocked)

#### US-006: Regex URL detection with box-drawing exclusion
**Description:** As a terminal user, I want bare URLs in terminal output to be detected by regex so that links printed by Claude Code (to docs, PRs, files) are clickable without OSC 8.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** US-005

**Acceptance Criteria:**
- [ ] URL regex matches protocols: `https://`, `http://`, `git://`, `ssh:`, `ftp://`, `file://`, `mailto:`
- [ ] Box-drawing characters (U+2500-U+257F) are excluded from regex first-character match to prevent TUI frames from triggering false positives
- [ ] Regex is compiled once at terminal creation, not per-hover
- [ ] Detection runs on the line text at the mouse cursor position during Ctrl+hover (not every frame)
- [ ] Given `echo "See https://docs.anthropic.com for details"`, when Ctrl+hovering over the URL, then it is detected
- [ ] Given a TUI frame `┌──────────┐`, when Ctrl+hovering, then no hyperlink is detected
- [ ] Given a URL wrapped across two lines without OSC 8 `id`, when hovering, then only the visible segment on the hovered line is detected

#### US-007: Hyperlink hover rendering and click-to-open
**Description:** As a terminal user, I want URLs to show as underlined text when hovering with Ctrl held, and I want Ctrl+click to open the URL in my default browser.

**Priority:** P1
**Size:** M (3 pts)
**Dependencies:** US-005, US-006

**Acceptance Criteria:**
- [ ] When Ctrl is held and mouse hovers over a detected hyperlink, the link text renders with underline and a distinct color (e.g., blue from theme or a `link_text` theme slot)
- [ ] Ctrl+click on a detected URL opens it via `open::that(url)` or equivalent
- [ ] A tooltip showing the full URL appears near the cursor when hovering (rendered as a GPUI element above the terminal content)
- [ ] When Ctrl is released, link styling reverts to normal terminal colors
- [ ] Given a long URL `https://github.com/anthropics/claude-code/pull/12345/files#diff-abc`, when hovering, then the tooltip shows the full URL
- [ ] Given Ctrl+click on a non-allowlisted scheme, when clicked, then nothing happens and no error is shown
- [ ] Given no Ctrl held, when clicking on a URL, then normal terminal click behavior occurs (no link activation)

---

### EP-003: Terminal State Resilience

Ensure terminal state is correctly managed across TUI agent lifecycle — startup, focus changes, crashes, and clean exits. After this epic, the terminal never enters a broken state from agent sessions.

**Definition of Done:** Bracketed paste mode is reset on child exit. Terminal modes are tracked per-PTY and cleaned up on PTY death. Synchronized output is respected.

#### US-008: Bracketed paste mode cleanup on child exit
**Description:** As a terminal user, I want bracketed paste mode to be automatically reset when a child process exits so that the terminal never gets stuck in bracketed paste mode after a Claude Code crash.

**Priority:** P0
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] On `AlacEvent::ChildExit` or `AlacEvent::Exit`, send `\x1b[?2004l` (disable bracketed paste) to the PTY if `TermMode::BRACKETED_PASTE` was active
- [ ] On `AlacEvent::ChildExit`, also reset `FOCUS_IN_OUT`, `MOUSE_MODE`, and `ALT_SCREEN` modes via their disable sequences
- [ ] Mode reset happens before the "process exited" overlay is shown
- [ ] Given Claude Code running with bracketed paste enabled, when killing the process (close pane), then the next terminal session does NOT have leftover bracketed paste markers in paste output
- [ ] Given a normal shell exit (Ctrl+D), when the shell exits cleanly, then no unnecessary mode resets are sent (the shell already cleaned up)
- [ ] Given Claude Code crashes (SIGKILL), when the PTY closes, then all terminal modes are reset to defaults

#### US-009: Synchronized output support
**Description:** As a TUI user, I want PaneFlow to respect DEC synchronized output markers so that rapid redraws from Claude Code and Codex do not cause visual tearing.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] Verify that alacritty_terminal's built-in `StdSyncHandler` (DCS `=1s` / `=2s` with 150ms timeout) is active and functioning in PaneFlow's VTE pipeline
- [ ] When a TUI app sends `DCS =1s` (begin sync), output accumulates in the sync buffer without triggering repaints
- [ ] When `DCS =2s` (end sync) is received OR the 150ms timeout elapses, the buffered output is flushed and a single repaint is triggered
- [ ] The existing `sync_bytes_count` check in `pty_reader_loop` at `terminal.rs:2030-2037` correctly gates Wakeup events during sync
- [ ] Given Claude Code rendering a large diff output, when observing the terminal, then no tearing or partial frames are visible
- [ ] Given a stuck sync (app crashes during sync buffer), when 150ms elapses, then the buffer is flushed automatically (no hang)

#### US-010: IPC SendText and SendKeystroke for agent interaction
**Description:** As an AI agent orchestration tool, I want to send text and keystrokes to a running terminal session via IPC so that PaneFlow's embedded wrapper scripts (`claude`, `codex`) can control terminal sessions programmatically.

**Priority:** P1
**Size:** S (2 pts)
**Dependencies:** None

**Acceptance Criteria:**
- [ ] IPC method `surface.send_text(surface_id, text)` writes arbitrary text bytes to the terminal's PTY (no bracketed paste wrapping)
- [ ] IPC method `surface.send_keystroke(surface_id, keystroke)` converts a keystroke description (e.g., `"ctrl-c"`, `"enter"`) to escape sequence via `to_esc_str()` and writes to PTY
- [ ] Both methods validate `surface_id` and return a JSON-RPC error for invalid IDs
- [ ] Given a running `cat` process, when calling `surface.send_text(id, "hello\n")`, then `hello` appears followed by a newline
- [ ] Given an invalid surface_id, when calling `surface.send_text`, then a JSON-RPC error with code `-32602` is returned

---

## Functional Requirements

- FR-01: The system must validate all TUI agent features against a documented test matrix before any release
- FR-02: The system must detect OSC 8 hyperlinks from cell attributes and regex-detected bare URLs
- FR-03: The system must NOT open URLs with schemes other than http, https, mailto, and file (with localhost validation)
- FR-04: The system must reset terminal modes (bracketed paste, mouse, focus, alt screen) on child process exit
- FR-05: The system must respect DEC synchronized output markers to prevent TUI tearing
- FR-06: The system must provide IPC methods for programmatic terminal interaction

## Non-Functional Requirements

- **Reliability:** Zero terminal state leaks after TUI agent exit — all modes reset to defaults
- **Performance:** URL regex detection must complete within 1ms per line (lazy, on-hover only — not every frame)
- **Security:** OSC 8 URL scheme allowlist enforced. No arbitrary scheme opens. `file://` restricted to localhost.
- **Compatibility:** All test matrix cases pass with Claude Code v2.x and Codex CLI latest

## Edge Cases & Error States

| # | Scenario | Trigger | Expected Behavior |
|---|----------|---------|-------------------|
| 1 | Claude Code crashes mid-output | SIGKILL during render | All terminal modes reset, exit overlay shown |
| 2 | OSC 8 with extremely long URL | Malicious 100KB URL | Truncate at 2048 bytes, store truncated version |
| 3 | Bracketed paste already disabled on exit | Normal shell cleanup | No double-disable sent, no error |
| 4 | URL in middle of box-drawing frame | `│https://x.com│` | URL detected between box chars, box chars not included |
| 5 | Focus change during synchronized output | Ctrl+Tab while sync buffering | Focus events queued, delivered after sync flush |
| 6 | IPC send_text to exited terminal | Race condition | JSON-RPC error returned, no panic |
| 7 | Middle-click paste with empty primary selection | No prior selection | No-op, no error |
| 8 | Image paste with no image in clipboard | Only text in clipboard | Normal text paste, no raw 0x16 sent |

## Risks & Mitigations

| # | Risk | Probability | Impact | Mitigation |
|---|------|------------|--------|------------|
| 1 | Validation discovers many bugs | Medium | Medium | US-004 is sized L (5pts) as buffer. If >5 bugs, create follow-up PRD. |
| 2 | OSC 8 cell attribute not accessible | Low | Medium | **VALIDATED:** `Cell::hyperlink()` confirmed at `term/cell.rs:219` in Alacritty fork. |
| 3 | URL regex performance impact | Low | Low | Lazy detection (on Ctrl+hover only), compiled once, per-line scope. |
| 4 | Bracketed paste reset breaks nested sessions | Low | Medium | Only reset on ChildExit, not on normal mode changes. Check if process group leader is dead. |
| 5 | Synchronized output timeout (150ms) too long | Low | Low | Alacritty's default, battle-tested. Monitor but don't change. |

## Non-Goals

- **Kitty keyboard protocol** — Requires upstream alacritty_terminal changes. Deferred to future PRD.
- **OSC 5522 (Kitty clipboard extension)** — Kitty-only as of April 2026. Not enough ecosystem adoption.
- **Sixel/Kitty/iTerm2 inline image protocols** — No image rendering in terminal grid. Image paste forwards raw Ctrl+V.
- **Automated CI/CD** — PaneFlow has no CI. Test matrix is manual. Automation is a separate initiative.
- **Vi mode for scrollback** — Copy mode covers basic keyboard selection. Vim-style motions deferred.

## Files to Modify

| File | Changes |
|------|---------|
| `src-app/src/terminal.rs` | US-008: mode reset on ChildExit. US-009: verify sync handler. US-010: IPC send_text/send_keystroke dispatch. |
| `src-app/src/terminal_element.rs` | US-005: read `cell.hyperlink()` during layout. US-006: regex URL detection. US-007: hover rendering + tooltip. |
| `src-app/src/ipc.rs` | US-010: add `surface.send_text` and `surface.send_keystroke` methods. |
| `src-app/src/theme.rs` | US-007: add `link_text` color slot (optional). |

## Files NOT to Modify

- `src-app/src/keys.rs` — All key mappings already implemented and correct.
- `src-app/src/mouse.rs` — Mouse protocol already correct (button 3 on release).
- `src-app/src/pty.rs` — PTY backend complete, SHLVL handled.
- `src-app/src/split.rs` — Split system not in scope.
- `crates/paneflow-config/` — No config changes needed for Release 1.

## Technical Considerations

- **OSC 8 implementation path:** Read `cell.hyperlink()` during `build_layout()` cell iteration. Group adjacent cells with the same hyperlink `id` into `HyperlinkZone` structs. Store in `LayoutState`. During `paint()`, if Ctrl is held and mouse is over a zone, apply link style. Ctrl+click dispatches `open::that(url)`.
- **URL regex:** Compile once in `TerminalElement::new()` or as a `lazy_static`. Pattern should match Zed's `terminal_hyperlinks.rs` URL regex. Exclude box-drawing range U+2500-257F from first-char position.
- **Mode reset timing:** In `sync()` match on `AlacEvent::ChildExit`, before any UI state update, write mode reset sequences to PTY. The PTY is still open at this point (child exited but fd not closed), so writes succeed.
- **Synchronized output:** Already gated by `processor.sync_bytes_count()` check in `pty_reader_loop` at `terminal.rs:2030-2037`. US-009 is primarily verification + documentation, not new code.

## Success Metrics

| Metric | Baseline (current) | Target | Timeframe | How Measured |
|--------|-------------------|--------|-----------|-------------|
| TUI agent test matrix pass rate | 0% (no matrix exists) | 100% | Release 1 | Manual test matrix execution |
| Terminal state leaks after agent exit | Unknown | 0 | Release 1 | Bracketed paste mode test after kill |
| URL clickability | 0% (no hyperlink support) | 100% of detected URLs | Release 2 | Count clickable vs non-clickable URLs in Claude Code session |
| Validation bugs found and fixed | N/A | All found bugs fixed | Release 1 | Bug count from US-002/US-003 |

## Dependency Graph

```
US-001 (test matrix)
  ├── US-002 (validate Claude Code) ──┐
  └── US-003 (validate Codex CLI) ────┤
                                       └── US-004 (fix bugs)

US-005 (OSC 8 detection)
  └── US-006 (regex URL detection)
       └── US-007 (hover + click-to-open)

US-008 (bracketed paste cleanup) — independent
US-009 (synchronized output) — independent
US-010 (IPC send_text) — independent
```

Release 1: US-001 → US-002 + US-003 (parallel) → US-004 → US-008
Release 2: US-005 → US-006 → US-007, US-009, US-010 (parallel)

## Relationship to Existing PRDs

This PRD **supersedes** `prd-zed-terminal-parity.md` for the scope of TUI agent support. The parity PRD's 28 stories covered broader terminal compliance; this PRD is focused specifically on Claude Code and Codex CLI. Stories from the parity PRD that are not covered here (IME, drag-and-drop, display-only terminal, vi mode) remain valid for a future PRD but are not blocking TUI agent workflows.
[/PRD]
