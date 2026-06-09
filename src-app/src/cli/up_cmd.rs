//! `paneflow up <file>` — declarative agent workspaces (US-009/US-011/US-012).
//!
//! Loads a `paneflow.workspace.toml`, resolves each pane's `agent` to its CLI
//! launch command (verifying the binary is on PATH for an atomic failure), and
//! calls the `workspace.up` IPC method. `--dry-run` prints the resolved plan
//! without touching the running instance.

use paneflow_config::schema::PaneFlowConfig;
use paneflow_ipc_client::IpcTransport;
use serde_json::json;

use super::workspace_spec::{self, PaneSpec};
use super::{CliError, EXIT_OK};
use crate::agent_launcher::TerminalAgent;

/// `paneflow up <file> [--dry-run]`.
pub fn up(client: &impl IpcTransport, file: &str, dry_run: bool) -> Result<i32, CliError> {
    let src = std::fs::read_to_string(file)
        .map_err(|e| CliError::runtime(format!("cannot read '{file}': {e}")))?;
    let spec = workspace_spec::load(&src).map_err(CliError::runtime)?;

    let config = paneflow_config::loader::load_config();
    let mut panes = Vec::with_capacity(spec.panes.len());
    for (idx, pane) in spec.panes.iter().enumerate() {
        let command = resolve_command(idx, pane, &config)?;
        panes.push(json!({
            "cwd": pane.cwd,
            "command": command,
            "prompt": pane.prompt,
            "focus": pane.focus,
            "env": pane.env,
            "name": pane.name,
        }));
    }

    let params = json!({
        "name": spec.name,
        "layout": spec.layout.as_ipc(),
        "panes": panes,
    });

    if dry_run {
        // Print the resolved plan (agents -> commands) without touching the
        // running instance.
        super::print_json(&params)?;
        return Ok(EXIT_OK);
    }

    let result = client
        .call("workspace.up", params)
        .map_err(CliError::runtime)?;
    super::print_json(&result)?;
    Ok(EXIT_OK)
}

/// Resolve a pane's launch command: an `agent` maps to its CLI launch command
/// (and the binary is verified on PATH for an atomic failure before any pane
/// spawns, US-012); a raw `command` passes through; neither leaves a bare shell.
fn resolve_command(
    idx: usize,
    pane: &PaneSpec,
    config: &PaneFlowConfig,
) -> Result<Option<String>, CliError> {
    let Some(agent) = pane.agent.as_deref() else {
        return Ok(pane.command.clone());
    };
    let resolved = resolve_agent(agent)
        .ok_or_else(|| CliError::runtime(format!("pane {idx}: unknown agent '{agent}'")))?;
    if !resolved.is_installed() {
        return Err(CliError::runtime(format!(
            "pane {idx}: agent '{agent}' ({}) not found on PATH",
            resolved.binary()
        )));
    }
    Ok(Some(resolved.launch_command(config)))
}

/// Map a friendly agent name from the spec to a [`TerminalAgent`]. Accepts
/// hyphen or underscore separators and the bare `claude` alias for Claude Code.
fn resolve_agent(name: &str) -> Option<TerminalAgent> {
    let normalized = name.trim().to_lowercase().replace('-', "_");
    let tag = match normalized.as_str() {
        "claude" => "claude_code",
        other => other,
    };
    TerminalAgent::from_tag(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_agent_accepts_aliases() {
        assert_eq!(resolve_agent("claude"), Some(TerminalAgent::ClaudeCode));
        assert_eq!(
            resolve_agent("claude-code"),
            Some(TerminalAgent::ClaudeCode)
        );
        assert_eq!(
            resolve_agent("Claude_Code"),
            Some(TerminalAgent::ClaudeCode)
        );
        assert_eq!(resolve_agent("codex"), Some(TerminalAgent::Codex));
        assert_eq!(resolve_agent("gemini"), Some(TerminalAgent::Gemini));
        assert_eq!(resolve_agent("nope"), None);
    }
}
