//! opencode writer (EP-003 US-010).
//!
//! opencode's schema diverges from every other agent:
//! - the container key is **`mcp`**, not `mcpServers`;
//! - the entry is `{type: "local", command: [<path>], enabled: true}` -
//!   `command` is an **array**, with the binary path as its first element.
//!
//! Config lives at `~/.config/opencode/opencode.json` (XDG; `%APPDATA%`
//! on Windows). No CLI mutates server config, so this is always a direct
//! merge - preserving `$schema` and any sibling `mcp.*` entries.
//!
//! **Volatility:** opencode's config schema is young; re-verify the `mcp`
//! key, `type: "local"`, and array `command` if registration regresses.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::agents::{support, AgentConfigWriter, InstallOutcome, StatusOutcome, UninstallOutcome};
use crate::detect::{self, Presence};

const CLI: &str = "opencode";
const CONTAINER: &str = "mcp";

pub struct OpenCode {
    config_path: Option<PathBuf>,
}

impl OpenCode {
    #[must_use]
    pub fn new() -> Self {
        Self {
            config_path: support::opencode_config(),
        }
    }

    fn path(&self) -> Result<&Path> {
        self.config_path
            .as_deref()
            .ok_or_else(|| anyhow!("cannot resolve config dir for opencode.json"))
    }

    fn entry(bridge: &str) -> serde_json::Value {
        // `command` is an ARRAY for opencode; `type: "local"` marks a stdio
        // child process; `enabled: true` activates it.
        json!({ "type": "local", "command": [bridge], "enabled": true })
    }
}

impl Default for OpenCode {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentConfigWriter for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }
    fn label(&self) -> &'static str {
        "opencode"
    }

    fn presence(&self) -> Presence {
        let paths: Vec<PathBuf> = self.config_path.clone().into_iter().collect();
        detect::detect(Some(CLI), &paths)
    }

    fn install(&self, bridge: &Path) -> Result<InstallOutcome> {
        let bridge_s = bridge.to_string_lossy().into_owned();
        support::json_install(self.path()?, CONTAINER, Self::entry(&bridge_s))
    }

    fn uninstall(&self) -> Result<UninstallOutcome> {
        support::json_uninstall(self.path()?, CONTAINER)
    }

    fn status(&self, bridge: &Path) -> Result<StatusOutcome> {
        // opencode stores `command` as an array → use the array extractor.
        support::json_status(self.path()?, CONTAINER, bridge, support::array_command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_writer(path: PathBuf) -> OpenCode {
        OpenCode {
            config_path: Some(path),
        }
    }

    #[test]
    fn install_writes_local_array_entry_under_mcp() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("opencode.json");
        let w = test_writer(p.clone());
        assert_eq!(
            w.install(Path::new("/data/paneflow-mcp")).unwrap(),
            InstallOutcome::Installed
        );
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        let entry = &v["mcp"]["paneflow"];
        assert_eq!(entry["type"], json!("local"));
        assert_eq!(
            entry["command"],
            json!(["/data/paneflow-mcp"]),
            "command is an array"
        );
        assert_eq!(entry["enabled"], json!(true));
        // Must NOT land under mcpServers.
        assert!(v.get("mcpServers").is_none());
    }

    #[test]
    fn install_preserves_schema_and_sibling_mcp_entries() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("opencode.json");
        std::fs::write(
            &p,
            serde_json::to_vec(&json!({
                "$schema": "https://opencode.ai/config.json",
                "mcp": { "weather": { "type": "local", "command": ["weather-mcp"], "enabled": true } }
            }))
            .unwrap(),
        )
        .unwrap();
        let w = test_writer(p.clone());
        w.install(Path::new("/data/paneflow-mcp")).unwrap();

        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(v["$schema"], json!("https://opencode.ai/config.json"));
        assert_eq!(v["mcp"]["weather"]["command"], json!(["weather-mcp"]));
        assert_eq!(
            v["mcp"]["paneflow"]["command"],
            json!(["/data/paneflow-mcp"])
        );
    }

    #[test]
    fn status_reads_array_command_and_flags_stale() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("opencode.json");
        let w = test_writer(p);
        w.install(Path::new("/old/paneflow-mcp")).unwrap();
        assert_eq!(
            w.status(Path::new("/old/paneflow-mcp")).unwrap(),
            StatusOutcome::Installed {
                path: "/old/paneflow-mcp".into()
            }
        );
        assert_eq!(
            w.status(Path::new("/new/paneflow-mcp")).unwrap(),
            StatusOutcome::StalePath {
                found: "/old/paneflow-mcp".into(),
                expected: "/new/paneflow-mcp".into()
            }
        );
    }
}
