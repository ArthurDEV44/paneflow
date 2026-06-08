//! Claude Code hook-config injection + Codex hook config + Windows JSONL tee
//! (US-052 split).

use crate::locate_sibling_hook_binary;
use std::env;
// The cross-platform atomic writers call `tmp.flush()` (method form), which
// needs the `Write` trait in scope on both platforms.
use std::io::Write;
use std::path::{Path, PathBuf};
// US-052: the Windows JSONL-tee path (`run_codex_with_jsonl_tee`, cfg'd out on
// Unix) reuses the exec module's exit-status mapping and needs the process /
// OS-string types. All cfg-gated so they aren't flagged unused on the Unix
// build where the tee code is absent.
#[cfg(not(unix))]
use crate::exec::exit_code_from_status;
#[cfg(not(unix))]
use std::ffi::OsString;
#[cfg(not(unix))]
use std::process::ExitCode;

// ---------------------------------------------------------------------------
// Hook config injection (US-005) — idempotent `.claude/settings.local.json`
// ---------------------------------------------------------------------------

/// Claude Code 2.x hook events the shim registers handlers for. `SubagentStop`
/// is intentionally omitted — the server maps it to `ai.stop` identically to
/// `Stop` (US-002), and registering both would produce duplicate IPC frames.
pub(crate) const CLAUDE_HOOK_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "Notification",
    "Stop",
    "PreToolUse",
    "PostToolUse",
];

/// Bare-name fallback used when `locate_sibling_hook_binary()` cannot resolve
/// the absolute path to the sibling hook binary (test harness, exotic FS,
/// `current_exe` failure). Detection (`is_paneflow_hook_command`) is
/// basename-based so this fallback is also recognized for cleanup, alongside
/// the absolute-path form normally written by `resolve_hook_command`.
///
/// Cleanup matches by basename as a belt-and-suspenders complement to the
/// `_paneflow_managed` marker: Claude Code does not guarantee unknown fields
/// survive re-serialization (anthropics/claude-code#5886), so the command
/// string is the only round-trip-stable identifier we can rely on.
///
/// Known namespace-collision limitation: any user-authored hook command whose
/// program basename is literally `paneflow-ai-hook` (or `.exe` on Windows)
/// will be treated as PaneFlow-managed and removed on cleanup. The basename
/// rule narrows this further than the previous bare-prefix rule did, but the
/// theoretical collision remains.
pub(crate) const HOOK_COMMAND_PREFIX: &str = "paneflow-ai-hook ";

/// Render `path` for inclusion in an `eprintln!` going to the user's
/// terminal. Replaces bytes outside the printable ASCII range (`0x20..=0x7E`)
/// with `?` to defuse ANSI-escape-sequence injection via a maliciously
/// named CWD or `.claude/` directory. Path content is never a secret — it
/// was always going to be visible on stderr — but without this scrub, a
/// crafted directory could clear the screen, set the terminal title, or
/// inject false log lines (Phase 7 security audit, MEDIUM finding).
pub(crate) fn safe_path_display(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .collect()
}

/// Returns true iff `$PANEFLOW_SOCKET_PATH` is set and points at an existing
/// filesystem entry. We treat unset/unreachable as "PaneFlow not running":
/// in that state, installing hook config is pointless (the hooks would
/// invoke `paneflow-ai-hook`, which would fail silently per PRD constraint
/// C4) and we instead sweep any orphan entries left by a previous SIGKILL'd
/// session. Existence-only check keeps this cross-platform — Unix sockets
/// and Windows named pipes both surface as `Path::exists() == true` when
/// the listener is bound.
pub(crate) fn paneflow_ipc_reachable() -> bool {
    let Some(raw) = env::var_os("PANEFLOW_SOCKET_PATH") else {
        return false;
    };
    if raw.is_empty() {
        return false;
    }
    Path::new(&raw).exists()
}

/// Best-effort removal of PaneFlow-managed entries from an existing hook
/// config file when no active IPC channel is reachable. Reads, runs
/// `remove_fn`, writes back (or deletes if the file is now empty). All
/// failures swallow silently — a sweep that fails just retries on the
/// next shim invocation. Used by `install()` to clean up after a previous
/// SIGKILL'd session that never got to fire its `Drop` impl.
pub(crate) fn sweep_orphan_hook_config(
    settings_path: &Path,
    remove_fn: fn(&mut serde_json::Value),
) {
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let before = root.clone();
    remove_fn(&mut root);
    if root == before {
        // Nothing to sweep — file has no PaneFlow entries.
        return;
    }
    let is_empty = root
        .as_object()
        .map(serde_json::Map::is_empty)
        .unwrap_or(false);
    let _ = if is_empty {
        std::fs::remove_file(settings_path)
    } else {
        write_atomic(settings_path, &root)
    };
}

