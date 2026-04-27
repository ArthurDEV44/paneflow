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

fn handle_connection(stream: Stream, request_tx: mpsc::Sender<IpcRequest>) {
    // `Stream::try_clone` is provided by `interprocess::TryClone` and
    // works on both Unix domain sockets and Windows named pipes. One
    // handle reads, the other writes, so request/response flow does not
    // fight over a single mutable cursor.
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    let mut writer = writer_stream;

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
