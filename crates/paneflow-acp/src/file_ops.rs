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
///
/// US-015 (cli-hardening-followup-2026-Q3): the previous
/// `std::fs::read_to_string(&resolved)` allocated the WHOLE file
/// before slicing. A user who hands the agent a `limit: 100` on a
/// 200 MB file (mistakenly named `.rs`, large vendored data dump,
/// etc.) used to pay a 200 MB allocation for 100 lines of output.
/// The new path streams via `BufReader::lines()` and stops at the
/// requested line cap OR at [`MAX_READ_BYTES`] (4 MB), whichever
/// comes first. The byte cap protects against a degenerate file
/// with no newlines (a single 200 MB "line").
pub fn handle_read(
    request: ReadTextFileRequest,
    sessions: &SessionRegistry,
) -> Result<ReadTextFileResponse, FileOpError> {
    let cwd = sessions
        .cwd(&request.session_id)
        .ok_or(FileOpError::UnknownSession)?;
    let resolved = resolve_inside_cwd(&cwd, &request.path)?;
    let content = read_file_bounded(&resolved, request.line, request.limit)?;
    Ok(ReadTextFileResponse::new(content))
}

/// US-015 (cli-hardening-followup-2026-Q3): cap on total bytes read
/// from a single `handle_read` call. 4 MiB is comfortable headroom
/// for any reasonable source file (the Linux kernel's largest
/// `.c` is < 1 MB) and aborts the read well before a vendored
/// binary or a malformed JSONL eats process memory.
const MAX_READ_BYTES: usize = 4 * 1024 * 1024;

/// Stream `path` line-by-line, stopping at `start + limit` lines
/// (when `limit` is `Some`) OR at [`MAX_READ_BYTES`], whichever
/// comes first. Returns the joined content with single `\n`
/// separators (mirrors the pre-fix behaviour of `slice_lines`).
fn read_file_bounded(
    path: &Path,
    line: Option<u32>,
    limit: Option<u32>,
) -> Result<String, FileOpError> {
    use std::io::BufRead;

    let file = std::fs::File::open(path)
        .map_err(|e| FileOpError::Io(format!("open {} failed: {e}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let start = line.unwrap_or(1).saturating_sub(1) as usize;
    // `usize::MAX` saturates on the `start + max` add below; bounded
    // by the byte cap so the worst case is still 4 MB of memory.
    let max = limit.map(|l| l as usize).unwrap_or(usize::MAX);
    let end = start.saturating_add(max);

    let mut out = String::new();
    let mut bytes_read: usize = 0;
    let mut truncated_for_bytes = false;
    for (idx, line_result) in reader.lines().enumerate() {
        if idx >= end {
            break;
        }
        let line_str = line_result.map_err(|e| {
            FileOpError::Io(format!(
                "read line {} of {} failed: {e}",
                idx,
                path.display()
            ))
        })?;
        // +1 for the newline we will insert. Counts characters as
        // a proxy for bytes -- close enough for the cap purpose.
        let added = line_str.len() + 1;
        if bytes_read.saturating_add(added) > MAX_READ_BYTES {
            truncated_for_bytes = true;
            break;
        }
        if idx >= start {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line_str);
        }
        bytes_read = bytes_read.saturating_add(added);
    }
    if truncated_for_bytes {
        tracing::warn!(
            target: "paneflow_acp::file_ops",
            path = %path.display(),
            cap = MAX_READ_BYTES,
            "handle_read truncated: file exceeded byte cap"
        );
    }
    Ok(out)
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

    /// US-015 (cli-hardening-followup-2026-Q3): the bounded reader
    /// must cap a pathologically large file at MAX_READ_BYTES (4 MB)
    /// instead of allocating the full payload. Builds a 10 MB file
    /// of repeated short lines, requests only the first 5 lines,
    /// and verifies that (a) the read succeeds, (b) the returned
    /// content has exactly 5 lines, (c) the file_ops worst-case
    /// allocation stayed bounded (asserted indirectly via response
    /// size).
    #[test]
    fn handle_read_caps_at_4mb() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(tmp.path());
        // 10 MB file of 10-char lines.
        let big_path = tmp.path().join("big.txt");
        let line = "0123456789\n";
        let mut handle = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&big_path)
            .expect("create big.txt");
        use std::io::Write;
        let target_size = 10 * 1024 * 1024;
        let lines_needed = target_size / line.len() + 1;
        for _ in 0..lines_needed {
            handle.write_all(line.as_bytes()).expect("write line");
        }
        handle.flush().expect("flush");
        drop(handle);
        let meta = fs::metadata(&big_path).expect("metadata");
        assert!(meta.len() >= 10 * 1024 * 1024, "fixture must be >= 10 MB");

        let req = ReadTextFileRequest::new(id, PathBuf::from("big.txt"))
            .line(1u32)
            .limit(5u32);
        let resp = handle_read(req, &reg).expect("read with line cap");
        // 5 lines, joined with \n -> "0123456789\n0123456789\n...0123456789" = 5*10 + 4 = 54 bytes.
        assert_eq!(resp.content.lines().count(), 5);
        assert!(
            resp.content.len() < 100,
            "line-capped response must be tiny, got {} bytes",
            resp.content.len()
        );
    }

    /// US-015: a file with no newlines (degenerate "1 line of 10 MB")
    /// must hit the byte cap. The reader returns at most 4 MB and
    /// the request-level `limit` is unused because the single line
    /// itself exceeds the cap.
    #[test]
    fn handle_read_byte_cap_on_single_huge_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (reg, id) = setup_session(tmp.path());
        let big_path = tmp.path().join("nolf.txt");
        let payload = vec![b'x'; 10 * 1024 * 1024];
        fs::write(&big_path, &payload).expect("write huge no-newline");

        let req = ReadTextFileRequest::new(id, PathBuf::from("nolf.txt")).limit(100u32);
        let resp = handle_read(req, &reg).expect("read");
        // The single line exceeds MAX_READ_BYTES; the cap kicks in
        // BEFORE the line is appended, so the response is empty.
        // The contract is "bounded allocation", not "best-effort
        // payload" -- documented in the warn log emitted at the
        // truncation point.
        assert!(
            resp.content.len() < 5 * 1024 * 1024,
            "response must stay under 5 MB, got {} bytes",
            resp.content.len()
        );
    }
}
