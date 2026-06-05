//! Git-metadata probing for workspace CWDs: branch detection, diff stats, and
//! worktree-aware `.git` lookup. All functions are pure (no shared mutable
//! state) and cross-platform — `git diff --shortstat` is spawned for diff
//! stats, everything else reads `.git/HEAD` directly without subprocesses.
//!
//! Extracted from `workspace.rs` per US-030 of the src-app refactor PRD.

/// Git diff statistics for a workspace directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
            } else if trimmed.contains("deletion")
                && let Some(n) = trimmed.split_whitespace().next()
            {
                deletions = n.parse().unwrap_or(0);
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
pub(super) fn read_capped(path: &std::path::Path, limit: u64) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut content = String::new();
    file.take(limit).read_to_string(&mut content)?;
    Ok(content)
}

/// Find the `.git` directory for a working directory.
///
/// Walks up from `cwd` to find the nearest `.git` entry. For worktrees (`.git`
/// is a file), follows the `gitdir:` pointer to return the actual git metadata
/// directory where `HEAD` and `index` reside.
pub fn find_git_dir(cwd: &str) -> Option<std::path::PathBuf> {
    let mut search_dir = std::path::Path::new(cwd);
    let git_path = loop {
        let candidate = search_dir.join(".git");
        if candidate.exists() {
            break candidate;
        }
        match search_dir.parent() {
            Some(parent) => search_dir = parent,
            None => return None,
        }
    };

    if git_path.is_file() {
        // Worktree: .git is a file containing "gitdir: <path>"
        let content = read_capped(&git_path, 512).ok()?;
        let gitdir = content.trim().strip_prefix("gitdir: ")?.to_owned();
        let gitdir_path = if std::path::Path::new(&gitdir).is_absolute() {
            std::path::PathBuf::from(&gitdir)
        } else {
            git_path
                .parent()
                .unwrap_or(std::path::Path::new(cwd))
                .join(&gitdir)
        };
        Some(gitdir_path)
    } else if git_path.is_dir() {
        Some(git_path)
    } else {
        None
    }
}

/// Canonicalize a path, falling back to the input when it cannot be resolved
/// (e.g. the path does not exist). Canonicalization is what lets two sibling
/// worktrees of the same repo produce an *identical* `repo_root`, since their
/// `commondir` pointers (`../..`) both collapse to the same absolute path.
fn canonicalize_or(path: &std::path::Path) -> std::path::PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Lexically collapse `.`/`..` components without touching the filesystem.
///
/// US-056: a relative `commondir` (`../..`) joined onto a worktree git dir
/// yields a path littered with `..`. When the target exists `canonicalize_or`
/// resolves them, but on a missing/unresolvable path it returns the raw form —
/// so two sibling worktrees would *not* collapse to the same `repo_root`.
/// Normalizing first guarantees the best-effort fallback still collapses them.
/// This mirrors the component walk `canonicalize` performs, minus symlink
/// resolution (a leading `..` with nothing to pop is preserved verbatim).
fn normalize_lexically(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut stack: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                _ => stack.push(comp),
            },
            other => stack.push(other),
        }
    }
    let mut out = std::path::PathBuf::new();
    for comp in stack {
        out.push(comp.as_os_str());
    }
    out
}

/// Resolve the shared (main) `.git` directory for a given worktree git dir.
///
/// For a linked worktree, `git_dir` is `<main>/.git/worktrees/<name>` and holds
/// a `commondir` file pointing — usually relatively, e.g. `../..` — at the
/// shared `<main>/.git`. For a normal checkout, `git_dir` is already the main
/// `.git` and no `commondir` file exists, so it is returned as-is. An absolute
/// `commondir` is honored verbatim (no double-join). Result is canonicalized
/// when the path exists, otherwise returned best-effort (never panics).
pub fn resolve_main_git_dir(git_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let commondir_file = git_dir.join("commondir");
    let main_git_dir = if commondir_file.is_file() {
        let content = read_capped(&commondir_file, 512).ok()?;
        let rel = content.trim();
        if rel.is_empty() {
            return None;
        }
        let p = std::path::Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            normalize_lexically(&git_dir.join(p))
        }
    } else {
        git_dir.to_path_buf()
    };
    Some(canonicalize_or(&main_git_dir))
}

/// Resolve `(repo_root, is_worktree)` from a working directory's git dir.
///
/// - `repo_root`: the working directory of the shared repository (parent of the
///   main `.git`), canonicalized so sibling worktrees of one repo yield an
///   identical value — the key invariant for grouping siblings.
/// - `is_worktree`: true when `git_dir` is a *linked* worktree (it carries a
///   `commondir` file). The main checkout of a repo is not a worktree.
///
/// Never panics: a bare repo, a missing `commondir`, or an unresolvable path
/// degrades to a best-effort `repo_root` (possibly `None`).
pub fn resolve_repo_root(git_dir: &std::path::Path) -> (Option<std::path::PathBuf>, bool) {
    let is_worktree = git_dir.join("commondir").is_file();
    let repo_root = resolve_main_git_dir(git_dir)
        .and_then(|main_git| main_git.parent().map(|p| p.to_path_buf()));
    (repo_root, is_worktree)
}

