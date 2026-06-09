//! Declarative workspace spec for `paneflow up` (US-007).
//!
//! A `paneflow.workspace.toml` describes one workspace: a layout preset plus a
//! list of panes, each with an optional cwd, an agent (or raw command) to run,
//! and a prompt to pre-fill. `deny_unknown_fields` turns a misspelled key into
//! an explicit error rather than a silently-ignored field — important for a
//! hand-edited config. Business invariants (non-empty, within MAX_PANES, not
//! both `agent` and `command`) are validated after deserialization.
//!
//! Example:
//! ```toml
//! name = "feature-x"
//! layout = "even_h"
//!
//! [[panes]]
//! cwd = "~/dev/backend"
//! agent = "claude"
//! prompt = "review the diff on this branch"
//! focus = true
//!
//! [[panes]]
//! cwd = "~/dev/frontend"
//! agent = "codex"
//! ```

use std::collections::HashMap;

use serde::Deserialize;

use crate::layout::MAX_PANES;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSpec {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub layout: LayoutPreset,
    #[serde(default)]
    pub panes: Vec<PaneSpec>,
}

/// Layout preset. Mirrors the keyboard layout actions and the `build_up_layout`
/// server helper. `even_h` (side by side) is the default.
#[derive(Debug, Default, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutPreset {
    #[default]
    EvenH,
    EvenV,
    MainVertical,
    Tiled,
}

impl LayoutPreset {
    /// The `layout` string the `workspace.up` IPC method expects.
    pub fn as_ipc(self) -> &'static str {
        match self {
            LayoutPreset::EvenH => "even_h",
            LayoutPreset::EvenV => "even_v",
            LayoutPreset::MainVertical => "main_vertical",
            LayoutPreset::Tiled => "tiled",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaneSpec {
    /// Working directory; `~` is expanded server-side by `canonicalize_workspace_cwd`.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Agent to launch (claude / codex / opencode / gemini / …). Mutually
    /// exclusive with `command`.
    #[serde(default)]
    pub agent: Option<String>,
    /// Raw command to run instead of an agent. Mutually exclusive with `agent`.
    #[serde(default)]
    pub command: Option<String>,
    /// Prompt to pre-fill into the agent's input box (never auto-submitted).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Whether this pane is the focused / main one.
    #[serde(default)]
    pub focus: Option<bool>,
    /// Per-pane env overrides, merged over the global `terminal.env` default.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Optional pane name.
    #[serde(default)]
    pub name: Option<String>,
}

/// Parse + validate a workspace spec from TOML source.
pub fn load(src: &str) -> Result<WorkspaceSpec, String> {
    let spec: WorkspaceSpec = toml::from_str(src).map_err(|e| e.to_string())?;
    spec.validate()?;
    Ok(spec)
}

impl WorkspaceSpec {
    fn validate(&self) -> Result<(), String> {
        if self.panes.is_empty() {
            return Err("workspace spec has no [[panes]]".to_string());
        }
        if self.panes.len() > MAX_PANES {
            return Err(format!(
                "too many panes ({} > {MAX_PANES})",
                self.panes.len()
            ));
        }
        for (i, pane) in self.panes.iter().enumerate() {
            if pane.agent.is_some() && pane.command.is_some() {
                return Err(format!(
                    "pane {i}: set either `agent` or `command`, not both"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_valid_spec() {
        let spec = load(
            r#"
            name = "feat-x"
            layout = "main_vertical"

            [[panes]]
            cwd = "/tmp"
            agent = "claude"
            prompt = "do the thing"
            focus = true

            [[panes]]
            command = "cargo watch"
            "#,
        )
        .expect("valid spec");
        assert_eq!(spec.name.as_deref(), Some("feat-x"));
        assert_eq!(spec.layout, LayoutPreset::MainVertical);
        assert_eq!(spec.panes.len(), 2);
        assert_eq!(spec.panes[0].agent.as_deref(), Some("claude"));
        assert_eq!(spec.panes[0].focus, Some(true));
        assert_eq!(spec.panes[1].command.as_deref(), Some("cargo watch"));
    }

    #[test]
    fn layout_defaults_to_even_h() {
        let spec = load("[[panes]]\nagent = \"codex\"\n").expect("valid");
        assert_eq!(spec.layout, LayoutPreset::EvenH);
        assert_eq!(spec.layout.as_ipc(), "even_h");
    }

    #[test]
    fn rejects_unknown_field() {
        let err = load("agnt = \"claude\"\n[[panes]]\nagent = \"claude\"\n").unwrap_err();
        assert!(
            err.contains("agnt") || err.contains("unknown"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_agent_and_command_together() {
        let err = load("[[panes]]\nagent = \"claude\"\ncommand = \"vim\"\n").unwrap_err();
        assert!(err.contains("either"), "got: {err}");
    }

    #[test]
    fn rejects_empty_panes() {
        let err = load("name = \"x\"\n").unwrap_err();
        assert!(err.contains("no [[panes]]"), "got: {err}");
    }
}