/// Shared scaffold for both Claude and Codex hook config installation:
/// validate the config dir (creating it if absent), read+parse any existing
/// JSON tree (treating corrupt JSON as empty), apply `merge_fn`, and write
/// atomically. Returns `Some((settings_path, created_dir))` on success;
/// `None` when the filesystem refuses our writes or the config dir is
/// occupied by a non-directory.
///
/// Both `HookConfigGuard::install_at` (Claude) and
/// `CodexHookConfigGuard::install_at` (Codex) layer their type-construction
/// on top of this — the heavy filesystem + JSON work was previously
/// duplicated across ~80 lines apart.
pub(crate) fn install_hook_config_file(
    config_dir: &Path,
    config_filename: &str,
    tool_label: &str,
    merge_fn: fn(&mut serde_json::Value),
) -> Option<(PathBuf, bool)> {
    let settings_path = config_dir.join(config_filename);
    // `exists()` returns true for both files and directories; we need to
    // distinguish so a stale `.claude`/`.codex` regular file (e.g. left
    // behind by an earlier tool) doesn't masquerade as a usable directory
    // and silently break hook injection further down. Surface the case
    // with an actionable message instead of letting `write_atomic` fail
    // with a cryptic ENOTDIR from inside `tempfile::NamedTempFile::new_in`.
    let existed_as_dir = config_dir.is_dir();
    let exists_as_other = config_dir.exists() && !existed_as_dir;

    if exists_as_other {
        eprintln!(
            "paneflow-shim: {} exists but is not a directory; remove or \
             rename it to enable {tool_label} hooks this session",
            safe_path_display(config_dir)
        );
        return None;
    }

    if !existed_as_dir {
        if let Err(e) = std::fs::create_dir_all(config_dir) {
            eprintln!(
                "paneflow-shim: cannot create {} ({e}); {tool_label} hooks \
                 disabled this session",
                safe_path_display(config_dir)
            );
            return None;
        }
    }

    let existing = std::fs::read_to_string(&settings_path).unwrap_or_default();
    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(&existing) {
            Ok(v) => v,
            Err(e) => {
                // Corrupt JSON treated as empty; overwriting is preferable
                // to aborting the shim and leaving the user with a broken
                // settings file they can't fix from inside the AI tool.
                eprintln!(
                    "paneflow-shim: {} contained invalid JSON ({e}); \
                     overwriting with a fresh config",
                    safe_path_display(&settings_path)
                );
                serde_json::json!({})
            }
        }
    };

    merge_fn(&mut root);

    if let Err(e) = write_atomic(&settings_path, &root) {
        eprintln!(
            "paneflow-shim: cannot write {} ({e}); {tool_label} hooks \
             disabled this session",
            safe_path_display(&settings_path)
        );
        if !existed_as_dir {
            let _ = std::fs::remove_dir(config_dir);
        }
        return None;
    }

    Some((settings_path, !existed_as_dir))
}

/// Shared cleanup used by both guards' `Drop` impls: read the settings
/// file, run `remove_fn` to strip PaneFlow's entries, then either delete
/// the file (if now empty) and rmdir the config dir (if we created it),
/// or write the cleaned tree back atomically. All failures swallow
/// silently — Drop must never panic, and any error here means the next
/// shim invocation's merge-idempotency will converge the state.
pub(crate) fn cleanup_hook_config_file(
    settings_path: &Path,
    config_dir: &Path,
    created_dir: bool,
    remove_fn: fn(&mut serde_json::Value),
) {
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };

    remove_fn(&mut root);

    let is_empty = root
        .as_object()
        .map(serde_json::Map::is_empty)
        .unwrap_or(false);

    if is_empty {
        let _ = std::fs::remove_file(settings_path);
        if created_dir {
            // `remove_dir` only succeeds if the directory is empty — safe
            // even if the user dropped other files into the config dir.
            let _ = std::fs::remove_dir(config_dir);
        }
    } else {
        let _ = write_atomic(settings_path, &root);
    }
}

/// RAII guard: writes PaneFlow's hook config on construction, removes it on
/// drop. The guard must live for the duration of the child Claude Code
/// process, then drop normally when `main()` returns — this is why US-005
/// forces `run_real()` (vs `exec()`) so destructors actually fire.
pub(crate) struct HookConfigGuard {
    settings_path: PathBuf,
    claude_dir: PathBuf,
    // Whether the shim created `.claude/`. Only rmdir if we created it, so we
    // don't clobber a user-created directory that happened to be empty.
    created_dir: bool,
}

