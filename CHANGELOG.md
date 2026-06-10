# Changelog

Notable changes to Paneflow are summarized here. Release artifacts and full
notes are available on the [GitHub Releases](https://github.com/ArthurDEV44/paneflow/releases) page.

## [Unreleased]

## [0.4.0] - 2026-06-10

### Added

- `paneflow` CLI: a scriptable control plane over the IPC socket. `ls`,
  `read` and `search` expose pane scrollback with a unified target selector;
  `new`, `select`, `split` and `focus` drive the layout; `send` feeds text to
  a pane behind a scripting gate and never auto-submits; `key` sends a single
  non-submitting keystroke; `wait` blocks on pane readiness as an
  orchestration primitive.
- `paneflow up`: declarative agent workspaces. One command builds a
  workspace from a spec (per-pane cwd, agent to launch, prompt prefill),
  backed by a `workspace.up` IPC handler.
- Worktree-per-agent: a `worktree = "branch"` field in `up` gives each pane
  its own git worktree, with `.env*` copy, an optional setup command, a
  `${port_offset}` variable for port isolation, clean teardown when the
  workspace closes and pruning of orphaned worktrees at startup.
- `paneflow flow`: a flow engine for multi-agent pipelines. `flow.toml`
  declares spawn steps with `ready.pattern` barriers, gated send steps,
  `foreach` fan-out and fan-in, `capture` to pass data between steps, plus
  validation with cycle detection, `--dry-run`, reporting, exit codes and
  state resume. Submission stays double-gated end to end.
- Attention routing: a pane whose agent waits for input glows and its tab
  shows an attention dot; the desktop notification carries the agent's
  question; `Ctrl+Shift+J` cycles to the next waiting agent across
  workspaces; hovering the pane badge peeks at the question without
  stealing focus.
- Persistent agent-notification hooks: `paneflow hooks setup` installs a
  durable hook for supported agents, `paneflow hooks status` reports each
  agent honestly, and the launch shim defers to a persistent hook instead
  of injecting an ephemeral one.
- Turn-end desktop notification when the window is unfocused.

### Changed

- Agents view rebuilt as a Codex-style cockpit: rail sections (Search,
  Pinned, Projects, Chats), free chats anchored to the home directory, a
  contextual top bar with a thread overflow menu, and unified
  selection/empty states.
- Cockpit chrome across every mode: full-height rails with a floating
  rail-confined title bar, the update call-to-action moved into the sidebar
  in Cli/Agents, a single-row Diff toolbar with the scope breadcrumb
  inline, and quieter text inputs (1px white caret).
- The sessions sidebar now follows the active workspace instead of staying
  bound to the previous one.
- "PaneFlow Light" is temporarily out of the bundled theme set pending a
  light-theme redesign; configs naming it fall back to One Dark.

### Fixed

- A literal `--update-and-exit` token passed as a CLI or hooks argument can
  no longer hijack the process into the self-updater.

## [0.3.9] - 2026-06-09

- Rebuilt the native terminal engine on upstream `alacritty_terminal` with
  rendering parity: OSC 8 hyperlinks, configurable cursor shapes, a live
  scrollbar, and faithful cursor and alt-screen input handling.
- Added PTY teardown and exit-status reporting so a closed shell reports how it
  ended, plus golden snapshot tests that lock terminal rendering against
  regressions.
- Added a Terminal settings tab and a terminal configuration block in the config
  schema and loader.
- Hardened self-update end to end: release artifacts are now signed in CI, every
  download is verified against an embedded minisign key before install, updates
  swap in atomically with crash recovery, and an unsigned build refuses to
  self-update.
- Added per-platform update verification: macOS codesign and spctl gating with
  Team ID pinning, Windows Authenticode through WinVerifyTrust, hardened tar.gz
  and AppImage extraction, and native host architecture detection for Rosetta
  and WOW64.
- Eliminated panics on untrusted input across session restore, config parsing,
  IPC, date handling, and layout, replacing defensive indexing with fail-safe
  accessors.
- Bounded every external surface against resource exhaustion: the IPC server
  caps line size, concurrency, and idle time; external subprocesses run under a
  timeout with a watchdog; ingress and DoS caps are centralized in one module.
- Moved blocking work off the render thread: session saves, git diff stats,
  config loads, font enumeration, and the recursive file watcher now run in the
  background, with a cached config feeding every frame.
- Sanitized untrusted content paths: markdown rendering strips bidi and
  zero-width characters, git refs are stripped of control bytes before they
  reach agent prompts, and session ids are validated to block argument
  injection.
- Validated and clamped all persisted config and session input, with atomic
  write-and-rename for `paneflow.json` and symmetric bounds shared across
  session, IPC, and the config schema.
- Hardened terminal and shim lifecycle: PID-reuse guards, an environment
  deny-list and scrollback sanitization on session restore, codex flock
  serialization, and correct orphan cleanup under systemd.
- Improved Windows portability: portable shell launches, correct LOCALAPPDATA
  casing, Git for Windows PATH augmentation, and `dirs`-based home resolution.
- Reduced per-frame allocations in terminal paint, sidebar recompute, and
  layout, with memoized derivations and zero-allocation leaf lookups.
- Fixed non-US keyboard input, decoupled Alt-on-arrows from the option-as-meta
  setting, and reworked the keybindings editor to be action-indexed with
  collision detection.

## [0.3.8] - 2026-06-02

- Changed the Agents view to a terminal-only model: each thread now launches a
  CLI coding agent directly in a terminal pane with a pre-filled prompt instead
  of an in-app chat, keeping the agent in its native terminal with permission
  bypass respected exactly as the tab-bar buttons do.
- Added eleven launchable agents alongside Claude Code, Codex, OpenCode, Pi, and
  Hermes: Grok, Amp, Cursor, Gemini, Kiro, Antigravity, Copilot, CodeBuddy,
  Factory, Qoder, and Openclaw, each with its own tab-bar button, icon, and
  Settings visibility toggle.
- Each Terminal Thread now remembers which agent it launches and restores it on
  the next session.
- Removed the in-app ACP chat, its conversation timeline and composer, and the
  separate agent sign-in page; agents now authenticate in their own terminal.
- Hardened the Git diff viewer with safer working-tree reads, a shared
  generated-file skip-list, and a watcher-refresh race fix.
- Polished open-source onboarding: community-health files, issue templates, and
  README positioning on the agent cockpit and cross-platform story.

## [0.3.7] - 2026-06-01

- Added an in-app Git diff viewer with file trees, sticky headers, hunk jumps,
  gutter line numbers, per-file diffstats, and word-level highlighting.
- Added branch review flows that open selected agents in real terminal panes
  with a review prompt scoped to the branch worktree.
- Added hunk/file diff copy actions for sending precise context to agents.
- Improved Worktree branch-column behavior so deselecting a branch is explicit.

## [0.3.6] - 2026-05-29

- Added docked Agent Sessions and Files sidebars.
- Added markdown-file opening from the Files panel into an adjacent pane.
- Added drag-to-reorder tabs within a pane and drag-to-move tabs between panes.

## [0.3.5] - 2026-05-29

- Added the Paneflow MCP bridge so capable agents can read pane output through
  `list_panes`, `read_pane`, and `search_pane`.
- Added `paneflow mcp install`, `uninstall`, and `status` commands.
- Added readable pane references, persistent tab renames, and clipboard copy for
  pane references.

## [0.3.4] - 2026-05-28

- Hardened the CLI-agent subsystem for long sessions: bounded caches, parser
  limits, safer IPC behavior, better logging, and reduced retained UI state.
- Improved hot paths for markdown streaming, code highlighting, persisted-item
  collection, and activity-state computation.
- Added CI audit coverage and benchmark baselines for key performance paths.
- Changed `claude_code_bypass_permissions` to default to `false` on fresh
  installs.

## [0.3.3] - 2026-05-27

- Added multi-session tracking for concurrent Claude Code, Codex, and other
  agent sessions in the same workspace.
- Added Ctrl/Cmd-click handling for `file:line:column` references in terminal
  output and assistant messages.
- Added IPC singleton protection to prevent two app instances from racing over
  the same socket.
- Improved ACP client capability declarations for richer Codex and Claude Code
  streams.

## [0.3.2] - 2026-05-26

- Added Terminal Threads as first-class sidebar entries backed by Paneflow's PTY
  stack.
- Added editable project and thread names using the same text widget as the
  composer.
- Added background thread-title generation and title cleanup for agent-provided
  titles.

## [0.3.1] - 2026-05-26

- Maintenance release. See the GitHub compare link for the full commit list.

## [0.3.0] - 2026-05-25

- Opened the 0.3.x release line. See the GitHub compare link for the full commit
  list.

[Unreleased]: https://github.com/ArthurDEV44/paneflow/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.9...v0.4.0
[0.3.9]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.8...v0.3.9
[0.3.8]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/ArthurDEV44/paneflow/releases/tag/v0.3.5
[0.3.4]: https://github.com/ArthurDEV44/paneflow/releases/tag/v0.3.4
[0.3.3]: https://github.com/ArthurDEV44/paneflow/releases/tag/v0.3.3
[0.3.2]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ArthurDEV44/paneflow/compare/v0.2.17...v0.3.0
