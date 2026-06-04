//! US-003: kill-on-parent-death guard for spawned agent CLIs and PTYs.
//!
//! Goal: when Paneflow dies for any reason (including `kill -9`), the
//! child processes it spawned -- `claude`, `codex`, `opencode`, the
//! shells started inside agent terminals -- must die with it. Without
//! this, orphans are reparented to PID 1 (Unix) or kept alive by the
//! kernel (Windows) and continue streaming, consuming the user's API
//! tokens until their natural timeout.
//!
//! Implementation status by OS:
//!
//! - **Windows (full)**. [`install_process_job`] creates a Job Object
//!   with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and assigns the running
//!   Paneflow process to it on startup. Every process spawned by
//!   Paneflow after that point inherits the job by default (unless it
//!   explicitly sets `CREATE_BREAKAWAY_FROM_JOB`, which neither
//!   `paneflow-acp` nor `portable-pty` do). When Paneflow exits, the
//!   last job handle is closed and Windows kills every member -- agent
//!   CLI, ConPTY host, descendants.
//!
//! - **Linux + macOS (partial)**. [`install_process_job`] is a no-op
//!   shim. A real fix requires injecting `prctl(PR_SET_PDEATHSIG)`
//!   (Linux) or a `kqueue NOTE_EXIT` watcher (macOS) inside the
//!   child's `pre_exec` closure -- but `portable-pty::CommandBuilder`
//!   does not expose `pre_exec`, and `paneflow-acp::spawn` hides
//!   the `std::process::Command` behind its own API. Closing that
//!   gap is a v2 follow-up requiring upstream changes in both crates.
//!   Today the graceful-shutdown path is covered by `Drop` discipline:
//!   `AgentTerminalSession::Drop` (US-009) and `SessionRuntime::Drop`
//!   release master fds and signal child processes when Paneflow exits
//!   cleanly. The `kill -9` case on Unix still leaks orphan agent
//!   CLIs and is a documented known limitation.

#[cfg(target_os = "windows")]
mod windows_impl {
    use win32job::{ExtendedLimitInfo, Job};

    /// Build a Job Object with `KILL_ON_JOB_CLOSE` and assign the
    /// running Paneflow process to it. Children inherit the job
    /// automatically.
    ///
    /// The job handle is deliberately leaked: the Win32 contract is
    /// "kill on last handle close", and the last handle is the one
    /// held by the running Paneflow process. Storing the `Job` in a
    /// static would risk dropping it on hot-reload or a future
    /// teardown path, which would dissociate the children before
    /// Paneflow truly exits.
    pub(super) fn install() -> Result<(), Box<dyn std::error::Error>> {
        let mut info = ExtendedLimitInfo::default();
        info.limit_kill_on_job_close();
        let job = Job::create_with_limit_info(&info)?;
        job.assign_current_process()?;
        std::mem::forget(job);
        Ok(())
    }
}

/// Install the process-wide kill-on-parent-death guard. Call once,
/// early in `fn main()`, before any agent CLI or PTY is spawned.
///
/// Failure is non-fatal: a hosted environment that forbids
/// `CreateJobObject` (rare; restricted container or denied ACL) means
/// orphan-on-crash is back to "best effort", but Paneflow itself
/// remains functional. Caller logs the error and proceeds.
pub fn install_process_job() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::install()
    }
    #[cfg(not(target_os = "windows"))]
    {
        // No-op on Linux + macOS until paneflow-acp and portable-pty
        // expose a pre_exec hook; see the module-level docstring.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The call must NOT panic on any OS. On Windows it attempts a
    /// real Job Object install; everywhere else the no-op shim
    /// short-circuits cleanly. We treat the Windows return value as
    /// best-effort: some hosted CI runners (GitHub Actions Windows,
    /// Azure DevOps) put the test process inside a parent Job Object
    /// with `JOB_OBJECT_LIMIT_BREAKAWAY_OK` cleared and an ACL that
    /// denies `AssignProcessToJobObject`. Win8+ allows nested jobs in
    /// general, but those restricted parent jobs still reject the
    /// assignment with `ERROR_ACCESS_DENIED`. A test panic in that
    /// case would just be CI noise -- the production call site at
    /// `main.rs:1030` already logs the error and proceeds without
    /// blocking startup.
    #[test]
    fn install_process_job_does_not_panic() {
        // Calling twice is also safe -- on Windows the second call
        // creates a second job and the OS handles the case where the
        // process is already a member. On Linux/macOS both calls are
        // no-ops.
        let _ = install_process_job();
        let _ = install_process_job();
    }

    /// Linux/macOS contract: the call must be a no-op. The behavioural
    /// assertion is that we did not silently fall through to a panic
    /// or to a `unimplemented!()`. Returning Ok is the contract.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn unix_install_is_a_documented_no_op() {
        assert!(install_process_job().is_ok());
    }
}
