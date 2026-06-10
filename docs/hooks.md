# Agent notification hooks (`paneflow hooks`)

Paneflow ships a tiny callback binary, `paneflow-ai-hook`, that an agent CLI
runs on lifecycle events (prompt submitted, tool use, stop, notification) to
report its turn state to a running Paneflow instance over the IPC socket. That
state drives the sidebar activity indicators and the turn-end desktop
notification (EP-004, `prd-cli-agent-orchestration`).

There are two ways the hook gets registered, and a single authority rule that
keeps them from firing twice.

## Two installers

| Installer | Scope | Where it writes | Lifetime |
|-----------|-------|-----------------|----------|
| **Ephemeral shim** (`paneflow-shim`) | project | `./.claude/settings.local.json` in the launched project | written on agent launch, swept on exit |
| **Persistent setup** (`paneflow hooks setup`) | user | `~/.claude/settings.json` | written once, survives restarts and Paneflow updates |

The shim copy references the version-pinned binary under
`cache_dir()/paneflow/bin/<VERSION>/`; the persistent copy references the
**stable, non-versioned** path under `data_dir()/paneflow/bin/paneflow-ai-hook`
(`runtime_paths::ai_hook_binary_path`), so the path written into your config
never goes stale across updates.

Both write the *byte-identical* matcher-group shape, tagged with a
`_paneflow_managed` marker, so each side recognizes the other's entries.

## Authority rule (anti double-firing)

**The persistent user-scope install wins.** When `paneflow hooks setup` has
installed managed hooks in `~/.claude/settings.json`, the shim detects them
(`persistent_claude_hooks_present`, reusing the same shape detector) and:

1. **skips** its ephemeral `./.claude/settings.local.json` injection, and
2. **sweeps** any orphan `settings.local.json` it left on a prior run.

Result: the agent fires each event exactly once (one `ai.*` frame per event, no
duplicates) and no `settings.local.json` is planted in your project tree once
you have run `hooks setup`.

If you have **not** run `hooks setup`, the shim's ephemeral injection is the
only mechanism, and it cleans up after itself on exit.

## Commands

```bash
paneflow hooks setup       # install persistent hooks for every supported agent
paneflow hooks status      # report per-agent install state
paneflow hooks uninstall   # remove only Paneflow-managed hooks (no clobber)
```

Exit codes mirror `paneflow mcp`: `0` success (or no agent detected), `1` an
agent errored, `2` usage error. Writes are atomic, backed up, and refuse to
overwrite a present-but-invalid JSON config.

`uninstall` removes only the `_paneflow_managed` matcher-groups; your own hooks
and every other key in the file are left untouched. To fully revert: run
`paneflow hooks uninstall`, then (if you never want the shim's ephemeral copy
either) there is nothing else to clean up because the shim removes its own file
on exit.

## Per-agent support

Only **Claude Code** exposes a verified, file-based user-scope notification-hook
surface, so it is the only agent that receives a persistent install. The other
detected agents are reported honestly rather than given a fabricated shape:

- **Codex**: hooks are injected per-launch by the shim (project scope); no
  user-scope install applies.
- **Gemini / opencode**: no notification-hook mechanism exists today, so there
  is nothing to install (reported as unsupported).

On Windows, Codex uses a JSONL tee rather than file hooks; the shim handles that
path at launch.
