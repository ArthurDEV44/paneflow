//! Workspace — a named collection of terminal panes with a split layout.

use paneflow_config::schema::LayoutNode;

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

/// Read up to `limit` bytes from a file, returning the content as a `String`.
/// Prevents unbounded reads from malicious or corrupted files.
fn read_capped(path: &std::path::Path, limit: u64) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut content = String::new();
    file.take(limit).read_to_string(&mut content)?;
    Ok(content)
}

/// Detect the current git branch for a working directory.
///
/// Walks up from `cwd` to find the nearest `.git` directory or worktree file,
/// then reads `HEAD` directly (no subprocess). Returns `(branch_name, is_git_repo)`.
/// - Normal branch: `("main", true)`
/// - Detached HEAD: `("(abc1234)", true)`
/// - Not a git repo: `("", false)`
///
/// Only `refs/heads/` branches are resolved; tags and remote refs return empty.
pub fn detect_branch(cwd: &str) -> (String, bool) {
    // Walk up the directory tree to find .git (mirrors how git itself resolves)
    let mut search_dir = std::path::Path::new(cwd);
    let git_path = loop {
        let candidate = search_dir.join(".git");
        if candidate.exists() {
            break candidate;
        }
        match search_dir.parent() {
            Some(parent) => search_dir = parent,
            None => return (String::new(), false),
        }
    };

    // Resolve HEAD path — .git may be a directory or a worktree file
    let head_path = if git_path.is_file() {
        // Worktree: .git is a file containing "gitdir: <path>"
        let content = match read_capped(&git_path, 512) {
            Ok(c) => c,
            Err(_) => return (String::new(), false),
        };
        let gitdir = match content.trim().strip_prefix("gitdir: ") {
            Some(path) => path.to_owned(),
            None => return (String::new(), false),
        };
        let gitdir_path = if std::path::Path::new(&gitdir).is_absolute() {
            std::path::PathBuf::from(&gitdir)
        } else {
            git_path
                .parent()
                .unwrap_or(std::path::Path::new(cwd))
                .join(&gitdir)
        };
        gitdir_path.join("HEAD")
    } else if git_path.is_dir() {
        git_path.join("HEAD")
    } else {
        return (String::new(), false);
    };

    let content = match read_capped(&head_path, 512) {
        Ok(c) => c,
        Err(_) => return (String::new(), false),
    };
    let content = content.trim();

    if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
        (branch.to_string(), true)
    } else if content.chars().all(|c| c.is_ascii_hexdigit())
        && (content.len() == 40 || content.len() == 64)
    {
        // Detached HEAD — raw SHA-1 (40 chars) or SHA-256 (64 chars)
        let short = &content[..7];
        (format!("({short})"), true)
    } else {
        // Unrecognized format (tag ref, remote ref, corrupted) — git repo but branch unknown
        (String::new(), true)
    }
}

pub struct Workspace {
    pub title: String,
    /// Working directory at creation time. Does not update when the shell `cd`s.
    pub cwd: String,
    pub root: Option<LayoutTree>,
    /// Saved layout tree when zoomed. `Some(tree)` means the workspace is zoomed
    /// and `root` contains only the zoomed pane as a single Leaf.
    pub saved_layout: Option<LayoutTree>,
    /// Cached git diff stats, refreshed by a background poller.
    pub git_stats: GitDiffStats,
    /// Current git branch name. Empty string when not a git repo or branch unknown.
    pub git_branch: String,
    /// Whether this workspace's CWD is inside a git repository.
    pub is_git_repo: bool,
}

