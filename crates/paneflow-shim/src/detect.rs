//! Tool detection + real-binary location (US-052 split).

use std::env;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Tool detection - from `current_exe()` filename stem
// ---------------------------------------------------------------------------

/// Read `std::env::current_exe()` and return the tool identity, or `None`
/// if the shim was invoked under an unexpected name (direct `paneflow-shim`
/// invocation, custom rename, etc.).
pub(crate) fn detect_tool() -> Option<&'static str> {
    let exe = env::current_exe().ok()?;
    let stem = exe.file_stem()?.to_str()?;
    detect_tool_from_stem(stem)
}

/// Every binary name the US-008 extractor may materialize this shim under -
/// the wire-format tool identity IS the binary name. MUST stay in sync with
/// `TerminalAgent::binary()` in `src-app/src/agent_launcher.rs` (the shim
/// stays dependency-free of the app crate, so the list is mirrored here;
/// the app-side `wrapped_stems_match_shim_detect_list` test pins the
/// source list so any drift fails the build over there).
pub(crate) const WRAPPED_TOOLS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "pi",
    "hermes",
    "grok",
    "amp",
    "cursor-agent",
    "gemini",
    "kiro-cli",
    "agy",
    "copilot",
    "codebuddy",
    "droid",
    "qodercli",
    "openclaw",
];

/// Testable inner: map a filename stem to the tool identity. Only exact
/// lowercase matches against [`WRAPPED_TOOLS`] are accepted - US-008
/// controls the extracted filenames, so anything else here means the
/// binary has been renamed or invoked directly.
pub(crate) fn detect_tool_from_stem(stem: &str) -> Option<&'static str> {
    WRAPPED_TOOLS.iter().find(|t| **t == stem).copied()
}

// ---------------------------------------------------------------------------
// PATH walk - find the real AI binary, excluding the shim's own directory
// ---------------------------------------------------------------------------

/// Candidate executable names to probe in each `$PATH` entry. Unix looks for
/// a bare filename; Windows tries `.exe` first, then `.cmd` (covers both
/// native AI builds and Node-shipped wrappers like `claude.cmd`).
#[cfg(unix)]
pub(crate) fn candidate_names(tool: &str) -> Vec<String> {
    vec![tool.to_owned()]
}

#[cfg(windows)]
pub(crate) fn candidate_names(tool: &str) -> Vec<String> {
    vec![format!("{tool}.exe"), format!("{tool}.cmd")]
}

/// Walk `$PATH` and return the first entry that contains a matching
/// executable, skipping the shim's own directory AND any candidate
/// that is the shim binary itself by inode (US-017 hardlink defense).
pub(crate) fn find_real_binary(tool: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    // Per `install_method.rs:92-98`, always canonicalize `current_exe()` -
    // on Linux it may point at `/proc/self/exe` or follow through a symlink.
    let self_exe = env::current_exe().ok();
    let self_dir = self_exe
        .as_deref()
        .and_then(|p| p.parent().map(Path::to_path_buf));

    find_real_binary_in(
        tool,
        env::split_paths(&path_var),
        self_dir.as_deref(),
        self_exe.as_deref(),
    )
}

/// Pure inner - takes PATH entries as an iterator and optional
/// self-dir / self-exe paths, so tests can pass a controlled set
/// without mutating `$PATH` or relying on `current_exe()`.
pub(crate) fn find_real_binary_in<I>(
    tool: &str,
    path_entries: I,
    self_dir: Option<&Path>,
    self_exe: Option<&Path>,
) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    // Canonicalize once; `None` if the dir doesn't exist (in which case we
    // can't match anything against it, so the self-exclusion is a no-op and
    // PATH is walked in full - safer than silently skipping nothing).
    let self_canon = self_dir.and_then(|d| std::fs::canonicalize(d).ok());
    // US-017 (cli-hardening-followup-2026-Q3): capture the shim's
    // own file identity. A candidate matching
    // this identity is the shim itself reached via a hardlink and
    // must be skipped to break the recursive-spawn loop.
    let self_identity = self_exe.and_then(file_identity);
    let candidates = candidate_names(tool);

    for dir in path_entries {
        if same_canonical_dir(&self_canon, &dir) {
            continue;
        }
        for name in &candidates {
            let candidate = dir.join(name);
            if !candidate.is_file() {
                continue;
            }
            // US-037: on Unix, require the executable bit too. A non-executable
            // homonym (e.g. a `0644` data file named like the tool) earlier in
            // `$PATH` would otherwise be returned, and the subsequent spawn
            // fails `EACCES`/`ENOEXEC` *without* continuing the walk (unlike
            // `execvp`, which skips it). Skip it here so the real binary later
            // in `$PATH` is found.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = std::fs::metadata(&candidate)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false);
                if !executable {
                    continue;
                }
            }
            if is_same_file_as_shim(&self_identity, &candidate) {
                // US-017: hardlink-loop guard. The shim has no `log`
                // dep (would bloat the small binary); stderr is
                // already the diagnostic channel used elsewhere
                // in this binary (cf. line ~287).
                eprintln!(
                    "paneflow-shim: skipping {} -- matches shim identity (hardlink loop guard)",
                    candidate.display()
                );
                continue;
            }
            return Some(candidate);
        }
    }
    None
}

/// US-017 (cli-hardening-followup-2026-Q3): capture a file's
/// cross-platform identity. `same_file::Handle` uses the native
/// file identity on Unix and Windows, so hardlinks compare equal
/// without relying on unstable standard-library APIs.
///
/// Returns `None` when the path cannot be opened, in which case
/// the comparison degrades to a no-op and the dir-canonicalize
/// path remains the residual defense.
pub(crate) fn file_identity(path: &Path) -> Option<same_file::Handle> {
    same_file::Handle::from_path(path).ok()
}

pub(crate) fn is_same_file_as_shim(
    self_identity: &Option<same_file::Handle>,
    candidate: &Path,
) -> bool {
    match (self_identity.as_ref(), file_identity(candidate)) {
        (Some(a), Some(b)) => a == &b,
        _ => false,
    }
}

/// Returns `true` when `dir` canonicalizes to the same path as `self_canon`.
/// Handles symlinks, trailing slashes, and `..` segments that would otherwise
/// make two string-equal paths compare as different and vice versa.
///
/// This stays as the cheap fast-path filter that skips the shim's
/// own install directory wholesale (saves one `metadata` syscall
/// per candidate when the dir matches by canonicalization). The
/// inode-level defense against a hardlinked shim planted in a
/// different `$PATH` directory lives in [`is_same_file_as_shim`]
/// and runs for every candidate that survives the dir filter
/// (US-017 cli-hardening-followup-2026-Q3).
pub(crate) fn same_canonical_dir(self_canon: &Option<PathBuf>, dir: &Path) -> bool {
    match (self_canon, std::fs::canonicalize(dir).ok()) {
        (Some(s), Some(d)) => *s == d,
        _ => false,
    }
}
