# Changelog

Notable changes to Paneflow are summarized here. Release artifacts and full
notes are available on the [GitHub Releases](https://github.com/ArthurDEV44/paneflow/releases) page.

## [Unreleased]

## [0.5.2] - 2026-06-15

A Windows hotfix: the in-app updater now works on MSI installs. No changes on
Linux or macOS.

### Fixed

- Windows self-update. Clicking "Update" on an MSI install failed with "HOME
  environment variable is not set" and never updated. The running binary's
  install location was misdetected — `std::fs::canonicalize` returns the
  extended-length `\\?\C:\…` path on Windows, which did not match
  `%ProgramFiles%`, so the install was classified as unknown and the updater
  fell back to the Linux tar.gz path (which reads `$HOME`). MSI installs are
  now detected correctly and the update runs through msiexec end-to-end. As a
  safety net, an unknown install on Windows no longer routes to the Linux
  updater either.

  Note: because the currently-running build carries the old, broken detection,
  it cannot self-update to this fix — install the 0.5.2 `.msi` manually once
  from the releases page, and the in-app updater will work for every release
  after it.

## [0.5.1] - 2026-06-15

A Windows polish patch on top of 0.5.0: the app and installer now carry the
right icon, and the stray console window is gone. No changes on Linux or macOS.

### Fixed

- No more stray console window on Windows. paneflow.exe is now built as a
  GUI-subsystem binary, so launching it from Explorer, a shortcut or the Start
  Menu no longer opens an empty extra terminal window beside the app. The
  scriptable CLI (paneflow mcp install, paneflow ls, --version, …) still works:
  the process re-attaches to the parent console when started from a terminal.
- The paneflow.exe icon in Explorer. The bare executable embedded no Windows
  resource and fell back to the generic Windows icon; it now ships the same
  multi-resolution PaneFlow icon as the installer.
- The Windows installer icon. The 0.5.0 MSI still showed the old logo on its
  Start Menu shortcut and Add-or-Remove-Programs entry — the WiX icon was the
  one output the new-logo regeneration had missed. It is now regenerated from
  the new logo, and the icon pipeline mirrors it on every run so it can no
  longer go stale.

### Documentation

- Refreshed the Windows install docs for the signed v0.5.0 .msi: the native
  installer is now documented as an available path (WSL2 kept as the
  alternative), with a SmartScreen "Run anyway" walkthrough (publisher:
  StriveX) and signature-verification steps, replacing the stale "no native
  build / Q3 2026" framing across the docs.

## [0.5.0] - 2026-06-15

This release brings Paneflow to Windows and lands a ground-up redesign of the
app shell.

### Added

- Windows support. Paneflow now runs on Windows 10 and 11. The title bar
  carries native Windows 11 caption buttons and a full-width inset panel, new
  terminals default to PowerShell, and live agent-status updates are delivered
  reliably over named pipes.
- Inline settings. The settings window is replaced by a Codex-style settings
  surface embedded directly in the app, built on a shared set of select,
  toggle and card primitives, with every page rebuilt on those controls.
- The PaneFlow Light theme returns, paired with a light app shell, and the
  window backdrop now seeds itself from the active theme mode.
- Configurable font fallbacks. A user-editable font_fallbacks list lets you
  control the monospace fallback chain.

### Changed

- Cockpit chrome redesign. A reworked window chrome with a native backdrop,
  title-bar Files and Help menus, a Profile menu, and a sidebar toggle. The
  title bar now spans the full window width on every desktop platform.
- One menu language across the app. The title-bar dropdowns, the workspace and
  agents context menus, the theme picker, and the diff scope, project, branch
  and base pickers all share a single elevated surface and select-row style.
- The agent launcher is laid out as a grid of filled tiles, and the agents
  sidebar search field matches the settings search pill.
- The About dialog is restyled as a native app dialog, and hover backgrounds
  align with the active selected state.
- The option-as-meta default is now platform specific.

### Fixed

- Self-update reliability across platforms: the macOS app bundle relaunches
  correctly and handles translocation, AppImage installs are detected via
  $APPIMAGE with the right package-manager routing, the Fedora upgrade path
  refreshes its metadata first, and a mismatched-signature install surfaces a
  clearer hint.
- Terminal teardown is guarded against PID reuse and works on kernels built
  without CONFIG_PROC_CHILDREN.
- The GUI now adopts the login-shell PATH on launch, so tools on your shell
  PATH are found when Paneflow is started from a launcher.
- Turn-end desktop notifications carry the Paneflow icon, and widget text
  keybindings are re-registered on every keymap apply.
- Linux packages depend on fontconfig so the settings font picker is
  populated.

## [0.4.4] - 2026-06-11

### Changed

- The in-pane find bar is now a real editable field. It hosts the same text
  input the agent sidebar uses, so opening a search puts a live caret in the
  field with selection, IME and clipboard support, and the query updates the
  match list as you type. Its chrome follows the active theme (One Dark /
  PaneFlow Light) instead of a fixed palette, with search, regex, fleet,
  previous, next and close controls, and a status line that reads the match
  position, an empty result, or an invalid pattern.
- Every agent other than Claude Code now shows the same rotating arc the agent
  sidebar uses while it is thinking, in a soft neutral grey, replacing the
  Codex-style pulsing dots. Claude Code keeps its own glyph spinner and salmon
  identity colour.

## [0.4.3] - 2026-06-11

### Added

- Composer: a bottom-anchored multi-line input (secondary-shift-space) over
  the focused pane. Enter pre-fills the agent through bracketed paste
  without submitting, so the prompt is yours to review before it is sent;
  secondary-enter pre-fills and submits in one keystroke.
- Named broadcast groups: assign panes to a group (secondary-shift-b to
  toggle membership, secondary-shift-m for the picker), each marked by a
  3px coloured edge stripe. The Composer fans one prompt out to every live
  member of the active group and shows a transient recap of who received
  it, so a broadcast is never silent.
- Queued prompts for busy agents: a prompt sent to a generating agent is
  held ("1 queued" tab chip) and flushed automatically on that session's
  next idle transition, instead of being dropped or spliced into the
  running turn.
- Attention Queue (secondary-shift-k): a single overlay listing every agent
  waiting for input across all workspaces, with its question and how long
  it has waited, longest-waiting first. Enter warps straight to that pane.
- Launch Pad (secondary-shift-l): worktree, split, agent launch and
  first-prompt prefill in one gesture.
- Agent status beyond Claude Code and Codex: the sidebar states, tab dots
  and notifications now apply to any agent CLI launched through the shimmed
  PATH, identified by its binary name; an unrecognized tool is reported as
  itself instead of being mislabeled as Claude.
- Scrollbar match rail: an active search projects every match as a tick on
  the scrollbar track (decimated to the pixel grid, so 10 000 matches cost
  the same as 10), with the existing proportional track click to jump.
- Fleet grep: from any pane's find bar, the "Fleet" toggle (or Alt+F) runs
  the same query across every pane of every workspace off the render
  thread, lists the matching panes with counts, flashes a transient match
  badge on their tabs, and Enter teleports with the local search pre-armed.
- Per-pane font zoom: Ctrl+= / Ctrl+- / Ctrl+0 (Cmd on macOS) change the
  focused pane's font size by 1 px steps within 8-32 px, with the PTY grid
  reflowing like a window resize. Persisted per pane across restarts;
  panes without an override keep following the global setting.

- Fleet observability: the port/process scan now attributes results to each
  pane. Tabs show a compact identity pill for the agent CLI running inside
  (PID-detected across 16 known agents, persisted across restarts as a
  dimmed "last known" until confirmed) and per-pane port badges, clickable
  when the port belongs to a frontend dev server. When a pane announces a
  URL whose port is actually owned by another pane, its badge turns into an
  alert naming the owner.

- Errored agent state: when an agent CLI launched through the shimmed PATH
  exits non-zero, its session turns red (tab dot + sidebar badge) and the
  desktop notification says "agent exited (exit N)" instead of a false
  "agent finished". Human interrupts (Ctrl+C, pane close, external kill)
  still count as finished, never as errors.
- Stalled agent detection (on by default): a thinking agent that emits no
  hook activity for 5 minutes is flagged "stalled" in the sidebar, with one
  desktop notification per stall episode. Threshold configurable via
  `agent_stall_threshold_secs`; kill switch via `agent_stall_detection`.

### Changed

- Dev-server detection is now OS-authoritative. A port badge's clickable
  link is derived from the command line of the process that owns the
  socket, so it no longer depends on catching the dev server's banner line
  in the terminal output. The link survives an in-shell restart (nodemon, a
  plain re-run) that re-binds the port, and sustained agent output no longer
  starves the scan that picks up new ports.

### Fixed

- Agent sessions are reaped the moment their pane closes instead of
  lingering up to 30s for the periodic sweep, covering the cases where the
  exit hook never arrives (shim killed, agent started without the shim).
- A recycled process id can no longer keep a finished agent's status alive:
  a session pins its process start time, and a reused pid whose start time
  differs is treated as gone.

## [0.4.2] - 2026-06-10

### Changed

- New logo artwork. Every icon size (16-512, master 1024, .icns, .ico) is
  regenerated with a transparent keyline margin: the squircle body is
  rendered at ~80% of the canvas, the value GNOME and macOS icon grids
  converge on, so the icon no longer renders oversized next to
  spec-compliant peers in the GNOME Shell dash and macOS dock.

## [0.4.1] - 2026-06-10

### Added

- Live activity indicator on Agents thread rows: a row whose agent is
  working shows a Codex-style spinner, driven by the same `ai.*` signals as
  the pane badges.

### Changed

- Agents panel polish: stronger selected-row contrast against the rail, a
  faint hairline between rail and panel, and a 16px panel corner radius
  matching the Cli/Diff silhouette.

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

[Unreleased]: https://github.com/ArthurDEV44/paneflow/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/ArthurDEV44/paneflow/compare/v0.4.4...v0.5.0
[0.4.4]: https://github.com/ArthurDEV44/paneflow/compare/v0.4.3...v0.4.4
[0.4.3]: https://github.com/ArthurDEV44/paneflow/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/ArthurDEV44/paneflow/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/ArthurDEV44/paneflow/compare/v0.4.0...v0.4.1
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
