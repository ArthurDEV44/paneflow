//! Multi-worktree diff open handlers + worktree collection helpers,
//! extracted from `event_handlers.rs` (US-055 code-motion).

use gpui::{AppContext, Context, Window};

use crate::PaneFlowApp;

fn linked_worktree_root(git_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let content = read_small_text(&git_dir.join("gitdir"), 512).ok()?;
    let raw = content.trim();
    if raw.is_empty() {
        return None;
    }
    let git_file = std::path::Path::new(raw);
    let git_file = if git_file.is_absolute() {
        git_file.to_path_buf()
    } else {
        git_dir.join(git_file)
    };
    git_file.parent().map(|p| p.to_path_buf())
}

fn read_small_text(path: &std::path::Path, limit: u64) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut content = String::new();
    file.take(limit).read_to_string(&mut content)?;
    Ok(content)
}

fn diff_worktree_path(
    cwd: &str,
    repo_root: Option<&std::path::Path>,
    is_worktree: bool,
    git_dir: Option<&std::path::Path>,
) -> std::path::PathBuf {
    if is_worktree {
        git_dir
            .and_then(linked_worktree_root)
            .unwrap_or_else(|| std::path::PathBuf::from(cwd))
    } else {
        repo_root
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| std::path::PathBuf::from(cwd))
    }
}

fn push_unique_worktree(
    out: &mut Vec<crate::diff::DiffWorktree>,
    seen: &mut std::collections::HashSet<String>,
    path: std::path::PathBuf,
    branch: String,
    workspace_id: Option<u64>,
) {
    if seen.insert(norm_path(&path)) {
        out.push(crate::diff::DiffWorktree {
            path,
            branch,
            workspace_id,
        });
    }
}

