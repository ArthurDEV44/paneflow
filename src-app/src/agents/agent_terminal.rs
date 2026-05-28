//! US-018 (prd-agents-view.md): minimal agent-side PTY session.
//!
//! Distinct from `crate::terminal` (the visible CLI multiplexer's
//! Alacritty-grid-backed PTY): that one is wired to GPUI rendering
//! and a 4 ms event-batched poll loop. Agent terminals are
//! invisible-by-default execution contexts the agent uses to run
//! commands -- the PRD ships them with no live VT emulation, just a
//! growable in-memory output buffer + exit-code plumbing.
//!
//! Why a separate stack:
//! - The visible terminal would force every agent `bash -c "ls"` to
//!   instantiate an Alacritty grid + GPUI listener + event channel.
//!   Cost is real (a few hundred KB and a thread per terminal) and
//!   the data path is the wrong shape (the agent wants bytes, not
//!   pixels).
//! - Keeping the layers separate also matches Zed's split (`agent_servers/terminal.rs`
//!   in upstream vs. `terminal_view/terminal.rs`).
//!
//! What this module ships:
//! - [`AgentTerminalSpawner`] implementing
//!   [`paneflow_acp::TerminalSpawner`].
//! - [`AgentTerminalSession`] implementing
//!   [`paneflow_acp::TerminalSession`] -- one growable
//!   `Vec<u8>` output buffer, one [`portable_pty::Child`] handle,
//!   one reader thread.
//! - Sandbox check: `cwd` (if provided) is resolved relative to the
//!   session's working directory and verified inside it -- same
//!   policy as `paneflow_acp::file_ops`.
//!
//! What this module intentionally does NOT do:
//! - VT emulation (escape sequences pass through verbatim into the
//!   output buffer; the agent typically `strip-ansi`s its own
//!   output when displaying it).
//! - Live resize (the ACP `terminal/create` schema has no resize
//!   request; a `set_dimensions` was discussed for v2 but is not
//!   in 0.12).
//! - Stdin write-back from the agent (also out of the 0.12 schema).

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use agent_client_protocol::schema::{CreateTerminalRequest, TerminalExitStatus, TerminalId};
use paneflow_acp::{
    BoxFuture, SessionRegistry, TerminalError, TerminalOutputSnapshot, TerminalSession,
    TerminalSpawner,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Default cap for the in-memory output buffer when the agent did
/// not supply `output_byte_limit`. Matches the ACP recommendation
/// (~1 MB) and keeps even verbose long-running commands from
/// pinning the heap.
const DEFAULT_OUTPUT_LIMIT_BYTES: usize = 1 << 20;

/// Production [`TerminalSpawner`]. Holds an [`Arc<SessionRegistry>`]
/// so it can resolve the session's cwd for sandbox checks.
pub struct AgentTerminalSpawner {
    sessions: SessionRegistry,
}

impl AgentTerminalSpawner {
    pub fn new(sessions: SessionRegistry) -> Arc<Self> {
        Arc::new(Self { sessions })
    }
}

impl TerminalSpawner for AgentTerminalSpawner {
    fn create(
        &self,
        request: &CreateTerminalRequest,
    ) -> Result<Arc<dyn TerminalSession>, TerminalError> {
        // Resolve the cwd: prefer the request's explicit cwd, else
        // fall back to the session's cwd. Then verify it lies
        // inside the session sandbox.
        let session_cwd = self.sessions.cwd(&request.session_id).ok_or_else(|| {
            TerminalError::Other(anyhow::anyhow!(
                "Terminal blocked: unknown session id (no cwd registered)"
            ))
        })?;
        let cwd = match request.cwd.as_ref() {
            Some(p) if p.is_absolute() => p.clone(),
            Some(p) => session_cwd.join(p),
            None => session_cwd.clone(),
        };
        let canonical_cwd = std::fs::canonicalize(&cwd).map_err(|e| {
            TerminalError::Other(anyhow::anyhow!(
                "Terminal blocked: cannot canonicalize cwd {}: {e}",
                cwd.display()
            ))
        })?;
        let canonical_session_cwd = std::fs::canonicalize(&session_cwd).map_err(|e| {
            TerminalError::Other(anyhow::anyhow!(
                "Terminal blocked: cannot canonicalize session cwd {}: {e}",
                session_cwd.display()
            ))
        })?;
        if !canonical_cwd.starts_with(&canonical_session_cwd) {
            return Err(TerminalError::Other(anyhow::anyhow!(
                "Terminal blocked: cwd outside project root ({} not under {})",
                canonical_cwd.display(),
                canonical_session_cwd.display(),
            )));
        }

        let limit = request
            .output_byte_limit
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_OUTPUT_LIMIT_BYTES);
        let session = AgentTerminalSession::spawn(request, canonical_cwd, limit)?;
        Ok(Arc::new(session))
    }
}

