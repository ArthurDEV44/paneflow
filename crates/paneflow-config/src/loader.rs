// US-017: JSON config loader with validation

use crate::schema::{CommandDefinition, LayoutNode, PaneFlowConfig};
use std::path::PathBuf;
use thiserror::Error;
use tracing::warn;

/// Errors that can occur when loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    IoError(#[from] std::io::Error),
    #[error("invalid JSON in config file: {0}")]
    ParseError(#[from] serde_json::Error),
}

/// Returns the platform-appropriate config file path.
///
/// - Linux: `$XDG_CONFIG_HOME/paneflow/paneflow.json`
/// - macOS: `~/Library/Application Support/paneflow/paneflow.json`
/// - Windows: `%APPDATA%\paneflow\paneflow.json`
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("paneflow").join("paneflow.json"))
}

/// Load the PaneFlow configuration from the default platform path.
///
/// - If the config file does not exist, returns `PaneFlowConfig::default()`.
/// - If the file contains invalid JSON, logs a warning and returns defaults.
/// - Individual command entries with validation errors are skipped with warnings.
pub fn load_config() -> PaneFlowConfig {
    let Some(path) = config_path() else {
        warn!("could not determine config directory; using defaults");
        return PaneFlowConfig::default();
    };

    load_config_from_path(&path)
}

/// Load and validate configuration from a specific file path.
///
/// This is the core loading function, also useful for testing.
pub fn load_config_from_path(path: &std::path::Path) -> PaneFlowConfig {
    if !path.exists() {
        return PaneFlowConfig::default();
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            warn!(
                "failed to read config file {}: {e}; using defaults",
                path.display()
            );
            return PaneFlowConfig::default();
        }
    };

    parse_and_validate(&contents)
}

/// Parse a JSON string into a validated `PaneFlowConfig`.
///
/// Invalid JSON produces a warning and returns defaults.
/// Individual commands with validation errors are filtered out with warnings.
pub fn parse_and_validate(json: &str) -> PaneFlowConfig {
    let mut config: PaneFlowConfig = match serde_json::from_str(json) {
        Ok(c) => c,
        Err(e) => {
            warn!("invalid JSON in config: {e}; using defaults");
            return PaneFlowConfig::default();
        }
    };

    // Validate and filter commands.
    let validated: Vec<CommandDefinition> = config
        .commands
        .into_iter()
        .filter(validate_command)
        .collect();
    config.commands = validated;

    // Validate and fix layout nodes in-place.
    for cmd in &mut config.commands {
        if let Some(ref mut ws) = cmd.workspace {
            if let Some(ref mut layout) = ws.layout {
                validate_layout(layout);
            }
        }
    }

    config
}

/// Validate a single command definition. Returns `false` if it should be skipped.
fn validate_command(cmd: &CommandDefinition) -> bool {
    if cmd.name.trim().is_empty() {
        warn!("skipping command with blank name");
        return false;
    }
    true
}

/// Recursively validate and fix a layout node.
///
/// - Split nodes: must have exactly 2 children; ratio clamped to [0.1, 0.9].
/// - Pane nodes: must have >= 1 surface.
fn validate_layout(node: &mut LayoutNode) {
    match node {
        LayoutNode::Split {
            ref mut ratio,
            ref mut children,
            ..
        } => {
            // Clamp ratio to [0.1, 0.9].
            if let Some(r) = ratio {
                if *r < 0.1 {
                    warn!("split ratio {r} is below minimum; clamping to 0.1");
                    *r = 0.1;
                } else if *r > 0.9 {
                    warn!("split ratio {r} is above maximum; clamping to 0.9");
                    *r = 0.9;
                }
            }

            // Warn if children count is not exactly 2.
            if children.len() != 2 {
                warn!(
                    "split node has {} children (expected 2); truncating or padding",
                    children.len()
                );
                // Truncate to 2 if more, or pad with empty panes if fewer.
                while children.len() > 2 {
                    children.pop();
                }
                while children.len() < 2 {
                    children.push(LayoutNode::Pane {
                        surfaces: vec![Default::default()],
                    });
                }
            }

            // Recurse into children.
            for child in children.iter_mut() {
                validate_layout(child);
            }
        }
        LayoutNode::Pane {
            ref mut surfaces, ..
        } => {
            if surfaces.is_empty() {
                warn!("pane has no surfaces; adding a default surface");
                surfaces.push(Default::default());
            }
        }
    }
}

