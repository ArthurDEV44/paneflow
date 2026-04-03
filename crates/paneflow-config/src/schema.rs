// US-017: Config schema types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level PaneFlow configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PaneFlowConfig {
    /// Key-action shortcut mappings (e.g. "ctrl+t" -> "new_tab").
    pub shortcuts: HashMap<String, String>,
    /// Default shell binary path. `None` uses the system default.
    pub default_shell: Option<String>,
    /// Workspace command definitions (cmux-compatible format).
    pub commands: Vec<CommandDefinition>,
    /// Font settings for the terminal renderer (US-020).
    pub font: FontConfig,
    /// Terminal color theme — 16 ANSI colors + foreground/background (US-020).
    pub colors: ColorTheme,
    /// Number of scrollback lines per terminal pane (default 4000).
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: u32,
    /// UI accent color as hex string (e.g. "0091ff"). Default: cmux blue.
    pub accent_color: Option<String>,
}

fn default_scrollback() -> u32 {
    4000
}

impl Default for PaneFlowConfig {
    fn default() -> Self {
        Self {
            shortcuts: HashMap::new(),
            default_shell: None,
            commands: Vec::new(),
            font: FontConfig::default(),
            colors: ColorTheme::default(),
            scrollback_lines: default_scrollback(),
            accent_color: None,
        }
    }
}

/// Font configuration for terminal rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FontConfig {
    /// Font family name. `None` uses system monospace font.
    pub family: Option<String>,
    /// Font size in points (default 14.0).
    pub size: f32,
}

impl FontConfig {
    /// Clamped font size (6.0..=72.0).
    pub fn clamped_size(&self) -> f32 {
        self.size.clamp(6.0, 72.0)
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: None,
            size: 14.0,
        }
    }
}

/// Terminal color theme with 16 ANSI colors and foreground/background.
/// Colors are 6-digit hex strings (e.g. "cdd6f4").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ColorTheme {
    pub foreground: String,
    pub background: String,
    /// 16 ANSI colors: [black, red, green, yellow, blue, magenta, cyan, white,
    ///                   bright_black, bright_red, bright_green, bright_yellow,
    ///                   bright_blue, bright_magenta, bright_cyan, bright_white]
    pub ansi: Vec<String>,
}

impl Default for ColorTheme {
    fn default() -> Self {
        // Catppuccin Mocha palette
        Self {
            foreground: "cdd6f4".into(),
            background: "1e1e2e".into(),
            ansi: vec![
                "45475a".into(), // black
                "f38ba8".into(), // red
                "a6e3a1".into(), // green
                "f9e2af".into(), // yellow
                "89b4fa".into(), // blue
                "f5c2e7".into(), // magenta
                "94e2d5".into(), // cyan
                "bac2de".into(), // white
                "585b70".into(), // bright black
                "f38ba8".into(), // bright red
                "a6e3a1".into(), // bright green
                "f9e2af".into(), // bright yellow
                "89b4fa".into(), // bright blue
                "f5c2e7".into(), // bright magenta
                "94e2d5".into(), // bright cyan
                "a6adc8".into(), // bright white
            ],
        }
    }
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
