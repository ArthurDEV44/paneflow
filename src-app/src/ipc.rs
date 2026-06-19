//! JSON-RPC socket server for AI agent control.
//!
//! Listens on `<runtime_dir>/paneflow/paneflow.sock` on Unix and
//! `\\.\pipe\paneflow` on Windows (US-009). Each connection reads
//! newline-delimited JSON-RPC requests and writes responses.
//!
//! Cross-platform IPC is handled by the `interprocess` crate's
//! `local_socket` module, which dispatches to Unix domain sockets on
//! POSIX and named pipes on Windows transparently. The wire protocol
//! (newline-delimited JSON-RPC 2.0) is byte-identical across platforms.
//!
//! ## Trust model — local-only, owner-UID enforcement (US-010)
//!
//! The IPC server is **strictly local**: it has no network surface,
//! no port binding, no remote identity. Trust derives entirely from
//! filesystem and kernel-credential boundaries:
//!
//! - **Socket file mode 0600** (Unix): set immediately after bind in
//!   `bind_socket`. Non-owner processes on the same machine cannot
//!   `connect()` past the kernel filesystem check.
//! - **Peer-UID enforcement** (Unix): every accepted connection runs
//!   `getsockopt(SO_PEERCRED)` (Linux) / `LOCAL_PEERCRED` (macOS) and
//!   compares the peer's UID to the server's. A mismatch returns a
//!   JSON-RPC `-32001 permission denied` error envelope and closes
//!   the stream BEFORE any method dispatches. Defence-in-depth — if a
//!   privileged third party bypasses the file-mode check
//!   (e.g. CAP_DAC_OVERRIDE, mode-fixing automation), the kernel
//!   credential check still rejects them.
//! - **Windows** uses Named Pipes whose default DACL grants only the
//!   owning user + LocalSystem + Administrators. SDDL hardening is
//!   deferred (cf. `prd-stabilization-2026-q2.md` §10 out-of-scope).
//!
//! No HMAC tokens, no TLS — both would add complexity without
//! meaningful gain on a local-only socket. If the IPC ever grows a
//! network surface, that decision must be revisited.
//!
//! ## Per-method blast radius (US-012 cli-hardening-followup-2026-Q3)
//!
//! The trust model above gates *who* can connect (same-UID only). It
//! does NOT gate *what* an authorised client can do. The methods
//! below carry different blast radii once connected:
//!
//! - `system.*`: read-only health checks. Safe.
//! - `workspace.list` / `workspace.current` / `workspace.select` /
//!   `workspace.close`: navigation + workspace lifecycle. Visible
//!   side effects on the UI; no file/system mutation.
//! - `workspace.create`: spawns a PTY at `cwd`. `cwd` is
//!   canonicalised (US-014) and rejected if not a directory.
//! - `surface.split`: layout mutation, bounded by `MAX_PANES`.
//! - **`surface.send_text` / `surface.send_keystroke`: same-UID RCE
//!   primitive when enabled.** A connected client can inject
//!   arbitrary bytes (including `\n`) into any visible PTY,
//!   effectively running any shell command in the user's
//!   privileges. These are gated behind the
//!   `PANEFLOW_IPC_SCRIPTING=1` opt-in env var; when unset (the
//!   default), the handlers return JSON-RPC error
//!   `-32601 Method not enabled`. The intended consumer is the
//!   trusted same-UID `paneflow-ai-hook` binary; the wrapper
//!   installer can set the env var on the user's behalf with a
//!   visible prompt. `surface.send_keystroke` additionally
//!   rejects CRLF bytes regardless of the opt-in (CRLF injection
//!   bypass guard).
//! - `ai.*`: lifecycle telemetry from the AI hook. Read-only on
//!   the host UI side; safe.
//!
//! ## Methods
//!
//! - `system.ping` / `system.capabilities` / `system.identify` — stateless
//!   health checks handled directly on the socket thread.
//! - `workspace.list` / `workspace.current` / `workspace.select` /
//!   `workspace.close` — workspace navigation.
//! - `workspace.create` — accepts `name` (string, default "Terminal"),
//!   `cwd` (string path, optional) and `layout` (optional `LayoutNode`
//!   JSON, US-001). When `layout` is present, the new workspace's pane
//!   tree is built from the layout in a single round-trip; when absent,
//!   behavior is unchanged (a single default pane). A malformed `layout`
//!   payload returns the JSON-RPC `-32602 Invalid params` error envelope
//!   and leaves no orphan workspace behind.
//! - `workspace.restore_layout` — apply a `LayoutNode` to the active
//!   workspace (used by session restore).
//! - `surface.list` / `surface.send_text` / `surface.send_keystroke` /
//!   `surface.split` — pane operations.
//! - `ai.session_start` / `ai.prompt_submit` / `ai.tool_use` /
//!   `ai.notification` / `ai.stop` / `ai.exit` / `ai.session_end` — AI
//!   hook lifecycle (`ai.exit` carries the wrapped agent binary's real
//!   exit status, EP-004 US-010).
//!
//! Handlers may return a structured JSON-RPC error by emitting the
//! `_jsonrpc_error` sentinel (see `app::ipc_handler::JsonRpcError`); the
//! dispatcher promotes it to a proper `error` envelope. Legacy
//! application errors returned as `{"error": "string"}` continue to flow
//! through the `result` field for backward compatibility.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

use interprocess::TryClone;
use interprocess::local_socket::{GenericFilePath, Listener, ListenerOptions, Stream, prelude::*};
// `ListenerNonblockingMode` is only referenced by the Unix-only clobber-
// detection accept loop; gating the import keeps the Windows build warning-free.
#[cfg(unix)]
use interprocess::local_socket::ListenerNonblockingMode;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// IPC request type — sent from socket thread to GPUI thread
// ---------------------------------------------------------------------------