impl HookConfigGuard {
    /// Install in the project CWD. Returns `None` if the filesystem refuses
    /// our writes (read-only, permission denied, etc.) — the shim proceeds
    /// without hooks in that case, per PRD constraint C4. Also returns
    /// `None` (after sweeping any orphan entries left by a previous
    /// SIGKILL'd session) when no PaneFlow IPC socket is reachable: writing
    /// hook config that would invoke a dead handler would just create
    /// config noise per C4.
    pub(crate) fn install() -> Option<Self> {
        let cwd = env::current_dir().ok()?;
        let claude_dir = cwd.join(".claude");
        if !paneflow_ipc_reachable() {
            sweep_orphan_hook_config(
                &claude_dir.join("settings.local.json"),
                remove_paneflow_hooks,
            );
            return None;
        }
        Self::install_at(&claude_dir)
    }

    /// Testable inner. Takes the absolute path to the `.claude/` directory.
    /// Does NOT check `PANEFLOW_SOCKET_PATH` — the orphan-sweep gate lives
    /// in `install()` so unit tests can drive `install_at` without
    /// fabricating a live IPC socket.
    pub(crate) fn install_at(claude_dir: &Path) -> Option<Self> {
        let (settings_path, created_dir) = install_hook_config_file(
            claude_dir,
            "settings.local.json",
            "Claude Code",
            merge_paneflow_hooks,
        )?;
        Some(Self {
            settings_path,
            claude_dir: claude_dir.to_path_buf(),
            created_dir,
        })
    }
}

impl Drop for HookConfigGuard {
    fn drop(&mut self) {
        cleanup_hook_config_file(
            &self.settings_path,
            &self.claude_dir,
            self.created_dir,
            remove_paneflow_hooks,
        );
    }
}

/// Build the `"command"` string written into `settings.local.json` /
/// `hooks.json` for the given hook `event`. Prefers the absolute path to the
/// sibling `paneflow-ai-hook` binary (resolved via `current_exe().parent()`)
/// so hooks resolve even when Claude Code or Codex is launched outside the
/// PATH-injected shell paneflow normally provides — e.g., when a user runs
/// `claude` directly in a project where a previous shim invocation left a
/// managed `settings.local.json` in place. Falls back to the bare binary name
/// when `current_exe()` fails or the sibling isn't present (test harnesses
/// where the test binary lives in `target/debug/deps/`, exotic filesystems);
/// the bare form is still recognized by `is_paneflow_hook_command` for
/// detection and cleanup.
pub(crate) fn resolve_hook_command(event: &str) -> String {
    match locate_sibling_hook_binary() {
        Some(path) => format!("{} {}", path.display(), event),
        None => format!("{HOOK_COMMAND_PREFIX}{event}"),
    }
}

/// Returns `true` if `command` is a paneflow-managed hook command, regardless
/// of whether it uses the legacy bare-name format (`paneflow-ai-hook <Event>`)
/// or the absolute-path format produced by `resolve_hook_command`. Detection
/// is basename-based so it works across both shapes and across platforms
/// (`paneflow-ai-hook` on Unix, `paneflow-ai-hook.exe` on Windows).
///
/// Intentionally does NOT verify that the binary exists on disk: a stale
/// config pointing at a removed cache dir (e.g., user uninstalled paneflow,
/// `cargo clean` between sessions) must still be recognized so cleanup can
/// remove it on the next shim run.
pub(crate) fn is_paneflow_hook_command(command: &str) -> bool {
    let first_token = command.split_whitespace().next().unwrap_or("");
    let basename = Path::new(first_token)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(first_token);
    basename == "paneflow-ai-hook" || basename == "paneflow-ai-hook.exe"
}

/// Merge PaneFlow's hook handlers into the parsed settings tree. Idempotent:
/// if a PaneFlow handler for an event is already present (identified by the
/// command basename OR the `_paneflow_managed` marker), we don't duplicate.
pub(crate) fn merge_paneflow_hooks(root: &mut serde_json::Value) {
    let root_obj = match root.as_object_mut() {
        Some(o) => o,
        None => {
            *root = serde_json::json!({});
            // re-borrow after replacement — `if let Some(...)` because we
            // just assigned an object, so this unwrap is infallible.
            let Some(o) = root.as_object_mut() else {
                return;
            };
            o
        }
    };

    let hooks_entry = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let Some(hooks_obj) = hooks_entry.as_object_mut() else {
        // User's `hooks` key is not an object (e.g., user set it to a
        // string by mistake). Overwrite it — we own the managed entries.
        *hooks_entry = serde_json::json!({});
        return;
    };

    for event in CLAUDE_HOOK_EVENTS {
        let entry = hooks_obj
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));
        let Some(array) = entry.as_array_mut() else {
            // User's event value is not an array; skip this event rather
            // than clobber what might be intentional config.
            continue;
        };

        let already_installed = array.iter().any(is_paneflow_matcher_group);
        if already_installed {
            continue;
        }

        // The `_paneflow_managed` marker sits on the OUTER matcher-group
        // wrapper — that's where `is_paneflow_matcher_group` checks it.
        // The inner handler object carries only the Claude-Code-native
        // fields so we don't send unexpected custom fields to Claude Code's
        // command runner. Identification falls back to the `command`
        // basename if the outer marker is stripped.
        array.push(serde_json::json!({
            "_paneflow_managed": true,
            "hooks": [
                {
                    "type": "command",
                    "command": resolve_hook_command(event),
                    "timeout": 5,
                }
            ]
        }));
    }
}

