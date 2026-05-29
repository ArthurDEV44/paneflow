//! Shared plumbing for the per-agent writers (EP-003).
//!
//! - Config-path resolution (cross-platform, `dirs`-based).
//! - `shell_out` — run an agent's own CLI and surface a clean error on
//!   non-zero exit (preferred path for Claude Code / Codex per PRD D4).
//! - Format-generic install / uninstall / status built on the tested
//!   [`crate::merge`] + [`crate::io`] primitives, so every writer is
//!   idempotent and no-clobber without repeating the logic.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::agents::{InstallOutcome, StatusOutcome, UninstallOutcome};
use crate::{io, merge};

/// The entry name every writer registers under its container key.
pub(crate) const ENTRY: &str = "paneflow";

// ---------------------------------------------------------------------------
// Config paths (resolved against the real home / XDG dirs)
// ---------------------------------------------------------------------------

/// `~/.claude.json` — where `claude mcp add -s user` stores user-scope MCP
/// servers (verified 2026: NOT `~/.claude/settings.json`).
pub(crate) fn claude_config() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// `~/.codex/config.toml`.
pub(crate) fn codex_config() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("config.toml"))
}

/// `~/.gemini/settings.json`.
pub(crate) fn gemini_config() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemini").join("settings.json"))
}

/// opencode global config. Linux/macOS honor `$XDG_CONFIG_HOME` then
/// `~/.config`; Windows uses `%APPDATA%` via `dirs::config_dir()`.
pub(crate) fn opencode_config() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        dirs::config_dir().map(|d| d.join("opencode").join("opencode.json"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .map(|c| c.join("opencode").join("opencode.json"))
    }
}

// ---------------------------------------------------------------------------
// CLI shell-out
// ---------------------------------------------------------------------------

/// Is `cli` resolvable on `PATH`?
pub(crate) fn cli_on_path(cli: &str) -> bool {
    which::which(cli).is_ok()
}

/// Run `program args...`, capturing output. `Ok(())` iff it exits 0;
/// otherwise an error carrying the trimmed stderr (for `log`/report).
pub(crate) fn shell_out(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn `{program}`"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow!(
        "`{program} {}` exited with {}: {}",
        args.join(" "),
        output.status,
        // Some CLIs report errors on stdout; include both, trimmed.
        format!("{} {}", stderr.trim(), stdout.trim()).trim()
    ))
}

// ---------------------------------------------------------------------------
// JSON install / uninstall / status (Claude Code, Gemini, opencode)
// ---------------------------------------------------------------------------

/// Upsert `root[container][paneflow] = entry` at `path`, idempotently and
/// no-clobber. Returns `Installed` (new), `Updated` (entry changed), or
/// `AlreadyCurrent` (no-op). A present-but-invalid file is an error (never
/// overwritten).
pub(crate) fn json_install(
    path: &Path,
    container: &str,
    entry: serde_json::Value,
) -> Result<InstallOutcome> {
    let mut root = merge::read_json_or_default(path)?;
    let had_prior = root.get(container).and_then(|c| c.get(ENTRY)).is_some();
    let changed = merge::merge_json_entry(&mut root, container, ENTRY, entry)?;
    if !changed {
        return Ok(InstallOutcome::AlreadyCurrent);
    }
    io::write_if_changed(path, &merge::json_to_bytes(&root))?;
    Ok(if had_prior {
        InstallOutcome::Updated
    } else {
        InstallOutcome::Installed
    })
}

/// Remove `root[container][paneflow]` at `path`. No-op when the file or
/// entry is absent.
pub(crate) fn json_uninstall(path: &Path, container: &str) -> Result<UninstallOutcome> {
    if !path.exists() {
        return Ok(UninstallOutcome::NothingToRemove);
    }
    let mut root = merge::read_json_or_default(path)?;
    if !merge::remove_json_entry(&mut root, container, ENTRY) {
        return Ok(UninstallOutcome::NothingToRemove);
    }
    io::write_if_changed(path, &merge::json_to_bytes(&root))?;
    Ok(UninstallOutcome::Removed)
}

