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
                            let mut guard = match buf_handle.buffer.lock() {
                                Ok(g) => g,
                                Err(p) => p.into_inner(),
                            };
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
            let exit_status = match shared.child.lock() {
                Ok(mut guard) => poll_child_exit(&mut guard),
                Err(p) => poll_child_exit(&mut p.into_inner()),
            };
            let (output, truncated) = match shared.buffer.lock() {
                Ok(g) => g.snapshot(),
                Err(p) => p.into_inner().snapshot(),
            };
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
            // Block the tokio task on a dedicated blocking thread so
            // the single-threaded runtime can keep servicing other
            // commands. Poll cadence is 100 ms -- ACP agents tend to
            // tolerate this since `wait_for_exit` is a one-shot.
            tokio::task::spawn_blocking(move || {
                loop {
                    let exit = {
                        let mut guard = match shared.child.lock() {
                            Ok(g) => g,
                            Err(p) => p.into_inner(),
                        };
                        poll_child_exit(&mut guard)
                    };
                    if let Some(es) = exit {
                        return Ok(es);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
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
                    let guard = match shared.child.lock() {
                        Ok(g) => g,
                        Err(p) => p.into_inner(),
                    };
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
                        let exited = {
                            let mut guard = match shared.child.lock() {
                                Ok(g) => g,
                                Err(p) => p.into_inner(),
                            };
                            poll_child_exit(&mut guard).is_some()
                        };
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
                let mut guard = match shared.child.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
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
            // Drop the master fd to close the PTY; the reader thread
            // will see EOF and exit cleanly.
            {
                let mut guard = match shared.master.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                *guard = None;
            }
            // If the child has not exited yet, kill it.
            {
                let mut guard = match shared.child.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                if let ChildState::Running(c) = &mut *guard {
                    let _ = c.kill();
                }
                *guard = ChildState::Released;
            }
            Ok(())
        })
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