pub struct IpcRequest {
    pub method: String,
    pub params: Value,
    pub _id: Value,
    pub response_tx: mpsc::Sender<Value>,
    /// U-053: set by the socket thread when it gives up waiting (the 5 s
    /// dispatch timeout fired and the client already got an error). The GPUI
    /// consumer checks this before running the handler so a slow non-idempotent
    /// mutation (workspace.create, surface.split) can't execute after the
    /// client gave up — otherwise a client retry would create duplicate
    /// workspaces/panes.
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IpcState {
    Online,
    Disabled,
}

const IPC_STATE_ONLINE: u8 = 0;
const IPC_STATE_DISABLED: u8 = 1;

/// US-022: hard cap on the bytes a single newline-delimited request may
// US-013: JSON-RPC framing ceiling, centralized (see `crate::limits`). Still
// accessible as `super::MAX_REQUEST_LEN` from the tests submodule via this use.
use crate::limits::MAX_REQUEST_LEN;

/// US-022: ceiling on concurrently-served IPC connections. The accept loop
/// spawns one blocking thread per connection; without a cap a same-UID peer
/// opening connections in a loop fans out unbounded OS threads. Beyond this,
/// new connections are refused with backpressure (`-32000`) and closed.
const MAX_CONCURRENT_CONNECTIONS: usize = 16;

/// US-022: idle read deadline per connection. A peer that opens a connection
/// and then sends nothing (or stops mid-stream) otherwise pins its handler
/// thread forever. Enforced at the OS level via `set_recv_timeout`. Generous
/// enough never to cut a real request (clients send immediately on connect
/// and use one connection per request).
const IPC_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub(crate) struct IpcStatus {
    state: Arc<AtomicU8>,
}

impl IpcStatus {
    fn online() -> Self {
        Self {
            state: Arc::new(AtomicU8::new(IPC_STATE_ONLINE)),
        }
    }

    pub(crate) fn state(&self) -> IpcState {
        match self.state.load(Ordering::Acquire) {
            IPC_STATE_DISABLED => IpcState::Disabled,
            _ => IpcState::Online,
        }
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.state() == IpcState::Disabled
    }

