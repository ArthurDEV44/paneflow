//! Agent presence detection (EP-002 US-005).
//!
//! An agent is considered **present** if either its CLI is resolvable on
//! `PATH` (`which`) **or** any of its config files/dirs already exists. The
//! pure decision core [`Presence::from_signals`] is unit-tested; the impure
//! wrapper [`detect`] layers `which::which` + filesystem existence on top.

use std::path::{Path, PathBuf};

/// Whether an agent looks installed on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// CLI on PATH or a config file/dir exists.
    Present,
    /// Neither signal fired — the agent is skipped (not an error).
    Absent,
}

impl Presence {
    /// Pure decision core. `Present` iff the CLI is on PATH **or** a config
    /// path exists. Kept separate from IO so it is exhaustively unit-tested
    /// without touching the real PATH or filesystem.
    #[must_use]
    pub fn from_signals(cli_on_path: bool, config_exists: bool) -> Self {
        if cli_on_path || config_exists {
            Self::Present
        } else {
            Self::Absent
        }
    }

    #[must_use]
    pub fn is_present(self) -> bool {
        matches!(self, Self::Present)
    }
}

/// Real detection: is `cli` resolvable on `PATH`, or does any path in
/// `config_paths` exist on disk?
///
/// `cli` may be `None` for agents that ship no CLI (e.g. Gemini CLI exposes
/// no `gemini mcp add`; opencode is config-only here), in which case only
/// the config-path signal is consulted.
#[must_use]
pub fn detect(cli: Option<&str>, config_paths: &[PathBuf]) -> Presence {
    let cli_on_path = cli.is_some_and(|c| which::which(c).is_ok());
    let config_exists = config_paths.iter().any(|p| p.exists());
    Presence::from_signals(cli_on_path, config_exists)
}

/// Convenience: does `path` (file or dir) exist? Thin wrapper kept here so
/// writers express detection declaratively against this module.
#[must_use]
pub fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn present_when_cli_on_path() {
        assert_eq!(Presence::from_signals(true, false), Presence::Present);
    }

    #[test]
    fn present_when_config_exists() {
        assert_eq!(Presence::from_signals(false, true), Presence::Present);
    }

    #[test]
    fn present_when_both() {
        assert_eq!(Presence::from_signals(true, true), Presence::Present);
    }

    #[test]
    fn absent_when_neither() {
        assert_eq!(Presence::from_signals(false, false), Presence::Absent);
        assert!(!Presence::from_signals(false, false).is_present());
    }

    #[test]
    fn detect_uses_config_path_existence() {
        let dir = tempfile::TempDir::new().unwrap();
        let existing = dir.path().join("config.json");
        std::fs::write(&existing, b"{}").unwrap();
        let missing = dir.path().join("nope.json");

        // No CLI named, but an existing config path → Present.
        assert_eq!(
            detect(None, std::slice::from_ref(&existing)),
            Presence::Present
        );
        // No CLI, no existing path → Absent.
        assert_eq!(detect(None, &[missing]), Presence::Absent);
    }

    #[test]
    fn detect_absent_for_unresolvable_cli_and_no_config() {
        // A CLI name that cannot exist on PATH, and no config paths.
        let bogus = "paneflow-nonexistent-agent-cli-xyz";
        assert_eq!(detect(Some(bogus), &[]), Presence::Absent);
    }
}