/// Remove PaneFlow's hook handlers from the parsed settings tree. Leaves
/// user entries untouched. Collapses empty event arrays and the empty
/// `hooks` key so cleanup produces a minimal file.
pub(crate) fn remove_paneflow_hooks(root: &mut serde_json::Value) {
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let Some(hooks_obj) = root_obj.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return;
    };

    for event in CLAUDE_HOOK_EVENTS {
        let Some(array) = hooks_obj.get_mut(*event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        array.retain(|matcher_group| !is_paneflow_matcher_group(matcher_group));
    }

    // Drop empty event arrays.
    hooks_obj.retain(|_k, v| v.as_array().is_none_or(|a| !a.is_empty()));

    // Drop the empty `hooks` object so the outer file can in turn become
    // empty and be deleted.
    if hooks_obj.is_empty() {
        root_obj.remove("hooks");
    }
}

/// Returns `true` if `value` is a matcher-group object that PaneFlow owns.
/// Belt-and-suspenders: first checks the `_paneflow_managed` marker on the
/// outer wrapper, then falls back to scanning the inner `hooks` array for a
/// command whose basename matches `paneflow-ai-hook[.exe]` (legacy bare-name
/// or absolute-path form). The second pass catches the case where Claude
/// Code's own settings writer strips unknown fields, AND the case where a
/// previous shim version wrote a bare-name command (forward compatibility
/// with the migration to absolute paths).
pub(crate) fn is_paneflow_matcher_group(value: &serde_json::Value) -> bool {
    if value
        .get("_paneflow_managed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    value
        .get("hooks")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|inner| {
            inner.iter().any(|handler| {
                handler
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_paneflow_hook_command)
            })
        })
}

/// Atomic write via `NamedTempFile::persist`: create a tempfile in the same
/// directory (so `persist` can `rename` without crossing filesystems), write
/// the JSON, then rename over the target. This avoids torn reads if Claude
/// Code concurrently reads the file at prompt time.
pub(crate) fn write_atomic(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no parent",
        )
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value).map_err(std::io::Error::other)?;
    // Trailing newline matches what most human editors leave behind and
    // keeps diffs clean when users inspect the merged file.
    std::io::Write::write_all(&mut tmp, b"\n")?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Codex hook config injection (US-006, Unix) —
//   `.codex/hooks.json` + `~/.codex/config.toml`
// ---------------------------------------------------------------------------

/// Codex hook events (per Codex docs as of April 2026). Notably `Notification`
/// is NOT a Codex hook event — Claude Code has one but Codex does not. The
/// `paneflow-ai-hook` binary (US-003) accepts `Notification` as a valid event
/// name, but it's only fired from Windows JSONL `error` events (see
/// `parse_codex_event`), never from this hooks.json registration.
///
/// Unix-only: the only callers (`merge_codex_hooks`, `remove_codex_hooks`,
/// `CodexHookConfigGuard`) are all `#[cfg(unix)]`. On Windows the JSONL tee
/// path takes over, so this list would otherwise be dead code under
/// `-D warnings`.
#[cfg(unix)]
pub(crate) const CODEX_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
];

/// Marker for the TOML comment placed above `hooks = true` in
/// `~/.codex/config.toml`. Cleanup scans for this literal line.
#[cfg(unix)]
pub(crate) const CODEX_TOML_MARKER: &str = "# _paneflow_managed: true";

/// Resolve `~/.codex/config.toml` using only std. The shim has no `dirs` dep
/// and `HOME` is universally set on Unix. Returns `None` on Windows builds
/// (unreachable at runtime — this function is `#[cfg(unix)]` — but kept for
/// documentation).
#[cfg(unix)]
pub(crate) fn codex_global_config_toml() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".codex").join("config.toml"))
}

/// RAII guard for Codex's project-level hooks.json + the global config.toml
/// feature flag. Both are populated on construction and reverted on drop.
#[cfg(unix)]
pub(crate) struct CodexHookConfigGuard {
    hooks_json_path: PathBuf,
    codex_dir: PathBuf,
    /// Whether the shim created `./.codex/`. Only rmdir on drop if we did.
    created_dir: bool,
    /// Absolute path to `~/.codex/config.toml` (if resolvable); `None` means
    /// no `HOME` — skip global-config work entirely.
    config_toml_path: Option<PathBuf>,
    /// Whether we appended the feature-flag block on install. Drop undoes it
    /// only if we did; never touches a pre-existing user-authored flag.
    added_feature_flag: bool,
}

