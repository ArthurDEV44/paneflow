//! Bounded external-process execution shared across Paneflow crates.
//!
//! `std`-only, zero external dependencies, so it can be a dependency of the
//! embedded `paneflow-shim` without inflating the binary that ships inside the
//! main executable (EP-002, US-005).
//!
//! Every external subprocess in Paneflow must run under a wall-clock deadline
//! with a bounded stdout buffer and a null stdin, so that a hung mirror, a
//! PATH-hijacked agent binary, or a slow/dead network mount can never freeze a
//! caller or exhaust memory. [`run_with_timeout`] is that one primitive: it is
//! synchronous and is meant to run on a background thread (the codebase calls
//! it from inside `smol::unblock`), never on the GPUI render thread.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::error::Error;
use std::fmt;
use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Upper bound on captured stderr. stderr is diagnostics-only, so a small fixed
/// cap is enough; like stdout it is drained past the cap so the child never
/// blocks on a full pipe.
const STDERR_CAP: u64 = 64 * 1024;

/// How often [`run_with_timeout`] polls the child for exit. Small enough that a
/// fast command returns promptly, large enough that a multi-minute deadline
/// does not spin the CPU.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Windows `CREATE_NO_WINDOW` process-creation flag (`winbase.h`, `0x0800_0000`).
/// Paneflow runs as a GUI-subsystem process with no console of its own, so every
/// console subprocess it spawns (the `git diff --shortstat` poller, agent CLIs,
/// MCP probes) would otherwise get a freshly-allocated, *visible* console window
/// that flashes open and shut for the child's lifetime. Suppressing it is always
/// correct for `run_with_timeout` callers: they pipe stdout/stderr and null
/// stdin, so the child is non-interactive by construction and never needs a
/// console. Kept as a raw literal so this crate stays `std`-only / zero-dep (it
/// is embedded in `paneflow-shim`).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Why a bounded run did not produce a normal [`Output`].
#[derive(Debug)]
pub enum ProcError {
    /// The child could not be spawned.
    Spawn(io::Error),
    /// Polling the child's status failed.
    Wait(io::Error),
    /// The deadline elapsed before the child exited; the child was killed and
    /// reaped before this error was returned.
    Timeout,
}

impl fmt::Display for ProcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcError::Spawn(e) => write!(f, "failed to spawn process: {e}"),
            ProcError::Wait(e) => write!(f, "failed to poll process status: {e}"),
            ProcError::Timeout => {
                write!(f, "process exceeded its deadline and was killed")
            }
        }
    }
}

impl Error for ProcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ProcError::Spawn(e) | ProcError::Wait(e) => Some(e),
            ProcError::Timeout => None,
        }
    }
}

/// Run `cmd` to completion under a wall-clock `deadline`, capturing at most
/// `stdout_cap` bytes of stdout (and a small fixed cap of stderr).
///
/// - stdin is `/dev/null` so the child can never block waiting on a prompt.
/// - stdout/stderr are read on dedicated threads so a child that writes more
///   than the cap is drained (and discarded past the cap) instead of blocking
///   on a full pipe - bounded memory, no deadlock.
/// - the child is polled with [`Child::try_wait`]; if `deadline` elapses first
///   the child is killed and reaped and [`ProcError::Timeout`] is returned.
///
/// The returned [`Output::stdout`] is at most `stdout_cap` bytes; a child that
/// produced more exited normally (its excess was discarded), so the status is
/// still its real exit status.
pub fn run_with_timeout(
    mut cmd: Command,
    deadline: Duration,
    stdout_cap: u64,
) -> Result<Output, ProcError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // GUI-subsystem callers have no console of their own, so a console child
    // spawned without this flag pops a visible window that flashes open and shut
    // for the child's lifetime. The crate-level note on CREATE_NO_WINDOW explains
    // why suppression is always correct here (piped stdio + null stdin → the child
    // is non-interactive by construction and never needs a console).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(ProcError::Spawn)?;

    // Hand the pipe ends to reader threads BEFORE polling: if we polled while
    // the child filled a ~64 KiB pipe buffer it would block on write and we'd
    // kill a child that was not actually hung.
    let stdout_reader = child
        .stdout
        .take()
        .map(|pipe| spawn_bounded_reader(pipe, stdout_cap));
    let stderr_reader = child
        .stderr
        .take()
        .map(|pipe| spawn_bounded_reader(pipe, STDERR_CAP));

    let start = Instant::now();
    let status = loop {
        match child.try_wait().map_err(ProcError::Wait)? {
            Some(status) => break status,
            None => {
                if start.elapsed() >= deadline {
                    // Deadline hit. `kill()` only SENDS a signal (non-blocking);
                    // it is `wait()` and the reader `join`s that can block -
                    // and they block FOREVER on a child wedged in uninterruptible
                    // sleep (D-state on a dead NFS/CIFS mount: the exact US-035
                    // scenario), because SIGKILL is not delivered until the
                    // kernel unblocks the task, and the readers can't hit EOF
                    // while that child still holds the pipe write ends. So we
                    // kill best-effort and hand the reap + drain to a DETACHED
                    // thread, then return immediately - the deadline must bound
                    // when the CALLER is freed, not just when the child is
                    // signalled. (Bounded leak: one cleanup thread + its readers
                    // per timed-out process, finishing whenever the OS finally
                    // reaps the child.) On Windows a child that exited since the
                    // last `try_wait` makes `kill()` return `InvalidInput`; the
                    // `let _ =` absorbs it and the `wait()` below still reaps.
                    let _ = child.kill();
                    thread::spawn(move || {
                        let _ = child.wait();
                        join_reader(stdout_reader);
                        join_reader(stderr_reader);
                    });
                    return Err(ProcError::Timeout);
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    };

    Ok(Output {
        status,
        stdout: join_reader(stdout_reader),
        stderr: join_reader(stderr_reader),
    })
}

/// Read up to `cap` bytes from `pipe`, then drain and discard the remainder so
/// the child can finish writing and exit. Never retains more than `cap` bytes.
fn spawn_bounded_reader<R>(mut pipe: R, cap: u64) -> thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buf = Vec::new();
        // Keep at most `cap` bytes.
        let _ = pipe.by_ref().take(cap).read_to_end(&mut buf);
        // Drain the rest into a small scratch buffer and throw it away: a
        // chatty-but-honest child exits cleanly while a malicious stream stays
        // bounded in memory (and is ultimately stopped by the caller's
        // deadline, which kills the child). Throttle the drain: a producer that
        // continuously SATURATES the pipe (a hijacked `yes`-like binary) would
        // otherwise spin this thread at 100% of a core for the whole deadline.
        // A read that comes back full is the saturation signal, so sleep 1 ms
        // on it - that caps the discard rate near 8 MB/s at ~0 CPU while an
        // honest bursty producer (whose reads return short or block) drains at
        // full speed.
        let mut scratch = [0u8; 8 * 1024];
        loop {
            match pipe.read(&mut scratch) {
                Ok(0) | Err(_) => break,
                Ok(n) if n == scratch.len() => thread::sleep(Duration::from_millis(1)),
                Ok(_) => {}
            }
        }
        buf
    })
}

