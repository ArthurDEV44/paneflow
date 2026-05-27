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
//!   `ai.notification` / `ai.stop` / `ai.session_end` — AI hook lifecycle.
//!
//! Handlers may return a structured JSON-RPC error by emitting the
//! `_jsonrpc_error` sentinel (see `app::ipc_handler::JsonRpcError`); the
//! dispatcher promotes it to a proper `error` envelope. Legacy
//! application errors returned as `{"error": "string"}` continue to flow
//! through the `result` field for backward compatibility.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use interprocess::TryClone;
use interprocess::local_socket::{
    GenericFilePath, Listener, ListenerNonblockingMode, ListenerOptions, Stream, prelude::*,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// IPC request type — sent from socket thread to GPUI thread
// ---------------------------------------------------------------------------

pub struct IpcRequest {
    pub method: String,
    pub params: Value,
    pub _id: Value,
    pub response_tx: mpsc::Sender<Value>,
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
pub fn start_server() -> mpsc::Receiver<IpcRequest> {
    let (tx, rx) = mpsc::channel();

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

    std::thread::Builder::new()
        .name("paneflow-ipc".into())
        .spawn(move || {
            let Some(socket_path) = socket_path() else {
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
            }

            let listener = match bind_socket(&socket_path) {
                Some(l) => l,
                None => return,
            };

            #[cfg(unix)]
            let mut our_ino = socket_inode(&socket_path).unwrap_or(0);
            #[cfg(unix)]
            let mut last_health_check = std::time::Instant::now();
            #[cfg(unix)]
            let mut listener = listener;
            #[cfg(not(unix))]
            let listener = listener;

            // Non-blocking accept so we can periodically verify the socket
            // file (Unix) without starving connections. Stream I/O itself
            // stays blocking so `handle_connection` can use plain `BufRead`.
            listener
                .set_nonblocking(ListenerNonblockingMode::Accept)
                .ok();

            loop {
                match listener.accept() {
                    Ok(stream) => {
                        let tx = tx.clone();
                        std::thread::spawn(move || handle_connection(stream, tx));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No pending connection — brief sleep to avoid busy-spin
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => {
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
                            None => return,
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
        })
        .expect("Failed to spawn IPC thread");

    rx
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
        let _ = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600));
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

        // `interprocess::Stream` does not expose a uniform cross-platform
        // read timeout, so we read on a scratch thread and bound the
        // wait with `recv_timeout`. 300 ms is generous for a stateless
        // socket-thread handler; a live but unresponsive process within
        // that budget is functionally indistinguishable from "no peer"
        // and we proceed to bind (a hung peer that resumes after we've
        // bound will detect the clobber on its next tick — the original
        // failure mode, but now bounded to one stuck-instance edge case
        // rather than recurring every 5 s by design).
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = tx.send(line);
        });

        let Ok(line) = rx.recv_timeout(Duration::from_millis(300)) else {
            continue;
        };

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

fn handle_connection(stream: Stream, request_tx: mpsc::Sender<IpcRequest>) {
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

    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = match line {
            Ok(l) if l.is_empty() => continue,
            Ok(l) => l,
            Err(_) => break,
        };

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let method = req
                    .get("method")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = req.get("params").cloned().unwrap_or(json!({}));

                // Handle stateless methods directly on the socket thread
                match method.as_str() {
                    "system.ping" => {
                        json!({"jsonrpc": "2.0", "result": {"pong": true}, "id": id})
                    }
                    "system.capabilities" => {
                        json!({"jsonrpc": "2.0", "result": {
                            "methods": [
                                "system.ping", "system.capabilities", "system.identify",
                                "workspace.list", "workspace.create", "workspace.select",
                                "workspace.close", "workspace.current",
                                "workspace.restore_layout",
                                "surface.list", "surface.send_text", "surface.send_keystroke", "surface.split",
                                "ai.session_start",
                                "ai.prompt_submit",
                                "ai.tool_use",
                                "ai.notification",
                                "ai.stop",
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

        let mut response_str = serde_json::to_string(&response).unwrap_or_default();
        response_str.push('\n');
        if writer.write_all(response_str.as_bytes()).is_err() {
            break;
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
    let ipc_req = IpcRequest {
        method: method.clone(),
        params,
        _id: id.clone(),
        response_tx: resp_tx,
    };

    if request_tx.send(ipc_req).is_err() {
        return json!({"jsonrpc": "2.0", "error": {"code": -32000, "message": "App shutting down"}, "id": id});
    }

    // Wait for GPUI thread to process (timeout 5s)
    match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        // US-001: handlers may return a structured JSON-RPC error via the
        // `_jsonrpc_error` sentinel. `promote_response` rewrites those into
        // the proper `error` envelope and leaves all other shapes wrapped
        // under `result`.
        Ok(result) => crate::app::ipc_handler::promote_response(result, id),
        Err(_) => {
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