    fn disable(&self) {
        self.state.store(IPC_STATE_DISABLED, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Socket server
// ---------------------------------------------------------------------------

/// Start the IPC server on a dedicated OS thread.
/// Returns the receiver for IPC requests to be polled by the GPUI thread.
///
/// On Unix, the server monitors the socket file on disk and automatically
/// re-binds when another instance (e.g. `cargo run`) clobbers it. Without
/// this, the listener becomes orphaned (wrong inode) and all new connections
/// get `ECONNREFUSED`, silently disabling AI hook integration. Named pipes
/// on Windows have different lifecycle semantics (the second process to
/// claim the pipe name fails at creation, not silently), so the clobber
/// detection is Unix-only.
pub fn start_server() -> (
    mpsc::Receiver<IpcRequest>,
    IpcStatus,
    Arc<crate::ipc_events::EventBus>,
) {
    // US-012 (cli-hardening-followup-2026-Q3): one-time boot-time
    // warn-log when scripting is enabled. The per-call gate in
    // `surface.send_text` / `surface.send_keystroke` stays the
    // enforcement boundary; this log surfaces the active-RCE-primitive
    // posture in `paneflow-debug.log` so the operator notices when
    // PANEFLOW_IPC_SCRIPTING was inherited from a launcher script or
    // sourced .env file without their realising.
    if std::env::var("PANEFLOW_IPC_SCRIPTING").as_deref() == Ok("1") {
        tracing::warn!(
            "ipc.scripting_enabled is ON; any same-UID process can inject keystrokes into agent panes"
        );
    }

    let (tx, rx) = mpsc::channel();
    let status = IpcStatus::online();
    let thread_status = status.clone();

    // EP-002 (agent-control-plane): the outbound event bus. One handle stays in
    // start_server to be returned to the GPUI app (it broadcasts); a clone moves
    // into the IPC thread so each accepted connection can register a subscriber.
    let event_bus = crate::ipc_events::EventBus::new();
    let thread_event_bus = Arc::clone(&event_bus);

    // Singleton guard: probe the socket BEFORE the IPC thread spawns and
    // before `bind_socket` blindly `remove_file`s any existing socket. If
    // another live Paneflow instance is already listening, two parallel
    // processes will otherwise enter an endless mutual clobber loop —
    // each detects the other's rebind at the next 5 s health check, drops
    // its listener, and re-creates the file, perpetuating the cycle.
    // During every micro-window between drop and re-create, the AI shim's
    // `connect()` fails, an IPC message is silently lost, and a session's
    // `Thinking` / `Done` / `session_start` status stays stale forever.
    //
    // Escape hatch: `PANEFLOW_ALLOW_MULTIPLE=1` skips the guard for the
    // rare case of intentional side-by-side debug instances. Tests do
    // not call `start_server`, so they are unaffected.
    if std::env::var_os("PANEFLOW_ALLOW_MULTIPLE").is_none()
        && let Some(path) = socket_path()
        && let Some(info) = detect_existing_instance(&path)
    {
        eprintln!(
            "paneflow: another Paneflow instance is already running on {}.\n\
             Existing instance: {}\n\
             Close the open window first, or set PANEFLOW_ALLOW_MULTIPLE=1 to override.",
            path.display(),
            info
        );
        log::error!(
            "singleton guard: refusing to start; existing instance on {} ({})",
            path.display(),
            info
        );
        std::process::exit(1);
    }

    // US-005 (cli-hardening-followup-2026-Q3): the IPC thread spawn
    // is fallible (RLIMIT_NPROC exhaustion on a low-ulimit container,
    // EAGAIN on a fork-bombed host). The previous `.expect()` panicked
    // the GPUI main thread on that error, killing every active agent.
    // Mirror the runtime spawn pattern at `runtime.rs:1022-1034`:
    // log + return the `rx` early with no live producer; the consumer
    // is now responsible for tolerating a never-firing channel
    // (it does -- the GPUI poll path checks `try_recv` non-blocking).
    let spawn_result = std::thread::Builder::new()
        .name("paneflow-ipc".into())
        .spawn(move || {
            let Some(socket_path) = socket_path() else {
                thread_status.disable();
                log::warn!(
                    "paneflow: could not resolve a usable IPC socket path — IPC server disabled. \
                     See earlier runtime_paths warnings for the specific cause."
                );
                return;
            };

            // Only Unix needs the containing directory to exist — the
            // Windows named-pipe path lives in the kernel namespace, not
            // the filesystem.
            #[cfg(unix)]
            if let Some(parent) = socket_path.parent() {
                let _ = std::fs::create_dir_all(parent);
                // Lock the socket's containing dir to the owner. Under
                // $XDG_RUNTIME_DIR this already holds, but the fallback chain
                // ($TMPDIR / ~/.cache/run) can land in a world-traversable
                // /tmp — 0700 stops other local users from reaching the socket
                // at all (defense-in-depth atop the socket's own 0600 +
                // SO_PEERCRED).
                use std::os::unix::fs::PermissionsExt as _;
                let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
            }

            let listener = match bind_socket(&socket_path) {
                Some(l) => l,
                None => {
                    thread_status.disable();
                    return;
                }
            };

            #[cfg(unix)]
            let mut our_ino = socket_inode(&socket_path).unwrap_or(0);
            #[cfg(unix)]
            let mut last_health_check = std::time::Instant::now();
            #[cfg(unix)]
            let mut listener = listener;
            #[cfg(not(unix))]
            let listener = listener;

            // Non-blocking accept is UNIX-ONLY: it lets the loop periodically
            // re-verify the socket inode (clobber detection) without starving
            // connections. On Windows there is no inode/clobber check, and
            // `interprocess`'s non-blocking named-pipe accept mismanages pipe
            // instances — the accepted `Stream` does not correspond to the
            // client that actually wrote, so the handler's read blocks forever
            // (the frame is "sent OK" by the hook but never delivered to the
            // server read), and that blocked read aborts the whole process at
            // shutdown (STATUS_STACK_BUFFER_OVERRUN, observed in the field).
            // BLOCKING accept on Windows matches the reliable MockServer used
            // by the ai-hook integration suite: accept() waits in the kernel on
            // a single pending instance, so every client's data is delivered to
            // exactly the handler that accepted it. Stream I/O is blocking on
            // both platforms so `handle_connection` can use plain `BufRead`.
            #[cfg(unix)]
            listener
                .set_nonblocking(ListenerNonblockingMode::Accept)
                .ok();

            // US-022: bound the number of concurrently-served connections so a
            // peer opening sockets in a loop can't fan out unbounded threads.
            // Only this (single) accept thread increments; handler threads
            // decrement via the RAII guard below, so the load is exact.
            let active_connections = Arc::new(AtomicUsize::new(0));

            // Decrement the live-connection count on any handler exit path
            // (return, EOF, panic-unwind). Hoisted out of the spawn closure so
            // it can be constructed BEFORE the spawn and moved in: if the spawn
            // itself fails, the closure (and this guard) is dropped, running the
            // decrement and restoring the slot the `fetch_add` below claimed.
            struct ActiveGuard(Arc<AtomicUsize>);
            impl Drop for ActiveGuard {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::AcqRel);
                }
            }

            loop {
                match listener.accept() {
                    Ok(stream) => {
                        if active_connections.load(Ordering::Acquire) >= MAX_CONCURRENT_CONNECTIONS
                        {
                            reject_overloaded(stream);
                            continue;
                        }
                        active_connections.fetch_add(1, Ordering::AcqRel);
                        let guard = ActiveGuard(Arc::clone(&active_connections));
                        let tx = tx.clone();
                        let bus = Arc::clone(&thread_event_bus);
                        // EP-001 US-005 parity: use the fallible `Builder::spawn`,
                        // never the panicking `thread::spawn`. Under
                        // RLIMIT_NPROC / EAGAIN the latter panics and unwinds
                        // this accept thread, silently killing the IPC server
                        // (AI-hook status + MCP bridge go dark while the status
                        // flag still reads Online). On the `Err` path the moved
                        // `guard` and `stream` are dropped here -- the count is
                        // restored and the connection closed -- and the loop
                        // keeps accepting.
                        if let Err(e) = std::thread::Builder::new()
                            .name("paneflow-ipc-conn".into())
                            .spawn(move || {
                                let _guard = guard;
                                handle_connection(stream, tx, bus);
                            })
                        {
                            log::warn!(
                                "IPC: handler thread spawn failed ({e}); dropping this \
                                 connection. Check `ulimit -u` / container thread limits."
                            );
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No pending connection — brief sleep to avoid busy-spin
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => {
                        thread_status.disable();
                        log::error!("IPC accept error: {e}");
                        break;
                    }
                }

                // Every 5 seconds, verify our socket file hasn't been
                // clobbered (Unix inode check). Skipped on Windows: named
                // pipes don't have inodes and a concurrent `CreateNamedPipe`
                // fails loudly rather than silently orphaning us.
                #[cfg(unix)]
                if last_health_check.elapsed() >= Duration::from_secs(5) {
                    last_health_check = std::time::Instant::now();
                    let current_ino = socket_inode(&socket_path).unwrap_or(0);
                    if current_ino != our_ino {
                        log::warn!(
                            "IPC socket clobbered (inode {} → {}), re-binding",
                            our_ino,
                            current_ino
                        );
                        drop(listener);
                        match bind_socket(&socket_path) {
                            Some(l) => {
                                l.set_nonblocking(ListenerNonblockingMode::Accept).ok();
                                listener = l;
                                our_ino = socket_inode(&socket_path).unwrap_or(0);
                            }
                            None => {
                                thread_status.disable();
                                return;
                            }
                        }
                    }
                }
            }

            // interprocess' auto name reclamation unlinks the socket file
            // on `Listener::drop` for Unix; this explicit remove is a
            // belt-and-braces no-op there and never runs on Windows
            // (nothing to remove in the named-pipe namespace).
            #[cfg(unix)]
            let _ = std::fs::remove_file(&socket_path);
        });
    if let Err(e) = spawn_result {
        status.disable();
        tracing::error!(
            "IPC disabled: paneflow-ipc thread spawn failed: {e}. \
             Check `ulimit -u` / container thread limits. \
             External clients (paneflow-ai-hook) will not connect."
        );
        // `tx` was moved into the closure regardless of spawn outcome,
        // so on error the closure (and its captured `tx`) is dropped
        // here. The receiver `rx` then sees `Err(Disconnected)` on
        // every subsequent `try_recv`. The consumer at
        // `app/ipc_handler.rs:109` uses
        // `while let Ok(req) = self.ipc_rx.try_recv()` so both `Empty`
        // and `Disconnected` resolve to "no IPC work this tick" -- the
        // app runs normally, only external IPC clients can't reach it.
    }

    (rx, status, event_bus)
}

/// Bind a new listener at the given path/pipe name.
fn bind_socket(socket_path: &std::path::Path) -> Option<Listener> {
    // Unix: remove any stale socket file from a crashed prior run. The
    // interprocess crate's name reclamation handles graceful shutdown;
    // this pre-clean covers `kill -9` / SIGKILL / crash paths.
    // Windows: no-op; the kernel pipe namespace does not retain stale
    // entries after the owning process exits.
    #[cfg(unix)]
    let _ = std::fs::remove_file(socket_path);

    let name = match socket_path.to_fs_name::<GenericFilePath>() {
        Ok(n) => n,
        Err(e) => {
            log::error!(
                "Failed to build IPC socket name for {}: {e}",
                socket_path.display()
            );
            return None;
        }
    };

    let listener = match ListenerOptions::new().name(name).create_sync() {
        Ok(l) => l,
        Err(e) => {
            log::error!(
                "Failed to bind IPC socket at {}: {e}",
                socket_path.display()
            );
            return None;
        }
    };

    // chmod 0o600 — Unix only. Named pipes on Windows use ACLs; the
    // default DACL from `CreateNamedPipe` grants access to LocalSystem,
    // Administrators, and the owning user only, which matches the intent
    // of 0o600. A custom SecurityDescriptor could be set via
    // `ListenerOptions::security_descriptor` if we ever need to lock it
    // down further, but v1 accepts the default.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // U-031: the 0600 mode is the PRIMARY trust boundary (peer-UID is
        // defence-in-depth). If chmod fails, the socket keeps its umask-derived
        // creation mode — possibly group/world-connectable — so fail closed:
        // remove the socket and refuse to serve rather than expose it.
        if let Err(e) =
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        {
            log::error!(
                "IPC server: failed to chmod socket {} to 0600 ({e}); refusing to serve",
                socket_path.display()
            );
            let _ = std::fs::remove_file(socket_path);
            return None;
        }
    }
    log::info!("IPC server listening on {}", socket_path.display());
    Some(listener)
}

/// Get the inode number of a filesystem path (0 if the file doesn't exist).
/// Unix-only: used by the clobber-detection health check.
#[cfg(unix)]
fn socket_inode(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.ino())
}

/// Probe `socket_path` to determine whether another live Paneflow instance
/// is already serving on it.
///
/// Returns `Some(identity_string)` if a `system.identify` round-trip
/// succeeds and the response advertises `"PaneFlow"` — the caller must
/// refuse to start. Returns `None` for any other outcome (missing file,
/// stale socket from a SIGKILL'd prior run, non-Paneflow listener, parse
/// failure, timeout) — the caller can safely proceed to `bind_socket`'s
/// existing remove-then-rebind path.
///
/// Resilient to the rebind race window: the legacy `bind_socket` recreates
/// the socket on every 5 s clobber-detection tick, and during the few-ms
/// window between `drop(listener)` and `create_sync()` a `connect()` would
/// spuriously return `ECONNREFUSED`. We retry up to 3 times with a short
/// inter-attempt sleep to cross that window deterministically.
///
/// Once this guard is universally deployed, the rebind loop never starts
/// (the second instance exits before bind), so the multi-attempt is
/// belt-and-braces for the transition period and for SIGKILL recovery
/// races where the OS hasn't yet released the file.
fn detect_existing_instance(socket_path: &std::path::Path) -> Option<String> {
    // Fast bail-out: no socket file at all = definitely no instance.
    // Avoids the connect overhead in the common cold-start case.
    #[cfg(unix)]
    if !socket_path.exists() {
        return None;
    }

    let name = socket_path.to_fs_name::<GenericFilePath>().ok()?;

    for attempt in 0..3 {
        if attempt > 0 {
            // Cross the legacy rebind window. The bind_socket recreate
            // path is bounded by `remove_file` + `create_sync` + chmod —
            // typically well under 10 ms; 70 ms is a comfortable margin.
            std::thread::sleep(Duration::from_millis(70));
        }

        let Ok(mut stream) = Stream::connect(name.clone()) else {
            continue;
        };

        // US-022: bound the probe at the OS level (`set_recv_timeout`, same
        // mechanism as the bridge client) instead of a scratch thread that
        // leaked on every timeout. 300 ms is generous for a stateless
        // socket-thread handler; a live but unresponsive process within that
        // budget is functionally indistinguishable from "no peer" and we
        // proceed to bind. A hostile squatter on the path can neither stall us
        // (the deadline) nor feed us an unbounded line (the `take` cap).
        if stream
            .set_recv_timeout(Some(Duration::from_millis(300)))
            .is_err()
        {
            continue;
        }

        // Stateless ping handled directly on the peer's socket thread
        // (see `handle_connection`), so a live instance responds in
        // microseconds without any GPUI round-trip.
        if stream
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"system.identify\"}\n")
            .is_err()
        {
            continue;
        }
        let _ = stream.flush();

        let mut line = String::new();
        if BufReader::new(stream)
            .take(MAX_REQUEST_LEN)
            .read_line(&mut line)
            .is_err()
        {
            continue;
        }

        // The `system.identify` result includes `"name":"PaneFlow"` (see
        // `handle_connection`). Match on the literal so a non-Paneflow
        // listener squatting on the same path doesn't pin us to exit —
        // we'd rather clobber an unknown squatter than refuse to start.
        if line.contains("\"PaneFlow\"") {
            return Some(line.trim().to_string());
        }
    }

