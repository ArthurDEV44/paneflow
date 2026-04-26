#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::unwrap_in_result,
        clippy::panic
    )
)]
//! PaneFlow AI-binary shim.
//!
//! Copied or hardlinked (by US-008 extraction) as `claude` and `codex` into
//! the PaneFlow bin cache dir, which US-009 prepends to the PTY's `$PATH`.
//! When the user runs `claude` or `codex`, this shim:
//!
//! 1. Reads its own filename via `current_exe()` to decide which tool to
//!    front for (`detect_tool`).
//! 2. PATH-walks `$PATH`, **excluding its own directory**, to locate the
//!    real AI binary (`find_real_binary`). Self-exclusion prevents an
//!    infinite exec-loop when the shim dir is first on `$PATH`.
//! 3. Execs the real binary with argv and env passed through. On Unix,
//!    uses the `exec()` syscall for zero-fork process replacement; on
//!    Windows, spawns a child and propagates the exit code.
//!
//! US-004 scope: detect / find / exec only. Hook config injection
//! (`.claude/settings.local.json` via US-005; `.codex/hooks.json` via
//! US-006) and env-var injection (`$PANEFLOW_AI_TOOL` / `$PANEFLOW_AI_PID`
//! for US-003 consumption) are added in later stories by wrapping around
//! this skeleton.

use std::env;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

// ---------------------------------------------------------------------------
// Tool detection — from `current_exe()` filename stem
// ---------------------------------------------------------------------------

/// Read `std::env::current_exe()` and return the tool identity, or `None`
/// if the shim was invoked under an unexpected name (direct `paneflow-shim`
/// invocation, custom rename, etc.).
fn detect_tool() -> Option<&'static str> {
    let exe = env::current_exe().ok()?;
    let stem = exe.file_stem()?.to_str()?;
    detect_tool_from_stem(stem)
}

/// Testable inner: map a filename stem to the tool identity. Only the two
/// exact lowercase matches are accepted — US-008 controls the extracted
/// filenames, so anything else here means the binary has been renamed or
/// invoked directly.
fn detect_tool_from_stem(stem: &str) -> Option<&'static str> {
    match stem {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// PATH walk — find the real AI binary, excluding the shim's own directory
// ---------------------------------------------------------------------------

/// Candidate executable names to probe in each `$PATH` entry. Unix looks for
/// a bare filename; Windows tries `.exe` first, then `.cmd` (covers both
/// native AI builds and Node-shipped wrappers like `claude.cmd`).
#[cfg(unix)]
fn candidate_names(tool: &str) -> Vec<String> {
    vec![tool.to_owned()]
}

#[cfg(windows)]
fn candidate_names(tool: &str) -> Vec<String> {
    vec![format!("{tool}.exe"), format!("{tool}.cmd")]
}

/// Walk `$PATH` and return the first entry that contains a matching
/// executable, skipping the shim's own directory so we don't exec ourselves.
fn find_real_binary(tool: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    // Per `install_method.rs:92-98`, always canonicalize `current_exe()` —
    // on Linux it may point at `/proc/self/exe` or follow through a symlink.
    let self_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));

    find_real_binary_in(tool, env::split_paths(&path_var), self_dir.as_deref())
}

/// Pure inner — takes PATH entries as an iterator and an optional self dir,
/// so tests can pass a controlled set without mutating `$PATH` or relying
/// on `current_exe()`.
fn find_real_binary_in<I>(tool: &str, path_entries: I, self_dir: Option<&Path>) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    // Canonicalize once; `None` if the dir doesn't exist (in which case we
    // can't match anything against it, so the self-exclusion is a no-op and
    // PATH is walked in full — safer than silently skipping nothing).
    let self_canon = self_dir.and_then(|d| std::fs::canonicalize(d).ok());
    let candidates = candidate_names(tool);

    for dir in path_entries {
        if same_canonical_dir(&self_canon, &dir) {
            continue;
        }
        for name in &candidates {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Returns `true` when `dir` canonicalizes to the same path as `self_canon`.
/// Handles symlinks, trailing slashes, and `..` segments that would otherwise
/// make two string-equal paths compare as different and vice versa.
///
/// Known limitation (Phase 7 security audit, MEDIUM): this compares
/// directories, not inodes. An attacker with write access to a PATH
/// directory earlier than the shim's own dir (requires root for
/// `/usr/local/bin` etc.) could plant a *hardlink* of the shim there
/// named `claude` or `codex`. Running that hardlink would pass dir-based
/// self-exclusion on each hop and recurse indefinitely. Practical
/// exploitability is low: an attacker with that write access could plant
/// a real malicious binary instead, which is strictly simpler. A future
/// hardening story can tighten this with a Unix `(dev, ino)` inode
/// comparison via `std::os::unix::fs::MetadataExt`; out of US-004 AC scope.
fn same_canonical_dir(self_canon: &Option<PathBuf>, dir: &Path) -> bool {
    match (self_canon, std::fs::canonicalize(dir).ok()) {
        (Some(s), Some(d)) => *s == d,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Run chain — spawn the real AI binary and wait for it
// ---------------------------------------------------------------------------
//
// US-004 originally used `CommandExt::exec()` on Unix for zero-fork process
// replacement. US-005 introduced the `HookConfigGuard` drop-cleanup contract,
// which is incompatible with `exec()` — process replacement skips every Rust
// destructor, so the guard would never fire. Both platforms now use
// `Command::status()`; the shim pays one fork (~1-3 ms, well under the 15 ms
// budget) in exchange for reliable cleanup.
//
// `Command` inherits the parent env by default, so `.envs(env::vars_os())`
// is redundant — but the PRD AC bullet 5 lists it explicitly to make the
// env-pass-through contract discoverable in the source. The `.env(...)`
// calls afterward shadow per-key (Command::env is last-write-wins).
//
// PANEFLOW_AI_TOOL — set so `paneflow-ai-hook` (US-003) can tag every
// outbound IPC frame with the right tool identity (`claude` vs `codex`).
// Without this, `paneflow-ai-hook::detect_tool_from(None)` defaults to
// `TOOL_CLAUDE`, which makes the sidebar render "Claude thinking…" for
// every Codex turn — visible regression observed in the field.

fn run_real(tool: &str, path: &Path, args: &[OsString]) -> ExitCode {
    let mut cmd = std::process::Command::new(path);
    cmd.args(args)
        .envs(env::vars_os())
        .env("PANEFLOW_AI_TOOL", tool);

    // Unix only: reset signal disposition + unblock SIGINT in the child.
    //
    // Required because Rust's `Command` inherits the parent's signal mask
    // and dispositions across `execve`. The parent installs:
    //   - `SIG_IGN` for SIGHUP/SIGTERM (shim survives PTY close / kill)
    //   - `SIG_BLOCK` mask for SIGINT (consumed synchronously by the
    //     `sigwait` thread in `install_sigint_watcher`, so the shim can
    //     emit an `ai.stop` IPC frame on every Ctrl+C — including
    //     mid-response interrupts where claude/codex intentionally fire
    //     no `Stop` hook of their own).
    //
    // Without this `pre_exec` reset+unblock, the child would inherit both
    // and Ctrl+C would do absolutely nothing (the AI would never see it,
    // since `SIG_BLOCK`'d signals on a Linux process stay blocked across
    // `execve`).
    //
    // `pre_exec` runs in the forked child between fork() and execve(). All
    // calls below are async-signal-safe.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::sigaddset(&mut set, libc::SIGHUP);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            Ok(())
        });
    }

    // Install signal isolation BEFORE spawn so the child inherits the
    // mask/dispositions at fork (then `pre_exec` flips them back for the
    // child only). Doing this BEFORE `cmd.spawn()` closes the race window
    // where a Ctrl+C could land between spawn and signal-install.
    #[cfg(unix)]
    ignore_terminal_signals();
    #[cfg(unix)]
    install_sigint_watcher(tool);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("paneflow-shim: spawn '{}' failed: {e}", path.display());
            return ExitCode::from(127);
        }
    };

    match child.wait() {
        Ok(status) => {
            // `status.code()` is `None` only when the child was terminated
            // by a signal (Unix). The bash convention is `128 + signum`;
            // we can't faithfully reproduce that portably, so `1` is the
            // sentinel. u8::try_from rejects negative / out-of-range codes.
            let raw = status.code().unwrap_or(1);
            let byte = u8::try_from(raw).unwrap_or(1);
            ExitCode::from(byte)
        }
        Err(e) => {
            eprintln!("paneflow-shim: wait on '{}' failed: {e}", path.display());
            ExitCode::from(1)
        }
    }
}

/// Make the shim survive PTY-close + kill signals so the child
/// (claude/codex) can handle them without taking us down with it.
///
/// SIGINT is intentionally NOT in this list — it's handled by
/// `install_sigint_watcher` via `sigwait` so we can emit a per-interrupt
/// `ai.stop` IPC frame (mid-response Ctrl+C interrupts fire no hook from
/// claude/codex, so this is the only signal we have).
#[cfg(unix)]
fn ignore_terminal_signals() {
    // SAFETY: `libc::signal` with `SIG_IGN` is async-signal-safe and only
    // mutates the kernel signal disposition table for the current process.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
}

/// Block SIGINT in the shim, then spawn a dedicated thread that
/// `sigwait`s on it. On every Ctrl+C, send `ai.stop` to PaneFlow so the
/// sidebar loader transitions to `Finished` (then auto-resets to
/// `Inactive` after 5s server-side). This is the ONLY way to detect a
/// mid-stream interrupt because:
///   - Claude Code does not fire its `Stop` hook when a turn is
///     interrupted (only on natural completion).
///   - Codex does not fire any hook on `esc`/Ctrl+C either.
///
/// `sigwait` is the POSIX-correct synchronous-from-thread receive: no
/// async-signal-safety constraints, no self-pipe trick. Standard pattern
/// (see Stevens APUE §12.8 "pthread_sigmask").
#[cfg(unix)]
fn install_sigint_watcher(tool: &str) {
    // SAFETY: `pthread_sigmask` is thread-safe and only mutates the
    // calling thread's signal mask. Blocking SIGINT here propagates to
    // every thread spawned afterward (POSIX inheritance rule). The
    // `pre_exec` hook in `run_real` re-unblocks SIGINT in the child.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGINT);
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }

    let tool = tool.to_owned();
    let hook_path = locate_sibling_hook_binary();
    std::thread::spawn(move || {
        let Some(hook_path) = hook_path else {
            return;
        };
        loop {
            // SAFETY: `sigwait` blocks the calling thread until one of the
            // signals in `set` is delivered to the process. Returns 0 on
            // success and writes the received signal into `sig`. Spurious
            // wakeups are not part of the POSIX contract; if it ever does
            // return non-zero, exit the loop (the shim continues running,
            // we just lose interrupt-driven notifications for this
            // session — graceful degradation per PRD C4).
            let sig = unsafe {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGINT);
                let mut sig: libc::c_int = 0;
                if libc::sigwait(&set, &mut sig) != 0 {
                    return;
                }
                sig
            };
            if sig == libc::SIGINT {
                send_interrupt_stop(&hook_path, &tool);
            }
        }
    });
}