#[cfg(unix)]
impl CodexHookConfigGuard {
    /// Install in the project CWD. Mirrors `HookConfigGuard::install`: same
    /// orphan-sweep gate when `PANEFLOW_SOCKET_PATH` is unreachable, then
    /// delegate to `install_at`.
    pub(crate) fn install() -> Option<Self> {
        let cwd = env::current_dir().ok()?;
        let codex_dir = cwd.join(".codex");
        if !paneflow_ipc_reachable() {
            sweep_orphan_hook_config(&codex_dir.join("hooks.json"), remove_codex_hooks);
            return None;
        }
        Self::install_at(&codex_dir, codex_global_config_toml().as_deref())
    }

    /// Testable inner. `config_toml_path` is the absolute path to the global
    /// `config.toml` (usually `~/.codex/config.toml`); pass `None` to skip
    /// the feature-flag step entirely (used by tests that don't want to
    /// pollute the test runner's home dir).
    pub(crate) fn install_at(codex_dir: &Path, config_toml_path: Option<&Path>) -> Option<Self> {
        let (hooks_json_path, created_dir) =
            install_hook_config_file(codex_dir, "hooks.json", "Codex", merge_codex_hooks)?;

        // Codex-specific extra: enable the `hooks = true` feature flag
        // in `~/.codex/config.toml`. Failure here is non-fatal — the user
        // can enable it manually and the rest of the hook config is in
        // place. Runs AFTER the shared install so the per-project hooks.json
        // is on disk before we touch the global config.
        let added_feature_flag = config_toml_path
            .and_then(enable_codex_feature_flag)
            .unwrap_or(false);

        Some(Self {
            hooks_json_path,
            codex_dir: codex_dir.to_path_buf(),
            created_dir,
            config_toml_path: config_toml_path.map(Path::to_path_buf),
            added_feature_flag,
        })
    }
}

#[cfg(unix)]
impl Drop for CodexHookConfigGuard {
    fn drop(&mut self) {
        // Roll back the feature flag first so if something below fails, the
        // global config is left in a consistent state.
        if self.added_feature_flag {
            if let Some(p) = self.config_toml_path.as_deref() {
                disable_codex_feature_flag(p);
            }
        }
        cleanup_hook_config_file(
            &self.hooks_json_path,
            &self.codex_dir,
            self.created_dir,
            remove_codex_hooks,
        );
    }
}

#[cfg(unix)]
pub(crate) fn merge_codex_hooks(root: &mut serde_json::Value) {
    if !root.is_object() {
        *root = serde_json::json!({});
    }
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let hooks_entry = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let Some(hooks_obj) = hooks_entry.as_object_mut() else {
        *hooks_entry = serde_json::json!({});
        return;
    };

    for event in CODEX_HOOK_EVENTS {
        let entry = hooks_obj
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));
        let Some(array) = entry.as_array_mut() else {
            continue;
        };
        if array.iter().any(is_paneflow_matcher_group) {
            continue;
        }
        array.push(serde_json::json!({
            "_paneflow_managed": true,
            "hooks": [
                {
                    "type": "command",
                    "command": resolve_hook_command(event),
                    "timeout": 5,
                }
            ]
        }));
    }
}

#[cfg(unix)]
pub(crate) fn remove_codex_hooks(root: &mut serde_json::Value) {
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let Some(hooks_obj) = root_obj.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return;
    };
    for event in CODEX_HOOK_EVENTS {
        let Some(array) = hooks_obj.get_mut(*event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        array.retain(|m| !is_paneflow_matcher_group(m));
    }
    hooks_obj.retain(|_k, v| v.as_array().is_none_or(|a| !a.is_empty()));
    if hooks_obj.is_empty() {
        root_obj.remove("hooks");
    }
}

/// Append `[features]\nhooks = true` (with a marker comment) to the
/// file at `path` iff: (a) the file doesn't already have `hooks = true` (or
/// the pre-Codex-0.130 alias `codex_hooks = true`) anywhere, AND (b) there's
/// no existing `[features]` section — appending would create a duplicate-
/// section TOML error, so we abstain in that case and warn.
///
/// Returns `Some(true)` if we modified the file, `Some(false)` if the flag
/// was already present (no-op), `None` if we aborted due to a conflict or
/// I/O error.
/// US-027: RAII advisory `flock` guard. Serializes the codex `[features]`
/// read-modify-write across concurrent shims so two near-simultaneous `codex`
/// launches can't both append `[features]` (→ duplicate-section invalid TOML).
#[cfg(unix)]
pub(crate) struct FlockGuard {
    file: std::fs::File,
}

