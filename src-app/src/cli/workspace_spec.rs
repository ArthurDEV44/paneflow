//! Declarative workspace spec for `paneflow up` (US-007).
//!
//! A `paneflow.workspace.toml` describes one workspace: a layout preset plus a
//! list of panes, each with an optional cwd, an agent (or raw command) to run,
//! and a prompt to pre-fill. `deny_unknown_fields` turns a misspelled key into
//! an explicit error rather than a silently-ignored field - important for a
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
    /// Base port for `${port_offset}` allocation (US-008). Default 3000.
    /// Panes referencing `${port_offset}` in `env` values each get a free
    /// 10-port stride starting here (3000, 3010, …).
    #[serde(default)]
    pub port_base: Option<u16>,
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

#[derive(Debug, Clone, Deserialize)]
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
    /// Values may reference `${port_offset}` (US-008); any other `${…}` token
    /// is a validation error.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Optional pane name.
    #[serde(default)]
    pub name: Option<String>,
    /// Branch to isolate this pane on via a git worktree (EP-002, US-006).
    /// Requires `cwd` (inside a git repo); the pane spawns in
    /// `<repo>.worktrees/<branch-slug>` instead of `cwd`.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Copy the repo's top-level gitignored `.env*` files into the worktree
    /// (US-007). Default true; only meaningful with `worktree`.
    #[serde(default)]
    pub copy_env: Option<bool>,
    /// Command run inside the freshly created worktree BEFORE the agent
    /// spawns (US-007, e.g. `"bun install"`). Failure warns but never blocks.
    /// Only meaningful with `worktree`; Paneflow never guesses one.
    #[serde(default)]
    pub setup: Option<String>,
    /// Wall-clock bound for `setup`, seconds (default 300).
    #[serde(default)]
    pub setup_timeout_secs: Option<u64>,
    /// Worktree teardown at workspace close (US-009): `"auto"` (default -
    /// remove when clean, branch never deleted) or `"keep"`.
    #[serde(default)]
    pub worktree_teardown: Option<String>,
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
            validate_pane(i, pane)?;
        }
        Ok(())
    }
}

/// Per-pane invariants, shared with the flow spec (EP-003): `agent` XOR
/// `command`, plus the worktree-field rules below.
pub(super) fn validate_pane(i: usize, pane: &PaneSpec) -> Result<(), String> {
    if pane.agent.is_some() && pane.command.is_some() {
        return Err(format!(
            "pane {i}: set either `agent` or `command`, not both"
        ));
    }
    validate_worktree_fields(i, pane)
}

/// Worktree-field invariants (EP-002). `worktree` needs a `cwd` to locate the
/// repo; the companion fields are inert without `worktree`, so their presence
/// alone is a spec mistake worth refusing (deny_unknown_fields spirit). A
/// leading `-` in the branch would read as a git flag (CWE-88) - refused here
/// rather than trusted to downstream quoting.
fn validate_worktree_fields(i: usize, pane: &PaneSpec) -> Result<(), String> {
    match pane.worktree.as_deref() {
        Some(branch) => {
            if branch.is_empty() {
                return Err(format!("pane {i}: `worktree` must name a branch"));
            }
            if branch.starts_with('-') {
                return Err(format!(
                    "pane {i}: branch '{branch}' must not start with '-'"
                ));
            }
            // A branch whose filesystem slug is empty (dot-only: `.`, `..`)
            // has no safe directory name - `..` would be a traversal
            // component of the worktree path (NFR: slugs filesystem-safe).
            if crate::workspace::worktree::branch_slug(branch).is_empty() {
                return Err(format!(
                    "pane {i}: branch '{branch}' has no filesystem-safe name \
                     (dot-only names are not allowed)"
                ));
            }
            if pane.cwd.is_none() {
                return Err(format!(
                    "pane {i}: `worktree` requires `cwd` (to locate the git repository)"
                ));
            }
        }
        None => {
            for (field, set) in [
                ("copy_env", pane.copy_env.is_some()),
                ("setup", pane.setup.is_some()),
                ("setup_timeout_secs", pane.setup_timeout_secs.is_some()),
                ("worktree_teardown", pane.worktree_teardown.is_some()),
            ] {
                if set {
                    return Err(format!("pane {i}: `{field}` requires `worktree`"));
                }
            }
        }
    }
    if let Some(policy) = pane.worktree_teardown.as_deref()
        && !matches!(policy, "auto" | "keep")
    {
        return Err(format!(
            "pane {i}: `worktree_teardown` must be \"auto\" or \"keep\", got '{policy}'"
        ));
    }
    Ok(())
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

    #[test]
    fn parses_worktree_fields() {
        let spec = load(
            "port_base = 4000\n[[panes]]\ncwd = \"/tmp\"\nagent = \"claude\"\nworktree = \"feat/x\"\ncopy_env = false\nsetup = \"bun install\"\nworktree_teardown = \"keep\"\n",
        )
        .expect("valid");
        assert_eq!(spec.port_base, Some(4000));
        let p = &spec.panes[0];
        assert_eq!(p.worktree.as_deref(), Some("feat/x"));
        assert_eq!(p.copy_env, Some(false));
        assert_eq!(p.setup.as_deref(), Some("bun install"));
        assert_eq!(p.worktree_teardown.as_deref(), Some("keep"));
    }

    #[test]
    fn worktree_requires_cwd() {
        let err = load("[[panes]]\nagent = \"claude\"\nworktree = \"feat/x\"\n").unwrap_err();
        assert!(err.contains("requires `cwd`"), "got: {err}");
    }

    #[test]
    fn worktree_branch_must_not_look_like_a_flag() {
        // CWE-88: a leading '-' would be parsed as a git flag downstream.
        let err = load("[[panes]]\ncwd = \"/tmp\"\nagent = \"claude\"\nworktree = \"--force\"\n")
            .unwrap_err();
        assert!(err.contains("must not start with '-'"), "got: {err}");
    }

    #[test]
    fn worktree_branch_dot_only_is_rejected() {
        // A dot-only branch slug would be a `..` traversal component in the
        // (destructive) worktree path - refused at parse, atomically.
        for branch in ["..", ".", "..."] {
            let err = load(&format!(
                "[[panes]]\ncwd = \"/tmp\"\nagent = \"claude\"\nworktree = \"{branch}\"\n"
            ))
            .unwrap_err();
            assert!(
                err.contains("filesystem-safe"),
                "branch {branch}: got: {err}"
            );
        }
    }

    #[test]
    fn worktree_companion_fields_require_worktree() {
        let err = load("[[panes]]\nagent = \"claude\"\nsetup = \"bun install\"\n").unwrap_err();
        assert!(err.contains("`setup` requires `worktree`"), "got: {err}");
        let err = load("[[panes]]\nagent = \"claude\"\ncopy_env = true\n").unwrap_err();
        assert!(err.contains("`copy_env` requires `worktree`"), "got: {err}");
    }

    #[test]
    fn worktree_teardown_value_is_validated() {
        let err = load(
            "[[panes]]\ncwd = \"/tmp\"\nagent = \"claude\"\nworktree = \"x\"\nworktree_teardown = \"delete\"\n",
        )
        .unwrap_err();
        assert!(err.contains("auto"), "got: {err}");
    }

    #[test]
    fn rejects_too_many_panes() {
        // US-007 AC: the post-deserialization validation bounds the pane count
        // to MAX_PANES so a runaway spec can't drive the server past its cap.
        let src = "[[panes]]\nagent = \"claude\"\n".repeat(MAX_PANES + 1);
        let err = load(&src).unwrap_err();
        assert!(err.contains("too many panes"), "got: {err}");
    }
}