/// Spawn `paneflow-ai-hook Stop` with `{}` piped to stdin. Best-effort;
/// any failure is silent (worst case: this Ctrl+C doesn't clear the
/// loader, but the shim and the child remain unaffected).
///
/// Reaping policy: the wait happens on a detached helper thread, NOT on
/// the calling sigwait thread. If the hook hangs (socket back-pressure,
/// filesystem stall) the reaper thread hangs with it — but the sigwait
/// thread stays responsive, so the next Ctrl+C lands as a fresh `ai.stop`
/// rather than queuing behind the previous one. Without the helper, a
/// dropped `Child` would leak a zombie until shim exit.
#[cfg(unix)]
fn send_interrupt_stop(hook_path: &Path, tool: &str) {
    use std::io::Write;
    let Ok(mut child) = std::process::Command::new(hook_path)
        .arg("Stop")
        .env("PANEFLOW_AI_TOOL", tool)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"{}");
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

// ---------------------------------------------------------------------------
// Hook config injection (US-005) — idempotent `.claude/settings.local.json`
// ---------------------------------------------------------------------------

/// Claude Code 2.x hook events the shim registers handlers for. `SubagentStop`
/// is intentionally omitted — the server maps it to `ai.stop` identically to
/// `Stop` (US-002), and registering both would produce duplicate IPC frames.
const CLAUDE_HOOK_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "Notification",
    "Stop",
    "PreToolUse",
    "PostToolUse",
];

/// Bare-name fallback used when `locate_sibling_hook_binary()` cannot resolve
/// the absolute path to the sibling hook binary (test harness, exotic FS,
/// `current_exe` failure). Detection (`is_paneflow_hook_command`) is
/// basename-based so this fallback is also recognized for cleanup, alongside
/// the absolute-path form normally written by `resolve_hook_command`.
///
/// Cleanup matches by basename as a belt-and-suspenders complement to the
/// `_paneflow_managed` marker: Claude Code does not guarantee unknown fields
/// survive re-serialization (anthropics/claude-code#5886), so the command
/// string is the only round-trip-stable identifier we can rely on.
///
/// Known namespace-collision limitation: any user-authored hook command whose
/// program basename is literally `paneflow-ai-hook` (or `.exe` on Windows)
/// will be treated as PaneFlow-managed and removed on cleanup. The basename
/// rule narrows this further than the previous bare-prefix rule did, but the
/// theoretical collision remains.
const HOOK_COMMAND_PREFIX: &str = "paneflow-ai-hook ";

/// Render `path` for inclusion in an `eprintln!` going to the user's
/// terminal. Replaces bytes outside the printable ASCII range (`0x20..=0x7E`)
/// with `?` to defuse ANSI-escape-sequence injection via a maliciously
/// named CWD or `.claude/` directory. Path content is never a secret — it
/// was always going to be visible on stderr — but without this scrub, a
/// crafted directory could clear the screen, set the terminal title, or
/// inject false log lines (Phase 7 security audit, MEDIUM finding).
fn safe_path_display(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '?' })
        .collect()
}

/// Returns true iff `$PANEFLOW_SOCKET_PATH` is set and points at an existing
/// filesystem entry. We treat unset/unreachable as "PaneFlow not running":
/// in that state, installing hook config is pointless (the hooks would
/// invoke `paneflow-ai-hook`, which would fail silently per PRD constraint
/// C4) and we instead sweep any orphan entries left by a previous SIGKILL'd
/// session. Existence-only check keeps this cross-platform — Unix sockets
/// and Windows named pipes both surface as `Path::exists() == true` when
/// the listener is bound.
fn paneflow_ipc_reachable() -> bool {
    let Some(raw) = env::var_os("PANEFLOW_SOCKET_PATH") else {
        return false;
    };
    if raw.is_empty() {
        return false;
    }
    Path::new(&raw).exists()
}

/// Best-effort removal of PaneFlow-managed entries from an existing hook
/// config file when no active IPC channel is reachable. Reads, runs
/// `remove_fn`, writes back (or deletes if the file is now empty). All
/// failures swallow silently — a sweep that fails just retries on the
/// next shim invocation. Used by `install()` to clean up after a previous
/// SIGKILL'd session that never got to fire its `Drop` impl.
fn sweep_orphan_hook_config(settings_path: &Path, remove_fn: fn(&mut serde_json::Value)) {
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    let before = root.clone();
    remove_fn(&mut root);
    if root == before {
        // Nothing to sweep — file has no PaneFlow entries.
        return;
    }
    let is_empty = root
        .as_object()
        .map(serde_json::Map::is_empty)
        .unwrap_or(false);
    let _ = if is_empty {
        std::fs::remove_file(settings_path)
    } else {
        write_atomic(settings_path, &root)
    };
}

/// Shared scaffold for both Claude and Codex hook config installation:
/// validate the config dir (creating it if absent), read+parse any existing
/// JSON tree (treating corrupt JSON as empty), apply `merge_fn`, and write
/// atomically. Returns `Some((settings_path, created_dir))` on success;
/// `None` when the filesystem refuses our writes or the config dir is
/// occupied by a non-directory.
///
/// Both `HookConfigGuard::install_at` (Claude) and
/// `CodexHookConfigGuard::install_at` (Codex) layer their type-construction
/// on top of this — the heavy filesystem + JSON work was previously
/// duplicated across ~80 lines apart.
fn install_hook_config_file(
    config_dir: &Path,
    config_filename: &str,
    tool_label: &str,
    git_ignore_attribution: &str,
    merge_fn: fn(&mut serde_json::Value),
) -> Option<(PathBuf, bool)> {
    let settings_path = config_dir.join(config_filename);
    // `exists()` returns true for both files and directories; we need to
    // distinguish so a stale `.claude`/`.codex` regular file (e.g. left
    // behind by an earlier tool) doesn't masquerade as a usable directory
    // and silently break hook injection further down. Surface the case
    // with an actionable message instead of letting `write_atomic` fail
    // with a cryptic ENOTDIR from inside `tempfile::NamedTempFile::new_in`.
    let existed_as_dir = config_dir.is_dir();
    let exists_as_other = config_dir.exists() && !existed_as_dir;
    let first_write = !settings_path.exists();

    if exists_as_other {
        eprintln!(
            "paneflow-shim: {} exists but is not a directory; remove or \
             rename it to enable {tool_label} hooks this session",
            safe_path_display(config_dir)
        );
        return None;
    }

    if !existed_as_dir {
        if let Err(e) = std::fs::create_dir_all(config_dir) {
            eprintln!(
                "paneflow-shim: cannot create {} ({e}); {tool_label} hooks \
                 disabled this session",
                safe_path_display(config_dir)
            );
            return None;
        }
    }

    let existing = std::fs::read_to_string(&settings_path).unwrap_or_default();
    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(&existing) {
            Ok(v) => v,
            Err(e) => {
                // Corrupt JSON treated as empty; overwriting is preferable
                // to aborting the shim and leaving the user with a broken
                // settings file they can't fix from inside the AI tool.
                eprintln!(
                    "paneflow-shim: {} contained invalid JSON ({e}); \
                     overwriting with a fresh config",
                    safe_path_display(&settings_path)
                );
                serde_json::json!({})
            }
        }
    };

    merge_fn(&mut root);

    if let Err(e) = write_atomic(&settings_path, &root) {
        eprintln!(
            "paneflow-shim: cannot write {} ({e}); {tool_label} hooks \
             disabled this session",
            safe_path_display(&settings_path)
        );
        if !existed_as_dir {
            let _ = std::fs::remove_dir(config_dir);
        }
        return None;
    }

    if first_write {
        let dirname = config_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        eprintln!(
            "paneflow-shim: writing hook config to ./{dirname}/{config_filename} \
             (git-ignored by {git_ignore_attribution} convention)"
        );
    }

    Some((settings_path, !existed_as_dir))
}

/// Shared cleanup used by both guards' `Drop` impls: read the settings
/// file, run `remove_fn` to strip PaneFlow's entries, then either delete
/// the file (if now empty) and rmdir the config dir (if we created it),
/// or write the cleaned tree back atomically. All failures swallow
/// silently — Drop must never panic, and any error here means the next
/// shim invocation's merge-idempotency will converge the state.
fn cleanup_hook_config_file(
    settings_path: &Path,
    config_dir: &Path,
    created_dir: bool,
    remove_fn: fn(&mut serde_json::Value),
) {
    let Ok(content) = std::fs::read_to_string(settings_path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };

    remove_fn(&mut root);

    let is_empty = root
        .as_object()
        .map(serde_json::Map::is_empty)
        .unwrap_or(false);

    if is_empty {
        let _ = std::fs::remove_file(settings_path);
        if created_dir {
            // `remove_dir` only succeeds if the directory is empty — safe
            // even if the user dropped other files into the config dir.
            let _ = std::fs::remove_dir(config_dir);
        }
    } else {
        let _ = write_atomic(settings_path, &root);
    }
}

/// RAII guard: writes PaneFlow's hook config on construction, removes it on
/// drop. The guard must live for the duration of the child Claude Code
/// process, then drop normally when `main()` returns — this is why US-005
/// forces `run_real()` (vs `exec()`) so destructors actually fire.
struct HookConfigGuard {
    settings_path: PathBuf,
    claude_dir: PathBuf,
    // Whether the shim created `.claude/`. Only rmdir if we created it, so we
    // don't clobber a user-created directory that happened to be empty.
    created_dir: bool,
}