#[cfg(unix)]
impl FlockGuard {
    /// Acquire an exclusive advisory lock on `lock_path` (creating it),
    /// blocking until granted. Returns `None` if the lock file can't be opened
    /// or locked — callers then proceed best-effort (no worse than no lock).
    fn acquire(lock_path: &Path) -> Option<Self> {
        use std::os::unix::io::AsRawFd;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .ok()?;
        // SAFETY: `flock` on a valid owned fd; `LOCK_EX` blocks until the lock
        // is exclusively held. The lock is released in `Drop`.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        (rc == 0).then_some(Self { file })
    }
}

#[cfg(unix)]
impl Drop for FlockGuard {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: releasing our own advisory lock; ignore teardown errors.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(unix)]
pub(crate) fn enable_codex_feature_flag(path: &Path) -> Option<bool> {
    // US-027: hold an exclusive flock across the whole read-modify-write.
    // `write_text_atomic` guarantees byte-atomicity but NOT serialization, so
    // without this two concurrent shims both read a config lacking
    // `[features]`, both pass the guard below, and both append the section.
    // A failed lock acquisition degrades to the prior (unserialized) behavior.
    let _lock = path.parent().and_then(|dir| {
        let _ = std::fs::create_dir_all(dir);
        FlockGuard::acquire(&dir.join(".paneflow-codex.lock"))
    });

    let existing = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!(
                "paneflow-shim: cannot read {} ({e}); leaving Codex `hooks` feature flag alone",
                safe_path_display(path)
            );
            return None;
        }
    };

    if has_hooks_flag(&existing) {
        return Some(false);
    }
    if has_features_section(&existing) {
        eprintln!(
            "paneflow-shim: {} already has a [features] section without `hooks`; skipping auto-enable (add `hooks = true` there manually to enable Codex hooks)",
            safe_path_display(path)
        );
        return None;
    }

    // Safe append: ensure a trailing newline, then add our block.
    let mut next = existing.clone();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str(CODEX_TOML_MARKER);
    next.push('\n');
    next.push_str("[features]\nhooks = true\n");

    // Ensure the parent dir exists — the Codex config dir may be absent
    // if the user has never run Codex before, but the shim's own invocation
    // implies the binary is installed somewhere.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Err(e) = write_text_atomic(path, &next) {
        eprintln!(
            "paneflow-shim: cannot write {} ({e}); Codex `hooks` feature flag not set",
            safe_path_display(path)
        );
        return None;
    }
    Some(true)
}

#[cfg(unix)]
pub(crate) fn disable_codex_feature_flag(path: &Path) {
    let Ok(existing) = std::fs::read_to_string(path) else {
        return;
    };
    let Some(cleaned) = strip_codex_feature_block(&existing) else {
        // Marker not found, or the managed block was edited by the user.
        // Both cases: don't touch the file. No stderr noise — next shim
        // invocation's idempotent install will converge state eventually.
        return;
    };
    // If the file is now empty or only whitespace, delete it — we either
    // created it from nothing or the flag was the only content.
    let result = if cleaned.trim().is_empty() {
        std::fs::remove_file(path)
    } else {
        write_text_atomic(path, &cleaned)
    };
    if let Err(e) = result {
        // Phase 7 security audit MEDIUM #12: if cleanup write fails, the
        // managed block stays in `~/.codex/config.toml` and subsequent
        // shim runs see `hooks = true` already set, so they never
        // retry the cleanup. Surface the failure so the user can remove
        // the block manually. Known silent-failure mode.
        eprintln!(
            "paneflow-shim: could not revert {} ({e}); please remove the `{}` block manually",
            safe_path_display(path),
            CODEX_TOML_MARKER
        );
    }
}

/// Atomic text write via `tempfile::NamedTempFile::persist`. Mirrors the
/// existing `write_atomic` but for raw strings (not serde_json::Value).
/// Phase 7 security audit MEDIUM #3: concurrent shims both calling
/// `fs::write` on `~/.codex/config.toml` can produce torn bytes; rename
/// swap is the fix.
#[cfg(unix)]
pub(crate) fn write_text_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config path has no parent",
        )
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut tmp, content.as_bytes())?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn has_hooks_flag(content: &str) -> bool {
    // Codex 0.130 renamed `codex_hooks` -> `hooks`; accept both so the shim
    // stays silent for configs from either era.
    content.lines().any(|line| {
        let l = line.trim_start();
        if l.starts_with('#') {
            return false;
        }
        let stripped = l.split_once('#').map(|(k, _)| k).unwrap_or(l);
        let stripped = stripped.trim_end();
        match stripped.split_once('=') {
            Some((key, value)) => {
                let k = key.trim();
                (k == "hooks" || k == "codex_hooks") && value.trim() == "true"
            }
            None => false,
        }
    })
}

