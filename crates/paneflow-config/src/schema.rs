// US-017: Config schema types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level PaneFlow configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PaneFlowConfig {
    /// Key-action shortcut mappings (e.g. "ctrl+t" -> "new_tab").
    pub shortcuts: HashMap<String, String>,
    /// Default shell binary path. `None` uses the system default.
    pub default_shell: Option<String>,
    /// Terminal color theme name (e.g. "Catppuccin Mocha", "Dracula").
    pub theme: Option<String>,
    /// Workspace command definitions (cmux-compatible format).
    pub commands: Vec<CommandDefinition>,
    /// Window decoration mode: `"client"` (CSD, default) or `"server"` (SSD).
    pub window_decorations: Option<String>,
    /// Terminal line height multiplier (default: 1.4, valid range: 1.0–2.5).
    pub line_height: Option<f32>,
    /// Terminal font family (default: platform-specific monospace fallback).
    pub font_family: Option<String>,
    /// Terminal font size in pixels (default: 14.0, valid range: 8.0–32.0).
    pub font_size: Option<f32>,
    /// Treat Alt key as Meta (send ESC prefix). Default: true on Linux.
    /// Set to false for future macOS where Option produces Unicode characters.
    pub option_as_meta: Option<bool>,
    /// Opt-in desktop telemetry block.
    ///
    /// Tri-state semantics:
    /// - `None` (block missing from config): user has never been prompted.
    /// - `Some(TelemetryConfig { enabled: None })`: block exists but the
    ///   consent question is still unanswered (e.g. user dismissed the
    ///   first-run modal without choosing).
    /// - `Some(TelemetryConfig { enabled: Some(true|false) })`: explicit
    ///   user answer — consent granted or refused.
    ///
    /// The consent modal (US-011) only appears when `telemetry.enabled`
    /// resolves to `None` under both the outer and inner Option layers.
    /// No event is ever sent unless `enabled == Some(true)`.
    pub telemetry: Option<TelemetryConfig>,
}

/// Desktop telemetry consent state.
///
/// Kept in its own struct (rather than a bare `Option<bool>` on
/// `PaneFlowConfig`) so future telemetry-scoped settings (e.g. a per-user
/// `distinct_id` override, or per-event category toggles) can be added
/// without schema churn.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TelemetryConfig {
    /// Consent tri-state. `None` = unanswered, `Some(true)` = opted in,
    /// `Some(false)` = opted out. `PANEFLOW_NO_TELEMETRY=1` env var
    /// overrides this unconditionally at the client layer (US-012).
    pub enabled: Option<bool>,
}

/// A single command definition, compatible with the cmux workspace format.
///
/// Each entry is either a workspace definition (with `workspace`) or a simple
/// shell command (with `command`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandDefinition {
    /// Display name (must not be blank).
    pub name: String,
    /// Optional human-readable description.
    pub description: Option<String>,
    /// Search keywords for fuzzy matching.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Workspace layout definition (mutually exclusive with `command`).
    pub workspace: Option<WorkspaceDefinition>,
    /// Shell command string (mutually exclusive with `workspace`).
    pub command: Option<String>,
}

/// Workspace definition containing layout, working directory, and visual config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceDefinition {
    /// Workspace display name.
    pub name: Option<String>,
    /// Default working directory for the workspace.
    pub cwd: Option<String>,
    /// Color as a 6-digit hex string (e.g. "ff6600").
    pub color: Option<String>,
    /// Root layout node describing pane arrangement.
    pub layout: Option<LayoutNode>,
}

/// A node in the layout tree: either a leaf pane or a split container.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    /// A leaf pane containing one or more surfaces.
    Pane {
        /// Surfaces within this pane (must have >= 1).
        #[serde(default)]
        surfaces: Vec<SurfaceDefinition>,
    },
    /// A split container dividing space between 2 or more children.
    Split {
        /// Split direction: "horizontal" or "vertical".
        direction: String,
        /// Legacy: single split ratio for binary (2-child) layouts.
        /// Ignored when `ratios` is present. Defaults to 0.5 if omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ratio: Option<f64>,
        /// Per-child ratios for N-ary layouts. When present, must have
        /// the same length as `children`. Values should sum to ~1.0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ratios: Option<Vec<f64>>,
        /// 2 or more child layout nodes.
        #[serde(default)]
        children: Vec<LayoutNode>,
    },
}

impl LayoutNode {
    /// Count the number of leaf (Pane) nodes in the layout tree.
    pub fn leaf_count(&self) -> usize {
        match self {
            LayoutNode::Pane { .. } => 1,
            LayoutNode::Split { children, .. } => children.iter().map(|c| c.leaf_count()).sum(),
        }
    }

    /// Resolve per-child ratios for a Split node.
    ///
    /// Returns `ratios` if present, else converts legacy `ratio` to binary
    /// `[ratio, 1-ratio]`, else returns equal ratios for the child count.
    pub fn resolved_ratios(&self) -> Vec<f64> {
        match self {
            LayoutNode::Pane { .. } => vec![1.0],
            LayoutNode::Split {
                ratio,
                ratios,
                children,
                ..
            } => {
                if let Some(rs) = ratios {
                    return rs.clone();
                }
                if let Some(r) = ratio {
                    if children.len() == 2 {
                        return vec![*r, 1.0 - *r];
                    }
                }
                let n = children.len().max(1);
                vec![1.0 / n as f64; n]
            }
        }
    }
}

/// Persisted session state written to `~/.cache/paneflow/session.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionState {
    /// Schema version for forward-compatible migrations.
    pub version: u32,
    /// Index of the active workspace at save time.
    pub active_workspace: usize,
    /// Ordered list of workspace snapshots.
    pub workspaces: Vec<WorkspaceSession>,
}

/// Snapshot of a single workspace for session persistence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceSession {
    /// Workspace display title.
    pub title: String,
    /// Root working directory of the workspace.
    pub cwd: String,
    /// Layout tree (splits + panes). `None` means a single default pane.
    pub layout: Option<LayoutNode>,
    /// User-defined command buttons rendered in this workspace's tab bar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_buttons: Vec<ButtonCommand>,
}

/// A user-defined command button rendered in a workspace's tab bar.
/// Clicking the button sends `{command}\r` to the active terminal.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ButtonCommand {
    /// Stable identifier (opaque string) — survives reorderings and renames.
    pub id: String,
    /// Display name (also used as hover tooltip).
    pub name: String,
    /// Icon asset path relative to the `assets/` folder (e.g. `"icons/rocket.svg"`).
    pub icon: String,
    /// Shell command string, executed verbatim in the active terminal
    /// with a trailing `\r` appended (no bracketed-paste wrapping).
    pub command: String,
}

/// A surface within a pane (terminal, browser, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SurfaceDefinition {
    /// Surface type identifier: "terminal", "browser", etc.
    pub surface_type: Option<String>,
    /// Display name for this surface.
    pub name: Option<String>,
    /// Shell command to run in this surface.
    pub command: Option<String>,
    /// Working directory override for this surface.
    pub cwd: Option<String>,
    /// Extra environment variables.
    pub env: Option<HashMap<String, String>>,
    /// Whether this surface should receive initial focus.
    pub focus: Option<bool>,
    /// Saved scrollback text (plain, ANSI stripped). Up to 4000 lines / 400K chars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrollback: Option<String>,
}
