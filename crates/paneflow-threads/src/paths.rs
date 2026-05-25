//! Cross-platform default DB path resolution.
//!
//! Per US-006 AC: the DB lives at `~/.local/share/paneflow/threads/threads.db`
//! on Linux, with platform-appropriate equivalents on macOS and Windows.
//! We use [`dirs::data_local_dir`] (not `data_dir`) so on Windows the path
//! is `%LOCALAPPDATA%\paneflow\threads\threads.db` rather than the roamed
//! `%APPDATA%` -- thread blobs can be large and should not chase the user
//! across machines.

use std::path::PathBuf;

/// Canonical Paneflow threads-DB path. Returns `None` if the platform
/// helper cannot resolve a local data directory (broken environment).
/// Callers that need a guaranteed path should error out and tell the
/// user to set the right HOME-style env var; the agents view falls back
/// to disabling persistence in that scenario.
pub fn default_db_path() -> Option<PathBuf> {
    let base = dirs::data_local_dir()?;
    Some(base.join("paneflow").join("threads").join("threads.db"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_ends_in_threads_db() {
        // On hermetic CI the env may or may not provide `dirs`; we only
        // assert the *shape* if a path resolves.
        if let Some(path) = default_db_path() {
            let s = path.to_string_lossy();
            assert!(
                s.ends_with("paneflow/threads/threads.db")
                    || s.ends_with("paneflow\\threads\\threads.db"),
                "unexpected default path shape: {s}"
            );
        }
    }
}
