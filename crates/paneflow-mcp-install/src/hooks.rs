//! Persistent user-scope agent notification hooks (EP-004, prd-cli-agent-orchestration).
//!
//! `paneflow hooks setup` writes Paneflow's `paneflow-ai-hook` callback into an
//! agent's **user-scope** config so the agent reports its turn state to the
//! running Paneflow instance. This is the durable counterpart to the shim's
//! ephemeral, project-local injection (`paneflow-shim::hooks`): the shim writes
//! `./.claude/settings.local.json` per launch and removes it on exit; this
//! writes `~/.claude/settings.json` once and references the binary at its
//! stable, update-surviving path (`runtime_paths::ai_hook_binary_path`).
//!
//! Scope: Claude Code only for now - the only agent with a verified, file-based
//! notification-hook mechanism the callback plugs into (the shim injects hooks
//! for Claude + Codex; Gemini/opencode have no equivalent today). `setup`
//! reports other agents as unsupported rather than inventing a shape.
//!
//! The matcher-group shape and the `_paneflow_managed` marker are duplicated
//! from `paneflow-shim::hooks` (NOT shared: the shim is size-budgeted and does
//! not depend on this crate). They MUST stay byte-identical so the shim's
//! detection (`is_paneflow_matcher_group`) recognizes entries written here and
//! a future shim-skip (US-018) can suppress the redundant ephemeral injection.
//!
//! All writes go through [`crate::io::write_if_changed`] (idempotent, backed
//! up, atomic) and refuse to clobber a present-but-invalid config.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::agents::{InstallOutcome, StatusOutcome, UninstallOutcome};
use crate::{io, merge};

/// Claude Code 2.x hook events Paneflow registers. Mirror of
/// `paneflow-shim::hooks::CLAUDE_HOOK_EVENTS` (kept in sync deliberately).
/// `SubagentStop` is omitted - the server maps it to `ai.stop` like `Stop`, so
/// registering both would double-fire.
const CLAUDE_HOOK_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "Notification",
    "Stop",
    "PreToolUse",
    "PostToolUse",
];

/// Marker on the outer matcher-group wrapper identifying a Paneflow-managed
/// hook. Mirror of the shim's marker so both writers recognize each other.
const MANAGED_MARKER: &str = "_paneflow_managed";

/// `~/.claude/settings.json` - where Claude Code reads user-scope hooks. NOT
/// `~/.claude.json` (that is the MCP-server file `mcp install` targets).
fn claude_settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// Is Claude Code present (CLI on PATH or its config dir exists)?
fn claude_detected() -> bool {
    which::which("claude").is_ok()
        || dirs::home_dir()
            .map(|h| h.join(".claude").exists())
            .unwrap_or(false)
}

/// The managed matcher-group for one event - byte-identical to the shim's
/// shape so detection interoperates.
fn managed_group(hook_path: &Path, event: &str) -> Value {
    json!({
        MANAGED_MARKER: true,
        "hooks": [
            {
                "type": "command",
                "command": format!("{} {event}", hook_path.display()),
                "timeout": 5,
            }
        ]
    })
}

/// True iff `first token`'s basename is the ai-hook binary (legacy bare-name or
/// absolute-path form, Unix or Windows). Mirror of
/// `paneflow-shim::hooks::is_paneflow_hook_command`.
fn is_paneflow_hook_command(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    let base = Path::new(first)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(first);
    base == "paneflow-ai-hook" || base == "paneflow-ai-hook.exe"
}

/// True iff `group` is a Paneflow-managed matcher-group: the `_paneflow_managed`
/// marker, or (fallback, in case Claude Code's writer stripped the marker) an
/// inner command whose basename is the ai-hook binary.
fn is_managed_group(group: &Value) -> bool {
    if group.get(MANAGED_MARKER).and_then(Value::as_bool) == Some(true) {
        return true;
    }
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.iter().any(|h| {
                h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_paneflow_hook_command)
            })
        })
}