impl Default for crate::schema::SurfaceDefinition {
    fn default() -> Self {
        Self {
            surface_type: Some("terminal".to_string()),
            name: None,
            command: None,
            cwd: None,
            env: None,
            focus: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = PaneFlowConfig::default();
        assert!(config.shortcuts.is_empty());
        assert!(config.default_shell.is_none());
        assert!(config.commands.is_empty());
    }

    #[test]
    fn test_config_path_is_some() {
        // On most systems dirs::config_dir() succeeds.
        let path = config_path();
        assert!(path.is_some());
        let p = path.unwrap();
        assert!(p.ends_with("paneflow/paneflow.json") || p.ends_with("paneflow\\paneflow.json"));
    }

    #[test]
    fn test_missing_file_returns_defaults() {
        let config = load_config_from_path(std::path::Path::new("/nonexistent/path/config.json"));
        assert_eq!(config, PaneFlowConfig::default());
    }

    #[test]
    fn test_invalid_json_returns_defaults() {
        let config = parse_and_validate("this is not json {{{");
        assert_eq!(config, PaneFlowConfig::default());
    }

    #[test]
    fn test_empty_json_object_returns_defaults() {
        let config = parse_and_validate("{}");
        assert_eq!(config, PaneFlowConfig::default());
    }

    #[test]
    fn test_valid_minimal_config() {
        let json = r#"{
            "default_shell": "/bin/zsh",
            "shortcuts": {"ctrl+t": "new_tab"},
            "commands": []
        }"#;
        let config = parse_and_validate(json);
        assert_eq!(config.default_shell, Some("/bin/zsh".to_string()));
        assert_eq!(config.shortcuts.get("ctrl+t"), Some(&"new_tab".to_string()));
        assert!(config.commands.is_empty());
    }