/// One live agent terminal. Reader thread runs in the background
/// and pushes bytes into [`shared::buffer`] until the PTY EOFs or
/// the session is killed/released.
pub struct AgentTerminalSession {
    id: TerminalId,
    shared: Arc<SessionShared>,
}

struct SessionShared {
    buffer: Mutex<OutputBuffer>,
    child: Mutex<ChildState>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
}

#[derive(Default)]
struct OutputBuffer {
    bytes: Vec<u8>,
    truncated: bool,
    limit: usize,
}

impl OutputBuffer {
    fn with_limit(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            truncated: false,
            limit,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > self.limit {
            // Drop bytes from the front; the ACP spec mandates a
            // valid UTF-8 boundary on truncation. Drop chars instead
            // of raw bytes when the buffer happens to be UTF-8.
            let overflow = self.bytes.len() - self.limit;
            let drop_until = next_char_boundary(&self.bytes, overflow);
            self.bytes.drain(..drop_until);
            self.truncated = true;
        }
    }

    fn snapshot(&self) -> (String, bool) {
        // Strip ANSI/VT control sequences (CSI, OSC, DCS, SOS, PM, APC,
        // charset designators) plus C0/DEL controls except `\r \n \t \b`.
        // The raw buffer keeps the unchanged bytes so a future debug path
        // can re-expose them; only the snapshot the agent feeds to the LLM
        // is sanitised. Without this, output from `cargo`, `git`, `npm`,
        // `bat` and friends carries the SGR/cursor sequences verbatim into
        // the model context, which both wastes tokens and confuses tool
        // parsers that don't run `strip-ansi` themselves.
        let s = strip_ansi_bytes(&self.bytes);
        (s, self.truncated)
    }
}

/// Strip ANSI/VT escape sequences and most C0 controls from `bytes`,
/// returning a UTF-8 String (lossy decode for the rare non-UTF-8 byte).
/// Preserves the line-feed family (`\r \n \t \b`) so output structure
/// survives intact.
///
/// Designed for snapshot consumers (LLM context), not for live VT
/// emulation — the parser is a forward state machine, not a full vte
/// processor, and intentionally cheap.
fn strip_ansi_bytes(bytes: &[u8]) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            // ESC introducer — skip the whole sequence
            0x1b => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    // CSI: ESC [ params... final(0x40..=0x7E)
                    b'[' => {
                        i += 1;
                        while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1; // consume the final byte
                        }
                    }
                    // OSC: ESC ] params... terminated by BEL or ESC \
                    b']' => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x07 {
                                i += 1;
                                break;
                            }
                            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    // DCS / SOS / PM / APC: ESC P|X|^|_ ... ST (ESC \)
                    b'P' | b'X' | b'^' | b'_' => {
                        i += 1;
                        while i < bytes.len() {
                            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    // Charset designators: ESC ( ) * + then one byte
                    b'(' | b')' | b'*' | b'+' => {
                        i += 1;
                        if i < bytes.len() {
                            i += 1;
                        }
                    }
                    // Other single-byte ESC sequences (SS2 N, SS3 O, c, =, >, 7, 8…)
                    _ => {
                        i += 1;
                    }
                }
            }
            // Preserve structural whitespace
            b'\r' | b'\n' | b'\t' | 0x08 => {
                out.push(b);
                i += 1;
            }
            // Drop remaining C0 controls + DEL
            0x00..=0x1f | 0x7f => {
                i += 1;
            }
            // Printable + UTF-8 continuation bytes
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Return the smallest UTF-8 character-boundary offset that is >=
/// `start`. Falls back to `start` when the buffer is not UTF-8 (the
/// drain is then byte-precise rather than char-precise, which is
/// acceptable for the AC since the loss is bounded by 4 bytes).
fn next_char_boundary(bytes: &[u8], start: usize) -> usize {
    if start >= bytes.len() {
        return bytes.len();
    }
    // A byte is a UTF-8 leader (start byte) iff (b & 0b1100_0000) != 0b1000_0000.
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        if (b & 0b1100_0000) != 0b1000_0000 {
            return i;
        }
        i += 1;
    }
    bytes.len()
}