/// Remove all managed matcher-groups; collapse empty arrays + the `hooks` key.
/// Returns whether anything was removed.
fn remove_managed_hooks(root: &mut Value) -> bool {
    let Some(obj) = root.as_object_mut() else {
        return false;
    };
    let Some(hooks_obj) = obj.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    let mut removed = false;
    for event in CLAUDE_HOOK_EVENTS {
        if let Some(arr) = hooks_obj.get_mut(*event).and_then(Value::as_array_mut) {
            let before = arr.len();
            arr.retain(|g| !is_managed_group(g));
            removed |= arr.len() != before;
        }
    }
    hooks_obj.retain(|_k, v| v.as_array().is_none_or(|a| !a.is_empty()));
    if hooks_obj.is_empty() {
        obj.remove("hooks");
    }
    removed
}

/// Clear stale managed groups then add a fresh one per event pointing at
/// `hook_path`. Re-adding from scratch (rather than skip-if-present) keeps the
/// command path current across Paneflow updates.
fn set_managed_hooks(root: &mut Value, hook_path: &Path) {
    remove_managed_hooks(root);
    if !root.is_object() {
        *root = json!({});
    }
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    let hooks_entry = obj.entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks_obj) = hooks_entry.as_object_mut() else {
        // User set `hooks` to a non-object; we own the managed entries.
        *hooks_entry = json!({});
        return;
    };
    for event in CLAUDE_HOOK_EVENTS {
        let arr_entry = hooks_obj.entry(*event).or_insert_with(|| json!([]));
        let Some(arr) = arr_entry.as_array_mut() else {
            continue;
        };
        arr.push(managed_group(hook_path, event));
    }
}

/// Command strings of every managed group currently in the tree.
fn collect_managed_commands(root: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for event in CLAUDE_HOOK_EVENTS {
        let Some(arr) = hooks.get(*event).and_then(Value::as_array) else {
            continue;
        };
        for group in arr {
            if is_managed_group(group) {
                if let Some(cmd) = group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|h| h.get("command"))
                    .and_then(Value::as_str)
                {
                    out.push(cmd.to_string());
                }
            }
        }
    }
    out
}

fn classify(found_path: &str, expected: &Path) -> StatusOutcome {
    let expected = expected.to_string_lossy();
    if expected.is_empty() || found_path == expected {
        StatusOutcome::Installed {
            path: found_path.to_string(),
        }
    } else {
        StatusOutcome::StalePath {
            found: found_path.to_string(),
            expected: expected.into_owned(),
        }
    }
}

/// Install (or refresh) Claude Code's persistent Paneflow hooks. Idempotent and
/// no-clobber. Returns the config path + the outcome.
fn install(hook_path: &Path) -> Result<(PathBuf, InstallOutcome)> {
    let path =
        claude_settings_path().ok_or_else(|| anyhow!("cannot resolve ~/.claude/settings.json"))?;
    let mut root = merge::read_json_or_default(&path)?;
    let had_prior = !collect_managed_commands(&root).is_empty();
    set_managed_hooks(&mut root, hook_path);
    let wrote = io::write_if_changed(&path, &merge::json_to_bytes(&root)?)?;
    let outcome = if !wrote {
        InstallOutcome::AlreadyCurrent
    } else if had_prior {
        InstallOutcome::Updated
    } else {
        InstallOutcome::Installed
    };
    Ok((path, outcome))
}

fn uninstall() -> Result<UninstallOutcome> {
    let path =
        claude_settings_path().ok_or_else(|| anyhow!("cannot resolve ~/.claude/settings.json"))?;
    if !path.exists() {
        return Ok(UninstallOutcome::NothingToRemove);
    }
    let mut root = merge::read_json_or_default(&path)?;
    if !remove_managed_hooks(&mut root) {
        return Ok(UninstallOutcome::NothingToRemove);
    }
    io::write_if_changed(&path, &merge::json_to_bytes(&root)?)?;
    Ok(UninstallOutcome::Removed)
}

fn status(expected_hook_path: &Path) -> Result<StatusOutcome> {
    let path =
        claude_settings_path().ok_or_else(|| anyhow!("cannot resolve ~/.claude/settings.json"))?;
    if !path.exists() {
        return Ok(StatusOutcome::NotInstalled);
    }
    let root = merge::read_json_or_default(&path)?;
    let commands = collect_managed_commands(&root);
    let Some(first) = commands.first() else {
        return Ok(StatusOutcome::NotInstalled);
    };
    // The stored command is "<path> <event>" - compare the path token.
    let found_path = first.split_whitespace().next().unwrap_or("");
    Ok(classify(found_path, expected_hook_path))
}