#[cfg(unix)]
pub(crate) fn has_features_section(content: &str) -> bool {
    content.lines().any(|line| line.trim() == "[features]")
}

/// Remove the exact 3-line PaneFlow block (marker comment + `[features]` +
/// `hooks = true`, or the legacy `codex_hooks = true` for installs predating
/// Codex 0.130) from the file content. Returns the cleaned content if the
/// block was found and removed; `None` if the marker wasn't present.
#[cfg(unix)]
pub(crate) fn strip_codex_feature_block(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let marker_idx = lines.iter().position(|l| l.trim() == CODEX_TOML_MARKER)?;
    // We expect the marker to be followed by exactly:
    //   `[features]`
    //   `hooks = true` (or the legacy `codex_hooks = true`)
    // Remove those three lines; anything else means the file was edited
    // and we should bail to avoid clobbering user content.
    let tail = lines.get(marker_idx + 1..marker_idx + 3)?;
    // Exact-match both lines: `starts_with("hooks")` would also strip a
    // hypothetical future `hooks_experimental = ...` line that happened
    // to sit in the managed block. Exact match fails closed — safer to
    // leave the block untouched than over-delete. Accept either key name
    // because installs predating the Codex 0.130 rename wrote `codex_hooks`.
    let second = tail[1].trim();
    if tail[0].trim() != "[features]"
        || (second != "hooks = true" && second != "codex_hooks = true")
    {
        return None;
    }

    // Reconstruct: before marker + (possibly one blank line preceding the
    // marker that we added) + after the 3-line block.
    let mut head_end = marker_idx;
    // Strip a single trailing blank line we may have added before the marker
    // so we don't accumulate blanks across install/uninstall cycles.
    if head_end > 0 && lines[head_end - 1].is_empty() {
        head_end -= 1;
    }

    let mut out = String::new();
    for line in &lines[..head_end] {
        out.push_str(line);
        out.push('\n');
    }
    for line in &lines[marker_idx + 3..] {
        out.push_str(line);
        out.push('\n');
    }
    // Preserve original EOF-newline behavior: if the input had no trailing
    // newline and we removed content from the tail, trim one.
    if !content.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Codex Windows JSONL fallback (US-006, non-Unix)
// ---------------------------------------------------------------------------
//
// Codex hook machinery is disabled on Windows as of April 2026. Instead, the
// shim spawns `codex exec --json`, tees the child's stdout (so the user sees
// every byte unchanged), and dispatches `paneflow-ai-hook <Event>` subprocesses
// on recognized NDJSON events. Only the `exec` subcommand supports `--json`;
// interactive `codex` invocations on Windows pass through without tee — they
// simply won't produce sidebar state updates (documented regression).

#[cfg(not(unix))]
/// Decide whether to tee + inject `--json`. The `exec` subcommand is the only
/// Codex mode that emits NDJSON; other invocations (`codex`, `codex resume`,
/// `codex --help`) must pass through unmodified.
///
/// Scans for `"exec"` anywhere in argv rather than only at position 0, so
/// that global flags preceding the subcommand (e.g.,
/// `codex --config cfg.toml exec prompt`) are still detected. Known false-
/// positive edge: a user who passes `exec` as the VALUE of a preceding
/// value-taking flag (e.g., `codex --profile exec`) would get `--json`
/// injected; Codex then errors clearly and the user can bypass the shim
/// by invoking the real binary directly. Accepting this trade because
/// missing the tee on `codex --config X exec` is a silent regression,
/// while the false-positive is loud and bypassable.
pub(crate) fn rewrite_codex_args(args: &[OsString]) -> (Vec<OsString>, bool) {
    let Some(exec_idx) = args.iter().position(|a| a == "exec") else {
        return (args.to_vec(), false);
    };
    let mut rewritten = Vec::with_capacity(args.len() + 1);
    rewritten.extend_from_slice(&args[..=exec_idx]);
    rewritten.push(OsString::from("--json"));
    rewritten.extend(args[exec_idx + 1..].iter().cloned());
    (rewritten, true)
}

/// The full catalog of Codex NDJSON `"type"` discriminators documented at
/// <https://developers.openai.com/codex/noninteractive> as of April 2026.
/// `parse_codex_event`'s match arms MUST exhaustively cover every string in
/// this list so the schema-pin test can detect drift. When Codex adds a new
/// event type, add it to BOTH this const AND to a `match` arm in
/// `parse_codex_event` (even if the arm is just `None`).
///
/// Test-only: the runtime `parse_codex_event` enumerates the strings inline
/// rather than referencing this const, so without `cfg(test)` the const is
/// dead code under `-D warnings` on Windows non-test builds.
#[cfg(all(test, not(unix)))]
pub(crate) const KNOWN_CODEX_EVENT_TYPES: &[&str] = &[
    "thread.started",
    "turn.started",
    "turn.completed",
    "turn.failed",
    "item.started",
    "item.completed",
    "error",
];

/// Map a Codex NDJSON line to a `paneflow-ai-hook` event name. Returns
/// `None` for unrecognized / sub-event / malformed lines so the caller can
/// log-and-skip without breaking the tee loop.
#[cfg(not(unix))]
pub(crate) fn parse_codex_event(line: &str) -> Option<&'static str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let event_type = value.get("type")?.as_str()?;
    match event_type {
        "turn.started" => Some("UserPromptSubmit"),
        "turn.completed" => Some("Stop"),
        "error" => Some("Notification"),
        // Known-but-unmapped events: explicit arms (not a catch-all) so
        // adding a new Codex event type without touching this match arm
        // requires a code change, which the schema-pin test will then
        // either accept (if the fixture is updated) or fail loudly.
        "thread.started" | "turn.failed" | "item.started" | "item.completed" => None,
        // Truly unknown (new Codex event type we don't recognize yet):
        // silently skip. The schema-pin test cross-checks this against
        // KNOWN_CODEX_EVENT_TYPES so drift is surfaced at dev time.
        _ => None,
    }
}

