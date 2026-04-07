[PRD]
# PRD: AI Tool Hooks Integration (Claude Code + Codex)

## Changelog

| Version | Date | Author | Summary |
|---------|------|--------|---------|
| 1.0 | 2026-04-07 | Claude + Arthur | Initial draft — replace title-based detection with hooks-based integration modeled after cmux |

## Problem Statement

PaneFlow's current AI tool detection watches for braille spinner characters in the terminal's OSC title to infer whether Claude Code or Codex is "thinking". This approach is fundamentally broken:

1. **False positives on startup** — launching `claude` immediately shows a braille spinner in the title during the loading phase, before any prompt is submitted. The sidebar shows "Thinking" when Claude is actually idle.
2. **Cannot distinguish states** — the title contains `"{spinner} {project_name}"` regardless of whether the tool is loading, thinking, waiting for input, or running a tool. All states produce the same spinner signal.
3. **Incorrect exit handling** — Ctrl+C kills the spinner, triggering a grace timeout that transitions to "Needs input" instead of "Finished". SIGKILL leaves stale state permanently.
4. **No tool identification in title** — neither Claude Code nor Codex puts its name in the OSC title. The title is `"⠋ paneflow"` (project name), requiring fragile `/proc/cmdline` introspection to identify which tool is running.
5. **Performance overhead** — every title change (10/second during spinner) triggers the detection pipeline, including potential syscalls for process identification.

**The right solution exists:** Claude Code exposes 26 lifecycle hooks via `settings.json` that provide structured, precise lifecycle events (SessionStart, UserPromptSubmit, PreToolUse, Notification, Stop, SessionEnd). cmux uses these hooks with a wrapper script that injects `--settings '{"hooks":{...}}'` to route events through its IPC socket. This is the standard integration pattern endorsed by Anthropic.

**Why now:** The title-based detector has been iterated 4 times and still doesn't work reliably. The hooks API is stable and documented. cmux provides a proven reference implementation.

## Overview

Replace the title-based AI detection system with a hooks-based integration that uses Claude Code's official lifecycle API. The system consists of:

1. **Wrapper scripts** (`claude`, `codex`) that intercept AI tool commands, inject lifecycle hooks, and route events to PaneFlow's IPC socket
2. **New IPC methods** (`ai.session_start`, `ai.prompt_submit`, `ai.tool_use`, `ai.notification`, `ai.stop`, `ai.session_end`) that update workspace state from hook events
3. **Environment injection** at PTY spawn — prepend wrapper directory to PATH and set `PANEFLOW_SURFACE_ID`, `PANEFLOW_WORKSPACE_ID`, `PANEFLOW_SOCKET_PATH`
4. **Stale PID sweep** — 30-second timer using `kill(pid, 0)` to detect crashed AI tool processes and clean up sidebar state
5. **Removal of `ai_detector.rs`** — the entire title-based detection system is deleted

Key decisions:
- **Wrapper script over global config** — wrapper only activates inside PaneFlow terminals (env var guard), doesn't conflict with user's existing hooks, doesn't affect Claude sessions outside PaneFlow
- **Tool-agnostic IPC prefix `ai.*`** — same methods work for both Claude Code and Codex
- **cmux-compatible architecture** — same hook events, same env var pattern, same stale PID sweep. Users familiar with cmux will recognize the design

## Goals

| Goal | Target | Measurement |
|------|--------|-------------|
| Detection accuracy | 100% correct state for all Claude Code lifecycle events | Manual test: submit prompt → Running, tool use → Running, notification → Needs input, stop → Idle, exit → cleared |
| False positive rate | 0 false "Thinking" triggers when Claude is idle or loading | Manual test: launch claude, wait 10s without prompting — sidebar stays Inactive |
| Stale state cleanup | Stale sidebar entries cleared within 30 seconds of process death | Kill -9 claude process, time until sidebar clears |
| Hook overhead | < 50ms from hook event to sidebar update | Timestamp in hook script vs sidebar render (debug log) |

## Target Users

