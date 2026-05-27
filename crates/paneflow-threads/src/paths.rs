//! Cross-platform default DB path resolution.
//!
//! Per US-006 AC: the DB lives at `~/.local/share/paneflow/threads/threads.db`
//! on Linux, with platform-appropriate equivalents on macOS and Windows.
//! We use [`dirs::data_local_dir`] (not `data_dir`) so on Windows the path
//! is `%LOCALAPPDATA%\paneflow\threads\threads.db` rather than the roamed
//! `%APPDATA%` -- thread blobs can be large and should not chase the user
//! across machines.

use std::path::PathBuf;

/// Application directory namespace. Switches to `paneflow-dev` in debug
/// builds so dev-from-source instances never read/write the same
/// `threads.db` as the user's installed `/usr/bin/paneflow` (SQLite
/// would otherwise serialize access and leak threads between the two
/// instances). Mirrors `paneflow_app::runtime_paths::APP_SUBDIR` and
/// `paneflow_config::APP_SUBDIR`.
pub const APP_SUBDIR: &str = if cfg!(debug_assertions) {
    "paneflow-dev"
} else {
    "paneflow"
};

/// Canonical Paneflow threads-DB path. Returns `None` if the platform
/// helper cannot resolve a local data directory (broken environment).
/// Callers that need a guaranteed path should error out and tell the
/// user to set the right HOME-style env var; the agents view falls back
/// to disabling persistence in that scenario.
pub fn default_db_path() -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    Some(base.join(APP_SUBDIR).join("threads").join("threads.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_ends_in_threads_db() {
        // On hermetic CI the env may or may not provide `dirs`; we only
        // assert the *shape* if a path resolves. Both release and debug
        // builds end in `<subdir>/threads/threads.db`; the subdir is
        // either `paneflow` or `paneflow-dev` depending on the build
        // profile this test was compiled under.
        if let Some(path) = default_db_path() {
            let s = path.to_string_lossy();
            let suffix_unix = format!("{APP_SUBDIR}/threads/threads.db");
            let suffix_win = format!("{APP_SUBDIR}\\threads\\threads.db");
            assert!(
                s.ends_with(&suffix_unix) || s.ends_with(&suffix_win),
                "unexpected default path shape: {s} (expected to end with {suffix_unix})"
            );
        }
    }
}