fn join_reader(reader: Option<thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shell wrapper so the behavior tests stay readable across platforms.
    #[cfg(unix)]
    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }
    #[cfg(windows)]
    fn sh(script: &str) -> Command {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(script);
        c
    }

    #[test]
    fn completes_under_deadline_and_captures_stdout() {
        let out = run_with_timeout(sh("printf hello"), Duration::from_secs(5), 1 << 20)
            .expect("fast command should complete");
        assert!(out.status.success());
        // printf has no trailing newline on Unix; cmd `echo` would add CRLF, so
        // assert on a prefix to stay platform-tolerant.
        assert!(
            out.stdout.starts_with(b"hello"),
            "stdout was {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    #[test]
    fn sleeping_child_is_killed_at_the_deadline() {
        // A 30 s sleeper under a 150 ms deadline must return ~immediately with
        // Timeout, not block for 30 s.
        let start = Instant::now();
        let res = run_with_timeout(sh("sleep 30"), Duration::from_millis(150), 1 << 20);
        assert!(
            matches!(res, Err(ProcError::Timeout)),
            "expected Timeout, got {res:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "must not wait for the child to finish on its own"
        );
    }

    /// stdout cap: a 1 MB producer under a 4 KiB cap returns exactly the cap,
    /// the child still exits cleanly (drain), and we neither OOM nor hang.
    /// Unix-only because it leans on `/dev/zero`; the cap/drain logic is pure
    /// `std::io` and platform-agnostic (Windows verified by inspection).
    ///
    /// Volume is 1 MB (still 256x the cap, so a broken cap would buffer the
    /// whole megabyte and fail the length assert) rather than 10 MB: the drain
    /// deliberately throttles a pipe-saturating producer with a 1 ms sleep per
    /// 8 KiB read, so 10 MB is ~1220 sleeps whose scheduler jitter on a loaded
    /// CI runner can overshoot a tight deadline. The 30 s deadline then leaves
    /// ample headroom over the ~150 ms a 1 MB drain actually takes.
    #[cfg(unix)]
    #[test]
    fn stdout_cap_truncates_without_oom_or_hang() {
        let start = Instant::now();
        let out = run_with_timeout(
            sh("head -c 1000000 /dev/zero"),
            Duration::from_secs(30),
            4096,
        )
        .expect("producer should exit cleanly after its output is drained");
        assert!(out.status.success());
        assert_eq!(
            out.stdout.len(),
            4096,
            "stdout must be capped, not buffered"
        );
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "drain must let the child exit well under the deadline"
        );
    }

    #[test]
    fn nonzero_exit_status_is_reported_not_an_error() {
        let out = run_with_timeout(sh("exit 3"), Duration::from_secs(5), 1 << 20)
            .expect("a clean nonzero exit is an Output, not a ProcError");
        assert!(!out.status.success());
    }
}
