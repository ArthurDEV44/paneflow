//! US-011 (prd-git-diff-mode-2026-Q3.md): the three Git Diff view scopes and
//! the multi-project repo-group type. Pure types — the enumeration helpers that
//! read open workspaces live as `PaneFlowApp` methods (`collect_*`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::DiffWorktree;

/// Which change set the Git Diff mode shows. Persisted per session (US-015) via
/// snake_case serde; defaults to `Project` (the lowest-surprise zoom level).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    /// The active workspace only.
    #[default]
    Project,
    /// Every open workspace, grouped by repo.
    MultiProject,
    /// All sibling worktrees of the active repo (open + discovered on disk).
    Worktree,
}

impl DiffScope {
    /// Short label for the scope-selector trigger and rows.
    pub fn label(self) -> &'static str {
        match self {
            DiffScope::Project => "Project",
            DiffScope::MultiProject => "Multi-project",
            DiffScope::Worktree => "Worktree",
        }
    }

    /// The scopes in selector order (Project / Multi-project / Worktree).
    pub fn all() -> [DiffScope; 3] {
        [
            DiffScope::Project,
            DiffScope::MultiProject,
            DiffScope::Worktree,
        ]
    }

    /// US-015: stable snake_case string for session persistence (matches the
    /// serde `rename_all`). Kept as an explicit map so the persisted form is
    /// decoupled from any future variant renames.
    pub fn as_persisted(self) -> &'static str {
        match self {
            DiffScope::Project => "project",
            DiffScope::MultiProject => "multi_project",
            DiffScope::Worktree => "worktree",
        }
    }

    /// US-015: parse the persisted snake_case string; `None` for unknown values
    /// (a hand-edited / future session.json), letting the caller fall back to
    /// the default scope.
    pub fn from_persisted(s: &str) -> Option<DiffScope> {
        match s {
            "project" => Some(DiffScope::Project),
            "multi_project" => Some(DiffScope::MultiProject),
            "worktree" => Some(DiffScope::Worktree),
            _ => None,
        }
    }
}

/// One repo's change set for the Multi-project scope (US-014): the shared repo
/// root, its display name (last path component), and the open worktrees that
/// belong to it.
pub struct RepoGroup {
    pub repo_root: PathBuf,
    pub repo_name: String,
    pub worktrees: Vec<DiffWorktree>,
}