impl HookConfigGuard {
    /// Install in the project CWD. Returns `None` if the filesystem refuses
    /// our writes (read-only, permission denied, etc.) — the shim proceeds
    /// without hooks in that case, per PRD constraint C4. Also returns
    /// `None` (after sweeping any orphan entries left by a previous
    /// SIGKILL'd session) when no PaneFlow IPC socket is reachable: writing
    /// hook config that would invoke a dead handler would just create
    /// config noise per C4.
    fn install() -> Option<Self> {
        let cwd = env::current_dir().ok()?;
        let claude_dir = cwd.join(".claude");
        if !paneflow_ipc_reachable() {
            sweep_orphan_hook_config(
                &claude_dir.join("settings.local.json"),
                remove_paneflow_hooks,
            );
            return None;
        }
        Self::install_at(&claude_dir)
    }

    /// Testable inner. Takes the absolute path to the `.claude/` directory.
    /// Does NOT check `PANEFLOW_SOCKET_PATH` — the orphan-sweep gate lives
    /// in `install()` so unit tests can drive `install_at` without
    /// fabricating a live IPC socket.
    fn install_at(claude_dir: &Path) -> Option<Self> {
        let (settings_path, created_dir) = install_hook_config_file(
            claude_dir,
            "settings.local.json",
            "Claude Code",
            "Claude Code",
            merge_paneflow_hooks,
        )?;
        Some(Self {
            settings_path,
            claude_dir: claude_dir.to_path_buf(),
            created_dir,
        })
    }
}

impl Drop for HookConfigGuard {
    fn drop(&mut self) {
        cleanup_hook_config_file(
            &self.settings_path,
            &self.claude_dir,
            self.created_dir,
            remove_paneflow_hooks,
        );
    }
}

/// Build the `"command"` string written into `settings.local.json` /
/// `hooks.json` for the given hook `event`. Prefers the absolute path to the
/// sibling `paneflow-ai-hook` binary (resolved via `current_exe().parent()`)
/// so hooks resolve even when Claude Code or Codex is launched outside the
/// PATH-injected shell paneflow normally provides — e.g., when a user runs
/// `claude` directly in a project where a previous shim invocation left a
/// managed `settings.local.json` in place. Falls back to the bare binary name
/// when `current_exe()` fails or the sibling isn't present (test harnesses
/// where the test binary lives in `target/debug/deps/`, exotic filesystems);
/// the bare form is still recognized by `is_paneflow_hook_command` for
/// detection and cleanup.
fn resolve_hook_command(event: &str) -> String {
    match locate_sibling_hook_binary() {
        Some(path) => format!("{} {}", path.display(), event),
        None => format!("{HOOK_COMMAND_PREFIX}{event}"),
    }
}

/// Returns `true` if `command` is a paneflow-managed hook command, regardless
/// of whether it uses the legacy bare-name format (`paneflow-ai-hook <Event>`)
/// or the absolute-path format produced by `resolve_hook_command`. Detection
/// is basename-based so it works across both shapes and across platforms
/// (`paneflow-ai-hook` on Unix, `paneflow-ai-hook.exe` on Windows).
///
/// Intentionally does NOT verify that the binary exists on disk: a stale
/// config pointing at a removed cache dir (e.g., user uninstalled paneflow,
/// `cargo clean` between sessions) must still be recognized so cleanup can
/// remove it on the next shim run.
fn is_paneflow_hook_command(command: &str) -> bool {
    let first_token = command.split_whitespace().next().unwrap_or("");
    let basename = Path::new(first_token)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(first_token);
    basename == "paneflow-ai-hook" || basename == "paneflow-ai-hook.exe"
}

/// Merge PaneFlow's hook handlers into the parsed settings tree. Idempotent:
/// if a PaneFlow handler for an event is already present (identified by the
/// command basename OR the `_paneflow_managed` marker), we don't duplicate.
fn merge_paneflow_hooks(root: &mut serde_json::Value) {
    let root_obj = match root.as_object_mut() {
        Some(o) => o,
        None => {
            *root = serde_json::json!({});
            // re-borrow after replacement — `if let Some(...)` because we
            // just assigned an object, so this unwrap is infallible.
            let Some(o) = root.as_object_mut() else {
                return;
            };
            o
        }
    };

    let hooks_entry = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let Some(hooks_obj) = hooks_entry.as_object_mut() else {
        // User's `hooks` key is not an object (e.g., user set it to a
        // string by mistake). Overwrite it — we own the managed entries.
        *hooks_entry = serde_json::json!({});
        return;
    };

    for event in CLAUDE_HOOK_EVENTS {
        let entry = hooks_obj
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));
        let Some(array) = entry.as_array_mut() else {
            // User's event value is not an array; skip this event rather
            // than clobber what might be intentional config.
            continue;
        };

        let already_installed = array.iter().any(is_paneflow_matcher_group);
        if already_installed {
            continue;
        }

        // The `_paneflow_managed` marker sits on the OUTER matcher-group
        // wrapper — that's where `is_paneflow_matcher_group` checks it.
        // The inner handler object carries only the Claude-Code-native
        // fields so we don't send unexpected custom fields to Claude Code's
        // command runner. Identification falls back to the `command`
        // basename if the outer marker is stripped.
        array.push(serde_json::json!({
            "_paneflow_managed": true,
            "hooks": [
                {
                    "type": "command",
                    "command": resolve_hook_command(event),
                    "timeout": 5,
                }
            ]
        }));
    }
}

/// Remove PaneFlow's hook handlers from the parsed settings tree. Leaves
/// user entries untouched. Collapses empty event arrays and the empty
/// `hooks` key so cleanup produces a minimal file.
fn remove_paneflow_hooks(root: &mut serde_json::Value) {
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let Some(hooks_obj) = root_obj.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return;
    };

    for event in CLAUDE_HOOK_EVENTS {
        let Some(array) = hooks_obj.get_mut(*event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        array.retain(|matcher_group| !is_paneflow_matcher_group(matcher_group));
    }

    // Drop empty event arrays.
    hooks_obj.retain(|_k, v| v.as_array().is_none_or(|a| !a.is_empty()));

    // Drop the empty `hooks` object so the outer file can in turn become
    // empty and be deleted.
    if hooks_obj.is_empty() {
        root_obj.remove("hooks");
    }
}

/// Returns `true` if `value` is a matcher-group object that PaneFlow owns.
/// Belt-and-suspenders: first checks the `_paneflow_managed` marker on the
/// outer wrapper, then falls back to scanning the inner `hooks` array for a
/// command whose basename matches `paneflow-ai-hook[.exe]` (legacy bare-name
/// or absolute-path form). The second pass catches the case where Claude
/// Code's own settings writer strips unknown fields, AND the case where a
/// previous shim version wrote a bare-name command (forward compatibility
/// with the migration to absolute paths).
fn is_paneflow_matcher_group(value: &serde_json::Value) -> bool {
    if value
        .get("_paneflow_managed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    value
        .get("hooks")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|inner| {
            inner.iter().any(|handler| {
                handler
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_paneflow_hook_command)
            })
        })
}

/// Atomic write via `NamedTempFile::persist`: create a tempfile in the same
/// directory (so `persist` can `rename` without crossing filesystems), write
/// the JSON, then rename over the target. This avoids torn reads if Claude
/// Code concurrently reads the file at prompt time.
fn write_atomic(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "settings path has no parent",
        )
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value).map_err(std::io::Error::other)?;
    // Trailing newline matches what most human editors leave behind and
    // keeps diffs clean when users inspect the merged file.
    std::io::Write::write_all(&mut tmp, b"\n")?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Codex hook config injection (US-006, Unix) —
//   `.codex/hooks.json` + `~/.codex/config.toml`
// ---------------------------------------------------------------------------

/// Codex hook events (per Codex docs as of April 2026). Notably `Notification`
/// is NOT a Codex hook event — Claude Code has one but Codex does not. The
/// `paneflow-ai-hook` binary (US-003) accepts `Notification` as a valid event
/// name, but it's only fired from Windows JSONL `error` events (see
/// `parse_codex_event`), never from this hooks.json registration.
///
/// Unix-only: the only callers (`merge_codex_hooks`, `remove_codex_hooks`,
/// `CodexHookConfigGuard`) are all `#[cfg(unix)]`. On Windows the JSONL tee
/// path takes over, so this list would otherwise be dead code under
/// `-D warnings`.
#[cfg(unix)]
const CODEX_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
];

/// Marker for the TOML comment placed above `codex_hooks = true` in
/// `~/.codex/config.toml`. Cleanup scans for this literal line.
#[cfg(unix)]
const CODEX_TOML_MARKER: &str = "# _paneflow_managed: true";

/// Resolve `~/.codex/config.toml` using only std. The shim has no `dirs` dep
/// and `HOME` is universally set on Unix. Returns `None` on Windows builds
/// (unreachable at runtime — this function is `#[cfg(unix)]` — but kept for
/// documentation).
#[cfg(unix)]
fn codex_global_config_toml() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".codex").join("config.toml"))
}

/// RAII guard for Codex's project-level hooks.json + the global config.toml
/// feature flag. Both are populated on construction and reverted on drop.
#[cfg(unix)]
struct CodexHookConfigGuard {
    hooks_json_path: PathBuf,
    codex_dir: PathBuf,
    /// Whether the shim created `./.codex/`. Only rmdir on drop if we did.
    created_dir: bool,
    /// Absolute path to `~/.codex/config.toml` (if resolvable); `None` means
    /// no `HOME` — skip global-config work entirely.
    config_toml_path: Option<PathBuf>,
    /// Whether we appended the feature-flag block on install. Drop undoes it
    /// only if we did; never touches a pre-existing user-authored flag.
    added_feature_flag: bool,
}

#[cfg(unix)]
impl CodexHookConfigGuard {
    /// Install in the project CWD. Mirrors `HookConfigGuard::install`: same
    /// orphan-sweep gate when `PANEFLOW_SOCKET_PATH` is unreachable, then
    /// delegate to `install_at`.
    fn install() -> Option<Self> {
        let cwd = env::current_dir().ok()?;
        let codex_dir = cwd.join(".codex");
        if !paneflow_ipc_reachable() {
            sweep_orphan_hook_config(&codex_dir.join("hooks.json"), remove_codex_hooks);
            return None;
        }
        Self::install_at(&codex_dir, codex_global_config_toml().as_deref())
    }

