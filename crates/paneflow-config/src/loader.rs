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

/// Returns the platform-appropriate session file path.
///
/// - Linux: `$XDG_CACHE_HOME/paneflow/session.json`
/// - macOS: `~/Library/Caches/paneflow/session.json`
pub fn session_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("paneflow").join("session.json"))
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
    if !validated.is_empty() {
        warn!(
            "config contains {} command(s), but workspace commands are not yet implemented — they will be ignored",
            validated.len()
        );
    }
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
/// - Split nodes: must have >= 2 children; legacy `ratio` clamped to [0.1, 0.9];
///   per-child `ratios` clamped to [0.01, 1.0].
/// - Pane nodes: must have >= 1 surface.
pub fn validate_layout(node: &mut LayoutNode) {
    match node {
        LayoutNode::Split {
            ref mut ratio,
            ref mut ratios,
            ref mut children,
            ..
        } => {
            // Clamp legacy ratio to [0.1, 0.9]; reject non-finite values.
            if let Some(r) = ratio {
                if !r.is_finite() {
                    warn!("split ratio is NaN/Infinity; resetting to 0.5");
                    *r = 0.5;
                } else if *r < 0.1 {
                    warn!("split ratio {r} is below minimum; clamping to 0.1");
                    *r = 0.1;
                } else if *r > 0.9 {
                    warn!("split ratio {r} is above maximum; clamping to 0.9");
                    *r = 0.9;
                }
            }

            // Must have at least 2 children; pad if fewer.
            while children.len() < 2 {
                warn!(
                    "split node has {} children (need >= 2); padding",
                    children.len()
                );
                children.push(LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                });
            }

            // Validate per-child ratios: reject non-finite, fix length mismatch, normalize.
            if let Some(ref mut rs) = ratios {
                // Reject NaN/Infinity values.
                for r in rs.iter_mut() {
                    if !r.is_finite() {
                        warn!("per-child ratio is NaN/Infinity; resetting");
                        *r = 1.0 / children.len() as f64;
                    }
                }
                // Fix length mismatch: trim or extend to match children count.
                let n = children.len();
                if rs.len() != n {
                    warn!(
                        "ratios length ({}) != children count ({}); fixing",
                        rs.len(),
                        n
                    );
                    rs.resize(n, 1.0 / n as f64);
                }
                // Clamp individual values to [0.01, 1.0].
                for r in rs.iter_mut() {
                    *r = r.clamp(0.01, 1.0);
                }
                // Normalize to sum ~1.0.
                let sum: f64 = rs.iter().sum();
                if sum > 0.0 && (sum - 1.0).abs() > f64::EPSILON {
                    for r in rs.iter_mut() {
                        *r /= sum;
                    }
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
            scrollback: None,
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
                ..
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
    fn test_split_nary_children_accepted() {
        // 3+ children are valid in N-ary layout.
        let json = r#"{
            "commands": [{
                "name": "test",
                "keywords": [],
                "workspace": {
                    "layout": {
                        "type": "split",
                        "direction": "horizontal",
                        "ratios": [0.33, 0.33, 0.34],
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
            LayoutNode::Split {
                children, ratios, ..
            } => {
                assert_eq!(children.len(), 3);
                let rs = ratios.as_ref().unwrap();
                assert_eq!(rs.len(), 3);
                assert!((rs[0] - 0.33).abs() < f64::EPSILON);
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
            theme: Some("Dracula".to_string()),
            commands: vec![CommandDefinition {
                name: "test".to_string(),
                description: Some("A test command".to_string()),
                keywords: vec!["test".to_string()],
                workspace: None,
                command: Some("echo hello".to_string()),
            }],
            window_decorations: None,
            line_height: None,
            font_family: None,
            font_size: None,
            option_as_meta: None,
        };

        let json = serde_json::to_string_pretty(&config).unwrap();
        let reparsed = parse_and_validate(&json);
        assert_eq!(config, reparsed);
    }

    #[test]
    fn test_nary_layout_roundtrip() {
        // Build a 6-pane N-ary layout: 3 panes horizontal on top, 3 on bottom.
        let make_pane = |name: &str| LayoutNode::Pane {
            surfaces: vec![SurfaceDefinition {
                surface_type: Some("terminal".to_string()),
                name: Some(name.to_string()),
                ..Default::default()
            }],
        };

        let top_row = LayoutNode::Split {
            direction: "vertical".to_string(),
            ratio: None,
            ratios: Some(vec![0.33, 0.33, 0.34]),
            children: vec![make_pane("A"), make_pane("B"), make_pane("C")],
        };
        let bottom_row = LayoutNode::Split {
            direction: "vertical".to_string(),
            ratio: None,
            ratios: Some(vec![0.33, 0.33, 0.34]),
            children: vec![make_pane("D"), make_pane("E"), make_pane("F")],
        };
        let root = LayoutNode::Split {
            direction: "horizontal".to_string(),
            ratio: None,
            ratios: Some(vec![0.5, 0.5]),
            children: vec![top_row, bottom_row],
        };

        // Serialize to JSON and back.
        let json = serde_json::to_string_pretty(&root).unwrap();
        let deserialized: LayoutNode = serde_json::from_str(&json).unwrap();
        assert_eq!(root, deserialized);
    }

    #[test]
    fn test_legacy_binary_still_works() {
        // Legacy format with single `ratio` field (no `ratios`).
        let json = r#"{
            "type": "split",
            "direction": "horizontal",
            "ratio": 0.6,
            "children": [
                {"type": "pane", "surfaces": [{"surface_type": "terminal"}]},
                {"type": "pane", "surfaces": [{"surface_type": "terminal"}]}
            ]
        }"#;
        let node: LayoutNode = serde_json::from_str(json).unwrap();
        match &node {
            LayoutNode::Split {
                ratio,
                ratios,
                children,
                ..
            } => {
                assert_eq!(*ratio, Some(0.6));
                assert!(ratios.is_none());
                assert_eq!(children.len(), 2);
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn test_layout_node_leaf_count() {
        let single = LayoutNode::Pane {
            surfaces: vec![Default::default()],
        };
        assert_eq!(single.leaf_count(), 1);

        // 3-child flat split = 3 leaves
        let flat = LayoutNode::Split {
            direction: "vertical".to_string(),
            ratio: None,
            ratios: Some(vec![0.33, 0.33, 0.34]),
            children: vec![
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
            ],
        };
        assert_eq!(flat.leaf_count(), 3);

        // Nested: 2 rows of 3 = 6 leaves
        let nested = LayoutNode::Split {
            direction: "horizontal".to_string(),
            ratio: None,
            ratios: Some(vec![0.5, 0.5]),
            children: vec![flat.clone(), flat],
        };
        assert_eq!(nested.leaf_count(), 6);
    }

    #[test]
    fn test_resolved_ratios_nary() {
        let node = LayoutNode::Split {
            direction: "vertical".to_string(),
            ratio: None,
            ratios: Some(vec![0.25, 0.25, 0.5]),
            children: vec![
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
            ],
        };
        assert_eq!(node.resolved_ratios(), vec![0.25, 0.25, 0.5]);
    }

    #[test]
    fn test_resolved_ratios_legacy_binary() {
        let node = LayoutNode::Split {
            direction: "horizontal".to_string(),
            ratio: Some(0.6),
            ratios: None,
            children: vec![
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
            ],
        };
        let rs = node.resolved_ratios();
        assert_eq!(rs.len(), 2);
        assert!((rs[0] - 0.6).abs() < f64::EPSILON);
        assert!((rs[1] - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_resolved_ratios_fallback_equal() {
        let node = LayoutNode::Split {
            direction: "vertical".to_string(),
            ratio: None,
            ratios: None,
            children: vec![
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
                LayoutNode::Pane {
                    surfaces: vec![Default::default()],
                },
            ],
        };
        let rs = node.resolved_ratios();
        assert_eq!(rs.len(), 3);
        for r in &rs {
            assert!((r - 1.0 / 3.0).abs() < f64::EPSILON);
        }
    }

    // --- Session persistence round-trip tests (US-017) ---

    fn make_surface(cwd: &str) -> SurfaceDefinition {
        SurfaceDefinition {
            surface_type: Some("terminal".to_string()),
            cwd: Some(cwd.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_session_roundtrip_single_workspace() {
        let state = SessionState {
            version: 1,
            active_workspace: 0,
            workspaces: vec![WorkspaceSession {
                title: "main".to_string(),
                cwd: "/home/user/project".to_string(),
                layout: Some(LayoutNode::Pane {
                    surfaces: vec![make_surface("/home/user/project")],
                }),
                custom_buttons: vec![],
            }],
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
    }

    #[test]
    fn test_session_roundtrip_multiple_workspaces() {
        let state = SessionState {
            version: 1,
            active_workspace: 1,
            workspaces: vec![
                WorkspaceSession {
                    title: "frontend".to_string(),
                    cwd: "/home/user/web".to_string(),
                    layout: Some(LayoutNode::Pane {
                        surfaces: vec![make_surface("/home/user/web")],
                    }),
                    custom_buttons: vec![],
                },
                WorkspaceSession {
                    title: "backend".to_string(),
                    cwd: "/home/user/api".to_string(),
                    layout: Some(LayoutNode::Pane {
                        surfaces: vec![make_surface("/home/user/api")],
                    }),
                    custom_buttons: vec![],
                },
                WorkspaceSession {
                    title: "devops".to_string(),
                    cwd: "/home/user/infra".to_string(),
                    layout: None,
                    custom_buttons: vec![],
                },
            ],
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
        assert_eq!(restored.active_workspace, 1);
        assert_eq!(restored.workspaces.len(), 3);
    }

    #[test]
    fn test_session_roundtrip_nested_splits() {
        let state = SessionState {
            version: 1,
            active_workspace: 0,
            workspaces: vec![WorkspaceSession {
                title: "dev".to_string(),
                cwd: "/home/user".to_string(),
                custom_buttons: vec![],
                layout: Some(LayoutNode::Split {
                    direction: "horizontal".to_string(),
                    ratio: None,
                    ratios: Some(vec![0.6, 0.4]),
                    children: vec![
                        LayoutNode::Pane {
                            surfaces: vec![make_surface("/home/user/code")],
                        },
                        LayoutNode::Split {
                            direction: "vertical".to_string(),
                            ratio: None,
                            ratios: Some(vec![0.5, 0.5]),
                            children: vec![
                                LayoutNode::Pane {
                                    surfaces: vec![make_surface("/home/user/tests")],
                                },
                                LayoutNode::Pane {
                                    surfaces: vec![make_surface("/home/user/logs")],
                                },
                            ],
                        },
                    ],
                }),
            }],
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
        // Verify structure: root is split with 2 children, second child is also a split
        let layout = restored.workspaces[0].layout.as_ref().unwrap();
        assert_eq!(layout.leaf_count(), 3);
    }

    #[test]
    fn test_session_roundtrip_with_scrollback() {
        let state = SessionState {
            version: 1,
            active_workspace: 0,
            workspaces: vec![WorkspaceSession {
                title: "main".to_string(),
                cwd: "/tmp".to_string(),
                custom_buttons: vec![],
                layout: Some(LayoutNode::Pane {
                    surfaces: vec![SurfaceDefinition {
                        surface_type: Some("terminal".to_string()),
                        cwd: Some("/tmp".to_string()),
                        scrollback: Some(
                            "$ ls\nfile1.txt\nfile2.txt\n$ echo hello\nhello".to_string(),
                        ),
                        ..Default::default()
                    }],
                }),
            }],
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, restored);
        let surface = match &restored.workspaces[0].layout {
            Some(LayoutNode::Pane { surfaces }) => &surfaces[0],
            _ => panic!("expected pane"),
        };
        assert!(surface.scrollback.as_ref().unwrap().contains("hello"));
    }

    #[test]
    fn test_session_corrupted_json_returns_none() {
        // Truncated JSON — simulates crash during write
        let corrupted = r#"{"version":1,"active_workspace":0,"workspaces":[{"title":"ma"#;
        let result: Result<SessionState, _> = serde_json::from_str(corrupted);
        assert!(result.is_err(), "Corrupted JSON should fail to parse");
    }

    #[test]
    fn test_session_scrollback_none_omitted_from_json() {
        let surface = SurfaceDefinition {
            scrollback: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&surface).unwrap();
        assert!(
            !json.contains("scrollback"),
            "None scrollback should be omitted from JSON"
        );
    }
}