enum ChildState {
    Running(Box<dyn Child + Send + Sync>),
    Exited(TerminalExitStatus),
    Released,
}

/// US-002 (cli-hardening-followup-2026-Q3): hard deadline on
/// [`AgentTerminalSession::wait_for_exit`]. A SIGKILL race where the
/// OS reaps the zombie before our `poll_child_exit` can observe it
/// used to park a `tokio::spawn_blocking` thread forever. With the
/// blocking pool capped at 128, repeated such races leaked the pool
/// over a session lifetime. 30 s matches the upper bound of an
/// `agent_client_protocol` `terminal/create` response handler tolerance.
const WAIT_FOR_EXIT_DEADLINE: Duration = Duration::from_secs(30);

/// US-002 (cli-hardening-followup-2026-Q3): pure wait loop extracted
/// from [`AgentTerminalSession::wait_for_exit`] so unit tests can
/// exercise the deadline with a millisecond-scale fixture instead
/// of the 30 s production cap.
///
/// `poll` returns `Some(status)` when the child has exited and the
/// loop terminates with `Ok(status)`. `deadline` bounds the total
/// wait. `poll_interval` controls the sleep between polls and is
/// the bottleneck for test responsiveness -- tests use 1 ms.
/// `pid_for_log` is captured by the caller before the loop starts
/// and feeds the `Timeout` log line.
fn wait_for_exit_loop<F>(
    mut poll: F,
    deadline: Duration,
    poll_interval: Duration,
    pid_for_log: Option<u32>,
) -> Result<TerminalExitStatus, TerminalError>
where
    F: FnMut() -> Option<TerminalExitStatus>,
{
    let start = Instant::now();
    loop {
        if let Some(es) = poll() {
            return Ok(es);
        }
        let elapsed = start.elapsed();
        if elapsed >= deadline {
            let elapsed_secs = elapsed.as_secs();
            match pid_for_log {
                Some(pid) => log::warn!(
                    "wait_for_exit: child pid={pid} did not exit within {elapsed_secs}s; \
                     returning Timeout to release the tokio blocking thread"
                ),
                None => log::warn!(
                    "wait_for_exit: child pid=? did not exit within {elapsed_secs}s; \
                     returning Timeout to release the tokio blocking thread"
                ),
            }
            return Err(TerminalError::Timeout {
                operation: "wait_for_exit",
                elapsed_secs,
            });
        }
        std::thread::sleep(poll_interval);
    }
}

impl AgentTerminalSession {
    fn spawn(
        request: &CreateTerminalRequest,
        cwd: PathBuf,
        output_limit: usize,
    ) -> Result<Self, TerminalError> {
        let pty_system = native_pty_system();
        // ACP 0.12 has no `terminal/resize` request, so the dimensions chosen
        // here are fixed for the session's lifetime. Picking 24×80 (the
        // historical terminal default) forces tools that consult `$COLUMNS`
        // — `ls --color`, `tree`, `cargo` colour output, `column`, anything
        // routed through `tabulate` / `rich` — to wrap at 80 columns and
        // truncate / re-flow output, which then lands in the LLM context as
        // jumbled text. 120×500 keeps line widths wide enough for typical
        // source-code listings and stack traces, and offers enough vertical
        // room that a moderately long command run (a Rust build's warning
        // wall, a `git log`, an `npm install` tree) does not trigger
        // pagination logic in pagers that auto-detect a small screen.
        let pair = pty_system
            .openpty(PtySize {
                rows: 500,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::Other(anyhow::anyhow!("openpty failed: {e}")))?;

        let mut cmd = CommandBuilder::new(&request.command);
        for arg in &request.args {
            cmd.arg(arg);
        }
        for env in &request.env {
            cmd.env(&env.name, &env.value);
        }
        cmd.cwd(&cwd);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| TerminalError::Other(anyhow::anyhow!("spawn failed: {e}")))?;
        drop(pair.slave); // close slave fd on this side; the child holds the other end

        // Read the child's stdout via the master fd. portable-pty
        // multiplexes stdout + stderr through the PTY by design.
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| TerminalError::Other(anyhow::anyhow!("try_clone_reader failed: {e}")))?;