    /// Testable inner. `config_toml_path` is the absolute path to the global
    /// `config.toml` (usually `~/.codex/config.toml`); pass `None` to skip
    /// the feature-flag step entirely (used by tests that don't want to
    /// pollute the test runner's home dir).
    fn install_at(codex_dir: &Path, config_toml_path: Option<&Path>) -> Option<Self> {
        let (hooks_json_path, created_dir) =
            install_hook_config_file(codex_dir, "hooks.json", "Codex", "Codex", merge_codex_hooks)?;

        // Codex-specific extra: enable the `codex_hooks = true` feature flag
        // in `~/.codex/config.toml`. Failure here is non-fatal — the user
        // can enable it manually and the rest of the hook config is in
        // place. Runs AFTER the shared install so the per-project hooks.json
        // is on disk before we touch the global config.
        let added_feature_flag = config_toml_path
            .and_then(enable_codex_feature_flag)
            .unwrap_or(false);

        Some(Self {
            hooks_json_path,
            codex_dir: codex_dir.to_path_buf(),
            created_dir,
            config_toml_path: config_toml_path.map(Path::to_path_buf),
            added_feature_flag,
        })
    }
}

#[cfg(unix)]
impl Drop for CodexHookConfigGuard {
    fn drop(&mut self) {
        // Roll back the feature flag first so if something below fails, the
        // global config is left in a consistent state.
        if self.added_feature_flag {
            if let Some(p) = self.config_toml_path.as_deref() {
                disable_codex_feature_flag(p);
            }
        }
        cleanup_hook_config_file(
            &self.hooks_json_path,
            &self.codex_dir,
            self.created_dir,
            remove_codex_hooks,
        );
    }
}

#[cfg(unix)]
fn merge_codex_hooks(root: &mut serde_json::Value) {
    if !root.is_object() {
        *root = serde_json::json!({});
    }
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let hooks_entry = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let Some(hooks_obj) = hooks_entry.as_object_mut() else {
        *hooks_entry = serde_json::json!({});
        return;
    };

    for event in CODEX_HOOK_EVENTS {
        let entry = hooks_obj
            .entry(*event)
            .or_insert_with(|| serde_json::json!([]));
        let Some(array) = entry.as_array_mut() else {
            continue;
        };
        if array.iter().any(is_paneflow_matcher_group) {
            continue;
        }
        array.push(serde_json::json!({
            "_paneflow_managed": true,
            "hooks": [
                {
                    "type": "command",
                    "command": resolve_hook_command(event),
                    "timeout": 5,
                }
            ]
        }));
    }
}

#[cfg(unix)]
fn remove_codex_hooks(root: &mut serde_json::Value) {
    let Some(root_obj) = root.as_object_mut() else {
        return;
    };
    let Some(hooks_obj) = root_obj.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return;
    };
    for event in CODEX_HOOK_EVENTS {
        let Some(array) = hooks_obj.get_mut(*event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        array.retain(|m| !is_paneflow_matcher_group(m));
    }
    hooks_obj.retain(|_k, v| v.as_array().is_none_or(|a| !a.is_empty()));
    if hooks_obj.is_empty() {
        root_obj.remove("hooks");
    }
}

/// Append `[features]\ncodex_hooks = true` (with a marker comment) to the
/// file at `path` iff: (a) the file doesn't already have `codex_hooks = true`
/// anywhere, AND (b) there's no existing `[features]` section — appending
/// would create a duplicate-section TOML error, so we abstain in that case
/// and warn.
///
/// Returns `Some(true)` if we modified the file, `Some(false)` if the flag
/// was already present (no-op), `None` if we aborted due to a conflict or
/// I/O error.
#[cfg(unix)]
fn enable_codex_feature_flag(path: &Path) -> Option<bool> {
    let existing = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!(
                "paneflow-shim: cannot read {} ({e}); leaving codex_hooks feature flag alone",
                safe_path_display(path)
            );
            return None;
        }
    };

    if has_codex_hooks_flag(&existing) {
        return Some(false);
    }
    if has_features_section(&existing) {
        eprintln!(
            "paneflow-shim: {} already has a [features] section without codex_hooks; skipping auto-enable (add `codex_hooks = true` there manually to enable Codex hooks)",
            safe_path_display(path)
        );
        return None;
    }

    // Safe append: ensure a trailing newline, then add our block.
    let mut next = existing.clone();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str(CODEX_TOML_MARKER);
    next.push('\n');
    next.push_str("[features]\ncodex_hooks = true\n");

    // Ensure the parent dir exists — the Codex config dir may be absent
    // if the user has never run Codex before, but the shim's own invocation
    // implies the binary is installed somewhere.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if let Err(e) = write_text_atomic(path, &next) {
        eprintln!(
            "paneflow-shim: cannot write {} ({e}); codex_hooks feature flag not set",
            safe_path_display(path)
        );
        return None;
    }
    // Tell the user, ONCE, that we touched their global config. Phase 7
    // security audit LOW #2: editing `~/.codex/config.toml` expands the
    // shim's write surface outside the project, so the user deserves a
    // visible notice.
    eprintln!(
        "paneflow-shim: enabled codex_hooks feature flag in {} (reverted on exit)",
        safe_path_display(path)
    );
    Some(true)
}

#[cfg(unix)]
fn disable_codex_feature_flag(path: &Path) {
    let Ok(existing) = std::fs::read_to_string(path) else {
        return;
    };
    let Some(cleaned) = strip_codex_feature_block(&existing) else {
        // Marker not found, or the managed block was edited by the user.
        // Both cases: don't touch the file. No stderr noise — next shim
        // invocation's idempotent install will converge state eventually.
        return;
    };
    // If the file is now empty or only whitespace, delete it — we either
    // created it from nothing or the flag was the only content.
    let result = if cleaned.trim().is_empty() {
        std::fs::remove_file(path)
    } else {
        write_text_atomic(path, &cleaned)
    };
    if let Err(e) = result {
        // Phase 7 security audit MEDIUM #12: if cleanup write fails, the
        // managed block stays in `~/.codex/config.toml` and subsequent
        // shim runs see `codex_hooks = true` already set, so they never
        // retry the cleanup. Surface the failure so the user can remove
        // the block manually. Known silent-failure mode.
        eprintln!(
            "paneflow-shim: could not revert {} ({e}); please remove the `{}` block manually",
            safe_path_display(path),
            CODEX_TOML_MARKER
        );
    }
}

/// Atomic text write via `tempfile::NamedTempFile::persist`. Mirrors the
/// existing `write_atomic` but for raw strings (not serde_json::Value).
/// Phase 7 security audit MEDIUM #3: concurrent shims both calling
/// `fs::write` on `~/.codex/config.toml` can produce torn bytes; rename
/// swap is the fix.
#[cfg(unix)]
fn write_text_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config path has no parent",
        )
    })?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    std::io::Write::write_all(&mut tmp, content.as_bytes())?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(unix)]
fn has_codex_hooks_flag(content: &str) -> bool {
    content.lines().any(|line| {
        let l = line.trim_start();
        if l.starts_with('#') {
            return false;
        }
        let stripped = l.split_once('#').map(|(k, _)| k).unwrap_or(l);
        let stripped = stripped.trim_end();
        match stripped.split_once('=') {
            Some((key, value)) => key.trim() == "codex_hooks" && value.trim() == "true",
            None => false,
        }
    })
}

#[cfg(unix)]
fn has_features_section(content: &str) -> bool {
    content.lines().any(|line| line.trim() == "[features]")
}

