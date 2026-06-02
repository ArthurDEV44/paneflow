# Changelog

Notable changes to Paneflow are summarized here. Release artifacts and full
notes are available on the [GitHub Releases](https://github.com/ArthurDEV44/paneflow/releases) page.

## [Unreleased]

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

[Unreleased]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.7...HEAD
[0.3.7]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/ArthurDEV44/paneflow/releases/tag/v0.3.5
[0.3.4]: https://github.com/ArthurDEV44/paneflow/releases/tag/v0.3.4
[0.3.3]: https://github.com/ArthurDEV44/paneflow/releases/tag/v0.3.3
[0.3.2]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/ArthurDEV44/paneflow/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/ArthurDEV44/paneflow/compare/v0.2.17...v0.3.0