    None
}

/// Outcome of one capped request read (US-022).
#[derive(Debug, PartialEq, Eq)]
enum LineRead {
    /// Clean end of stream.
    Eof,
    /// The line reached `MAX_REQUEST_LEN` without a newline — oversized.
    TooLong,
    /// A complete (or trailing) line was read into the buffer.
    Got,
}

/// Read one newline-delimited request into `line`, capped at
/// [`MAX_REQUEST_LEN`]. `Take` is rebuilt per call so the limit is per-line;
/// a line that hits the cap without a terminating newline is reported as
/// [`LineRead::TooLong`] rather than allocated unboundedly (the DoS the cap
/// exists to stop). Pure framing logic, unit-tested below.
fn read_capped_line(reader: &mut impl BufRead, line: &mut String) -> std::io::Result<LineRead> {
    line.clear();
    // `by_ref()` reborrows so `Take` owns a `&mut reader`, not `reader` itself
    // (the cap is per-call, and the caller keeps the reader for the next line).
    let n = reader.by_ref().take(MAX_REQUEST_LEN).read_line(line)?;
    if n == 0 {
        return Ok(LineRead::Eof);
    }
    if n as u64 >= MAX_REQUEST_LEN && !line.ends_with('\n') {
        return Ok(LineRead::TooLong);
    }
    Ok(LineRead::Got)
}

/// US-022 backpressure: refuse a connection once the concurrency cap is hit.
/// Writes one JSON-RPC error envelope and drops the stream (closing it) so the
/// peer gets a structured rejection rather than a silent hang.
fn reject_overloaded(mut stream: Stream) {
    let envelope = json!({
        "jsonrpc": "2.0",
        "error": {"code": -32000, "message": "server busy: too many concurrent connections"},
        "id": Value::Null,
    });
    let _ = writeln!(&mut stream, "{}", envelope);
    let _ = stream.flush();
}

fn handle_connection(
    stream: Stream,
    request_tx: mpsc::Sender<IpcRequest>,
    event_bus: Arc<crate::ipc_events::EventBus>,
) {
    // `event_bus` is consumed only by the Unix subscription path; silence the
    // unused-variable warning on Windows where event streaming is stubbed.
    #[cfg(not(unix))]
    let _ = &event_bus;

    // `Stream::try_clone` is provided by `interprocess::TryClone` and
    // works on both Unix domain sockets and Windows named pipes. One
    // handle reads, the other writes, so request/response flow does not
    // fight over a single mutable cursor.
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };

