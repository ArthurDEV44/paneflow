//! External-editor resolution + spawn for markdown link clicks.
//!
//! Wired by [`super::message_render`]: when the user clicks a
//! markdown link inside an assistant message, the handler asks this
//! module to open the target. The module picks a CLI based on the
//! user's `external_editor` setting (auto / system / explicit) and
//! spawns it with the path + `:line[:col]` suffix preserved -- all
//! four supported editors parse that natively to jump to the target
//! position. When no CLI is configured or detected, the caller falls
//! back to GPUI's `cx.open_url`, which defers to xdg-open / open /
//! start.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorCli {
    Zed,
    Cursor,
    Windsurf,
    VsCode,
}

impl EditorCli {
    fn bin_name(self) -> &'static str {
        match self {
            EditorCli::Zed => "zed",
            EditorCli::Cursor => "cursor",
            EditorCli::Windsurf => "windsurf",
            EditorCli::VsCode => "code",
        }
    }

    fn from_setting(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "zed" => Some(EditorCli::Zed),
            "cursor" => Some(EditorCli::Cursor),
            "windsurf" => Some(EditorCli::Windsurf),
            "code" | "vscode" | "vs-code" => Some(EditorCli::VsCode),
            _ => None,
        }
    }
}

/// Preference order for `"auto"` detection. Tweakable later if a new
/// editor enters the rotation; the ordering is not visible to users
/// since the detected pick is opaque from their perspective.
const PREFERRED_ORDER: &[EditorCli] = &[
    EditorCli::Zed,
    EditorCli::Cursor,
    EditorCli::Windsurf,
    EditorCli::VsCode,
];

/// Detect the first installed editor from [`PREFERRED_ORDER`]. The
/// result is cached for the lifetime of the process since `PATH`
/// changes mid-session are vanishingly rare and re-running `which`
/// on every click would otherwise burn one syscall sweep per link.
fn detect_first_available() -> Option<EditorCli> {
    static CACHE: std::sync::OnceLock<Option<EditorCli>> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        for editor in PREFERRED_ORDER {
            if which::which(editor.bin_name()).is_ok() {
                return Some(*editor);
            }
        }
        None
    })
}

/// Resolve the editor to use for the current click, honouring the
/// `external_editor` config field. Returns `None` when the user opted
/// into `"system"` or when `"auto"` finds nothing installed; callers
/// are expected to fall through to the system opener.
fn resolve_active() -> Option<EditorCli> {
    let config = paneflow_config::loader::load_config();
    match config.external_editor.as_deref() {
        Some("system") => None,
        Some("auto") | None | Some("") => detect_first_available(),
        Some(other) => EditorCli::from_setting(other).or_else(detect_first_available),
    }
}

/// Try to open `href` in the configured editor. `href` is the raw
/// markdown link string (`foo.rs` or `foo.rs:42` or `foo.rs:42:8`);
/// the `:line[:col]` suffix is preserved verbatim because every
/// editor in [`PREFERRED_ORDER`] supports it natively.
///
/// Returns `true` when the editor was spawned successfully; `false`
/// when no editor is configured / detected or the spawn failed. The
/// caller should fall back to the system opener on `false`.
pub fn open(href: &str, cwd: Option<&Path>) -> bool {
    let Some(editor) = resolve_active() else {
        return false;
    };
    // Workspace scoping: refuse paths outside the active cwd so an
    // agent-controlled markdown link cannot trick the user into opening
    // /etc/passwd or ~/.ssh/id_rsa in their configured editor (where
    // editor plugins, history, or sync could then exfiltrate the
    // content). The `:line[:col]` suffix is excluded from the check
    // and re-attached on the happy path.
    let (path_part, _) = split_line_col_suffix(href);
    if !is_inside_workspace(Path::new(path_part), cwd) {
        log::warn!(
            "external_editor: blocked open of {} -- target is outside the active workspace cwd ({:?})",
            path_part,
            cwd,
        );
        return false;
    }
    let target = absolute_target(href, cwd);
    match Command::new(editor.bin_name()).arg(&target).spawn() {
        Ok(_child) => {
            log::debug!("external_editor: spawned {} {target}", editor.bin_name());
            true
        }
        Err(err) => {
            log::warn!(
                "external_editor: spawn {} {target} failed: {err}",
                editor.bin_name(),
            );
            false
        }
    }
}