/// Remove the exact 3-line PaneFlow block (marker comment + `[features]` +
/// `codex_hooks = true`) from the file content. Returns the cleaned content
/// if the block was found and removed; `None` if the marker wasn't present.
#[cfg(unix)]
fn strip_codex_feature_block(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let marker_idx = lines.iter().position(|l| l.trim() == CODEX_TOML_MARKER)?;
    // We expect the marker to be followed by exactly:
    //   `[features]`
    //   `codex_hooks = true`
    // Remove those three lines; anything else means the file was edited
    // and we should bail to avoid clobbering user content.
    let tail = lines.get(marker_idx + 1..marker_idx + 3)?;
    // Exact-match both lines: `starts_with("codex_hooks")` would also strip
    // a hypothetical future `codex_hooks_experimental = ...` line that
    // happened to sit in the managed block. Exact match fails closed —
    // safer to leave the block untouched than over-delete.
    if tail[0].trim() != "[features]" || tail[1].trim() != "codex_hooks = true" {
        return None;
    }

    // Reconstruct: before marker + (possibly one blank line preceding the
    // marker that we added) + after the 3-line block.
    let mut head_end = marker_idx;
    // Strip a single trailing blank line we may have added before the marker
    // so we don't accumulate blanks across install/uninstall cycles.
    if head_end > 0 && lines[head_end - 1].is_empty() {
        head_end -= 1;
    }

    let mut out = String::new();
    for line in &lines[..head_end] {
        out.push_str(line);
        out.push('\n');
    }
    for line in &lines[marker_idx + 3..] {
        out.push_str(line);
        out.push('\n');
    }
    // Preserve original EOF-newline behavior: if the input had no trailing
    // newline and we removed content from the tail, trim one.
    if !content.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Codex Windows JSONL fallback (US-006, non-Unix)
// ---------------------------------------------------------------------------
//
// Codex hook machinery is disabled on Windows as of April 2026. Instead, the
// shim spawns `codex exec --json`, tees the child's stdout (so the user sees
// every byte unchanged), and dispatches `paneflow-ai-hook <Event>` subprocesses
// on recognized NDJSON events. Only the `exec` subcommand supports `--json`;
// interactive `codex` invocations on Windows pass through without tee — they
// simply won't produce sidebar state updates (documented regression).

#[cfg(not(unix))]
/// Decide whether to tee + inject `--json`. The `exec` subcommand is the only
/// Codex mode that emits NDJSON; other invocations (`codex`, `codex resume`,
/// `codex --help`) must pass through unmodified.
///
/// Scans for `"exec"` anywhere in argv rather than only at position 0, so
/// that global flags preceding the subcommand (e.g.,
/// `codex --config cfg.toml exec prompt`) are still detected. Known false-
/// positive edge: a user who passes `exec` as the VALUE of a preceding
/// value-taking flag (e.g., `codex --profile exec`) would get `--json`
/// injected; Codex then errors clearly and the user can bypass the shim
/// by invoking the real binary directly. Accepting this trade because
/// missing the tee on `codex --config X exec` is a silent regression,
/// while the false-positive is loud and bypassable.
fn rewrite_codex_args(args: &[OsString]) -> (Vec<OsString>, bool) {
    let Some(exec_idx) = args.iter().position(|a| a == "exec") else {
        return (args.to_vec(), false);
    };
    let mut rewritten = Vec::with_capacity(args.len() + 1);
    rewritten.extend_from_slice(&args[..=exec_idx]);
    rewritten.push(OsString::from("--json"));
    rewritten.extend(args[exec_idx + 1..].iter().cloned());
    (rewritten, true)
}

/// The full catalog of Codex NDJSON `"type"` discriminators documented at
/// <https://developers.openai.com/codex/noninteractive> as of April 2026.
/// `parse_codex_event`'s match arms MUST exhaustively cover every string in
/// this list so the schema-pin test can detect drift. When Codex adds a new
/// event type, add it to BOTH this const AND to a `match` arm in
/// `parse_codex_event` (even if the arm is just `None`).
///
/// Test-only: the runtime `parse_codex_event` enumerates the strings inline
/// rather than referencing this const, so without `cfg(test)` the const is
/// dead code under `-D warnings` on Windows non-test builds.
#[cfg(all(test, not(unix)))]
const KNOWN_CODEX_EVENT_TYPES: &[&str] = &[
    "thread.started",
    "turn.started",
    "turn.completed",
    "turn.failed",
    "item.started",
    "item.completed",
    "error",
];

/// Map a Codex NDJSON line to a `paneflow-ai-hook` event name. Returns
/// `None` for unrecognized / sub-event / malformed lines so the caller can
/// log-and-skip without breaking the tee loop.
#[cfg(not(unix))]
fn parse_codex_event(line: &str) -> Option<&'static str> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let event_type = value.get("type")?.as_str()?;
    match event_type {
        "turn.started" => Some("UserPromptSubmit"),
        "turn.completed" => Some("Stop"),
        "error" => Some("Notification"),
        // Known-but-unmapped events: explicit arms (not a catch-all) so
        // adding a new Codex event type without touching this match arm
        // requires a code change, which the schema-pin test will then
        // either accept (if the fixture is updated) or fail loudly.
        "thread.started" | "turn.failed" | "item.started" | "item.completed" => None,
        // Truly unknown (new Codex event type we don't recognize yet):
        // silently skip. The schema-pin test cross-checks this against
        // KNOWN_CODEX_EVENT_TYPES so drift is surfaced at dev time.
        _ => None,
    }
}