const HOOKS_USAGE: &str = "\
paneflow hooks - register the Paneflow agent-notification hooks with your agents

Usage:
  paneflow hooks setup       Install persistent hooks for every supported agent
  paneflow hooks uninstall   Remove the Paneflow hooks
  paneflow hooks status      Report the hook installation state per agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HooksCommand {
    Setup,
    Uninstall,
    Status,
}

impl HooksCommand {
    fn parse(arg: Option<&str>) -> Option<Self> {
        match arg {
            Some("setup") => Some(Self::Setup),
            Some("uninstall") => Some(Self::Uninstall),
            Some("status") => Some(Self::Status),
            _ => None,
        }
    }
}

/// Entry point for `paneflow hooks <subcommand>`. `args` is everything after
/// `paneflow hooks`. `hook_path` is the stable ai-hook location resolved by the
/// caller (`runtime_paths::ai_hook_binary_path()`), or `None` when `data_dir()`
/// is unresolvable. Exit codes mirror `mcp`: 0 success / no agent, 1 error,
/// 2 usage.
#[must_use]
pub fn run_hooks_cli(args: &[String], hook_path: Option<PathBuf>) -> i32 {
    run_hooks_with(
        args,
        hook_path.as_deref(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

pub(crate) fn run_hooks_with(
    args: &[String],
    hook_path: Option<&Path>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let Some(command) = HooksCommand::parse(args.first().map(String::as_str)) else {
        let _ = writeln!(err, "{HOOKS_USAGE}");
        return 2;
    };

    match command {
        HooksCommand::Setup => {
            let Some(hook_path) = hook_path else {
                let _ = writeln!(
                    err,
                    "hooks: the paneflow-ai-hook binary is unavailable (data dir unresolvable); cannot install"
                );
                return 1;
            };
            let code = if !claude_detected() {
                let _ = writeln!(out, "claude-code: not detected (skipped)");
                0
            } else {
                match install(hook_path) {
                    Ok((path, outcome)) => {
                        let verb = match outcome {
                            InstallOutcome::Installed => "installed",
                            InstallOutcome::Updated => "updated",
                            InstallOutcome::AlreadyCurrent => "already current",
                        };
                        let _ = writeln!(out, "claude-code: hooks {verb} ({})", path.display());
                        0
                    }
                    Err(e) => {
                        let _ = writeln!(err, "claude-code: error: {e:#}");
                        1
                    }
                }
            };
            report_other_agents(out);
            code
        }
        HooksCommand::Uninstall => match uninstall() {
            Ok(UninstallOutcome::Removed) => {
                let _ = writeln!(out, "claude-code: hooks removed");
                0
            }
            Ok(UninstallOutcome::NothingToRemove) => {
                let _ = writeln!(out, "claude-code: no Paneflow hooks present");
                0
            }
            Err(e) => {
                let _ = writeln!(err, "claude-code: error: {e:#}");
                1
            }
        },
        HooksCommand::Status => {
            let expected = hook_path.unwrap_or_else(|| Path::new(""));
            let code = match status(expected) {
                Ok(StatusOutcome::Installed { path }) => {
                    let _ = writeln!(out, "claude-code: installed ({path})");
                    0
                }
                Ok(StatusOutcome::StalePath { found, expected }) => {
                    let _ = writeln!(
                        out,
                        "claude-code: stale (found {found}, expected {expected})"
                    );
                    0
                }
                Ok(StatusOutcome::NotInstalled) => {
                    let _ = writeln!(out, "claude-code: not installed");
                    0
                }
                Err(e) => {
                    let _ = writeln!(err, "claude-code: error: {e:#}");
                    1
                }
            };
            report_other_agents(out);
            code
        }
    }
}

/// Report the hook state of the non-Claude agents honestly (parity with the
/// `mcp` per-agent report; only emits a line for an agent present on PATH).
/// Codex hooks are injected per-launch by the shim (project-scope), so no
/// user-scope install applies; Gemini and opencode have no notification-hook
/// mechanism, so there is nothing to install rather than a fabricated shape.
fn report_other_agents(out: &mut dyn Write) {
    if which::which("codex").is_ok() {
        let _ = writeln!(
            out,
            "codex: hooks injected per-launch by the shim (no user-scope install)"
        );
    }
    if which::which("gemini").is_ok() {
        let _ = writeln!(out, "gemini: no notification-hook mechanism (unsupported)");
    }
    if which::which("opencode").is_ok() {
        let _ = writeln!(
            out,
            "opencode: no notification-hook mechanism (unsupported)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn managed_group_matches_shim_shape() {
        let g = managed_group(Path::new("/bin/paneflow-ai-hook"), "Stop");
        assert_eq!(g[MANAGED_MARKER], json!(true));
        assert_eq!(g["hooks"][0]["type"], json!("command"));
        assert_eq!(
            g["hooks"][0]["command"],
            json!("/bin/paneflow-ai-hook Stop")
        );
        assert_eq!(g["hooks"][0]["timeout"], json!(5));
        assert!(is_managed_group(&g));
    }

    #[test]
    fn set_then_remove_round_trips_to_empty() {
        let mut root = json!({});
        set_managed_hooks(&mut root, Path::new("/bin/paneflow-ai-hook"));
        // One managed group per event.
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 1, "{root}");
        assert_eq!(
            collect_managed_commands(&root).len(),
            CLAUDE_HOOK_EVENTS.len()
        );
        assert!(remove_managed_hooks(&mut root));
        // `hooks` collapses away entirely.
        assert!(root.get("hooks").is_none(), "{root}");
    }

    #[test]
    fn set_preserves_user_hooks_and_other_keys() {
        let mut root = json!({
            "theme": "dark",
            "hooks": {
                "Stop": [ { "hooks": [ { "type": "command", "command": "my-own-hook" } ] } ]
            }
        });
        set_managed_hooks(&mut root, Path::new("/bin/paneflow-ai-hook"));
        // User key untouched.
        assert_eq!(root["theme"], json!("dark"));
        // The user's Stop hook survives alongside the managed one.
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "{root}");
        assert!(stop
            .iter()
            .any(|g| g["hooks"][0]["command"] == json!("my-own-hook")));
        // Removing managed leaves the user's hook intact.
        remove_managed_hooks(&mut root);
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0]["hooks"][0]["command"], json!("my-own-hook"));
    }

    #[test]
    fn install_is_idempotent_and_updates_on_path_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("settings.json");

        // Drive the file path directly (bypass ~/.claude resolution) via the
        // tree helpers + the io layer, mirroring what install() does.
        let writeit = |hook: &str| {
            let mut root = merge::read_json_or_default(&path).unwrap();
            let had = !collect_managed_commands(&root).is_empty();
            set_managed_hooks(&mut root, Path::new(hook));
            let wrote = io::write_if_changed(&path, &merge::json_to_bytes(&root).unwrap()).unwrap();
            (wrote, had)
        };

        let (wrote, had) = writeit("/v1/paneflow-ai-hook");
        assert!(wrote && !had, "first install writes, no prior");
        // Re-run identical → no write.
        let (wrote, had) = writeit("/v1/paneflow-ai-hook");
        assert!(!wrote && had, "idempotent re-run");
        // New path (Paneflow update) → rewrite, prior present.
        let (wrote, had) = writeit("/v2/paneflow-ai-hook");
        assert!(wrote && had, "path change rewrites");

        let root = read(&path);
        assert_eq!(
            root["hooks"]["Stop"][0]["hooks"][0]["command"],
            json!("/v2/paneflow-ai-hook Stop")
        );
        // Exactly one managed group per event after the update (no stacking).
        assert_eq!(root["hooks"]["Stop"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn run_hooks_with_rejects_bad_subcommand() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_hooks_with(&["bogus".to_string()], None, &mut out, &mut err);
        assert_eq!(code, 2);
        assert!(String::from_utf8_lossy(&err).contains("Usage"));
    }

    #[test]
    fn setup_without_hook_path_errors() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_hooks_with(&["setup".to_string()], None, &mut out, &mut err);
        assert_eq!(code, 1);
        assert!(String::from_utf8_lossy(&err).contains("unavailable"));
    }
}