/// Decide whether `target` is safe to hand to an editor CLI. Resolves
/// the target against `cwd` and rejects anything that does not live
/// under the canonicalised workspace root. Returns `false` when no
/// `cwd` is supplied -- we cannot prove the link is safe without an
/// anchor, so the guard fails closed.
///
/// `canonicalize` is preferred (it resolves symlinks and collapses
/// `..` segments) so a symlinked escape under the workspace cannot
/// slip through a string-prefix check. When the target does not yet
/// exist (rare but possible -- an agent may link to a file it has
/// just claimed to create), the lexical absolute path is used so the
/// `starts_with` comparison still has meaning. Windows uses the same
/// path; `canonicalize` returns the verbatim-prefixed `\\?\C:\...`
/// form which preserves drive-letter casing for both sides of the
/// comparison.
fn is_inside_workspace(target: &Path, cwd: Option<&Path>) -> bool {
    let Some(cwd) = cwd else {
        return false;
    };
    let abs_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        cwd.join(target)
    };
    let Ok(cwd_canon) = cwd.canonicalize() else {
        return false;
    };
    let target_resolved = match abs_target.canonicalize() {
        Ok(p) => p,
        Err(_) => match std::path::absolute(&abs_target) {
            Ok(p) => p,
            Err(_) => return false,
        },
    };
    target_resolved.starts_with(&cwd_canon)
}

/// Resolve a (possibly relative) link target into an absolute
/// filesystem path, keeping any `:line[:col]` suffix intact so the
/// editor CLI can jump to position. When the path is already
/// absolute or no cwd is available, the input is forwarded as-is.
fn absolute_target(href: &str, cwd: Option<&Path>) -> String {
    let (path_part, suffix) = split_line_col_suffix(href);
    let path = Path::new(path_part);
    let abs: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(cwd) = cwd {
        cwd.join(path)
    } else {
        return href.to_string();
    };
    format!("{}{}", abs.display(), suffix)
}

