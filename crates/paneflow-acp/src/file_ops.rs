//! Sandboxed file read/write handlers for ACP.
//!
//! Every `WriteTextFileRequest` / `ReadTextFileRequest` must resolve inside
//! the session's cwd (set at `session/new` time and tracked by the
//! [`SessionRegistry`][crate::session::SessionRegistry]). The check uses
//! [`std::fs::canonicalize`] on both sides before the prefix comparison so
//! symlinks cannot escape the sandbox (FR-10 + Risk #5 of the PRD).
//!
//! Canonicalization fails when the target file does not yet exist (the
//! common case for the first write). To work around that we canonicalize
//! the longest existing ancestor and re-attach the missing suffix --
//! semantically equivalent for prefix checks because every component of
//! the suffix is a literal name, not a symlink.

use crate::session::SessionRegistry;
use agent_client_protocol::schema::{
    ReadTextFileRequest, ReadTextFileResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Outcome of a file-op handler. The variants map to clear ACP error
/// shapes when bubbled up to the agent (see [`crate::client`]).
#[derive(Debug)]
pub enum FileOpError {
    UnknownSession,
    NotInsideCwd { resolved: PathBuf, cwd: PathBuf },
    Io(String),
}

impl fmt::Display for FileOpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSession => write!(
                f,
                "unknown session id (not registered with SessionRegistry)"
            ),
            Self::NotInsideCwd { resolved, cwd } => write!(
                f,
                "Write blocked: path outside project root ({} not under {})",
                resolved.display(),
                cwd.display()
            ),
            Self::Io(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for FileOpError {}

/// Handle a `WriteTextFileRequest`. The path is canonicalized and verified
/// to lie inside the session's cwd before any write happens. On a
/// sandbox-violation, a warning is emitted via `tracing::warn!`.
pub fn handle_write(
    request: WriteTextFileRequest,
    sessions: &SessionRegistry,
) -> Result<WriteTextFileResponse, FileOpError> {
    let cwd = sessions
        .cwd(&request.session_id)
        .ok_or(FileOpError::UnknownSession)?;
    let resolved = resolve_inside_cwd(&cwd, &request.path)?;
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            FileOpError::Io(format!("create_dir_all {} failed: {e}", parent.display()))
        })?;
    }
    std::fs::write(&resolved, request.content.as_bytes())
        .map_err(|e| FileOpError::Io(format!("write {} failed: {e}", resolved.display())))?;
    tracing::debug!(
        target: "paneflow_acp::file_ops",
        path = %resolved.display(),
        bytes = request.content.len(),
        "WriteTextFile applied",
    );
    Ok(WriteTextFileResponse::new())
}

/// Handle a `ReadTextFileRequest`. Same sandboxing as
/// [`handle_write`]. Honors the optional `line` (1-based) and `limit`
/// parameters by selecting a contiguous slice of `\n`-separated lines.
pub fn handle_read(
    request: ReadTextFileRequest,
    sessions: &SessionRegistry,
) -> Result<ReadTextFileResponse, FileOpError> {
    let cwd = sessions
        .cwd(&request.session_id)
        .ok_or(FileOpError::UnknownSession)?;
    let resolved = resolve_inside_cwd(&cwd, &request.path)?;
    let raw = std::fs::read_to_string(&resolved)
        .map_err(|e| FileOpError::Io(format!("read {} failed: {e}", resolved.display())))?;
    let content = slice_lines(&raw, request.line, request.limit);
    Ok(ReadTextFileResponse::new(content))
}

/// Resolve `path` against `cwd` and verify the result is inside `cwd`
/// after canonicalization. Returns the canonicalized absolute path on
/// success.
///
/// Errors:
/// - [`FileOpError::Io`] if `cwd` itself does not exist or cannot be
///   canonicalized.
/// - [`FileOpError::NotInsideCwd`] if the resolved path escapes `cwd`
///   (logged at WARN so an operator can see the rejection).
pub(crate) fn resolve_inside_cwd(cwd: &Path, path: &Path) -> Result<PathBuf, FileOpError> {
    let canonical_cwd = std::fs::canonicalize(cwd)
        .map_err(|e| FileOpError::Io(format!("canonicalize cwd {}: {e}", cwd.display())))?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let canonical = canonicalize_with_missing_suffix(&candidate)?;
    if !canonical.starts_with(&canonical_cwd) {
        tracing::warn!(
            target: "paneflow_acp::file_ops",
            requested = %candidate.display(),
            resolved = %canonical.display(),
            cwd = %canonical_cwd.display(),
            "Write blocked: path outside project root",
        );
        return Err(FileOpError::NotInsideCwd {
            resolved: canonical,
            cwd: canonical_cwd,
        });
    }
    Ok(canonical)
}