        let id = TerminalId::from(format!("agent-term-{}", next_terminal_id()));
        let shared = Arc::new(SessionShared {
            buffer: Mutex::new(OutputBuffer::with_limit(output_limit)),
            child: Mutex::new(ChildState::Running(child)),
            master: Mutex::new(Some(pair.master)),
        });

        // Reader thread: pump bytes into the shared buffer until EOF.
        let buf_handle = Arc::clone(&shared);
        std::thread::Builder::new()
            .name(format!("paneflow-agent-term-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut guard = lock_with_poison_log(&buf_handle.buffer, "buffer");
                            guard.append(&buf[..n]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| {
                TerminalError::Other(anyhow::anyhow!("reader thread spawn failed: {e}"))
            })?;

        Ok(Self { id, shared })
    }
}

impl TerminalSession for AgentTerminalSession {
    fn id(&self) -> TerminalId {
        self.id.clone()
    }

    fn snapshot(&self) -> BoxFuture<'_, Result<TerminalOutputSnapshot, TerminalError>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            // Check if the child has exited (non-blocking poll).
            let exit_status = poll_child_exit(&mut lock_with_poison_log(&shared.child, "child"));
            let (output, truncated) = lock_with_poison_log(&shared.buffer, "buffer").snapshot();
            Ok(TerminalOutputSnapshot {
                output,
                truncated,
                exit_status,
            })
        })
    }

    fn wait_for_exit(&self) -> BoxFuture<'_, Result<TerminalExitStatus, TerminalError>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            // Capture the PID once for the warn-log on the timeout
            // path. May be `None` when the child has already been
            // dropped before this future polls, in which case the
            // log just says `pid=?` -- still actionable.
            let pid_for_log = {
                let guard = lock_with_poison_log(&shared.child, "child");
                match &*guard {
                    ChildState::Running(c) => c.process_id(),
                    _ => None,
                }
            };
            // Block the tokio task on a dedicated blocking thread so
            // the single-threaded runtime can keep servicing other
            // commands. Poll cadence is 100 ms -- ACP agents tend to
            // tolerate this since `wait_for_exit` is a one-shot.
            tokio::task::spawn_blocking(move || {
                wait_for_exit_loop(
                    || poll_child_exit(&mut lock_with_poison_log(&shared.child, "child")),
                    WAIT_FOR_EXIT_DEADLINE,
                    Duration::from_millis(100),
                    pid_for_log,
                )
            })
            .await
            .map_err(|e| TerminalError::Other(anyhow::anyhow!("wait_for_exit join failed: {e}")))?
        })
    }

    fn kill(&self) -> BoxFuture<'_, Result<(), TerminalError>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            // AC #7: SIGTERM + 2 s wait + SIGKILL. portable-pty's
            // `Child::kill` maps to TerminateProcess on Windows and
            // SIGKILL on Unix, which is too aggressive on the first
            // try. We use `child::process_id` + libc::kill on Unix
            // for the SIGTERM courtesy; Windows skips the grace
            // period and goes straight to TerminateProcess.
            tokio::task::spawn_blocking(move || {
                let pid_opt = {
                    let guard = lock_with_poison_log(&shared.child, "child");
                    match &*guard {
                        ChildState::Running(c) => c.process_id(),
                        _ => None,
                    }
                };
                #[cfg(unix)]
                if let Some(pid) = pid_opt
                    && pid > 0
                {
                    // Send SIGTERM to the whole process group (`-pid`) so a
                    // command that forked workers (npm script wrapping a
                    // dev-server, make spawning compilers) gets every member
                    // signalled. portable-pty calls `setsid()` on the spawned
                    // child (unix.rs:220), so `pid` is both the PID and the
                    // PGID — `kill(-pgid, SIGTERM)` is the canonical POSIX
                    // group-signal idiom.
                    //
                    // SAFETY: libc::kill with a negative pid > 0 captured by
                    // value targets the process group; the pid was validated
                    // alive at the moment we read it from the child handle.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGTERM);
                    }
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(50));
                        let exited =
                            poll_child_exit(&mut lock_with_poison_log(&shared.child, "child"))
                                .is_some();
                        if exited {
                            return Ok(());
                        }
                    }
                    // Grace period expired. Escalate to SIGKILL on the whole
                    // group before the portable-pty fallback below (which
                    // only signals the leader's PID via SIGHUP — too soft
                    // for a hung process group).
                    // SAFETY: same constraints as the SIGTERM above.
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = pid_opt; // suppress unused on Windows
                // Fallback: portable-pty's ChildKiller (SIGHUP on Unix,
                // TerminateProcess on Windows). On Unix this is now belt-and-
                // suspenders — the SIGKILL above already reaped the group.
                let mut guard = lock_with_poison_log(&shared.child, "child");
                if let ChildState::Running(c) = &mut *guard {
                    let _ = c.kill();
                    // Drive the poll to reap zombies.
                    let _ = poll_child_exit(&mut guard);
                }
                Ok(())
            })
            .await
            .map_err(|e| TerminalError::Other(anyhow::anyhow!("kill join failed: {e}")))?
        })
    }

    fn release(&self) -> BoxFuture<'_, Result<(), TerminalError>> {
        let shared = Arc::clone(&self.shared);
        Box::pin(async move {
            release_sync(&shared);
            Ok(())
        })
    }
}

