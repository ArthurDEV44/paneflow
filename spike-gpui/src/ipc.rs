//! JSON-RPC socket server for AI agent control.
//!
//! Listens on `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`.
//! Each connection reads newline-delimited JSON-RPC requests and writes responses.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;

use serde_json::{json, Value};

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
pub fn start_server() -> mpsc::Receiver<IpcRequest> {
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("paneflow-ipc".into())
        .spawn(move || {
            let Some(socket_path) = socket_path() else {
                log::warn!("Cannot determine XDG_RUNTIME_DIR — IPC server disabled");
                return;
            };

            // Ensure parent directory exists
            if let Some(parent) = socket_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            // Remove stale socket file
            let _ = std::fs::remove_file(&socket_path);

            let listener = match UnixListener::bind(&socket_path) {
                Ok(l) => l,
                Err(e) => {
                    log::error!(
                        "Failed to bind IPC socket at {}: {e}",
                        socket_path.display()
                    );
                    return;
                }
            };

            log::info!("IPC server listening on {}", socket_path.display());

            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        std::thread::spawn(move || handle_connection(stream, tx));
                    }
                    Err(e) => {
                        log::error!("IPC accept error: {e}");
                        break;
                    }
                }
            }

            // Cleanup socket file on exit
            let _ = std::fs::remove_file(&socket_path);
        })
        .expect("Failed to spawn IPC thread");

    rx
}

fn handle_connection(stream: UnixStream, request_tx: mpsc::Sender<IpcRequest>) {
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
                                "surface.list", "surface.send_text"
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
        Ok(result) => json!({"jsonrpc": "2.0", "result": result, "id": id}),
        Err(_) => {
            json!({"jsonrpc": "2.0", "error": {"code": -32001, "message": "Request timeout"}, "id": id})
        }
    }
}

fn socket_path() -> Option<PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(dirs::runtime_dir)?;
    Some(runtime_dir.join("paneflow").join("paneflow.sock"))
}
