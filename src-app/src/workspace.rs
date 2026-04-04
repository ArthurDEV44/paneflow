//! Workspace — a named collection of terminal panes with a split layout.

use crate::pane::Pane;
use crate::split::LayoutTree;
use gpui::{App, Entity, Window};

/// Git diff statistics for a workspace directory.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct GitDiffStats {
    pub insertions: usize,
    pub deletions: usize,
}

impl GitDiffStats {
    /// Run `git diff --shortstat` in the given directory and parse the result.
    pub fn from_cwd(cwd: &str) -> Self {
        let output = std::process::Command::new("git")
            .args(["diff", "--shortstat"])
            .current_dir(cwd)
            .output();

        let Ok(output) = output else {
            return Self::default();
        };
        if !output.status.success() {
            return Self::default();
        }

        let text = String::from_utf8_lossy(&output.stdout);
        Self::parse_shortstat(&text)
    }

    /// Parse `git diff --shortstat` output, e.g.:
    /// " 3 files changed, 42 insertions(+), 7 deletions(-)"
    fn parse_shortstat(text: &str) -> Self {
        let mut insertions = 0usize;
        let mut deletions = 0usize;

        for part in text.split(',') {
            let trimmed = part.trim();
            if trimmed.contains("insertion") {
                if let Some(n) = trimmed.split_whitespace().next() {
                    insertions = n.parse().unwrap_or(0);
                }
            } else if trimmed.contains("deletion") {
                if let Some(n) = trimmed.split_whitespace().next() {
                    deletions = n.parse().unwrap_or(0);
                }
            }
        }

        Self {
            insertions,
            deletions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.insertions == 0 && self.deletions == 0
    }
}

pub struct Workspace {
    pub title: String,
    /// Working directory at creation time. Does not update when the shell `cd`s.
    pub cwd: String,
    pub root: Option<LayoutTree>,
    /// Cached git diff stats, refreshed by a background poller.
    pub git_stats: GitDiffStats,
}

impl Workspace {
    pub fn new(title: impl Into<String>, pane: Entity<Pane>) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".into());
        let git_stats = GitDiffStats::from_cwd(&cwd);
        Self {
            title: title.into(),
            cwd,
            root: Some(LayoutTree::Leaf(pane)),
            git_stats,
        }
    }

    pub fn with_cwd(title: impl Into<String>, cwd: std::path::PathBuf, pane: Entity<Pane>) -> Self {
        let cwd_str = cwd.display().to_string();
        let git_stats = GitDiffStats::from_cwd(&cwd_str);
        Self {
            title: title.into(),
            cwd: cwd_str,
            root: Some(LayoutTree::Leaf(pane)),
            git_stats,
        }
    }

    pub fn pane_count(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.leaf_count())
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        if let Some(root) = &self.root {
            root.focus_first(window, cx);
        }
    }
}