/// Canonicalize the longest existing ancestor of `path` and re-attach the
/// missing suffix. This lets us prefix-check destinations that do not yet
/// exist (the common "first write" case) while still resolving any
/// symlinks present in existing ancestors.
fn canonicalize_with_missing_suffix(path: &Path) -> Result<PathBuf, FileOpError> {
    if let Ok(c) = std::fs::canonicalize(path) {
        return Ok(c);
    }
    let mut suffix: Vec<Component<'_>> = Vec::new();
    let mut cursor: &Path = path;
    loop {
        let parent = cursor.parent().ok_or_else(|| {
            FileOpError::Io(format!("no existing ancestor for path {}", path.display()))
        })?;
        if let Some(name) = cursor.file_name() {
            // We push the *name* component; `Component` borrows from
            // `path`, so we collect names via `OsStr` first and convert
            // back to `PathBuf` segments outside the loop.
            suffix.push(Component::Normal(name));
        }
        if parent.exists() {
            let mut canonical = std::fs::canonicalize(parent)
                .map_err(|e| FileOpError::Io(format!("canonicalize {}: {e}", parent.display())))?;
            for c in suffix.into_iter().rev() {
                canonical.push(c.as_os_str());
            }
            return Ok(canonical);
        }
        cursor = parent;
    }
}

fn slice_lines(content: &str, line: Option<u32>, limit: Option<u32>) -> String {
    if line.is_none() && limit.is_none() {
        return content.to_string();
    }
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    let max = limit.map(|l| l as usize).unwrap_or(usize::MAX);
    content
        .lines()
        .skip(start)
        .take(max)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::SessionId;
    use std::fs;

    fn session_id(s: &str) -> SessionId {
        SessionId::from(s.to_string())
    }

    fn setup_session(dir: &Path) -> (SessionRegistry, SessionId) {
        let reg = SessionRegistry::new();
        let id = session_id("sess-fs");
        reg.register(id.clone(), dir.to_path_buf());
        (reg, id)
    }

    #[test]
    fn write_inside_cwd_succeeds() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(tmp.path());
        let req = WriteTextFileRequest::new(id, PathBuf::from("hello.txt"), "hi");
        let resp = handle_write(req, &reg).expect("write should succeed");
        assert!(resp.meta.is_none());
        let on_disk = fs::read_to_string(tmp.path().join("hello.txt")).expect("read");
        assert_eq!(on_disk, "hi");
    }

    #[test]
    fn write_outside_cwd_is_rejected() {
        let project = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(project.path());
        let req = WriteTextFileRequest::new(id, outside.path().join("evil.txt"), "no");
        let err = handle_write(req, &reg).expect_err("must reject");
        match err {
            FileOpError::NotInsideCwd { resolved, cwd } => {
                assert!(
                    !resolved.starts_with(&cwd),
                    "rejected path should be outside cwd: {} vs {}",
                    resolved.display(),
                    cwd.display(),
                );
            }
            other => panic!("expected NotInsideCwd, got {other:?}"),
        }
        // The evil path must not have been created.
        assert!(!outside.path().join("evil.txt").exists());
    }

    #[test]
    fn write_via_symlink_escape_is_rejected() {
        // Symlink test only meaningful on Unix; gate accordingly.
        #[cfg(unix)]
        {
            let project = tempfile::tempdir().expect("tempdir");
            let outside = tempfile::tempdir().expect("tempdir");
            // Create symlink inside project pointing outside.
            let link = project.path().join("escape");
            std::os::unix::fs::symlink(outside.path(), &link).expect("symlink");
            let (reg, id) = setup_session(project.path());
            let req = WriteTextFileRequest::new(id, PathBuf::from("escape/evil.txt"), "no");
            let err = handle_write(req, &reg).expect_err("symlink escape must reject");
            assert!(matches!(err, FileOpError::NotInsideCwd { .. }));
        }
    }

    #[test]
    fn write_unknown_session_returns_unknown_session() {
        let reg = SessionRegistry::new();
        let req = WriteTextFileRequest::new(session_id("ghost"), PathBuf::from("x.txt"), "");
        let err = handle_write(req, &reg).expect_err("must reject unknown session");
        assert!(matches!(err, FileOpError::UnknownSession));
    }

    #[test]
    fn read_existing_inside_cwd_returns_content() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(tmp.path());
        fs::write(tmp.path().join("hello.txt"), "hello world").expect("seed");
        let req = ReadTextFileRequest::new(id, PathBuf::from("hello.txt"));
        let resp = handle_read(req, &reg).expect("read should succeed");
        assert_eq!(resp.content, "hello world");
    }

    #[test]
    fn read_missing_file_returns_io_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(tmp.path());
        let req = ReadTextFileRequest::new(id, PathBuf::from("missing.txt"));
        let err = handle_read(req, &reg).expect_err("missing file must error");
        assert!(matches!(err, FileOpError::Io(_)), "got {err:?}");
    }

    #[test]
    fn read_with_line_and_limit_returns_slice() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(tmp.path());
        fs::write(tmp.path().join("multi.txt"), "a\nb\nc\nd\ne").expect("seed");
        let req = ReadTextFileRequest::new(id, PathBuf::from("multi.txt"))
            .line(2u32)
            .limit(2u32);
        let resp = handle_read(req, &reg).expect("read");
        assert_eq!(resp.content, "b\nc");
    }
}
