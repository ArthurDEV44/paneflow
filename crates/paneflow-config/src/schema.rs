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
    /// Workspace command definitions (cmux-compatible format).
    pub commands: Vec<CommandDefinition>,
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
    /// A split container dividing space between exactly 2 children.
    Split {
        /// Split direction: "horizontal" or "vertical".
        direction: String,
        /// Split ratio in [0.1, 0.9]. Defaults to 0.5 if omitted.
        ratio: Option<f64>,
        /// Exactly 2 child layout nodes.
        #[serde(default)]
        children: Vec<LayoutNode>,
    },
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
}