#[cfg(not(unix))]
pub(crate) fn run_codex_with_jsonl_tee(path: &Path, args: &[OsString]) -> ExitCode {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let mut child = match std::process::Command::new(path)
        .args(args)
        .envs(env::vars_os())
        .env("PANEFLOW_AI_TOOL", "codex")
        .env("PANEFLOW_AI_PID", std::process::id().to_string())
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "paneflow-shim: spawn '{}' failed: {e}",
                safe_path_display(path)
            );
            return ExitCode::from(127);
        }
    };

    // Resolve the sibling hook binary ONCE before the tee loop. Bare-name
    // `Command::new("paneflow-ai-hook")` works on Windows only when the
    // shim cache dir is already on `%PATH%`, which is a per-PTY contract
    // not guaranteed at the moment this function spawns its first hook.
    // Resolving by absolute path matches every other call site in this
    // crate (`send_interrupt_stop`, `notify_session_end`).
    let hook_path = locate_sibling_hook_binary();

    // Take stdout so we can tee it. If the child was spawned without piped
    // stdout somehow, skip the tee entirely and fall through to wait().
    if let Some(stdout) = child.stdout.take() {
        // Reader thread: keeps the child's stdout drained (prevents pipe
        // fills) and dispatches hook events as they arrive.
        let tee_handle = std::thread::spawn(move || {
            let mut out = std::io::stdout().lock();
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                // Echo the line verbatim, preserving Codex's NDJSON so any
                // downstream tooling (jq, log-tailers) still works.
                let _ = std::io::Write::write_all(&mut out, line.as_bytes());
                let _ = std::io::Write::write_all(&mut out, b"\n");
                let _ = std::io::Write::flush(&mut out);

                if let Some(event) = parse_codex_event(&line) {
                    // Fire-and-forget: we don't wait on the hook binary so
                    // the tee loop keeps pace with Codex's output. This
                    // function is compiled only on non-Unix (`#[cfg(not
                    // (unix))]`) — on Windows, dropping the `Child` handle
                    // releases the OS handle and the subprocess runs to
                    // completion independently, with the OS reaping it
                    // when it exits. No zombies, no cleanup required.
                    //
                    // Known limitation (Phase 7 audit, MEDIUM #8): there is
                    // no rate limit. If Codex ever emits thousands of events
                    // per second, the shim would spawn thousands of
                    // subprocesses per second. Typical Codex output is a
                    // few events/sec, so this is tolerated without state.
                    if let Some(ref hp) = hook_path {
                        let _ = std::process::Command::new(hp)
                            .arg(event)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn();
                    }
                }
            }
        });
        // Join the reader thread AFTER wait() so all buffered output makes
        // it to the user before we exit.
        let status = child.wait();
        let _ = tee_handle.join();
        match status {
            Ok(s) => exit_code_from_status(&s),
            Err(e) => {
                eprintln!(
                    "paneflow-shim: wait on '{}' failed: {e}",
                    safe_path_display(path)
                );
                ExitCode::from(127)
            }
        }
    } else {
        match child.wait() {
            Ok(s) => exit_code_from_status(&s),
            Err(e) => {
                eprintln!(
                    "paneflow-shim: wait on '{}' failed: {e}",
                    safe_path_display(path)
                );
                ExitCode::from(127)
            }
        }
    }
}