/// US-019 (audit P2-8): lock a `Mutex<T>` with a log line on poison
/// recovery. `into_inner()` keeps Paneflow running with the recovered
/// state (preferred over re-panicking) but a poisoned mutex means a
/// prior thread panicked mid-write -- the recovered state may be
/// inconsistent. Logging at warn surfaces that risk in any bug report
/// instead of swallowing it silently. `site` is the field name (e.g.
/// `"buffer"`, `"child"`, `"master"`, `"permission_broker_pending"`)
/// so the log narrows down the affected lock.
///
/// `pub(super)` so [`crate::agents::runtime::PermissionBroker`] can
/// share the same recovery-with-log discipline (US-019 review
/// follow-up). Lives here because the 9 in-file call sites dominate
/// the function's gravity.
pub(super) fn lock_with_poison_log<'a, T>(
    m: &'a Mutex<T>,
    site: &str,
) -> std::sync::MutexGuard<'a, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => {
            log::warn!(
                target: "paneflow_app::agents::agent_terminal",
                "{site} mutex poisoned; recovering state -- inner state may be inconsistent: {p}"
            );
            p.into_inner()
        }
    }
}

/// Synchronous body of [`AgentTerminalSession::release`]. Drives the
/// reader thread to EOF by terminating the child first, then drops
/// the master fd. Idempotent: the `Option::take`-style discipline on
/// `master` and the `ChildState::Released` terminal state make repeat
/// calls cheap no-ops.
///
/// Ordering matters here. The reader thread is parked on `read()`
/// against a fd cloned via `try_clone_reader()` -- a separate fd
/// from the master we hold. Closing the master alone does NOT EOF
/// that cloned fd; EOF only arrives when the child closes the slave,
/// which happens when the child exits. So the kill is the load-
/// bearing step. On Unix, [`portable_pty::Child::kill`] sends SIGHUP,
/// which a SIGHUP-immune child (Node-based dev servers with their
/// own SIGHUP handler, daemon-mode wrappers) can survive -- leaving
/// the reader thread blocked indefinitely and the [`Arc<SessionShared>`]
/// alive. We escalate to SIGKILL on the process group first
/// (portable-pty calls `setsid()` in its `pre_exec`, so the child's
/// pid is its pgid), then run the portable-pty fallback as belt-
/// and-suspenders, then close the master.
///
/// Race with the async [`AgentTerminalSession::release`]: both paths
/// can call this concurrently (Drop on the GPUI main thread + ACP
/// release on the smol pool). The two `Mutex` lock sites serialise
/// them: the second caller observes [`Option::take`]-ed `master` and
/// [`ChildState::Released`] and skips its work. No double-kill, no
/// use-after-free.
///
/// Used both by [`TerminalSession::release`] (ACP-driven path) and
/// the [`Drop`] impl below (panic / unexpected-shutdown path).
fn release_sync(shared: &SessionShared) {
    // Kill the child first so the kernel delivers EOF to the reader
    // thread via the slave-close path; otherwise dropping the master
    // alone races against a stubborn child.
    {
        let mut guard = lock_with_poison_log(&shared.child, "child");
        if let ChildState::Running(c) = &mut *guard {
            #[cfg(unix)]
            if let Some(pid) = c.process_id()
                && pid > 0
            {
                // SAFETY: portable-pty calls `setsid()` in pre_exec so
                // pid == pgid; negating it targets the whole process
                // group with the canonical POSIX group-signal idiom.
                // SIGKILL is delivered immediately and cannot be
                // caught/ignored even by debugger-attached children
                // (modulo the always-residual ptrace edge case).
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            // portable-pty fallback: SIGHUP on Unix, TerminateProcess
            // on Windows. On Unix this is belt-and-suspenders after
            // SIGKILL above; on Windows it is the primary signal.
            let _ = c.kill();
            // Drive the poll to reap the zombie, if any.
            let _ = poll_child_exit(&mut guard);
        }
        *guard = ChildState::Released;
    }
    {
        let mut guard = lock_with_poison_log(&shared.master, "master");
        *guard = None;
    }
}

impl Drop for AgentTerminalSession {
    fn drop(&mut self) {
        // If the ACP-driven `release()` already ran, this is a cheap
        // no-op (Option::take leaves None, ChildState::Released stays).
        // If the runtime task panicked before getting a chance to call
        // release, this is the last guard against a leaked master fd
        // and a reader thread blocked on read().
        release_sync(&self.shared);
    }
}

/// Try to reap the child. Returns `Some(status)` once and only once
/// per child -- subsequent calls return the cached exit status from
/// `ChildState::Exited`.
fn poll_child_exit(state: &mut ChildState) -> Option<TerminalExitStatus> {
    match state {
        ChildState::Exited(es) => Some(es.clone()),
        ChildState::Released => None,
        ChildState::Running(c) => match c.try_wait() {
            Ok(Some(status)) => {
                let es = exit_status_from_portable(&status);
                *state = ChildState::Exited(es.clone());
                Some(es)
            }
            _ => None,
        },
    }
}

fn exit_status_from_portable(status: &portable_pty::ExitStatus) -> TerminalExitStatus {
    let mut es = TerminalExitStatus::new();
    es.exit_code = Some(status.exit_code());
    es
}

static NEXT_TERMINAL_ID: AtomicU64 = AtomicU64::new(1);

fn next_terminal_id() -> u64 {
    NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed)
}