/// Read-only state of the `paneflow` JSON entry at `path`. `extract`
/// pulls the command path out of the entry (string for most agents, first
/// array element for opencode). `expected` is the current bridge path used
/// to flag staleness; an empty `expected` means "cannot judge".
pub(crate) fn json_status(
    path: &Path,
    container: &str,
    expected: &Path,
    extract: impl Fn(&serde_json::Value) -> Option<String>,
) -> Result<StatusOutcome> {
    if !path.exists() {
        return Ok(StatusOutcome::NotInstalled);
    }
    let root = merge::read_json_or_default(path)?;
    let Some(entry) = root.get(container).and_then(|c| c.get(ENTRY)) else {
        return Ok(StatusOutcome::NotInstalled);
    };
    let found = extract(entry).unwrap_or_default();
    Ok(classify(found, expected))
}

// ---------------------------------------------------------------------------
// TOML install / uninstall / status (Codex)
// ---------------------------------------------------------------------------

/// Codex's parent table for MCP servers.
pub(crate) const CODEX_TABLE: &str = "mcp_servers";

pub(crate) fn toml_install(path: &Path, command: &str) -> Result<InstallOutcome> {
    let mut doc = merge::read_toml_or_default(path)?;
    let had_prior = doc.get(CODEX_TABLE).and_then(|t| t.get(ENTRY)).is_some();
    let changed = merge::upsert_toml_entry(&mut doc, CODEX_TABLE, ENTRY, command, &[])?;
    if !changed {
        return Ok(InstallOutcome::AlreadyCurrent);
    }
    io::write_if_changed(path, &merge::toml_to_bytes(&doc))?;
    Ok(if had_prior {
        InstallOutcome::Updated
    } else {
        InstallOutcome::Installed
    })
}

pub(crate) fn toml_uninstall(path: &Path) -> Result<UninstallOutcome> {
    if !path.exists() {
        return Ok(UninstallOutcome::NothingToRemove);
    }
    let mut doc = merge::read_toml_or_default(path)?;
    if !merge::remove_toml_entry(&mut doc, CODEX_TABLE, ENTRY) {
        return Ok(UninstallOutcome::NothingToRemove);
    }
    io::write_if_changed(path, &merge::toml_to_bytes(&doc))?;
    Ok(UninstallOutcome::Removed)
}

pub(crate) fn toml_status(path: &Path, expected: &Path) -> Result<StatusOutcome> {
    if !path.exists() {
        return Ok(StatusOutcome::NotInstalled);
    }
    let doc = merge::read_toml_or_default(path)?;
    let found = doc
        .get(CODEX_TABLE)
        .and_then(|t| t.get(ENTRY))
        .and_then(|e| e.get("command"))
        .and_then(|c| c.as_str());
    match found {
        None => Ok(StatusOutcome::NotInstalled),
        Some(f) => Ok(classify(f.to_string(), expected)),
    }
}

// ---------------------------------------------------------------------------
// Tolerant current-path readers (for idempotency / update detection before
// a shell-out; never error — a bad read just means "unknown")
// ---------------------------------------------------------------------------

pub(crate) fn current_json_command(
    path: &Path,
    container: &str,
    extract: impl Fn(&serde_json::Value) -> Option<String>,
) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let root: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    extract(root.get(container)?.get(ENTRY)?)
}