    #[test]
    fn test_blank_name_skipped() {
        let json = r#"{
            "commands": [
                {"name": "", "keywords": []},
                {"name": "  ", "keywords": []},
                {"name": "valid", "keywords": ["test"]}
            ]
        }"#;
        let config = parse_and_validate(json);
        assert_eq!(config.commands.len(), 1);
        assert_eq!(config.commands[0].name, "valid");
    }

    #[test]
    fn test_command_with_workspace() {
        let json = r#"{
            "commands": [{
                "name": "dev",
                "description": "Development workspace",
                "keywords": ["dev", "work"],
                "workspace": {
                    "name": "Dev Workspace",
                    "cwd": "/home/user/projects",
                    "color": "ff6600",
                    "layout": {
                        "type": "split",
                        "direction": "horizontal",
                        "ratio": 0.5,
                        "children": [
                            {
                                "type": "pane",
                                "surfaces": [{"surface_type": "terminal", "command": "vim"}]
                            },
                            {
                                "type": "pane",
                                "surfaces": [{"surface_type": "terminal", "command": "cargo watch"}]
                            }
                        ]
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        assert_eq!(config.commands.len(), 1);
        let cmd = &config.commands[0];
        assert_eq!(cmd.name, "dev");
        assert_eq!(cmd.description.as_deref(), Some("Development workspace"));

        let ws = cmd.workspace.as_ref().unwrap();
        assert_eq!(ws.name.as_deref(), Some("Dev Workspace"));
        assert_eq!(ws.color.as_deref(), Some("ff6600"));

        match ws.layout.as_ref().unwrap() {
            LayoutNode::Split {
                direction,
                ratio,
                children,
            } => {
                assert_eq!(direction, "horizontal");
                assert_eq!(*ratio, Some(0.5));
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected split layout"),
        }
    }

    #[test]
    fn test_command_with_shell_command() {
        let json = r#"{
            "commands": [{
                "name": "htop",
                "keywords": ["monitor"],
                "command": "htop"
            }]
        }"#;
        let config = parse_and_validate(json);
        assert_eq!(config.commands.len(), 1);
        assert_eq!(config.commands[0].command.as_deref(), Some("htop"));
        assert!(config.commands[0].workspace.is_none());
    }

    #[test]
    fn test_split_ratio_clamped_low() {
        let json = r#"{
            "commands": [{
                "name": "test",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "split",
                        "direction": "vertical",
                        "ratio": 0.01,
                        "children": [
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]},
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]}
                        ]
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Split { ratio, .. } => {
                assert!((ratio.unwrap() - 0.1).abs() < f64::EPSILON);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_split_ratio_clamped_high() {
        let json = r#"{
            "commands": [{
                "name": "test",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "split",
                        "direction": "vertical",
                        "ratio": 0.99,
                        "children": [
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]},
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]}
                        ]
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Split { ratio, .. } => {
                assert!((ratio.unwrap() - 0.9).abs() < f64::EPSILON);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_split_wrong_children_count() {
        // 3 children should be truncated to 2.
        let json = r#"{
            "commands": [{
                "name": "test",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "split",
                        "direction": "horizontal",
                        "children": [
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]},
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]},
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]}
                        ]
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Split { children, .. } => {
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_split_zero_children_padded() {
        let json = r#"{
            "commands": [{
                "name": "test",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "split",
                        "direction": "horizontal",
                        "children": []
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Split { children, .. } => {
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_pane_no_surfaces_gets_default() {
        let json = r#"{
            "commands": [{
                "name": "test",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "pane",
                        "surfaces": []
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Pane { surfaces } => {
                assert_eq!(surfaces.len(), 1);
                assert_eq!(surfaces[0].surface_type.as_deref(), Some("terminal"));
            }
            _ => panic!("expected pane"),
        }
    }

    #[test]
    fn test_load_from_file() {
        use std::io::Write;
        let mut tmp = NamedTempFile::new().unwrap();
        write!(
            tmp,
            r#"{{"default_shell": "/bin/bash", "commands": [{{"name": "ls", "keywords": [], "command": "ls -la"}}]}}"#
        )
        .unwrap();

        let config = load_config_from_path(tmp.path());
        assert_eq!(config.default_shell, Some("/bin/bash".to_string()));
        assert_eq!(config.commands.len(), 1);
        assert_eq!(config.commands[0].name, "ls");
    }

    #[test]
    fn test_load_from_file_invalid_json() {
        use std::io::Write;
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "not valid json!!").unwrap();

        let config = load_config_from_path(tmp.path());
        assert_eq!(config, PaneFlowConfig::default());
    }

    #[test]
    fn test_nested_split_validation() {
        let json = r#"{
            "commands": [{
                "name": "nested",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "split",
                        "direction": "horizontal",
                        "ratio": 0.5,
                        "children": [
                            {
                                "type": "split",
                                "direction": "vertical",
                                "ratio": 0.05,
                                "children": [
                                    {"type": "pane", "surfaces": [{"surface_type": "terminal"}]},
                                    {"type": "pane", "surfaces": [{"surface_type": "terminal"}]}
                                ]
                            },
                            {"type": "pane", "surfaces": [{"surface_type": "terminal"}]}
                        ]
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Split { children, .. } => {
                // Inner split should have ratio clamped to 0.1.
                match &children[0] {
                    LayoutNode::Split { ratio, .. } => {
                        assert!((ratio.unwrap() - 0.1).abs() < f64::EPSILON);
                    }
                    _ => panic!("expected nested split"),
                }
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_surface_with_env_and_focus() {
        let json = r#"{
            "commands": [{
                "name": "envtest",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "pane",
                        "surfaces": [{
                            "surface_type": "terminal",
                            "name": "main",
                            "command": "cargo run",
                            "cwd": "/tmp",
                            "env": {"RUST_LOG": "debug"},
                            "focus": true
                        }]
                    }
                }
            }]
        }"#;
        let config = parse_and_validate(json);
        let ws = config.commands[0].workspace.as_ref().unwrap();
        match ws.layout.as_ref().unwrap() {
            LayoutNode::Pane { surfaces } => {
                assert_eq!(surfaces.len(), 1);
                let s = &surfaces[0];
                assert_eq!(s.name.as_deref(), Some("main"));
                assert_eq!(s.command.as_deref(), Some("cargo run"));
                assert_eq!(s.cwd.as_deref(), Some("/tmp"));
                assert_eq!(s.focus, Some(true));
                let env = s.env.as_ref().unwrap();
                assert_eq!(env.get("RUST_LOG"), Some(&"debug".to_string()));
            }
            _ => panic!("expected pane"),
        }
    }

    #[test]
    fn test_serialization_roundtrip() {
        let config = PaneFlowConfig {
            shortcuts: {
                let mut m = HashMap::new();
                m.insert("ctrl+n".to_string(), "new_window".to_string());
                m
            },
            default_shell: Some("/bin/fish".to_string()),
            commands: vec![CommandDefinition {
                name: "test".to_string(),
                description: Some("A test command".to_string()),
                keywords: vec!["test".to_string()],
                workspace: None,
                command: Some("echo hello".to_string()),
            }],
        };

        let json = serde_json::to_string_pretty(&config).unwrap();
        let reparsed = parse_and_validate(&json);
        assert_eq!(config, reparsed);
    }
}