/// Split `path:line[:col]` into the path prefix and the trailing
/// suffix (including the leading `:`). Mirrors the heuristic of
/// `message_render::strip_line_col_suffix` but returns both halves so
/// the caller can re-attach the suffix to the absolute path before
/// handing it to the editor.
fn split_line_col_suffix(href: &str) -> (&str, &str) {
    let mut idx = href.len();
    let mut split = href.len();
    for _ in 0..2 {
        let Some(prev_colon) = href[..idx].rfind(':') else {
            break;
        };
        let tail = &href[prev_colon + 1..idx];
        if tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            break;
        }
        split = prev_colon;
        idx = prev_colon;
    }
    href.split_at(split)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_bare_path_intact() {
        let (path, suffix) = split_line_col_suffix("src/foo.rs");
        assert_eq!(path, "src/foo.rs");
        assert_eq!(suffix, "");
    }

    #[test]
    fn split_extracts_single_line_suffix() {
        let (path, suffix) = split_line_col_suffix("src/foo.rs:42");
        assert_eq!(path, "src/foo.rs");
        assert_eq!(suffix, ":42");
    }

    #[test]
    fn split_extracts_line_and_column_suffix() {
        let (path, suffix) = split_line_col_suffix("src/foo.rs:42:8");
        assert_eq!(path, "src/foo.rs");
        assert_eq!(suffix, ":42:8");
    }

    #[test]
    fn split_does_not_strip_more_than_two_numeric_segments() {
        let (path, suffix) = split_line_col_suffix("src/foo.rs:1:2:3");
        // Only the last two trailing numeric segments are treated as
        // line/col; the third one is left attached to the path so we
        // don't mangle exotic filenames like `tag:1:2:3`.
        assert_eq!(path, "src/foo.rs:1");
        assert_eq!(suffix, ":2:3");
    }

    #[test]
    fn split_ignores_non_numeric_tail() {
        let (path, suffix) = split_line_col_suffix("crates/foo:bar.rs");
        assert_eq!(path, "crates/foo:bar.rs");
        assert_eq!(suffix, "");
    }

    #[cfg(unix)]
    #[test]
    fn absolute_target_resolves_against_cwd() {
        let cwd = Path::new("/home/arthur/dev/proj");
        let target = absolute_target("src/foo.rs:42", Some(cwd));
        assert_eq!(target, "/home/arthur/dev/proj/src/foo.rs:42");
    }

    #[cfg(windows)]
    #[test]
    fn absolute_target_resolves_against_cwd() {
        // Windows: `Path::join` uses `\`, so the absolute prefix and
        // separators differ from Unix. Verify shape (suffix preserved
        // + cwd-prefixed) rather than a hardcoded forward-slash string.
        let cwd = Path::new(r"C:\Users\arthur\proj");
        let target = absolute_target("src/foo.rs:42", Some(cwd));
        assert!(
            target.starts_with(r"C:\Users\arthur\proj"),
            "absolute_target output {target:?} should start with cwd",
        );
        assert!(
            target.ends_with(":42"),
            "absolute_target output {target:?} should preserve :42 suffix",
        );
    }

    #[cfg(unix)]
    #[test]
    fn absolute_target_passes_absolute_path_through() {
        let target = absolute_target("/etc/hostname:3", Some(Path::new("/tmp")));
        assert_eq!(target, "/etc/hostname:3");
    }

    #[cfg(windows)]
    #[test]
    fn absolute_target_passes_absolute_path_through() {
        // Windows absolute path = drive-letter + backslash. Pick a path
        // that's guaranteed-absolute on every Windows machine.
        let target = absolute_target(r"C:\Windows\notepad.exe:3", Some(Path::new(r"C:\tmp")));
        assert_eq!(target, r"C:\Windows\notepad.exe:3");
    }

    #[test]
    fn editor_cli_from_setting_accepts_vscode_aliases() {
        assert_eq!(EditorCli::from_setting("code"), Some(EditorCli::VsCode));
        assert_eq!(EditorCli::from_setting("vscode"), Some(EditorCli::VsCode));
        assert_eq!(EditorCli::from_setting("VS-CODE"), Some(EditorCli::VsCode));
    }

    #[test]
    fn editor_cli_from_setting_rejects_unknown_values() {
        assert_eq!(EditorCli::from_setting("nvim"), None);
        assert_eq!(EditorCli::from_setting("system"), None);
        assert_eq!(EditorCli::from_setting(""), None);
    }

    /// US-011: a relative path under the workspace cwd is allowed.
    #[test]
    fn is_inside_workspace_inside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("src").join("foo.rs");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "fn main() {}").expect("write");
        assert!(is_inside_workspace(
            Path::new("src/foo.rs"),
            Some(dir.path())
        ));
    }

    /// US-011: an absolute path that lives outside the workspace cwd
    /// is rejected -- the canonicalised target does not share the
    /// workspace prefix.
    #[cfg(unix)]
    #[test]
    fn is_inside_workspace_outside_absolute() {
        let dir = tempfile::tempdir().expect("tempdir");
        // /etc exists on every Unix and is guaranteed outside any
        // tempdir-rooted workspace.
        assert!(!is_inside_workspace(
            Path::new("/etc/hostname"),
            Some(dir.path())
        ));
    }

    /// US-011: a `..` traversal that escapes the workspace cwd is
    /// rejected even when the underlying file exists. Canonicalize
    /// collapses the segments before the prefix check.
    #[cfg(unix)]
    #[test]
    fn is_inside_workspace_outside_via_traversal() {
        let parent = tempfile::tempdir().expect("tempdir");
        let workspace = parent.path().join("workspace");
        let sibling = parent.path().join("sibling");
        std::fs::create_dir_all(&workspace).expect("mkdir workspace");
        std::fs::create_dir_all(&sibling).expect("mkdir sibling");
        let escape_target = sibling.join("secret.txt");
        std::fs::write(&escape_target, "secret").expect("write");
        // From the workspace, `../sibling/secret.txt` escapes the cwd.
        assert!(!is_inside_workspace(
            Path::new("../sibling/secret.txt"),
            Some(&workspace),
        ));
    }

    /// US-011: with no cwd anchor the guard fails closed.
    #[test]
    fn is_inside_workspace_no_cwd() {
        assert!(!is_inside_workspace(Path::new("anything.rs"), None));
    }
}