#[cfg(not(unix))]
fn run_codex_with_jsonl_tee(path: &Path, args: &[OsString]) -> ExitCode {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let mut child = match std::process::Command::new(path)
        .args(args)
        .envs(env::vars_os())
        .env("PANEFLOW_AI_TOOL", "codex")
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "paneflow-shim: spawn '{}' failed: {e}",
                safe_path_display(path)
            );
            return ExitCode::from(127);
        }
    };

    // Resolve the sibling hook binary ONCE before the tee loop. Bare-name
    // `Command::new("paneflow-ai-hook")` works on Windows only when the
    // shim cache dir is already on `%PATH%`, which is a per-PTY contract
    // not guaranteed at the moment this function spawns its first hook.
    // Resolving by absolute path matches every other call site in this
    // crate (`send_interrupt_stop`, `notify_session_end`).
    let hook_path = locate_sibling_hook_binary();

    // Take stdout so we can tee it. If the child was spawned without piped
    // stdout somehow, skip the tee entirely and fall through to wait().
    if let Some(stdout) = child.stdout.take() {
        // Reader thread: keeps the child's stdout drained (prevents pipe
        // fills) and dispatches hook events as they arrive.
        let tee_handle = std::thread::spawn(move || {
            let mut out = std::io::stdout().lock();
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                // Echo the line verbatim, preserving Codex's NDJSON so any
                // downstream tooling (jq, log-tailers) still works.
                let _ = std::io::Write::write_all(&mut out, line.as_bytes());
                let _ = std::io::Write::write_all(&mut out, b"\n");
                let _ = std::io::Write::flush(&mut out);

                if let Some(event) = parse_codex_event(&line) {
                    // Fire-and-forget: we don't wait on the hook binary so
                    // the tee loop keeps pace with Codex's output. This
                    // function is compiled only on non-Unix (`#[cfg(not
                    // (unix))]`) — on Windows, dropping the `Child` handle
                    // releases the OS handle and the subprocess runs to
                    // completion independently, with the OS reaping it
                    // when it exits. No zombies, no cleanup required.
                    //
                    // Known limitation (Phase 7 audit, MEDIUM #8): there is
                    // no rate limit. If Codex ever emits thousands of events
                    // per second, the shim would spawn thousands of
                    // subprocesses per second. Typical Codex output is a
                    // few events/sec, so this is tolerated without state.
                    if let Some(ref hp) = hook_path {
                        let _ = std::process::Command::new(hp)
                            .arg(event)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn();
                    }
                }
            }
        });
        // Join the reader thread AFTER wait() so all buffered output makes
        // it to the user before we exit.
        let status = child.wait();
        let _ = tee_handle.join();
        match status {
            Ok(s) => {
                let code = s.code().unwrap_or(1);
                ExitCode::from(u8::try_from(code).unwrap_or(1))
            }
            Err(e) => {
                eprintln!(
                    "paneflow-shim: wait on '{}' failed: {e}",
                    safe_path_display(path)
                );
                ExitCode::from(127)
            }
        }
    } else {
        match child.wait() {
            Ok(s) => {
                let code = s.code().unwrap_or(1);
                ExitCode::from(u8::try_from(code).unwrap_or(1))
            }
            Err(e) => {
                eprintln!(
                    "paneflow-shim: wait on '{}' failed: {e}",
                    safe_path_display(path)
                );
                ExitCode::from(127)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let Some(tool) = detect_tool() else {
        // Direct invocation (`./paneflow-shim`) or unexpected rename. Exit 2
        // matches `getopts` convention for "usage error" — the one case
        // where stderr output is acceptable because the user's command
        // cannot proceed regardless of PaneFlow state.
        eprintln!(
            "paneflow-shim: invoked under an unexpected name; copy or \
             hardlink this binary as 'claude' or 'codex' and put that \
             directory first on $PATH."
        );
        return ExitCode::from(2);
    };

    let Some(real) = find_real_binary(tool) else {
        // Same rationale: stderr output is the bash convention for
        // "command not found" (exit 127). The user's invocation cannot
        // succeed, so silent fail would be worse than a clear message.
        eprintln!("paneflow-shim: could not find real '{tool}' on PATH after self-exclusion");
        return ExitCode::from(127);
    };

    // Install hook config guards before spawning the child, remove on drop.
    // Bindings are held to end of `main` so destructors fire after
    // `run_real` returns; `None` is the graceful-degradation path for a
    // read-only FS / missing permissions (PRD C4).
    let _claude_guard = (tool == "claude").then(HookConfigGuard::install).flatten();
    #[cfg(unix)]
    let _codex_guard = (tool == "codex")
        .then(CodexHookConfigGuard::install)
        .flatten();

    let args: Vec<OsString> = env::args_os().skip(1).collect();

    // Windows + codex: the JSONL tee path substitutes for config-file hooks.
    // Gated on detecting the `exec` subcommand; interactive codex falls
    // through to the plain `run_real` path without tee.
    #[cfg(not(unix))]
    let code = if tool == "codex" {
        let (final_args, should_tee) = rewrite_codex_args(&args);
        if should_tee {
            run_codex_with_jsonl_tee(&real, &final_args)
        } else {
            run_real(tool, &real, &final_args)
        }
    } else {
        run_real(tool, &real, &args)
    };
    #[cfg(unix)]
    let code = run_real(tool, &real, &args);

    // The real AI binary has exited. Neither claude nor codex fires a
    // session-end hook event of their own, so the sidebar loader would
    // stick indefinitely if the user quit during a `Thinking` turn (no
    // `Stop` hook fired ⇒ no 5s auto-reset armed). Best-effort poke at
    // `paneflow-ai-hook SessionEnd` to send a single `ai.session_end`
    // IPC frame; the server clears `ai_state` to `Inactive`. Any failure
    // here is silent — the worst case is a stale loader, not a broken
    // shell.
    notify_session_end(tool);

    code
}

/// Best-effort notify of `ai.session_end` after the real AI binary exits.
///
/// Locates `paneflow-ai-hook` next to this shim binary (US-008 extracts
/// both into the same cache dir, and `current_exe()` handles symlink/
/// hardlink resolution per `find_real_binary` precedent), then spawns it
/// with `SessionEnd` and the `PANEFLOW_AI_TOOL` env so the hook tags the
/// frame with the right tool identity. Inherits `PANEFLOW_SOCKET_PATH`
/// and `PANEFLOW_WORKSPACE_ID` from the shim's own env (they were set
/// by `pty_session::inject_ai_hook_env`).
///
/// Blocking wait with no explicit timeout: the hook's only work is a
/// single Unix-socket write of a tiny JSON frame, typically <5 ms. The
/// PRD's 15 ms latency budget for shim overhead (US-004 AC) is preserved
/// even adding this — a Unix-socket connect+write is well under that
/// alone, and we're outside the spawn-to-exec critical path here (the
/// user's command has already returned its exit code).
fn notify_session_end(tool: &str) {
    let Some(hook_path) = locate_sibling_hook_binary() else {
        return;
    };
    let _ = std::process::Command::new(&hook_path)
        .arg("SessionEnd")
        .env("PANEFLOW_AI_TOOL", tool)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Resolve `paneflow-ai-hook` (or `.exe` on Windows) sitting in the same
/// directory as this shim binary. Returns `None` if `current_exe()`
/// fails or the sibling isn't a regular file — in either case, the
/// caller silently skips notification.
fn locate_sibling_hook_binary() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let dir = exe.parent()?;
    #[cfg(unix)]
    let name = "paneflow-ai-hook";
    #[cfg(windows)]
    let name = "paneflow-ai-hook.exe";
    let candidate = dir.join(name);
    candidate.is_file().then_some(candidate)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tool_from_stem_maps_known_stems() {
        assert_eq!(detect_tool_from_stem("claude"), Some("claude"));
        assert_eq!(detect_tool_from_stem("codex"), Some("codex"));
    }

    #[test]
    fn detect_tool_from_stem_rejects_everything_else() {
        assert_eq!(detect_tool_from_stem("paneflow-shim"), None);
        assert_eq!(detect_tool_from_stem("Claude"), None, "case-sensitive");
        assert_eq!(detect_tool_from_stem("claude-code"), None);
        assert_eq!(detect_tool_from_stem(""), None);
        assert_eq!(detect_tool_from_stem(" "), None);
    }

    #[cfg(unix)]
    #[test]
    fn candidate_names_unix_returns_bare_tool() {
        assert_eq!(candidate_names("claude"), vec!["claude".to_owned()]);
        assert_eq!(candidate_names("codex"), vec!["codex".to_owned()]);
    }

    #[cfg(windows)]
    #[test]
    fn candidate_names_windows_tries_exe_then_cmd() {
        assert_eq!(
            candidate_names("claude"),
            vec!["claude.exe".to_owned(), "claude.cmd".to_owned()],
            ".exe must be tried before .cmd so native builds win over wrappers"
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_real_binary_in_locates_tempdir_binary() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake = dir.path().join("claude");
        std::fs::File::create(&fake).unwrap();

        let found = find_real_binary_in("claude", vec![dir.path().to_owned()], None);
        assert_eq!(found.as_deref(), Some(fake.as_path()));
    }

    /// Windows counterpart: Claude Code ships as `claude.cmd` (a Node.js
    /// wrapper) on Windows today. The walk must find that file when no
    /// `.exe` exists alongside. When a `.exe` exists, it must win.
    #[cfg(windows)]
    #[test]
    fn find_real_binary_in_locates_cmd_then_exe_on_windows() {
        let dir = tempfile::TempDir::new().unwrap();
        let cmd_path = dir.path().join("claude.cmd");
        std::fs::File::create(&cmd_path).unwrap();

        // With only .cmd present, the walk falls through to it.
        let found = find_real_binary_in("claude", vec![dir.path().to_owned()], None);
        assert_eq!(found.as_deref(), Some(cmd_path.as_path()));

        // With both .exe and .cmd present, .exe wins per candidate ordering.
        let exe_path = dir.path().join("claude.exe");
        std::fs::File::create(&exe_path).unwrap();
        let found = find_real_binary_in("claude", vec![dir.path().to_owned()], None);
        assert_eq!(
            found.as_deref(),
            Some(exe_path.as_path()),
            "native .exe must take precedence over the .cmd wrapper"
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_real_binary_in_excludes_self_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let fake = dir.path().join("claude");
        std::fs::File::create(&fake).unwrap();

        // The tempdir appears as both the only PATH entry AND as the self
        // dir. The self-exclusion must skip it and yield `None` — otherwise
        // the shim would exec itself and recurse.
        let found = find_real_binary_in("claude", vec![dir.path().to_owned()], Some(dir.path()));
        assert!(found.is_none(), "self_dir must be excluded from PATH walk");
    }

    #[cfg(unix)]
    #[test]
    fn find_real_binary_in_walks_past_self_dir_to_find_real_binary() {
        // Simulates the production layout: PATH = [shim_dir, real_dir].
        // The shim entry is self_dir and must be skipped; the second entry
        // yields the real binary.
        let shim_dir = tempfile::TempDir::new().unwrap();
        let real_dir = tempfile::TempDir::new().unwrap();

        // Create a fake `claude` in the shim dir too — this would cause
        // infinite recursion in production if self-exclusion didn't work.
        std::fs::File::create(shim_dir.path().join("claude")).unwrap();
        let real_fake = real_dir.path().join("claude");
        std::fs::File::create(&real_fake).unwrap();

        let found = find_real_binary_in(
            "claude",
            vec![shim_dir.path().to_owned(), real_dir.path().to_owned()],
            Some(shim_dir.path()),
        );
        assert_eq!(found.as_deref(), Some(real_fake.as_path()));
    }

    #[test]
    fn find_real_binary_in_returns_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        // Empty dir, no matching binary anywhere on the passed "PATH".
        let found = find_real_binary_in("claude", vec![dir.path().to_owned()], None);
        assert!(found.is_none());
    }

    #[test]
    fn find_real_binary_in_tolerates_nonexistent_path_entries() {
        // PATH in the wild routinely contains stale directories (old
        // Python virtualenvs, uninstalled packages, typo'd PATH edits).
        // The walker must skip them silently rather than erroring.
        let dirs = vec![
            PathBuf::from("/definitely/does/not/exist/foo"),
            PathBuf::from("/also/not/real/bar"),
        ];
        let found = find_real_binary_in("claude", dirs, None);
        assert!(found.is_none());
    }

    /// Linux-gated timing guard. Replaces the PRD's "criterion benchmark"
    /// (PRD US-004 AC bullet 7) with a lightweight check that stays within
    /// the 15 ms budget even with a realistic number of stale `$PATH`
    /// entries. Criterion would pull ~30 dev-deps for one number; this
    /// guards the same invariant at ~zero cost.
    #[cfg(target_os = "linux")]
    #[test]
    fn find_real_binary_in_completes_under_15ms_budget() {
        let dirs: Vec<PathBuf> = (0..20)
            .map(|i| PathBuf::from(format!("/tmp/paneflow-nonexistent-{i}")))
            .collect();

        let start = std::time::Instant::now();
        let _ = find_real_binary_in("claude", dirs, None);
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(15),
            "PATH walk must complete under 15 ms; got {elapsed:?}"
        );
    }

    // ---------- US-005: HookConfigGuard ----------
    //
    // All tests call `HookConfigGuard::install_at` with a tempdir-backed
    // `.claude/` path rather than mutating `std::env::current_dir()` — the
    // same env-free discipline used by US-002/003 tests.

    use serde_json::json;

    fn read_settings(claude_dir: &Path) -> serde_json::Value {
        let content = std::fs::read_to_string(claude_dir.join("settings.local.json")).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    fn count_paneflow_entries(root: &serde_json::Value, event: &str) -> usize {
        root["hooks"][event]
            .as_array()
            .map(|a| a.iter().filter(|v| is_paneflow_matcher_group(v)).count())
            .unwrap_or(0)
    }

    #[test]
    fn install_at_creates_file_with_all_five_events() {
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");

        let guard = HookConfigGuard::install_at(&claude_dir)
            .expect("install_at into an empty tempdir must succeed");

        let root = read_settings(&claude_dir);
        for event in CLAUDE_HOOK_EVENTS {
            let handlers = root["hooks"][*event].as_array().unwrap();
            assert_eq!(
                handlers.len(),
                1,
                "expected exactly one matcher-group for {event}"
            );

            // The exact command shape (bare name vs. absolute path) depends on
            // whether `current_exe()` finds a sibling `paneflow-ai-hook` —
            // which it does NOT in `cargo test` (test binary lives under
            // `target/debug/deps/`, hook binary lives under `target/debug/`).
            // Assert the contract instead of the format: it must be detectable
            // by `is_paneflow_hook_command`, and it must end with the event
            // name so Claude Code dispatches to the correct handler.
            let cmd = handlers[0]
                .pointer("/hooks/0/command")
                .and_then(|v| v.as_str())
                .expect("command must be a string");
            assert!(
                is_paneflow_hook_command(cmd),
                "{event}: command {cmd:?} must be recognized as paneflow-managed"
            );
            assert!(
                cmd.ends_with(&format!(" {event}")),
                "{event}: command {cmd:?} must end with the event name"
            );

            let timeout = handlers[0].pointer("/hooks/0/timeout").unwrap();
            assert_eq!(
                timeout,
                &json!(5),
                "timeout is in seconds per Claude Code docs"
            );

            // The marker sits on the OUTER matcher-group wrapper, not on
            // the inner Claude-Code-native handler (we don't pollute the
            // handler object with custom fields that Claude Code would
            // ignore anyway).
            assert_eq!(
                handlers[0].get("_paneflow_managed"),
                Some(&json!(true)),
                "outer matcher-group must carry the managed marker"
            );
            assert!(
                handlers[0].pointer("/hooks/0/_paneflow_managed").is_none(),
                "inner handler object must NOT carry the custom marker"
            );
        }

        drop(guard);
        // We created both the dir and the file — cleanup must remove both.
        assert!(!claude_dir.join("settings.local.json").exists());
        assert!(!claude_dir.exists());
    }

    #[test]
    fn install_at_preserves_existing_user_hooks_and_permissions() {
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        // Pre-existing settings: user config + one of their own hooks on
        // UserPromptSubmit that must survive both install and cleanup.
        let initial = json!({
            "permissions": { "allow": ["Bash(ls:*)"] },
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [{ "type": "command", "command": "echo user-hook" }] }
                ]
            }
        });
        std::fs::write(
            claude_dir.join("settings.local.json"),
            serde_json::to_string_pretty(&initial).unwrap(),
        )
        .unwrap();

        let guard = HookConfigGuard::install_at(&claude_dir).unwrap();

        // After install: user entry + PaneFlow entry side-by-side.
        let root = read_settings(&claude_dir);
        let arr = root["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "user + paneflow entries coexist");
        assert_eq!(
            arr.iter().filter(|v| is_paneflow_matcher_group(v)).count(),
            1
        );
        // Unrelated sections untouched.
        assert_eq!(root["permissions"]["allow"][0], json!("Bash(ls:*)"));

        drop(guard);

        // After drop: only the user's hook remains; the file persists
        // because the user's content is non-empty.
        let root = read_settings(&claude_dir);
        let arr = root["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let surviving_cmd = arr[0].pointer("/hooks/0/command").unwrap();
        assert_eq!(surviving_cmd, &json!("echo user-hook"));
        assert_eq!(root["permissions"]["allow"][0], json!("Bash(ls:*)"));
        // We did NOT create `.claude/`, so cleanup must leave it in place.
        assert!(claude_dir.exists());
    }

    #[test]
    fn install_at_is_idempotent_on_reinstall() {
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");

        let first = HookConfigGuard::install_at(&claude_dir).unwrap();
        // Second install on top of the first must NOT duplicate entries.
        let second = HookConfigGuard::install_at(&claude_dir).unwrap();

        let root = read_settings(&claude_dir);
        for event in CLAUDE_HOOK_EVENTS {
            assert_eq!(
                count_paneflow_entries(&root, event),
                1,
                "{event} must carry exactly one PaneFlow entry after re-install"
            );
        }

        drop(second);
        drop(first); // idempotent drop: second pass reads the already-cleaned file
    }

    #[test]
    fn cleanup_removes_managed_entries_even_when_marker_was_stripped() {
        // Simulate Claude Code re-serializing and stripping the
        // `_paneflow_managed` marker from the inner hook object. The
        // belt-and-suspenders prefix check on `command` must still detect
        // and clean up the handler. (anthropics/claude-code#5886)
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        let stripped = json!({
            "hooks": {
                "Stop": [
                    {
                        // `_paneflow_managed` on the outer wrapper is gone.
                        "hooks": [
                            {
                                "type": "command",
                                "command": "paneflow-ai-hook Stop",
                                "timeout": 5
                            }
                        ]
                    }
                ]
            }
        });
        std::fs::write(
            claude_dir.join("settings.local.json"),
            serde_json::to_string(&stripped).unwrap(),
        )
        .unwrap();

        // Install (no-op — detects our entry via command prefix) then drop.
        let guard = HookConfigGuard::install_at(&claude_dir).unwrap();
        drop(guard);

        // File must be fully cleaned — only our entry existed, so after
        // cleanup the file is gone. The directory was created by the test
        // (simulating a user-owned `.claude/`), so the guard correctly
        // leaves it in place — the `cleanup_handles_preexisting_claude_dir`
        // test separately validates the "we created it, we rmdir it"
        // inverse case.
        assert!(!claude_dir.join("settings.local.json").exists());
    }

    #[test]
    fn cleanup_handles_preexisting_claude_dir_without_deleting_it() {
        // The user created `.claude/` themselves (for other Claude Code
        // files). Cleanup must NOT rmdir it, even when our settings file
        // was the only item inside.
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();

        let guard = HookConfigGuard::install_at(&claude_dir).unwrap();
        assert!(claude_dir.join("settings.local.json").exists());
        drop(guard);

        // Settings file gone (was only managed entries), but the directory
        // that the user already owned must remain.
        assert!(!claude_dir.join("settings.local.json").exists());
        assert!(
            claude_dir.exists(),
            "cleanup must not rmdir a user-owned .claude/"
        );
    }

    #[test]
    fn install_at_tolerates_corrupt_existing_json() {
        // A corrupt settings file (mid-edit save, interrupted write)
        // shouldn't abort the shim — we overwrite and proceed.
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("settings.local.json"), "{not json}").unwrap();

        let guard = HookConfigGuard::install_at(&claude_dir)
            .expect("corrupt JSON must not prevent install");
        let root = read_settings(&claude_dir);
        assert_eq!(count_paneflow_entries(&root, "UserPromptSubmit"), 1);

        drop(guard);
    }

    #[test]
    fn merge_does_not_clobber_user_hooks_in_other_events() {
        let td = tempfile::TempDir::new().unwrap();
        let claude_dir = td.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let initial = json!({
            "hooks": {
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo user" }] }
                ]
            }
        });
        std::fs::write(
            claude_dir.join("settings.local.json"),
            serde_json::to_string(&initial).unwrap(),
        )
        .unwrap();

        let guard = HookConfigGuard::install_at(&claude_dir).unwrap();

        let root = read_settings(&claude_dir);
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "user's Bash matcher + PaneFlow entry");
        // User's matcher preserved byte-for-byte.
        assert_eq!(arr[0]["matcher"], json!("Bash"));
        assert_eq!(
            arr[0].pointer("/hooks/0/command"),
            Some(&json!("echo user"))
        );

        drop(guard);

        let root = read_settings(&claude_dir);
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], json!("Bash"));
    }

    // ---------- US-006: CodexHookConfigGuard (Unix) ----------

    #[cfg(unix)]
    #[test]
    fn codex_install_at_creates_hooks_json_with_all_six_events() {
        let td = tempfile::TempDir::new().unwrap();
        let codex_dir = td.path().join(".codex");
        // Pass None for config.toml path so tests don't touch real `~/.codex`.
        let guard = CodexHookConfigGuard::install_at(&codex_dir, None)
            .expect("install_at on empty tempdir must succeed");

        let content = std::fs::read_to_string(codex_dir.join("hooks.json")).unwrap();
        let root: serde_json::Value = serde_json::from_str(&content).unwrap();

        for event in CODEX_HOOK_EVENTS {
            let handlers = root["hooks"][*event].as_array().unwrap();
            assert_eq!(
                handlers.len(),
                1,
                "expected exactly one matcher-group for Codex {event}"
            );
            assert_eq!(
                handlers[0].get("_paneflow_managed"),
                Some(&json!(true)),
                "outer wrapper must carry the managed marker"
            );
            let cmd = handlers[0]
                .pointer("/hooks/0/command")
                .and_then(|v| v.as_str())
                .expect("command must be a string");
            assert!(
                is_paneflow_hook_command(cmd),
                "{event}: command {cmd:?} must be recognized as paneflow-managed"
            );
            assert!(
                cmd.ends_with(&format!(" {event}")),
                "{event}: command {cmd:?} must end with the event name"
            );
        }

        // `Notification` is NOT a Codex hook — confirm the registration
        // respects the platform's actual event surface even though the
        // `paneflow-ai-hook` binary happens to accept that event name.
        assert!(
            root["hooks"].get("Notification").is_none(),
            "Codex hooks.json must not register a Notification event — it is not a Codex hook"
        );

        drop(guard);
        assert!(!codex_dir.join("hooks.json").exists());
        assert!(!codex_dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn codex_install_at_preserves_user_hooks_and_cleanup() {
        let td = tempfile::TempDir::new().unwrap();
        let codex_dir = td.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let initial = json!({
            "hooks": {
                "PreToolUse": [
                    { "hooks": [{ "type": "command", "command": "echo codex-user-hook" }] }
                ]
            }
        });
        std::fs::write(
            codex_dir.join("hooks.json"),
            serde_json::to_string_pretty(&initial).unwrap(),
        )
        .unwrap();

        let guard = CodexHookConfigGuard::install_at(&codex_dir, None).unwrap();
        let content = std::fs::read_to_string(codex_dir.join("hooks.json")).unwrap();
        let root: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "user + paneflow entries coexist");

        drop(guard);
        let content = std::fs::read_to_string(codex_dir.join("hooks.json")).unwrap();
        let root: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].pointer("/hooks/0/command"),
            Some(&json!("echo codex-user-hook"))
        );
    }

    // ---------- US-006: TOML feature-flag mutation (Unix) ----------

    #[cfg(unix)]
    #[test]
    fn enable_codex_feature_flag_creates_block_on_empty_file() {
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path().join("config.toml");

        let result = enable_codex_feature_flag(&path);
        assert_eq!(result, Some(true), "empty file should trigger an append");

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(CODEX_TOML_MARKER));
        assert!(content.contains("[features]"));
        assert!(content.contains("codex_hooks = true"));
    }

    #[cfg(unix)]
    #[test]
    fn enable_codex_feature_flag_noop_when_already_enabled() {
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path().join("config.toml");
        std::fs::write(&path, "[features]\ncodex_hooks = true\nother = false\n").unwrap();

        let result = enable_codex_feature_flag(&path);
        assert_eq!(result, Some(false), "already-enabled must be a no-op");

        // File unchanged.
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(CODEX_TOML_MARKER));
    }

    #[cfg(unix)]
    #[test]
    fn enable_codex_feature_flag_abstains_on_existing_features_section() {
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path().join("config.toml");
        // User already has `[features]` without codex_hooks — appending
        // another `[features]` would trigger a duplicate-section TOML
        // parse error on Codex's side, so the shim must abstain.
        std::fs::write(&path, "[features]\nother_flag = false\n").unwrap();

        let result = enable_codex_feature_flag(&path);
        assert_eq!(result, None);

        // File untouched.
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(CODEX_TOML_MARKER));
        assert!(!content.contains("codex_hooks"));
    }

    #[cfg(unix)]
    #[test]
    fn disable_codex_feature_flag_removes_managed_block() {
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path().join("config.toml");
        std::fs::write(&path, "[user_stuff]\nkey = 1\n").unwrap();

        // Install adds our block.
        let added = enable_codex_feature_flag(&path).unwrap();
        assert!(added);
        let with_block = std::fs::read_to_string(&path).unwrap();
        assert!(with_block.contains(CODEX_TOML_MARKER));

        // Cleanup strips exactly our 3-line block, preserving the user's
        // original content byte-for-byte.
        disable_codex_feature_flag(&path);
        let cleaned = std::fs::read_to_string(&path).unwrap();
        assert_eq!(cleaned, "[user_stuff]\nkey = 1\n");
    }

    #[cfg(unix)]
    #[test]
    fn disable_codex_feature_flag_deletes_empty_file() {
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path().join("config.toml");

        // Install from nothing — file becomes just our 3-line block.
        assert_eq!(enable_codex_feature_flag(&path), Some(true));
        disable_codex_feature_flag(&path);
        assert!(!path.exists(), "config.toml we created must be removed");
    }

    #[cfg(unix)]
    #[test]
    fn codex_guard_wires_feature_flag_through_config_toml() {
        let td = tempfile::TempDir::new().unwrap();
        let codex_dir = td.path().join(".codex");
        let config_toml = td.path().join("config.toml");

        let guard = CodexHookConfigGuard::install_at(&codex_dir, Some(&config_toml)).unwrap();

        let toml_content = std::fs::read_to_string(&config_toml).unwrap();
        assert!(toml_content.contains("codex_hooks = true"));
        assert!(toml_content.contains(CODEX_TOML_MARKER));

        drop(guard);
        // Config.toml cleanup: since this test created config.toml from
        // nothing, the file should be removed.
        assert!(!config_toml.exists());
    }

    // ---------- US-006: Windows JSONL parser + argv rewrite ----------
    //
    // These tests are cross-platform (no `#[cfg]`) because `parse_codex_event`
    // and `rewrite_codex_args` are compile-gated `#[cfg(not(unix))]`. We
    // re-gate the tests to keep them compiling on Windows CI. On Unix the
    // functions don't exist, so the tests go away cleanly.

    #[cfg(not(unix))]
    #[test]
    fn parse_codex_event_maps_known_types_to_paneflow_events() {
        assert_eq!(
            parse_codex_event(r#"{"type":"turn.started"}"#),
            Some("UserPromptSubmit")
        );
        assert_eq!(
            parse_codex_event(r#"{"type":"turn.completed","usage":{"input_tokens":1}}"#),
            Some("Stop")
        );
        assert_eq!(
            parse_codex_event(r#"{"type":"error","message":"oops"}"#),
            Some("Notification")
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn parse_codex_event_returns_none_for_unmapped_known_types() {
        // These types ARE emitted by Codex but we intentionally don't
        // translate them into IPC frames — `thread.started` isn't a
        // meaningful sidebar state, `turn.failed` is already covered by
        // `error`-style notifications, and `item.*` are sub-events that
        // would over-fire the loader.
        for t in &[
            "thread.started",
            "turn.failed",
            "item.started",
            "item.completed",
        ] {
            let line = format!(r#"{{"type":"{t}"}}"#);
            assert_eq!(
                parse_codex_event(&line),
                None,
                "{t} must be silently skipped"
            );
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn parse_codex_event_returns_none_for_invalid_or_unknown_input() {
        assert_eq!(parse_codex_event(""), None);
        assert_eq!(parse_codex_event("   "), None);
        assert_eq!(parse_codex_event("not json"), None);
        assert_eq!(parse_codex_event(r#"{"no_type": true}"#), None);
        assert_eq!(
            parse_codex_event(r#"{"type":"unknown.future.event"}"#),
            None
        );
    }

    /// Schema-pin test (PRD US-006 AC bullet 3): assert that every entry in
    /// `KNOWN_CODEX_EVENT_TYPES` has an explicit mapping in `parse_codex_event`
    /// AND the fixture below covers every known event 1:1. When Codex adds a
    /// new event type:
    ///   1. Add the new type string to `KNOWN_CODEX_EVENT_TYPES`.
    ///   2. Add a match arm for it in `parse_codex_event` (even if it's
    ///      `None` — the arm must be explicit, not the catch-all).
    ///   3. Add a `(type, expected)` entry to the fixture below.
    ///
    /// Failing to do (3) trips the `fixture.len() == KNOWN.len()` check
    /// with a clear message. Failing to do (2) trips the membership assert.
    /// Failing to do (1) still compiles but leaves the catch-all `_ => None`
    /// arm of `parse_codex_event` handling the new type silently — which is
    /// acceptable (loader doesn't update) but is the one drift mode this
    /// test cannot catch without actually running Codex.
    #[cfg(not(unix))]
    #[test]
    fn parse_codex_event_schema_pin() {
        let fixture: &[(&str, Option<&str>)] = &[
            ("thread.started", None),
            ("turn.started", Some("UserPromptSubmit")),
            ("turn.completed", Some("Stop")),
            ("turn.failed", None),
            ("item.started", None),
            ("item.completed", None),
            ("error", Some("Notification")),
        ];

        assert_eq!(
            fixture.len(),
            KNOWN_CODEX_EVENT_TYPES.len(),
            "KNOWN_CODEX_EVENT_TYPES has {} entries but fixture has {}; \
             update both together when Codex adds a new event type",
            KNOWN_CODEX_EVENT_TYPES.len(),
            fixture.len()
        );

        for (codex_type, expected) in fixture {
            assert!(
                KNOWN_CODEX_EVENT_TYPES.contains(codex_type),
                "fixture contains {codex_type} but KNOWN_CODEX_EVENT_TYPES \
                 does not — add it there and to parse_codex_event's match"
            );
            let line = format!(r#"{{"type":"{codex_type}"}}"#);
            let actual = parse_codex_event(&line);
            assert_eq!(
                actual, *expected,
                "schema drift: Codex event {codex_type} mapped to {actual:?}; \
                 expected {expected:?}. If Codex has added / renamed this \
                 event, update KNOWN_CODEX_EVENT_TYPES, parse_codex_event, \
                 and this fixture together."
            );
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn rewrite_codex_args_injects_json_after_exec_at_any_position() {
        // `exec` at argv[0] — classic case.
        let exec_first = vec![
            OsString::from("exec"),
            OsString::from("--model"),
            OsString::from("o4"),
        ];
        let (rewritten, should_tee) = rewrite_codex_args(&exec_first);
        assert!(should_tee);
        assert_eq!(
            rewritten,
            vec![
                OsString::from("exec"),
                OsString::from("--json"),
                OsString::from("--model"),
                OsString::from("o4"),
            ]
        );

        // Global flag before subcommand: `codex --config cfg.toml exec prompt`
        // — the Phase 6 reviewer's SHOULD_FIX #8 scenario. The scan-anywhere
        // fix ensures we still detect `exec` and inject `--json` after it.
        let global_then_exec = vec![
            OsString::from("--config"),
            OsString::from("cfg.toml"),
            OsString::from("exec"),
            OsString::from("prompt"),
        ];
        let (rewritten, should_tee) = rewrite_codex_args(&global_then_exec);
        assert!(
            should_tee,
            "global flag before `exec` must still trigger tee"
        );
        assert_eq!(
            rewritten,
            vec![
                OsString::from("--config"),
                OsString::from("cfg.toml"),
                OsString::from("exec"),
                OsString::from("--json"),
                OsString::from("prompt"),
            ]
        );

        // Interactive invocation — no `exec` token, no tee.
        let interactive: Vec<OsString> = vec![];
        let (rewritten, should_tee) = rewrite_codex_args(&interactive);
        assert!(!should_tee);
        assert_eq!(rewritten, interactive);

        // Other subcommand — still no tee.
        let resume = vec![OsString::from("resume")];
        let (rewritten, should_tee) = rewrite_codex_args(&resume);
        assert!(!should_tee);
        assert_eq!(rewritten, resume);
    }

    // ---------- Hook-command detection (basename rule) ----------

    /// The legacy bare-name format MUST stay recognized so a shim upgrade
    /// can clean up `settings.local.json` files written by the previous
    /// version (which used `format!("paneflow-ai-hook {event}")` directly).
    #[test]
    fn is_paneflow_hook_command_accepts_legacy_bare_name() {
        for event in CLAUDE_HOOK_EVENTS {
            let cmd = format!("paneflow-ai-hook {event}");
            assert!(
                is_paneflow_hook_command(&cmd),
                "legacy bare-name format must be recognized: {cmd:?}"
            );
        }
    }

    /// New absolute-path format produced by `resolve_hook_command` when a
    /// sibling binary is present. This is the production case for end users.
    #[test]
    fn is_paneflow_hook_command_accepts_unix_absolute_path() {
        let cmd = "/home/user/.cache/paneflow/bin/0.1.0/paneflow-ai-hook Stop";
        assert!(is_paneflow_hook_command(cmd));

        let cmd = "/usr/local/bin/paneflow-ai-hook PreToolUse";
        assert!(is_paneflow_hook_command(cmd));
    }

    /// Windows variant: the binary basename is `paneflow-ai-hook.exe` and
    /// `Path::file_name` on Unix still extracts the trailing component
    /// correctly when the input uses forward slashes (Path semantics differ
    /// on Windows for `\`, but the basename rule covers both names).
    #[test]
    fn is_paneflow_hook_command_accepts_exe_basename() {
        let cmd = "/some/path/paneflow-ai-hook.exe Stop";
        assert!(is_paneflow_hook_command(cmd));
    }

    /// Fix B (orphan cleanup): even if the binary at the absolute path no
    /// longer exists on disk, the command must still be recognized so
    /// `remove_paneflow_hooks` can purge stale entries written by an
    /// earlier paneflow install that has since been removed.
    #[test]
    fn is_paneflow_hook_command_recognizes_orphans_without_filesystem_check() {
        // Path that almost certainly does not exist — the function must NOT
        // touch the filesystem.
        let cmd = "/nonexistent/old/cache/paneflow-ai-hook UserPromptSubmit";
        assert!(
            is_paneflow_hook_command(cmd),
            "orphaned absolute paths must be detectable for cleanup"
        );
    }

    /// User hooks must NOT be misclassified as paneflow-managed. The
    /// basename rule narrows the namespace collision risk vs. the previous
    /// bare-prefix rule, but rejection of common user patterns is the
    /// primary safety property.
    #[test]
    fn is_paneflow_hook_command_rejects_user_hooks() {
        let user_hooks = [
            "echo hello",
            "/usr/bin/git status",
            "node my-hook.js",
            "paneflow-shim Stop",               // sibling binary, different name
            "my-paneflow-ai-hook Stop",         // similar but distinct basename
            "/path/to/paneflow-ai-hook-2 Stop", // suffixed name
            "",                                 // empty
            "   ",                              // whitespace only
            "notarealcommand",                  // no event
        ];
        for cmd in user_hooks {
            assert!(
                !is_paneflow_hook_command(cmd),
                "user hook {cmd:?} must NOT be classified as paneflow-managed"
            );
        }
    }

    /// Round-trip property: `resolve_hook_command` must produce a string
    /// that `is_paneflow_hook_command` recognizes, regardless of which
    /// branch (sibling-found or bare-name fallback) was taken. Without
    /// this, a user could end up with hooks they cannot clean up.
    #[test]
    fn resolve_hook_command_output_is_recognized_by_detector() {
        for event in CLAUDE_HOOK_EVENTS {
            let cmd = resolve_hook_command(event);
            assert!(
                is_paneflow_hook_command(&cmd),
                "resolve_hook_command output must be detectable: {cmd:?}"
            );
            assert!(
                cmd.ends_with(&format!(" {event}")),
                "resolve_hook_command output must end with the event name: {cmd:?}"
            );
        }
    }
}