pub(crate) fn current_toml_command(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    doc.get(CODEX_TABLE)?
        .get(ENTRY)?
        .get("command")?
        .as_str()
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Command-path extractors
// ---------------------------------------------------------------------------

/// `command` as a plain string (Claude Code, Gemini).
pub(crate) fn string_command(entry: &serde_json::Value) -> Option<String> {
    entry.get("command")?.as_str().map(str::to_string)
}

/// `command` as an array whose first element is the binary path (opencode).
pub(crate) fn array_command(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("command")?
        .as_array()?
        .first()?
        .as_str()
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Compare a found command path against the expected bridge path.
fn classify(found: String, expected: &Path) -> StatusOutcome {
    let expected = expected.to_string_lossy();
    if expected.is_empty() || found == expected {
        StatusOutcome::Installed { path: found }
    } else {
        StatusOutcome::StalePath {
            found,
            expected: expected.into_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_install_then_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        let entry = json!({ "command": "/p", "args": [] });

        assert_eq!(
            json_install(&p, "mcpServers", entry.clone()).unwrap(),
            InstallOutcome::Installed
        );
        // Re-run with identical entry → no-op.
        assert_eq!(
            json_install(&p, "mcpServers", entry).unwrap(),
            InstallOutcome::AlreadyCurrent
        );
        // Different path → Updated.
        assert_eq!(
            json_install(&p, "mcpServers", json!({ "command": "/q", "args": [] })).unwrap(),
            InstallOutcome::Updated
        );
    }

    #[test]
    fn json_install_preserves_siblings() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(
            &p,
            serde_json::to_vec(&json!({
                "mcpServers": { "other": { "command": "x" } },
                "theme": "dark"
            }))
            .unwrap(),
        )
        .unwrap();

        json_install(&p, "mcpServers", json!({ "command": "/p" })).unwrap();
        let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after["mcpServers"]["other"]["command"], json!("x"));
        assert_eq!(after["theme"], json!("dark"));
        assert_eq!(after["mcpServers"]["paneflow"]["command"], json!("/p"));
    }

    #[test]
    fn json_install_refuses_invalid_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(&p, b"{ broken").unwrap();
        assert!(json_install(&p, "mcpServers", json!({})).is_err());
        // The invalid file was NOT overwritten.
        assert_eq!(std::fs::read(&p).unwrap(), b"{ broken");
    }

    #[test]
    fn json_uninstall_removes_only_target() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(
            &p,
            serde_json::to_vec(&json!({
                "mcpServers": { "paneflow": { "command": "/p" }, "other": { "command": "x" } }
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            json_uninstall(&p, "mcpServers").unwrap(),
            UninstallOutcome::Removed
        );
        let after: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(after["mcpServers"].get("paneflow").is_none());
        assert_eq!(after["mcpServers"]["other"]["command"], json!("x"));
        // Second uninstall → nothing to remove.
        assert_eq!(
            json_uninstall(&p, "mcpServers").unwrap(),
            UninstallOutcome::NothingToRemove
        );
    }

    #[test]
    fn json_status_reports_installed_and_stale() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("settings.json");
        std::fs::write(
            &p,
            serde_json::to_vec(&json!({ "mcpServers": { "paneflow": { "command": "/cur" } } }))
                .unwrap(),
        )
        .unwrap();

        assert_eq!(
            json_status(&p, "mcpServers", Path::new("/cur"), string_command).unwrap(),
            StatusOutcome::Installed {
                path: "/cur".into()
            }
        );
        assert_eq!(
            json_status(&p, "mcpServers", Path::new("/new"), string_command).unwrap(),
            StatusOutcome::StalePath {
                found: "/cur".into(),
                expected: "/new".into()
            }
        );
    }

    #[test]
    fn json_status_not_installed_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("missing.json");
        assert_eq!(
            json_status(&p, "mcpServers", Path::new("/x"), string_command).unwrap(),
            StatusOutcome::NotInstalled
        );
    }

    #[test]
    fn toml_install_idempotent_and_preserves_comments() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, b"# my codex config\nmodel = \"gpt-5\"\n").unwrap();

        assert_eq!(toml_install(&p, "/p").unwrap(), InstallOutcome::Installed);
        let txt = std::fs::read_to_string(&p).unwrap();
        assert!(txt.contains("# my codex config"));
        assert!(txt.contains("model = \"gpt-5\""));
        assert!(txt.contains("paneflow"));
        // Idempotent.
        assert_eq!(
            toml_install(&p, "/p").unwrap(),
            InstallOutcome::AlreadyCurrent
        );
        // Updated path.
        assert_eq!(toml_install(&p, "/q").unwrap(), InstallOutcome::Updated);
    }

    #[test]
    fn toml_uninstall_and_status() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("config.toml");
        toml_install(&p, "/cur").unwrap();

        assert_eq!(
            toml_status(&p, Path::new("/cur")).unwrap(),
            StatusOutcome::Installed {
                path: "/cur".into()
            }
        );
        assert_eq!(
            toml_status(&p, Path::new("/new")).unwrap(),
            StatusOutcome::StalePath {
                found: "/cur".into(),
                expected: "/new".into()
            }
        );
        assert_eq!(toml_uninstall(&p).unwrap(), UninstallOutcome::Removed);
        assert_eq!(
            toml_uninstall(&p).unwrap(),
            UninstallOutcome::NothingToRemove
        );
    }

    #[test]
    fn array_command_extracts_first_element() {
        let entry = json!({ "type": "local", "command": ["/bin/paneflow-mcp"], "enabled": true });
        assert_eq!(array_command(&entry), Some("/bin/paneflow-mcp".to_string()));
    }
}
