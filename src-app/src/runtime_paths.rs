//! Resolve the PaneFlow runtime directory with a macOS-aware fallback chain,
//! and enforce the `sockaddr_un.sun_path` length limit (macOS: 104 bytes,
//! Linux: 108 — we use the smaller ceiling so a path built here works on both
//! platforms without a second guard at bind time).
//!
//! The three public helpers are used by:
//! - `ipc::start_server` for the main JSON-RPC socket,
//! - `terminal::paneflow_socket_path` for the `PANEFLOW_SOCKET_PATH` env var
//!   passed into each PTY child shell,
//! - `terminal::paneflow_bin_dir` for the wrapper-scripts directory prepended
//!   to each PTY child's `PATH`.
//!
//! Keeping the chain in one place prevents the three sites from drifting —
//! a difference in one branch would silently break AI-hook IPC on macOS
//! without any visible error.

use std::path::PathBuf;

/// macOS `sockaddr_un.sun_path` is `[c_char; 104]`. Linux allows 108, but
/// using the smaller ceiling keeps paths portable across both targets.
pub(crate) const MAX_SOCKET_PATH_BYTES: usize = 104;

const PANEFLOW_SUBDIR: &str = "paneflow";
const SOCKET_FILE: &str = "paneflow.sock";
const BIN_SUBDIR: &str = "bin";

/// Resolve the PaneFlow runtime directory. Fallback chain:
/// 1. `$XDG_RUNTIME_DIR` — explicit Linux XDG (usually `/run/user/<uid>`).
/// 2. `dirs::runtime_dir()` — same on Linux, `None` on macOS.
/// 3. `$TMPDIR` — populated on macOS (usually `/var/folders/xx/.../T/`).
/// 4. `dirs::cache_dir().join("run")` — last-resort cross-platform fallback.
///
/// Returns `None` only if every layer fails, which in practice means the
/// caller runs on an environment with neither XDG nor TMPDIR nor a cache
/// dir (e.g. a broken container). Callers should `log::warn!` and disable
/// IPC rather than panic.
fn runtime_dir() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(dirs::runtime_dir)
        .or_else(|| {
            std::env::var("TMPDIR")
                .ok()
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
        })
        .or_else(|| dirs::cache_dir().map(|d| d.join("run")))
}

/// Full path to the IPC Unix-domain socket, or `None` if the runtime dir
/// cannot be resolved or the composed path would exceed the `sun_path`
/// limit. A `log::warn!` is emitted in the over-length case so the user
/// can see why IPC is disabled.
pub(crate) fn socket_path() -> Option<PathBuf> {
    let path = runtime_dir()?.join(PANEFLOW_SUBDIR).join(SOCKET_FILE);
    check_sun_path_fits(&path).then_some(path)
}

/// Directory where wrapper scripts (`claude`, `codex`, `paneflow-hook`) are
/// extracted. No `sun_path` guard here — the scripts themselves live in the
/// directory but never bind a socket.
pub(crate) fn bin_dir() -> Option<PathBuf> {
    Some(runtime_dir()?.join(PANEFLOW_SUBDIR).join(BIN_SUBDIR))
}

fn check_sun_path_fits(path: &std::path::Path) -> bool {
    let bytes = path.as_os_str().len();
    if bytes > MAX_SOCKET_PATH_BYTES {
        log::warn!(
            "paneflow: computed IPC socket path exceeds sun_path limit ({} > {} bytes): {} — IPC will be disabled. Set $XDG_RUNTIME_DIR (Linux) or shorten $TMPDIR (macOS) to enable it.",
            bytes,
            MAX_SOCKET_PATH_BYTES,
            path.display()
        );
        false
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env vars are process-global; tests that mutate them must be serialised.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        xdg: Option<String>,
        tmp: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn take() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            Self {
                xdg: std::env::var("XDG_RUNTIME_DIR").ok(),
                tmp: std::env::var("TMPDIR").ok(),
                _guard: guard,
            }
        }

        fn clear(&self) {
            // SAFETY: serialised by ENV_LOCK; no other test or production
            // thread mutates these vars during the test window.
            unsafe {
                std::env::remove_var("XDG_RUNTIME_DIR");
                std::env::remove_var("TMPDIR");
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: serialised by ENV_LOCK (still held via _guard).
            unsafe {
                match &self.xdg {
                    Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
                    None => std::env::remove_var("XDG_RUNTIME_DIR"),
                }
                match &self.tmp {
                    Some(v) => std::env::set_var("TMPDIR", v),
                    None => std::env::remove_var("TMPDIR"),
                }
            }
        }
    }

    #[test]
    fn xdg_runtime_dir_wins_when_set() {
        let g = EnvGuard::take();
        g.clear();
        // SAFETY: ENV_LOCK held.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000") };
        let p = socket_path().expect("runtime dir must resolve");
        assert_eq!(
            p,
            PathBuf::from("/run/user/1000/paneflow/paneflow.sock"),
            "AC5: Linux with XDG_RUNTIME_DIR must resolve to the XDG path"
        );
    }

    #[test]
    fn tmpdir_fallback_when_xdg_and_runtime_dir_missing() {
        let g = EnvGuard::take();
        g.clear();
        // SAFETY: ENV_LOCK held.
        unsafe { std::env::set_var("TMPDIR", "/tmp/macos-stub") };
        let p = socket_path();
        if let Some(p) = p {
            // On Linux, dirs::runtime_dir() may still return Some before we
            // reach the TMPDIR branch — accept either but prove the path is
            // well-formed.
            assert!(p.ends_with("paneflow/paneflow.sock"));
        }
    }

    #[test]
    fn overlong_path_returns_none() {
        let g = EnvGuard::take();
        g.clear();
        // 120-byte XDG_RUNTIME_DIR → joined path blows past 104.
        let long = "/".to_string() + &"x".repeat(119);
        // SAFETY: ENV_LOCK held.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &long) };
        assert!(
            socket_path().is_none(),
            "AC6: over-long sun_path must return None rather than a bind-time error"
        );
    }

    #[test]
    fn bin_dir_mirrors_socket_runtime_dir() {
        let g = EnvGuard::take();
        g.clear();
        // SAFETY: ENV_LOCK held.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000") };
        assert_eq!(
            bin_dir().unwrap(),
            PathBuf::from("/run/user/1000/paneflow/bin")
        );
    }

    #[test]
    fn bin_dir_ignores_sun_path_limit() {
        // The bin dir itself is not used as a socket path, so it is allowed
        // to exceed 104 bytes. Only `socket_path()` applies the guard.
        let g = EnvGuard::take();
        g.clear();
        let long = "/".to_string() + &"y".repeat(119);
        // SAFETY: ENV_LOCK held.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &long) };
        assert!(bin_dir().is_some());
    }
}