/// Unused; keeps the dependency graph honest. Some integration
/// tests want to construct a registry directly without the spawner.
#[allow(dead_code)]
fn _ensure_dep_graph_complete() -> HashMap<TerminalId, TerminalExitStatus> {
    HashMap::new()
}

#[cfg(test)]
mod strip_ansi_tests {
    use super::strip_ansi_bytes;

    #[test]
    fn plain_text_unchanged() {
        assert_eq!(strip_ansi_bytes(b"hello world\n"), "hello world\n");
    }

    #[test]
    fn csi_color_sequence_removed() {
        // `cargo` colours errors with `\x1b[1;31m...\x1b[0m`.
        let input = b"\x1b[1;31merror\x1b[0m: bad";
        assert_eq!(strip_ansi_bytes(input), "error: bad");
    }

    #[test]
    fn cursor_movement_csi_removed() {
        // Progress bars (npm, tqdm) use `\x1b[2K\x1b[1A`.
        let input = b"\x1b[2K\x1b[1AOK\n";
        assert_eq!(strip_ansi_bytes(input), "OK\n");
    }

    #[test]
    fn osc_title_bel_terminated() {
        // Shell prompt setting window title: `\x1b]0;title\x07`.
        let input = b"before\x1b]0;dir\x07after";
        assert_eq!(strip_ansi_bytes(input), "beforeafter");
    }

    #[test]
    fn osc_title_st_terminated() {
        // Strict OSC termination uses ST (`\x1b\\`) instead of BEL.
        let input = b"before\x1b]2;t\x1b\\after";
        assert_eq!(strip_ansi_bytes(input), "beforeafter");
    }