### Developer running Claude Code in PaneFlow
- **Role:** Software engineer using Claude Code as a coding assistant inside PaneFlow terminal panes
- **Behaviors:** Launches `claude` in a PaneFlow pane, submits prompts, reviews tool use, responds to notifications, exits with Ctrl+C or `/exit`
- **Pain points:** Sidebar shows wrong state (Thinking when idle, Needs input when exited), no way to tell at a glance if Claude is actually working
- **Success looks like:** Sidebar card accurately reflects Claude's real state at all times — Running (with tool name), Needs input, Idle, or nothing when inactive

## Research Findings

### Claude Code Hooks API (Anthropic, official)
- **26 lifecycle events** available, configured via `settings.json` `hooks` key
- **Subprocess execution model**: hook command receives JSON on stdin, returns JSON on stdout, exit code 0 = success, exit code 2 = block action
- **Relevant events for sidebar**: `SessionStart` (register PID), `UserPromptSubmit` (Running), `PreToolUse` (Running + tool name), `Notification` (Needs input), `Stop` (Idle), `SessionEnd` (clear all)
- **Matcher syntax**: empty string or `"*"` matches all tools. `PreToolUse` is the only hook that should run async (so it doesn't block tool execution)
- **`--settings` flag merges** with existing user hooks — does not replace them

### Codex CLI Hooks (OpenAI, official)
- **5 events**: `SessionStart`, `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`
- **Config**: `~/.codex/hooks.json` — simpler format than Claude Code
- **Same stdin/stdout contract** as Claude Code
- **No `SessionEnd` or `Notification` events** — stale PID sweep is essential for cleanup

### cmux Reference Implementation
- **Wrapper script** at `Resources/bin/claude` (202 lines bash) — guards on `CMUX_SURFACE_ID` env var, injects `--settings` with hooks JSON
- **6 hook events** routed to `cmux claude-hook <subcommand>` CLI
- **Session persistence** in `~/.cmuxterm/claude-hook-sessions.json` (7-day TTL)
- **Stale PID sweep** every 30s via `kill(pid, 0)` + ESRCH check
- **PATH injection** at terminal spawn — prepends `Resources/bin/` to child process PATH
- **Env vars injected**: `CMUX_SURFACE_ID`, `CMUX_WORKSPACE_ID`, `CMUX_SOCKET_PATH`

### PaneFlow Existing IPC
- **JSON-RPC 2.0** at `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`
- **50ms polling loop** in GPUI async task drains mpsc channel
- **Per-request response channel** — synchronous request/response pattern
- **Adding methods**: add match arm in `handle_ipc` + declare in `system.capabilities`

## Scope Boundaries

### In Scope
- Claude Code wrapper script with hook injection
- New IPC methods for AI lifecycle events
- Environment injection at PTY spawn (PATH + env vars)
- Stale PID sweep timer
- Sidebar state updates from IPC events
- Removal of `ai_detector.rs` title-based detection
- Debug log file support (`PANEFLOW_AI_DEBUG=1`)

### Out of Scope
- Codex wrapper script (deferred — architecture supports it, implementation follows)
- Verbose tool descriptions in sidebar (e.g., "Reading foo.rs") — deferred to follow-on
- Notification relay to desktop notifications (existing `notify-send` hook in user's config handles this)
- Session persistence file (cmux's `claude-hook-sessions.json`) — not needed for MVP; PID sweep handles cleanup
- Transcript summarization for stop notifications
- Hook configuration UI in PaneFlow settings

## Epic Structure

### EP-001: Environment & Wrapper Foundation
Prepare the terminal environment and wrapper script infrastructure.

### EP-002: IPC Methods & State Management
Add IPC methods for AI lifecycle events and update workspace state.

### EP-003: Stale Session Cleanup
Detect and clean up crashed AI tool processes.

### EP-004: Cleanup & Migration
Remove the old title-based detector and debug infrastructure.

## User Stories

### EP-001: Environment & Wrapper Foundation

#### US-001: Inject PaneFlow env vars at PTY spawn
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** None

As a PaneFlow terminal, I need to expose workspace/surface identity and IPC socket path as environment variables so that wrapper scripts can route events to the correct workspace.

**Acceptance Criteria:**
- [ ] `PANEFLOW_WORKSPACE_ID` is set to the workspace's unique ID (u64) in the child process environment
- [ ] `PANEFLOW_SURFACE_ID` is set to a unique pane identifier in the child process environment
- [ ] `PANEFLOW_SOCKET_PATH` is set to the IPC socket path (`$XDG_RUNTIME_DIR/paneflow/paneflow.sock`)
- [ ] Environment variables are NOT set when PaneFlow is not running (no system-wide pollution)
- [ ] Existing terminal functionality is not affected by the new env vars

**Implementation notes:** Modify `TerminalState::new()` in `terminal.rs` to add env vars to the PTY child process environment before spawn. Use the workspace ID from the `Workspace` struct and generate a surface ID from the `TerminalView` entity.

---

#### US-002: Write Claude Code wrapper script
**Priority:** P0 (Must Have) | **Size:** M (3) | **Dependencies:** US-001

As a PaneFlow user, when I type `claude` in a PaneFlow terminal, I want the command to be intercepted by a wrapper that injects lifecycle hooks routing to PaneFlow's IPC socket.

**Acceptance Criteria:**
- [ ] Wrapper script at `src-app/assets/bin/claude` is a valid bash script
- [ ] Guard: if `PANEFLOW_SURFACE_ID` is not set, exec the real `claude` binary unchanged (transparent passthrough)
- [ ] Guard: if IPC socket is not reachable (1s timeout), exec the real `claude` unchanged
- [ ] Injects `--settings '{"hooks":{...}}'` with hooks for: `SessionStart`, `UserPromptSubmit`, `PreToolUse` (async), `Notification`, `Stop`, `SessionEnd`
- [ ] Each hook calls a lightweight IPC helper: `paneflow-hook <event>` which reads stdin JSON and sends to socket
- [ ] Exports `PANEFLOW_CLAUDE_PID=$$` before exec (for stale PID tracking)
- [ ] Unsets `CLAUDECODE` env var to prevent nested session detection interference
- [ ] Generates a session UUID if no `--session-id` / `--resume` / `--continue` flag is passed

---

#### US-003: Write IPC hook helper script
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-001

As the wrapper script, I need a lightweight helper that reads Claude Code's JSON hook payload from stdin and sends it as a JSON-RPC 2.0 request to PaneFlow's IPC socket.

**Acceptance Criteria:**
- [ ] `src-app/assets/bin/paneflow-hook` is a valid bash script
- [ ] Reads the hook event name from `$1` argument
- [ ] Reads the JSON payload from stdin (compact, single read)
- [ ] Maps event name to IPC method: `session-start` → `ai.session_start`, etc.
- [ ] Sends JSON-RPC 2.0 request to `$PANEFLOW_SOCKET_PATH` via `socat` or netcat (with fallback)
- [ ] Includes `workspace_id`, `surface_id`, `pid` from environment in the request params
- [ ] Exits 0 on success, non-zero on socket error (non-blocking — does not prevent Claude from running)
- [ ] Total execution time < 100ms (no heavy dependencies)

---

#### US-004: Prepend wrapper directory to terminal PATH
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-002

As PaneFlow, I need to make the wrapper scripts available to terminals by prepending their directory to the child process PATH.

**Acceptance Criteria:**
- [ ] At PTY spawn, wrapper scripts are written from embedded assets to `$XDG_RUNTIME_DIR/paneflow/bin/`
- [ ] Scripts are written with executable permissions (0o755)
- [ ] `$XDG_RUNTIME_DIR/paneflow/bin/` is prepended to the child process PATH
- [ ] PATH is not modified if the directory is already present (deduplication)
- [ ] The wrapper directory is created if it doesn't exist
- [ ] User's original PATH entries are preserved in order

---

### EP-002: IPC Methods & State Management

#### US-005: Implement `ai.session_start` IPC method
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-001

As the hook system, when Claude Code starts a session, I need to register the agent PID with the correct workspace for stale detection.

**Acceptance Criteria:**
- [ ] Method `ai.session_start` accepts params: `workspace_id`, `surface_id`, `pid`, `tool` ("claude" or "codex"), `session_id`
- [ ] Stores the agent PID in the workspace's `agent_pids: HashMap<String, u32>` (keyed by tool name)
- [ ] Does NOT change the sidebar visible state (SessionStart just registers, doesn't mean "thinking")
- [ ] Returns `{"registered": true}` on success
- [ ] Handles invalid workspace_id gracefully (returns error, does not panic)

---

#### US-006: Implement `ai.prompt_submit` IPC method
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-005

As the hook system, when the user submits a prompt to Claude Code, I need to update the sidebar to show "Running".

**Acceptance Criteria:**
- [ ] Method `ai.prompt_submit` accepts params: `workspace_id`, `surface_id`, `tool`
- [ ] Sets `workspace.ai_state = AiToolState::Thinking(tool)` and triggers `cx.notify()` for sidebar repaint
- [ ] Starts the loader animation (reuse existing `start_loader_animation`)
- [ ] Clears any pending notifications for this workspace
- [ ] Returns `{"status": "running"}`

---

#### US-007: Implement `ai.tool_use` IPC method
**Priority:** P1 (Should Have) | **Size:** S (2) | **Dependencies:** US-006

As the hook system, when Claude Code uses a tool, I need to keep the sidebar showing "Running" (and optionally show which tool).

**Acceptance Criteria:**
- [ ] Method `ai.tool_use` accepts params: `workspace_id`, `surface_id`, `tool`, `tool_name` (the Claude tool being used, e.g., "Edit", "Bash")
- [ ] Keeps `workspace.ai_state = AiToolState::Thinking(tool)` (no state change if already Thinking)
- [ ] Stores `tool_name` for potential future verbose display
- [ ] Returns `{"status": "running"}`

---

#### US-008: Implement `ai.notification` IPC method
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-005

As the hook system, when Claude Code sends a notification (needs user input), I need to update the sidebar and push a bell notification.

**Acceptance Criteria:**
- [ ] Method `ai.notification` accepts params: `workspace_id`, `surface_id`, `tool`, `message`
- [ ] Sets `workspace.ai_state = AiToolState::WaitingForInput(tool)` and triggers `cx.notify()`
- [ ] Pushes a `Notification` entry to the bell menu with the message text
- [ ] Returns `{"status": "waiting"}`

---

#### US-009: Implement `ai.stop` IPC method
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-005

As the hook system, when Claude Code finishes responding, I need to update the sidebar to show "Idle" / "Done".

**Acceptance Criteria:**
- [ ] Method `ai.stop` accepts params: `workspace_id`, `surface_id`, `tool`
- [ ] Sets `workspace.ai_state = AiToolState::Finished(tool)` and triggers `cx.notify()`
- [ ] Pushes a "finished" notification to the bell menu
- [ ] After 5 seconds, auto-resets `ai_state` to `Inactive` (reuse existing `FINISHED_RESET` pattern)
- [ ] Returns `{"status": "idle"}`

---

#### US-010: Implement `ai.session_end` IPC method
**Priority:** P0 (Must Have) | **Size:** S (2) | **Dependencies:** US-005

As the hook system, when Claude Code exits (session ends), I need to clear all AI state for this workspace.

**Acceptance Criteria:**
- [ ] Method `ai.session_end` accepts params: `workspace_id`, `surface_id`, `tool`
- [ ] Sets `workspace.ai_state = AiToolState::Inactive`
- [ ] Removes the agent PID from `workspace.agent_pids`
- [ ] Clears any pending notifications for this workspace
- [ ] Triggers `cx.notify()` for sidebar repaint
- [ ] Returns `{"cleared": true}`

---

### EP-003: Stale Session Cleanup

#### US-011: Stale PID sweep timer
**Priority:** P0 (Must Have) | **Size:** M (3) | **Dependencies:** US-005

As PaneFlow, I need to detect when AI tool processes die without firing `SessionEnd` (SIGKILL, crash) and clean up their sidebar state.

**Acceptance Criteria:**
- [ ] A background timer fires every 30 seconds after app startup
- [ ] For each workspace with a registered agent PID, probes with `libc::kill(pid, 0)`
- [ ] If `kill` returns -1 with `ESRCH` (no such process), clears `ai_state` and removes PID from `agent_pids`
- [ ] If `kill` returns -1 with `EPERM` (permission denied), keeps the PID (process exists, just can't signal)
- [ ] If `kill` returns 0, keeps the PID (process alive)
- [ ] Stale cleanup triggers `cx.notify()` for sidebar repaint
- [ ] Does not interfere with normal lifecycle (hooks still fire and update state)

---

### EP-004: Cleanup & Migration

#### US-012: Remove title-based AI detection
**Priority:** P0 (Must Have) | **Size:** M (3) | **Dependencies:** US-006, US-008, US-009, US-010

As the codebase, the old `ai_detector.rs` title-based detection must be removed since hooks now provide accurate state.

**Acceptance Criteria:**
- [ ] `ai_detector.rs` is deleted
- [ ] All references to `AiToolDetector`, `feed_title`, `tick()` are removed from `terminal.rs`
- [ ] The sync loop no longer feeds title changes to a detector
- [ ] The `ai_tick_counter` polling in the sync loop is removed
- [ ] `AiTool` and `AiToolState` enums move to a shared location (or stay in a renamed module)
- [ ] The `TerminalEvent::AiToolStateChanged` event is preserved (now emitted from IPC handler, not detector)
- [ ] `PANEFLOW_AI_DEBUG` file logger is removed
- [ ] All existing tests that referenced the old detector are updated or removed
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace -- -D warnings` is clean

---

#### US-013: Add `agent_pids` field to Workspace
**Priority:** P0 (Must Have) | **Size:** XS (1) | **Dependencies:** None

As the workspace model, I need a field to track registered AI agent PIDs for stale detection.

**Acceptance Criteria:**
- [ ] `Workspace` struct has `agent_pids: HashMap<String, u32>` field (keyed by tool name: "claude", "codex")
- [ ] Initialized as empty in all `Workspace` constructors
- [ ] Serialized/deserialized for session persistence (or excluded if agent state is transient)

## Dependency Graph

```
US-001 (env vars)
  ├── US-002 (claude wrapper) ──┐
  ├── US-003 (hook helper)      ├── US-004 (PATH prepend)
  └── US-013 (agent_pids field) │
       │                        │
       └── US-005 (session_start) ── depends on US-001 + US-013
            ├── US-006 (prompt_submit)
            ├── US-007 (tool_use)
            ├── US-008 (notification)
            ├── US-009 (stop)
            ├── US-010 (session_end)
            └── US-011 (stale PID sweep)
                     │
                     └── US-012 (remove old detector) ── depends on US-006, US-008, US-009, US-010
```

**Critical path:** US-001 → US-002 + US-003 → US-004 → US-005 → US-006/008/009/010 → US-012

**Parallelizable:** US-013 can run anytime. US-007 is P1 and can follow later. US-011 can run after US-005.

## Quality Gates

```bash
# Build
cargo build
cargo build --release

# Tests
cargo test --workspace

# Lint
cargo clippy --workspace -- -D warnings
cargo fmt --check

# Manual verification
PANEFLOW_AI_DEBUG=1 cargo run
# In PaneFlow terminal: claude "explain this project"
# Verify: sidebar shows Running → Needs input or Idle
# Verify: Ctrl+C exits cleanly → sidebar clears
# Verify: kill -9 <claude_pid> → sidebar clears within 30s
```

## Files NOT to Modify

| File | Reason |
|------|--------|
| `crates/paneflow-config/` | Config crate has no AI detection logic — leave untouched |
| `src-app/src/terminal_element.rs` | Rendering layer — no detection changes needed |
| `src-app/src/theme.rs` | Theme definitions — unrelated |
| `src-app/src/split.rs` | Split tree layout — unrelated |
| `src-app/src/keybindings.rs` | Keybinding registration — unrelated |

## Files to Create

| File | Purpose |
|------|---------|
| `src-app/assets/bin/claude` | Claude Code wrapper script (bash) |
| `src-app/assets/bin/paneflow-hook` | IPC hook helper script (bash) |

## Files to Modify

| File | Changes |
|------|---------|
| `src-app/src/terminal.rs` | Add env var injection at PTY spawn; remove `ai_detector` field and title-based feed; remove `scan_ai_spinner` |
| `src-app/src/main.rs` | Add new IPC method handlers in `handle_ipc`; add stale PID sweep timer; write wrapper scripts at startup |
| `src-app/src/workspace.rs` | Add `agent_pids: HashMap<String, u32>` field |
| `src-app/src/ipc.rs` | Declare new methods in `system.capabilities` |
| `src-app/src/pane.rs` | Remove `AiToolStateChanged` from detector-sourced match arm (now IPC-sourced) |

## Files to Delete

| File | Reason |
|------|---------|
| `src-app/src/ai_detector.rs` | Entire title-based detection system replaced by hooks |

[/PRD]