    // US-010: peer-UID enforcement happens BEFORE we wrap `stream` in
    // a BufReader, because the cleanest way to query peer credentials
    // on `interprocess::local_socket::Stream` is the trait method
    // `Stream::peer_creds()` (brought in by `prelude::*`), and that
    // method needs the bare stream — once wrapped in BufReader, the
    // method is no longer reachable through `get_ref()` (BufReader
    // only re-exports `Read`-shaped methods). The check is
    // `#[cfg(unix)]`-only; Windows pipe ACLs cover the same surface
    // (see module doc) and SDDL hardening is deferred per PRD §10.
    // On a peer-cred query failure we fall back to perms-0600 only
    // with a warn log (AC6) — the kernel filesystem check still
    // gates non-owner connects, so the residual exposure is bounded.
    let mut writer = writer_stream;

    #[cfg(unix)]
    {
        match auth::check_peer(&stream) {
            auth::AuthOutcome::Allow => {}
            auth::AuthOutcome::Deny {
                server_uid,
                peer_uid,
            } => {
                let envelope = json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32001,
                        "message": "permission denied: peer UID mismatch"
                    },
                    "id": Value::Null,
                });
                let _ = writeln!(&mut writer, "{}", envelope);
                let _ = writer.flush();
                log::warn!(
                    "IPC: rejecting connection (peer UID {}, server UID {})",
                    peer_uid,
                    server_uid
                );
                return;
            }
            auth::AuthOutcome::DegradedFallback => {
                // AC6: peer-cred query unavailable, perms-0600 stays
                // as the line of defence. Warn-log emitted inside
                // check_peer so the fallback isn't silent.
            }
        }
    }

    // US-022: drop a peer that opens a connection and then goes mute, so it
    // can't pin this handler thread forever. Enforced at the OS level; a
    // best-effort failure leaves the previous (blocking) behavior. NOTE: on
    // Windows named pipes this returns ErrorKind::Unsupported (no idle bound),
    // which is acceptable — clients send a frame immediately on connect.
    let _ = stream.set_recv_timeout(Some(IPC_IDLE_TIMEOUT));

    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        match read_capped_line(&mut reader, &mut line) {
            Ok(LineRead::Eof) => break,
            Ok(LineRead::TooLong) => {
                // US-022: oversized request → structured rejection + close,
                // never an unbounded allocation.
                let envelope = json!({
                    "jsonrpc": "2.0",
                    "error": {"code": -32600, "message": "request exceeds maximum length"},
                    "id": Value::Null,
                });
                let _ = writeln!(&mut writer, "{}", envelope);
                let _ = writer.flush();
                break;
            }
            Ok(LineRead::Got) => {}
            // Idle timeout (WouldBlock on Unix, TimedOut on Windows) or any
            // other read error → drop the connection.
            Err(_) => break,
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // `ai.*` frames from `paneflow-ai-hook` are fire-and-forget: the hook
        // writes one frame and closes its pipe IMMEDIATELY, never reading a
        // reply. Writing a JSON-RPC response back to that already-closed Windows
        // named pipe makes `interprocess`'s overlapped write panic internally,
        // and its `CannotUnwind` guard converts the panic to `abort()` —
        // crashing the WHOLE app (confirmed with a live debugger: the fault was
        // `handle_connection` → `write_all` → interprocess `CannotUnwind::drop`
        // → `std::process::abort`). So suppress the reply for those frames; the
        // hook never reads it. Request/response clients (`paneflow-ipc-client`)
        // keep the pipe open to read, so their replies are written normally.
        let mut suppress_reply = false;
        let response = match serde_json::from_str::<Value>(line) {
            Ok(req) => {
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let method = req
                    .get("method")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = req.get("params").cloned().unwrap_or(json!({}));

                // EP-002 (agent-control-plane): an `events.subscribe` connection
                // STOPS being request/response and becomes a persistent event
                // stream. On Unix it takes over this connection until the client
                // disconnects (bypassing the one-request Windows break below).
                // Windows is handled as a documented error in the match: named-
                // pipe streaming risks an `interprocess` overlapped-write abort
                // on client disconnect that can't be verified from a non-Windows
                // host (tracked EP-002 follow-up).
                #[cfg(unix)]
                if method == "events.subscribe" {
                    serve_subscription(&mut writer, &params, &event_bus);
                    return;
                }

                // Hook-chain diagnostic: confirm the IPC server received the
                // lifecycle frame at all (vs. the hook never connecting). Only
                // `ai.*` frames drive the sidebar status, so scope the log to
                // them to keep the trace readable. No-op unless PANEFLOW_HOOK_LOG.
                if method.starts_with("ai.") {
                    suppress_reply = true;
                    crate::ai_hooks::hook_diag(&format!(
                        "ipc server received {method} (tool={:?} pid={:?} ws={:?})",
                        params.get("tool"),
                        params.get("pid"),
                        params.get("workspace_id"),
                    ));
                }

                // Handle stateless methods directly on the socket thread
                match method.as_str() {
                    "system.ping" => {
                        json!({"jsonrpc": "2.0", "result": {"pong": true}, "id": id})
                    }
                    "system.capabilities" => {
                        json!({"jsonrpc": "2.0", "result": {
                            // EP-003 (orchestration-v2): expose the scripting
                            // gate so `paneflow flow` can refuse a submitting
                            // flow up-front (run AND --dry-run) instead of
                            // failing -32601 on its first send. Same process
                            // as the gate check in the send_* handlers.
                            "scripting": std::env::var("PANEFLOW_IPC_SCRIPTING")
                                .is_ok_and(|v| v == "1"),
                            "methods": [
                                "system.ping", "system.capabilities", "system.identify",
                                "workspace.list", "workspace.create", "workspace.select",
                                "workspace.close", "workspace.current",
                                "workspace.restore_layout", "workspace.up",
                                "surface.list", "surface.read", "surface.search", "surface.rename",
                                "surface.send_text", "surface.send_keystroke", "surface.split",
                                "surface.focus", "surface.status",
                                "fleet.list",
                                "events.subscribe",
                                "ai.session_start",
                                "ai.prompt_submit",
                                "ai.tool_use",
                                "ai.notification",
                                "ai.stop",
                                "ai.exit",
                                "ai.session_end"
                            ]
                        }, "id": id})
                    }
                    "system.identify" => {
                        json!({"jsonrpc": "2.0", "result": {
                            "name": "PaneFlow",
                            "version": env!("CARGO_PKG_VERSION"),
                            "protocol": "jsonrpc-2.0"
                        }, "id": id})
                    }
                    // EP-002: Windows event-streaming stub (the Unix path
                    // returns before reaching this match). Documented error so
                    // a `paneflow watch` on Windows fails clearly rather than
                    // hanging or risking the named-pipe write abort.
                    #[cfg(not(unix))]
                    "events.subscribe" => {
                        json!({"jsonrpc": "2.0", "error": {
                            "code": -32004,
                            "message": "event streaming is not supported on Windows yet; subscribe over the Unix socket"
                        }, "id": id})
                    }
                    _ => {
                        // Dispatch to GPUI thread and wait for response
                        dispatch_to_gpui(&request_tx, method, params, id)
                    }
                }
            }
            Err(e) => {
                json!({"jsonrpc": "2.0", "error": {"code": -32700, "message": format!("Parse error: {e}")}, "id": null})
            }
        };

        // Skip the reply for fire-and-forget `ai.*` frames (see `suppress_reply`
        // above): the hook has already closed its pipe, and writing to it aborts
        // the process inside `interprocess`. Other methods reply normally.
        if !suppress_reply {
            let mut response_str = serde_json::to_string(&response).unwrap_or_default();
            response_str.push('\n');
            if writer.write_all(response_str.as_bytes()).is_err() {
                break;
            }
        }

        // Windows: serve exactly ONE request per connection. Every paneflow
        // client closes after a single exchange — `paneflow-ai-hook` is
        // fire-and-forget (writes one frame, closes immediately without reading
        // the reply), and `paneflow-ipc-client` does one request/response then
        // drops the stream. Looping back to a SECOND read on a now peer-closed
        // Windows named pipe also aborts the ENTIRE process inside `interprocess`
        // (STATUS_STACK_BUFFER_OVERRUN 0xC0000409 — the read `__fastfail`s
        // instead of returning EOF; confirmed with a live debugger). Unix has no
        // such abort (EOF is returned cleanly), so it keeps the multi-request loop.
        #[cfg(windows)]
        break;
    }
}