    #[test]
    fn dcs_sequence_removed() {
        let input = b"x\x1bPdata\x1b\\y";
        assert_eq!(strip_ansi_bytes(input), "xy");
    }

    #[test]
    fn structural_whitespace_preserved() {
        let input = b"a\tb\rc\nd\x08e";
        assert_eq!(strip_ansi_bytes(input), "a\tb\rc\nd\x08e");
    }

    #[test]
    fn bell_and_other_c0_dropped() {
        // BEL (0x07) outside an OSC must be dropped, but a `\n` survives.
        let input = b"hi\x07!\nthere";
        assert_eq!(strip_ansi_bytes(input), "hi!\nthere");
    }

    #[test]
    fn truncated_csi_does_not_panic() {
        // Buffer truncated mid-CSI — must not panic, must not emit garbage.
        let input = b"good\x1b[38;5;";
        let out = strip_ansi_bytes(input);
        assert_eq!(out, "good");
    }

    #[test]
    fn lone_trailing_esc_dropped() {
        assert_eq!(strip_ansi_bytes(b"x\x1b"), "x");
    }

    #[test]
    fn utf8_continuation_bytes_kept() {
        // `é` = 0xC3 0xA9 — neither byte is in the ASCII printable range
        // but both must survive to keep UTF-8 decode valid.
        let input = "café".as_bytes();
        assert_eq!(strip_ansi_bytes(input), "café");
    }

    #[test]
    fn charset_designator_removed() {
        // VT100 charset switch: `\x1b(B` selects ASCII into G0.
        let input = b"x\x1b(By";
        assert_eq!(strip_ansi_bytes(input), "xy");
    }