/// Parse branch name from a known `.git` directory's `HEAD` file.
///
/// Returns `(branch_name, true)`. On read failure returns `("", true)` —
/// the directory is a git repo but the branch is unknown.
/// Only `refs/heads/` branches are resolved; tags and remote refs return empty.
pub(super) fn parse_head(git_dir: &std::path::Path) -> (String, bool) {
    let head_path = git_dir.join("HEAD");
    let content = match read_capped(&head_path, 512) {
        Ok(c) => c,
        Err(_) => return (String::new(), true),
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

/// Detect the current git branch for a working directory.
///
/// Walks up from `cwd` to find `.git`, reads `HEAD` directly (no subprocess).
/// Returns `(branch_name, is_git_repo)`.
/// - Normal branch: `("main", true)`
/// - Detached HEAD: `("(abc1234)", true)`
/// - Not a git repo: `("", false)`
pub fn detect_branch(cwd: &str) -> (String, bool) {
    match find_git_dir(cwd) {
        Some(git_dir) => parse_head(&git_dir),
        None => (String::new(), false),
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

    #[test]
    fn find_git_dir_normal_repo() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        let result = find_git_dir(dir.path().to_str().unwrap());
        assert_eq!(result, Some(git_dir));
    }

    #[test]
    fn find_git_dir_not_a_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = find_git_dir(dir.path().to_str().unwrap());
        assert_eq!(result, None);
    }

    #[test]
    fn find_git_dir_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let worktree_git_dir = dir
            .path()
            .join("main_repo")
            .join(".git")
            .join("worktrees")
            .join("wt1");
        std::fs::create_dir_all(&worktree_git_dir).unwrap();

        let work_dir = dir.path().join("wt1");
        std::fs::create_dir(&work_dir).unwrap();
        std::fs::write(
            work_dir.join(".git"),
            format!("gitdir: {}\n", worktree_git_dir.display()),
        )
        .unwrap();

        let result = find_git_dir(work_dir.to_str().unwrap());
        assert_eq!(result, Some(worktree_git_dir));
    }

    #[test]
    fn find_git_dir_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        let sub_dir = dir.path().join("src").join("lib");
        std::fs::create_dir_all(&sub_dir).unwrap();

        let result = find_git_dir(sub_dir.to_str().unwrap());
        assert_eq!(result, Some(git_dir));
    }

    #[test]
    fn resolve_repo_root_normal_repo() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();

        let (repo_root, is_worktree) = resolve_repo_root(&git_dir);
        assert!(!is_worktree);
        // repo_root is the canonicalized parent of `.git` (the repo working dir).
        assert_eq!(repo_root, Some(std::fs::canonicalize(dir.path()).unwrap()));
    }

    #[test]
    fn resolve_repo_root_worktree_relative_commondir() {
        let dir = tempfile::tempdir().unwrap();
        // Main repo at <root>/main, its .git at <root>/main/.git
        let main_git = dir.path().join("main").join(".git");
        std::fs::create_dir_all(&main_git).unwrap();
        // Linked worktree git dir at <root>/main/.git/worktrees/wt1
        let wt_git = main_git.join("worktrees").join("wt1");
        std::fs::create_dir_all(&wt_git).unwrap();
        // commondir points back to the main .git relatively: ../..
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();

        let (repo_root, is_worktree) = resolve_repo_root(&wt_git);
        assert!(is_worktree);
        assert_eq!(
            repo_root,
            Some(std::fs::canonicalize(dir.path().join("main")).unwrap())
        );
    }

    #[test]
    fn resolve_repo_root_worktree_absolute_commondir() {
        let dir = tempfile::tempdir().unwrap();
        let main_git = dir.path().join("main").join(".git");
        std::fs::create_dir_all(&main_git).unwrap();
        let wt_git = main_git.join("worktrees").join("wt2");
        std::fs::create_dir_all(&wt_git).unwrap();
        // Absolute commondir must be honored verbatim (no double-join).
        std::fs::write(
            wt_git.join("commondir"),
            format!("{}\n", main_git.display()),
        )
        .unwrap();

        let (repo_root, is_worktree) = resolve_repo_root(&wt_git);
        assert!(is_worktree);
        assert_eq!(
            repo_root,
            Some(std::fs::canonicalize(dir.path().join("main")).unwrap())
        );
    }

    #[test]
    fn resolve_repo_root_siblings_match() {
        // Two sibling worktrees of the same repo must produce an identical repo_root.
        let dir = tempfile::tempdir().unwrap();
        let main_git = dir.path().join("main").join(".git");
        std::fs::create_dir_all(&main_git).unwrap();
        let wt_a = main_git.join("worktrees").join("a");
        let wt_b = main_git.join("worktrees").join("b");
        std::fs::create_dir_all(&wt_a).unwrap();
        std::fs::create_dir_all(&wt_b).unwrap();
        std::fs::write(wt_a.join("commondir"), "../..\n").unwrap();
        std::fs::write(wt_b.join("commondir"), "../..\n").unwrap();

        let (root_a, _) = resolve_repo_root(&wt_a);
        let (root_b, _) = resolve_repo_root(&wt_b);
        assert!(root_a.is_some());
        assert_eq!(root_a, root_b);
    }

    #[test]
    fn normalize_lexically_collapses_dotdot() {
        // US-056: a relative commondir on a non-existent worktree must still
        // collapse `..` lexically so sibling worktrees resolve to the same
        // repo_root even when canonicalize can't (target missing on disk).
        let wt_git = std::path::Path::new("/nonexistent/main/.git/worktrees/wt1");
        assert_eq!(
            normalize_lexically(&wt_git.join("../..")),
            std::path::PathBuf::from("/nonexistent/main/.git")
        );
        // A leading `..` with nothing to pop is preserved verbatim.
        assert_eq!(
            normalize_lexically(std::path::Path::new("../foo/./bar")),
            std::path::PathBuf::from("../foo/bar")
        );
    }

    #[test]
    fn resolve_repo_root_missing_dir() {
        let (repo_root, is_worktree) = resolve_repo_root(std::path::Path::new("/nonexistent/.git"));
        assert!(!is_worktree);
        // Non-existent path: canonicalize fails, parent is still derivable.
        assert_eq!(repo_root, Some(std::path::PathBuf::from("/nonexistent")));
    }
}
