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
        // Lossy conversion: if the agent emits raw bytes that are
        // not valid UTF-8 (unusual for typical shells), we return
        // the best-effort decode instead of failing.
        let s = String::from_utf8_lossy(&self.bytes).into_owned();
        (s, self.truncated)
    }
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
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
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
                    // SIGTERM = 15.
                    // SAFETY: libc::kill is FFI but inert with a
                    // self-process pid; we pass an already-validated
                    // child pid.
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
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
                }
                let _ = pid_opt; // suppress unused on Windows
                // Fallback: SIGKILL via portable-pty.
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