impl Workspace {
    pub fn new(title: impl Into<String>, pane: Entity<Pane>) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".into());
        let git_stats = GitDiffStats::from_cwd(&cwd);
        let (git_branch, is_git_repo) = detect_branch(&cwd);
        Self {
            title: title.into(),
            cwd,
            root: Some(LayoutTree::Leaf(pane)),
            saved_layout: None,
            git_stats,
            git_branch,
            is_git_repo,
        }
    }

    pub fn with_cwd(title: impl Into<String>, cwd: std::path::PathBuf, pane: Entity<Pane>) -> Self {
        let cwd_str = cwd.display().to_string();
        let git_stats = GitDiffStats::from_cwd(&cwd_str);
        let (git_branch, is_git_repo) = detect_branch(&cwd_str);
        Self {
            title: title.into(),
            cwd: cwd_str,
            root: Some(LayoutTree::Leaf(pane)),
            saved_layout: None,
            git_stats,
            git_branch,
            is_git_repo,
        }
    }

    pub fn is_zoomed(&self) -> bool {
        self.saved_layout.is_some()
    }

    pub fn pane_count(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.leaf_count())
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        if let Some(root) = &self.root {
            root.focus_first(window, cx);
        }
    }

    /// Serialize the workspace layout to a `LayoutNode`.
    ///
    /// When zoomed, serializes the saved (un-zoomed) layout so that the full
    /// pane arrangement is captured rather than just the single zoomed pane.
    pub fn serialize_layout(&self, cx: &App) -> Option<LayoutNode> {
        let tree = self.saved_layout.as_ref().or(self.root.as_ref())?;
        Some(tree.serialize(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_branch_normal_branch() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let (branch, is_repo) = detect_branch(dir.path().to_str().unwrap());
        assert_eq!(branch, "main");
        assert!(is_repo);
    }

    #[test]
    fn detect_branch_feature_branch() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(
            git_dir.join("HEAD"),
            "ref: refs/heads/feature/JIRA-123-oauth\n",
        )
        .unwrap();

        let (branch, is_repo) = detect_branch(dir.path().to_str().unwrap());
        assert_eq!(branch, "feature/JIRA-123-oauth");
        assert!(is_repo);
    }

    #[test]
    fn detect_branch_detached_head() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(
            git_dir.join("HEAD"),
            "96fa6899ea34697257e84865fefc56beb42d6390\n",
        )
        .unwrap();

        let (branch, is_repo) = detect_branch(dir.path().to_str().unwrap());
        assert_eq!(branch, "(96fa689)");
        assert!(is_repo);
    }

    #[test]
    fn detect_branch_not_a_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        // No .git directory

        let (branch, is_repo) = detect_branch(dir.path().to_str().unwrap());
        assert_eq!(branch, "");
        assert!(!is_repo);
    }

    #[test]
    fn detect_branch_worktree_file() {
        let dir = tempfile::tempdir().unwrap();
        // Simulate a worktree: .git is a file pointing to a gitdir
        let worktree_git_dir = dir.path().join("worktree_git");
        std::fs::create_dir(&worktree_git_dir).unwrap();
        std::fs::write(worktree_git_dir.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();

        let work_dir = dir.path().join("work");
        std::fs::create_dir(&work_dir).unwrap();
        std::fs::write(
            work_dir.join(".git"),
            format!("gitdir: {}\n", worktree_git_dir.display()),
        )
        .unwrap();

        let (branch, is_repo) = detect_branch(work_dir.to_str().unwrap());
        assert_eq!(branch, "wt-branch");
        assert!(is_repo);
    }

    #[test]
    fn detect_branch_nonexistent_directory() {
        let (branch, is_repo) = detect_branch("/nonexistent/path/that/does/not/exist");
        assert_eq!(branch, "");
        assert!(!is_repo);
    }

    #[test]
    fn detect_branch_worktree_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        // Simulate a worktree with a relative gitdir path
        let worktree_git_dir = dir
            .path()
            .join("main_repo")
            .join(".git")
            .join("worktrees")
            .join("wt1");
        std::fs::create_dir_all(&worktree_git_dir).unwrap();
        std::fs::write(
            worktree_git_dir.join("HEAD"),
            "ref: refs/heads/relative-wt\n",
        )
        .unwrap();

        let work_dir = dir.path().join("wt1");
        std::fs::create_dir(&work_dir).unwrap();
        // Relative path from work_dir/.git to the worktree git dir
        std::fs::write(
            work_dir.join(".git"),
            "gitdir: ../main_repo/.git/worktrees/wt1\n",
        )
        .unwrap();

        let (branch, is_repo) = detect_branch(work_dir.to_str().unwrap());
        assert_eq!(branch, "relative-wt");
        assert!(is_repo);
    }

    #[test]
    fn detect_branch_subdirectory_of_repo() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/develop\n").unwrap();

        // Create a subdirectory without its own .git
        let sub_dir = dir.path().join("src").join("module");
        std::fs::create_dir_all(&sub_dir).unwrap();

        let (branch, is_repo) = detect_branch(sub_dir.to_str().unwrap());
        assert_eq!(branch, "develop");
        assert!(is_repo);
    }
}