impl PaneFlowApp {
    /// US-003 (prd-multi-worktree-diff) - action handler: open the
    /// multi-worktree diff view for the *active* workspace's repo. A no-op
    /// when the active workspace has no resolved `repo_root` (not a git repo).
    pub(crate) fn handle_open_multi_diff(
        &mut self,
        _: &crate::app::actions::OpenMultiDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo_root) = self
            .workspaces
            .get(self.active_idx)
            .and_then(|ws| ws.repo_root.clone())
        else {
            return;
        };
        self.open_multi_diff_for_repo(repo_root, window, cx);
    }

    /// Open a `DiffView` tab seeded with every sibling worktree sharing
    /// `repo_root`. The tab is hosted in the active workspace's focused pane
    /// (falling back to its first leaf); the diff content itself is
    /// repo-scoped, independent of which pane hosts it. Ephemeral - not
    /// persisted to the session. EP-002+ fills the seeded worktrees with
    /// real diff columns.
    /// Gather the sibling-worktree seed for a repo: one [`crate::diff::DiffWorktree`]
    /// per open workspace whose `repo_root` matches. US-005 of
    /// prd-git-diff-mode-2026-Q3.md extracted this from `open_multi_diff_for_repo`
    /// so the dedicated Diff mode (`rebuild_diff_view`) and the legacy tab path
    /// share one source of truth. Pure in-memory read - no git subprocess, safe
    /// to call on the main thread.
    pub(crate) fn collect_diff_worktrees(
        &self,
        repo_root: &std::path::Path,
    ) -> Vec<crate::diff::DiffWorktree> {
        let mut seen = std::collections::HashSet::new();
        let mut worktrees = Vec::new();
        for ws in self
            .workspaces
            .iter()
            .filter(|ws| ws.repo_root.as_deref() == Some(repo_root))
        {
            let path = diff_worktree_path(
                &ws.cwd,
                ws.repo_root.as_deref(),
                ws.is_worktree,
                ws.git_dir.as_deref(),
            );
            push_unique_worktree(
                &mut worktrees,
                &mut seen,
                path,
                ws.git_branch.clone(),
                Some(ws.id),
            );
        }
        worktrees
    }

    /// US-011: the active workspace as a single-element worktree seed (Project
    /// scope). Empty when there is no active workspace. Pure in-memory read.
    pub(crate) fn collect_project_worktrees(&self) -> Vec<crate::diff::DiffWorktree> {
        self.workspaces
            .get(self.active_idx)
            .map(|ws| {
                vec![crate::diff::DiffWorktree {
                    path: diff_worktree_path(
                        &ws.cwd,
                        ws.repo_root.as_deref(),
                        ws.is_worktree,
                        ws.git_dir.as_deref(),
                    ),
                    branch: ws.git_branch.clone(),
                    workspace_id: Some(ws.id),
                }]
            })
            .unwrap_or_default()
    }

    /// US-014: every open workspace grouped by canonicalized `repo_root`
    /// (Multi-project scope). `BTreeMap` keying gives stable repo ordering;
    /// workspaces with no resolved repo are skipped. Pure in-memory read.
    pub(crate) fn collect_multiproject_groups(&self) -> Vec<crate::diff::RepoGroup> {
        use std::collections::BTreeMap;
        let mut map: BTreeMap<
            std::path::PathBuf,
            (crate::diff::RepoGroup, std::collections::HashSet<String>),
        > = BTreeMap::new();
        for ws in &self.workspaces {
            let Some(root) = ws.repo_root.clone() else {
                continue;
            };
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            let path = diff_worktree_path(
                &ws.cwd,
                ws.repo_root.as_deref(),
                ws.is_worktree,
                ws.git_dir.as_deref(),
            );
            let (group, seen) = map.entry(root.clone()).or_insert_with(|| {
                (
                    crate::diff::RepoGroup {
                        repo_root: root.clone(),
                        repo_name: name,
                        worktrees: Vec::new(),
                    },
                    std::collections::HashSet::new(),
                )
            });
            push_unique_worktree(
                &mut group.worktrees,
                seen,
                path,
                ws.git_branch.clone(),
                Some(ws.id),
            );
        }
        map.into_values().map(|(group, _)| group).collect()
    }

    pub(crate) fn open_multi_diff_for_repo(
        &mut self,
        repo_root: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Gather sibling worktrees across all workspaces sharing this repo.
        let worktrees = self.collect_diff_worktrees(&repo_root);

        // Resolve a host pane from the active workspace (focused, else first leaf).
        let target_pane = {
            let Some(ws) = self.workspaces.get(self.active_idx) else {
                return;
            };
            let Some(root) = ws.root.as_ref() else {
                return;
            };
            root.focused_pane(window, cx)
                .or_else(|| root.collect_leaves().into_iter().next())
        };
        let Some(target_pane) = target_pane else {
            return;
        };

        let diff = cx.new(|cx| crate::diff::DiffView::new(repo_root, worktrees, cx));
        target_pane.update(cx, |pane, cx| {
            pane.add_diff_tab(diff, cx);
            cx.notify();
        });
        cx.notify();
    }
}

/// US-013: normalize a worktree path for dedup so the same checkout only gets
/// one diff column even when several workspaces/panes point at it.
fn norm_path(p: &std::path::Path) -> String {
    let resolved = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = resolved.to_string_lossy().into_owned();
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        s.to_lowercase()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_checkout_seeds_repo_root_not_subdir_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let subdir = repo.join("src");
        std::fs::create_dir_all(&subdir).unwrap();

        let root = diff_worktree_path(
            subdir.to_str().unwrap(),
            Some(&repo),
            false,
            Some(&repo.join(".git")),
        );

        assert_eq!(root, repo);
    }

    #[test]
    fn linked_worktree_seed_uses_gitdir_back_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let main_git = dir.path().join("main").join(".git");
        let wt = dir.path().join("repo.worktrees").join("feat");
        let wt_subdir = wt.join("src");
        let wt_git_dir = main_git.join("worktrees").join("feat");
        std::fs::create_dir_all(&wt_subdir).unwrap();
        std::fs::create_dir_all(&wt_git_dir).unwrap();
        std::fs::write(
            wt_git_dir.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();

        let root = diff_worktree_path(
            wt_subdir.to_str().unwrap(),
            Some(&dir.path().join("main")),
            true,
            Some(&wt_git_dir),
        );

        assert_eq!(root, wt);
    }

    #[test]
    fn push_unique_worktree_dedups_equivalent_paths() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        push_unique_worktree(&mut out, &mut seen, repo.clone(), "main".into(), Some(1));
        push_unique_worktree(&mut out, &mut seen, repo.join("."), "main".into(), Some(2));

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].workspace_id, Some(1));
    }
}