/// EP-002 (agent-control-plane): serve a persistent `events.subscribe` stream.
/// Registers a subscriber, writes a `subscribed` ack, then writes each pushed
/// event line until the client disconnects. A 30 s idle tick emits a heartbeat
/// (US-007) so a dead client is detected even when no events flow, and any
/// backlog shed under backpressure (US-004) is reported as a `dropped` marker.
/// Returns when a write fails (client gone) or the bus shuts down; the
/// `Subscription` drops here, unsubscribing (RAII). Unix-only: Windows named-
/// pipe streaming is stubbed at the dispatch site.
#[cfg(unix)]
fn serve_subscription(writer: &mut Stream, params: &Value, bus: &Arc<crate::ipc_events::EventBus>) {
    use std::sync::mpsc::RecvTimeoutError;

    const HEARTBEAT: Duration = Duration::from_secs(30);

    let filter = match crate::ipc_events::EventFilter::from_params(params) {
        Ok(f) => f,
        Err(msg) => {
            let err = json!({
                "jsonrpc": "2.0",
                "error": {"code": -32602, "message": msg},
                "id": Value::Null,
            });
            let _ = writeln!(writer, "{}", err);
            let _ = writer.flush();
            return;
        }
    };
    let sub = bus.subscribe(filter);
    let ack = json!({"type": "subscribed", "id": sub.id});
    if writeln!(writer, "{}", ack).is_err() || writer.flush().is_err() {
        return;
    }

    loop {
        // Report any events shed under backpressure since the last write.
        let dropped = sub.take_dropped();
        if dropped > 0 {
            let marker = json!({"type": "dropped", "count": dropped});
            if writeln!(writer, "{}", marker).is_err() || writer.flush().is_err() {
                break;
            }
        }
        match sub.rx.recv_timeout(HEARTBEAT) {
            Ok(line) => {
                // `line` already carries its trailing newline.
                if writer.write_all(line.as_bytes()).is_err() || writer.flush().is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let hb = json!({"type": "heartbeat"});
                if writeln!(writer, "{}", hb).is_err() || writer.flush().is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn dispatch_to_gpui(
    request_tx: &mpsc::Sender<IpcRequest>,
    method: String,
    params: Value,
    id: Value,
) -> Value {
    let (resp_tx, resp_rx) = mpsc::channel();
    // U-053: shared cancel flag — set if we time out below so the GPUI
    // consumer skips a request the client already gave up on.
    let cancelled = Arc::new(AtomicBool::new(false));
    let ipc_req = IpcRequest {
        method: method.clone(),
        params,
        _id: id.clone(),
        response_tx: resp_tx,
        cancelled: Arc::clone(&cancelled),
    };

    if request_tx.send(ipc_req).is_err() {
        return json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "App shutting down"}, "id": id});
    }

    // Wait for GPUI thread to process (timeout 5s).
    await_or_cancel(&resp_rx, &cancelled, std::time::Duration::from_secs(5), id)
}

/// Wait up to `timeout` for the GPUI handler's response. On timeout, set
/// `cancelled` so the GPUI consumer skips the (possibly not-yet-run) handler
/// — U-053: prevents a non-idempotent mutation from executing after the
/// client received a timeout error and (likely) retried. Split out so the
/// timeout/cancel contract is unit-testable without a 5 s wait.
fn await_or_cancel(
    resp_rx: &mpsc::Receiver<Value>,
    cancelled: &AtomicBool,
    timeout: Duration,
    id: Value,
) -> Value {
    match resp_rx.recv_timeout(timeout) {
        // US-001: handlers may return a structured JSON-RPC error via the
        // `_jsonrpc_error` sentinel. `promote_response` rewrites those into
        // the proper `error` envelope and leaves all other shapes wrapped
        // under `result`.
        Ok(result) => crate::app::ipc_handler::promote_response(result, id),
        Err(_) => {
            cancelled.store(true, Ordering::SeqCst);
            json!({"jsonrpc": "2.0", "error": {"code": -32001, "message": "Request timeout"}, "id": id})
        }
    }
}

fn socket_path() -> Option<PathBuf> {
    crate::runtime_paths::socket_path()
}

// ---------------------------------------------------------------------------
// US-010: peer-UID enforcement
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod auth {
    //! Peer-UID enforcement on the IPC server.
    //!
    //! Splits cleanly so each layer is testable in isolation:
    //!
    //! - [`authorize`]: pure policy decision — given a server UID and a
    //!   peer UID, allow or deny. No I/O, exhaustively unit-tested
    //!   (matching pair → allow, mismatched pair → deny).
    //! - [`server_uid`]: thin wrapper over `getuid(2)`.
    //! - [`check_peer`]: glue that runs `Stream::peer_creds()` (provided
    //!   by interprocess 2.4 — `getsockopt(SO_PEERCRED)` on Linux,
    //!   `LOCAL_PEERCRED` on macOS, `xucred` on the BSDs) and feeds
    //!   the result into `authorize`.
    //!
    //! [`check_peer`] returns an [`AuthOutcome`] the caller turns into
    //! the JSON-RPC envelope (or just keeps serving on
    //! `DegradedFallback`). The split keeps the policy fully covered
    //! by deterministic tests; the live-syscall integration is
    //! exercised by paneflow itself on every connection.

    use super::Stream;
    use interprocess::local_socket::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum AuthOutcome {
        /// Peer UID matches server UID — proceed to dispatch.
        Allow,
        /// Peer UID query succeeded and the value did NOT match the
        /// server's UID. Caller emits the JSON-RPC EPERM envelope.
        Deny { server_uid: u32, peer_uid: u32 },
        /// Peer UID could not be queried (very old kernel / exotic
        /// Unix without an `euid` field in `peer_creds()`). AC6:
        /// fall back to the perms-0600 file-mode line of defence and
        /// continue serving. The warn log fires inside [`check_peer`]
        /// so the fallback isn't silent.
        DegradedFallback,
    }

    /// Pure-function policy. Equality of effective UIDs is the
    /// allowlist.
    pub(super) fn authorize(server_uid: u32, peer_uid: u32) -> AuthOutcome {
        if server_uid == peer_uid {
            AuthOutcome::Allow
        } else {
            AuthOutcome::Deny {
                server_uid,
                peer_uid,
            }
        }
    }

    /// Resolve the running process's effective UID via `geteuid(2)`.
    ///
    /// `peer_creds().euid()` returns the peer's *effective* UID; we
    /// must compare against ours symmetrically. Calling `getuid()`
    /// (real UID) here would diverge from `geteuid()` under any
    /// privilege-separation wrapper (`sudo`, setuid, polkit-helped
    /// child) and either falsely accept or falsely reject a peer that
    /// shares one but not the other.
    pub(super) fn server_uid() -> u32 {
        // libc::uid_t is u32 on every supported target; the cast is a
        // no-op there but stays explicit for cross-target clarity.
        unsafe { libc::geteuid() as u32 }
    }

    /// Run the peer-credential query against the connected stream and
    /// translate the outcome. Defers the kernel-call mechanics to
    /// `interprocess::local_socket::Stream::peer_creds()` (`SO_PEERCRED`
    /// on Linux, `LOCAL_PEERCRED` on macOS, `xucred` on the BSDs);
    /// upstream owns those per-OS quirks so paneflow doesn't
    /// duplicate `getsockopt` boilerplate per target.
    pub(super) fn check_peer(stream: &Stream) -> AuthOutcome {
        let server = server_uid();
        match stream.peer_creds() {
            Ok(creds) => match creds.euid() {
                Some(peer) => authorize(server, peer),
                None => {
                    // `peer_creds()` succeeded but the platform doesn't
                    // expose an effective UID (NetBSD ucred lacks
                    // euid, for example). Same fallback as the Err
                    // branch — perms-0600 stays as the line of
                    // defence.
                    log::warn!(
                        "IPC: peer-cred query returned no euid on this OS; \
                         falling back to perms-0600 only"
                    );
                    AuthOutcome::DegradedFallback
                }
            },
            Err(e) => {
                log::warn!("IPC: peer-cred query failed ({e}); falling back to perms-0600 only");
                AuthOutcome::DegradedFallback
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn authorize_accepts_matching_uid() {
            assert_eq!(authorize(1000, 1000), AuthOutcome::Allow);
            assert_eq!(authorize(0, 0), AuthOutcome::Allow);
        }

        #[test]
        fn authorize_rejects_mismatched_uid() {
            assert_eq!(
                authorize(1000, 1001),
                AuthOutcome::Deny {
                    server_uid: 1000,
                    peer_uid: 1001,
                }
            );
            assert_eq!(
                authorize(1000, 0),
                AuthOutcome::Deny {
                    server_uid: 1000,
                    peer_uid: 0,
                }
            );
        }

        /// `geteuid(2)` must return the same value on two successive
        /// calls — the kernel doesn't change a process's effective UID
        /// without an explicit `setuid(2)` / `seteuid(2)` call. Stable
        /// across calls is the property the auth path actually relies
        /// on (we capture the server euid once and compare every
        /// incoming peer euid against it).
        #[test]
        fn server_uid_is_stable() {
            let a = server_uid();
            let b = server_uid();
            assert_eq!(a, b, "geteuid must be stable across calls");
        }

        /// Symmetric to `authorize_accepts_matching_uid` — root running
        /// the server is an explicit policy choice, not an accidental
        /// bypass: any non-root peer is denied even when the server is
        /// uid 0. The matching-UID accept at `(0, 0)` is the only
        /// root-to-root path; that case is intentional (a privileged
        /// IPC client speaking to a privileged paneflow run by the
        /// same operator).
        #[test]
        fn authorize_root_server_rejects_non_root_peer() {
            assert!(matches!(
                authorize(0, 1000),
                AuthOutcome::Deny {
                    server_uid: 0,
                    peer_uid: 1000
                }
            ));
        }
    }
}

#[cfg(test)]
mod framing_tests {
    use super::{LineRead, MAX_REQUEST_LEN, read_capped_line};
    use std::io::Cursor;

    #[test]
    fn capped_line_rejects_oversized_unterminated() {
        // US-022 negative test: a line that reaches the cap without a newline
        // is reported TooLong, never accumulated past the bound.
        let huge = vec![b'x'; MAX_REQUEST_LEN as usize + 64];
        let mut cur = Cursor::new(huge);
        let mut line = String::new();
        assert_eq!(
            read_capped_line(&mut cur, &mut line).unwrap(),
            LineRead::TooLong
        );
        assert!(line.len() as u64 <= MAX_REQUEST_LEN, "buffer stays bounded");
    }

    #[test]
    fn capped_line_accepts_normal_then_eof() {
        let mut cur = Cursor::new(b"{\"jsonrpc\":\"2.0\"}\n".to_vec());
        let mut line = String::new();
        assert_eq!(
            read_capped_line(&mut cur, &mut line).unwrap(),
            LineRead::Got
        );
        assert_eq!(line, "{\"jsonrpc\":\"2.0\"}\n");
        assert_eq!(
            read_capped_line(&mut cur, &mut line).unwrap(),
            LineRead::Eof
        );
    }

    #[test]
    fn capped_line_accepts_exactly_at_cap_with_newline() {
        // Boundary: a line of exactly MAX_REQUEST_LEN bytes whose final byte
        // is the newline is accepted (not a truncation).
        let mut body = vec![b'a'; MAX_REQUEST_LEN as usize - 1];
        body.push(b'\n');
        let mut cur = Cursor::new(body);
        let mut line = String::new();
        assert_eq!(
            read_capped_line(&mut cur, &mut line).unwrap(),
            LineRead::Got
        );
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::await_or_cancel;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn await_or_cancel_sets_flag_and_errors_on_timeout() {
        // U-053: when the GPUI handler doesn't respond within the deadline,
        // await_or_cancel must (a) return a -32001 timeout envelope to the
        // client AND (b) set the shared cancel flag so the GPUI consumer skips
        // the not-yet-run handler — preventing a duplicate non-idempotent
        // mutation on the client's retry. _tx is kept alive so we exercise the
        // Timeout path (not Disconnected); a short deadline keeps the test fast.
        let (_tx, rx) = mpsc::channel::<serde_json::Value>();
        let cancelled = AtomicBool::new(false);
        let resp = await_or_cancel(&rx, &cancelled, Duration::from_millis(20), json!(7));

        assert!(
            cancelled.load(Ordering::Acquire),
            "timeout must set the cancel flag so the GPUI side skips the request"
        );
        assert_eq!(resp["error"]["code"], -32001);
        assert_eq!(resp["id"], 7);
    }

    #[test]
    fn await_or_cancel_passes_through_response_without_cancelling() {
        // The happy path: a response arrives before the deadline → no cancel,
        // result promoted under `result` (no `_jsonrpc_error` sentinel here).
        let (tx, rx) = mpsc::channel::<serde_json::Value>();
        tx.send(json!({"status": "ok"})).unwrap();
        let cancelled = AtomicBool::new(false);
        let resp = await_or_cancel(&rx, &cancelled, Duration::from_secs(5), json!(3));

        assert!(
            !cancelled.load(Ordering::Acquire),
            "a timely response must not set the cancel flag"
        );
        assert_eq!(resp["result"]["status"], "ok");
        assert_eq!(resp["id"], 3);
    }
}