    #[test]
    fn realistic_cargo_output_cleaned() {
        let input = b"\x1b[1m\x1b[32m   Compiling\x1b[0m paneflow v0.3.2\n\x1b[1m\x1b[32m    Finished\x1b[0m `dev` profile";
        let out = strip_ansi_bytes(input);
        assert_eq!(
            out,
            "   Compiling paneflow v0.3.2\n    Finished `dev` profile"
        );
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;
    use portable_pty::{PtySize, native_pty_system};

    /// US-009: dropping an `AgentTerminalSession` without first calling
    /// `release()` must still close the master fd and mark the child
    /// state as terminal. Verified by state observation -- the master
    /// `Option` flips to `None` (its `Box<dyn MasterPty>` is dropped,
    /// closing the underlying fd) and the child mutex holds `Released`.
    #[test]
    fn agent_terminal_drop_releases_master_fd() {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let shared = Arc::new(SessionShared {
            buffer: Mutex::new(OutputBuffer::with_limit(1024)),
            // No child spawned -- the master-close half is what this
            // test guards. The Running -> Released transition is
            // already exercised by the production release() path.
            child: Mutex::new(ChildState::Released),
            master: Mutex::new(Some(pty.master)),
        });
        let inspect = Arc::clone(&shared);

        // Pre-drop sanity: master is Some.
        assert!(
            inspect.master.lock().expect("master lock").is_some(),
            "fixture should start with a live master fd",
        );

        let session = AgentTerminalSession {
            id: TerminalId::from("test-agent-term-drop".to_string()),
            shared,
        };
        drop(session);

        assert!(
            inspect.master.lock().expect("master lock").is_none(),
            "Drop must close the master fd via Option::take",
        );
        assert!(
            matches!(
                &*inspect.child.lock().expect("child lock"),
                ChildState::Released
            ),
            "Drop must mark child state Released",
        );
    }

    /// US-009: calling `release_sync` after `Drop` has already run is a
    /// safe no-op -- the `Option::take` discipline on `master` and the
    /// `ChildState::Released` terminal state make a second call cheap.
    #[test]
    fn release_sync_is_idempotent() {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        let shared = Arc::new(SessionShared {
            buffer: Mutex::new(OutputBuffer::with_limit(1024)),
            child: Mutex::new(ChildState::Released),
            master: Mutex::new(Some(pty.master)),
        });
        release_sync(&shared);
        release_sync(&shared); // must not panic, must not double-close
        assert!(shared.master.lock().expect("master lock").is_none());
    }

    /// US-009 (review follow-up): dropping a session whose child is
    /// `ChildState::Running` must actually terminate the child --
    /// closing the master alone does NOT EOF the reader thread (the
    /// cloned reader fd is independent), so the kill is load-bearing.
    /// This test spawns a long-lived `sleep` process, drops the
    /// session, and asserts the child exits within a short deadline.
    #[cfg(unix)]
    #[test]
    fn agent_terminal_drop_kills_running_child() {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");
        // A 60-second sleep is plenty for the deadline below and short
        // enough that a CI runner leaking it would still time out the
        // job rather than hang forever.
        let mut cmd = CommandBuilder::new("sleep");
        cmd.arg("60");
        let child = pty.slave.spawn_command(cmd).expect("spawn sleep");
        // Reaper handle clones don't outlive `child`, so we read the
        // pid up front for the post-drop liveness check.
        let pid = child.process_id().expect("child pid");
        let shared = Arc::new(SessionShared {
            buffer: Mutex::new(OutputBuffer::with_limit(1024)),
            child: Mutex::new(ChildState::Running(child)),
            master: Mutex::new(Some(pty.master)),
        });
        let inspect = Arc::clone(&shared);

        let session = AgentTerminalSession {
            id: TerminalId::from("test-agent-term-drop-kill".to_string()),
            shared,
        };
        drop(session);

        // The child mutex must have transitioned to Released.
        assert!(
            matches!(
                &*inspect.child.lock().expect("child lock"),
                ChildState::Released
            ),
            "Drop must transition child state to Released",
        );

        // The OS must agree the pid is gone. We give the kernel a
        // short grace period to reap; SIGKILL is delivered
        // synchronously but the post-mortem cleanup can take a few ms.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            // `kill(pid, 0)` probes liveness without actually
            // signalling. ESRCH (no such process) is the success
            // condition.
            // SAFETY: kill(0) is documented as side-effect-free.
            let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
            if !alive {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!(
            "Drop did not terminate child pid {pid} within the 3s deadline -- \
             SIGKILL escalation in release_sync is broken",
        );
    }

    /// US-002 (cli-hardening-followup-2026-Q3): the wait-loop must
    /// surface `TerminalError::Timeout` when `poll` keeps returning
    /// `None` past the deadline. A real SIGKILL-raced child would
    /// reach this branch (the kernel reaped the zombie before we
    /// could observe it). The production deadline is 30 s; the test
    /// drives a 50 ms deadline to keep `cargo test` fast.
    #[test]
    fn wait_for_exit_times_out_on_stuck_child() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let calls = AtomicU32::new(0);
        let start = std::time::Instant::now();
        let result = super::wait_for_exit_loop(
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                None
            },
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(1),
            Some(424242),
        );
        let elapsed = start.elapsed();
        match result {
            Err(super::TerminalError::Timeout {
                operation,
                elapsed_secs,
            }) => {
                assert_eq!(operation, "wait_for_exit");
                // 50 ms deadline truncates to 0 s in u64 — the
                // contract is that the value matches `elapsed_secs`
                // at the moment of the trip, which is < 1 here.
                assert_eq!(elapsed_secs, 0);
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
        // The poll loop must have fired at least a handful of times
        // before the deadline tripped -- guards against a regression
        // that short-circuits before the first poll.
        assert!(calls.load(Ordering::SeqCst) >= 3);
        // The loop must not run substantially past the deadline.
        // 250 ms is generous for slow CI runners.
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "loop ran for {elapsed:?}, deadline was 50ms",
        );
    }

    /// US-002 (cli-hardening-followup-2026-Q3): the happy path must
    /// still return `Ok(status)` when the child exits before the
    /// deadline trips.
    #[test]
    fn wait_for_exit_returns_ok_when_child_exits() {
        use super::TerminalExitStatus;
        use std::sync::atomic::{AtomicU32, Ordering};

        let calls = AtomicU32::new(0);
        let result = super::wait_for_exit_loop(
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                // Exit on the third poll so we know the loop ran
                // more than once.
                if n >= 2 {
                    Some(TerminalExitStatus::new().exit_code(0u32))
                } else {
                    None
                }
            },
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(1),
            None,
        );
        let es = result.expect("happy path must return Ok");
        assert_eq!(es.exit_code, Some(0));
    }
}
