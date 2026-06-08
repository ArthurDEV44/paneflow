//! Blocking JSON-RPC client for Paneflow's local IPC socket (US-005).
//!
//! Mirrors the server wire protocol at `src-app/src/ipc.rs`: newline-delimited
//! JSON-RPC 2.0 over an `interprocess` local socket (Unix domain socket /
//! Windows named pipe). Unlike `paneflow-ai-hook` (fire-and-forget), this
//! client is request/response — it reads back the one-line response the
//! server writes on the same connection.
//!
//! One connection per request: simple and robust (a stale connection can't
//! wedge the bridge). The server's peer-UID check passes because the bridge
//! runs as the same user that launched Paneflow.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use interprocess::local_socket::{prelude::*, GenericFilePath, Stream};
use serde_json::{json, Value};

/// Wire timeout for a single request/response round-trip. The server always
/// writes a response (it even synthesizes a `-32001 Request timeout`
/// envelope), so a stall this long means the process is wedged.
const IPC_TIMEOUT: Duration = Duration::from_secs(10);

/// Abstraction over "send a JSON-RPC request to Paneflow, get the `result`".
/// Lets the MCP layer and tools be unit-tested against a fake transport with
/// no live socket.
pub trait IpcTransport {
    /// Call a Paneflow IPC method. Returns the `result` value on success, or
    /// `Err(message)` on transport failure or a JSON-RPC `error` envelope.
    fn call(&self, method: &str, params: Value) -> Result<Value, String>;
}

/// Live client bound to a resolved socket path.
pub struct IpcClient {
    socket: PathBuf,
    next_id: AtomicU64,
}

impl IpcClient {
    pub fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            next_id: AtomicU64::new(1),
        }
    }
}

impl IpcTransport for IpcClient {
    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = build_request(id, method, params);
        let line = send_and_receive(&self.socket, &request).map_err(|e| {
            format!(
                "paneflow IPC unreachable at {} ({e}); is Paneflow running?",
                self.socket.display()
            )
        })?;
        parse_response(&line)
    }
}

/// Build a JSON-RPC 2.0 request frame.
pub(crate) fn build_request(id: u64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

/// Extract the `result` from a JSON-RPC response line, or translate an
/// `error` envelope / malformed line into `Err(message)`.
pub(crate) fn parse_response(line: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("invalid JSON-RPC response from paneflow: {e}"))?;
    if let Some(err) = value.get("error") {
        let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("paneflow error {code}: {message}"));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "paneflow response missing both `result` and `error`".to_string())
}

/// Open a connection, write the newline-terminated request, and read back one
/// newline-delimited response line.
///
/// US-023: the read deadline is enforced at the OS level via
/// `set_recv_timeout` (Unix `SO_RCVTIMEO`, Windows named-pipe read timeout).
/// The previous scratch-thread + `recv_timeout` pattern leaked one OS thread
/// and one socket FD on every timeout — the spawned reader owned `stream` and
/// stayed blocked in `read_line` forever (no deadline ever reached it), so an
/// agent retrying `read_pane` against a wedged Paneflow exhausted the
/// long-lived bridge's threads/FDs. With an OS deadline, `read_line` returns
/// the error itself, the owning `BufReader` drops, and the FD is released.
fn send_and_receive(socket: &Path, request: &Value) -> io::Result<String> {
    let name = socket.to_fs_name::<GenericFilePath>()?;
    let mut stream = Stream::connect(name)?;
    // Bound both directions on the same deadline: a peer that never drains our
    // write could otherwise wedge `write_all`.
    stream.set_recv_timeout(Some(IPC_TIMEOUT))?;
    stream.set_send_timeout(Some(IPC_TIMEOUT))?;

    let mut payload =
        serde_json::to_vec(request).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    payload.push(b'\n');
    stream.write_all(&payload)?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(_) => Ok(line),
        // SO_RCVTIMEO surfaces as EAGAIN/`WouldBlock` on Unix and `TimedOut`
        // on Windows — normalize both to a friendly timeout message.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "paneflow did not respond within 10s",
            ))
        }
        Err(e) => Err(e),
    }
}

/// Resolve the Paneflow IPC socket path. `PANEFLOW_SOCKET_PATH` (inherited
/// from the Paneflow PTY through the agent that launched this bridge) is
/// authoritative — it carries the exact path the running instance bound,
/// including the debug `paneflow-dev` vs release distinction. Falls back to
/// the release default when the env var is absent.
pub fn resolve_socket_path() -> Option<PathBuf> {
    if let Some(p) = socket_path_from_env(std::env::var("PANEFLOW_SOCKET_PATH").ok().as_deref()) {
        return Some(p);
    }
    default_socket_path()
}

/// Validate a `PANEFLOW_SOCKET_PATH` value: present and absolute. A relative
/// path means the env was clobbered or we're outside a Paneflow PTY.
pub(crate) fn socket_path_from_env(raw: Option<&str>) -> Option<PathBuf> {
    let path = PathBuf::from(raw?);
    path.is_absolute().then_some(path)
}

/// Best-effort default socket path, mirroring `src-app/src/runtime_paths.rs`
/// (release profile). The bridge can't know whether a debug `paneflow-dev`
/// instance is running, so it targets the release socket; the env var above
/// is the authoritative source and normally wins. Uses raw env (no `dirs`
/// dep) to keep the bridge's dependency tree minimal.
#[cfg(unix)]
fn default_socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("TMPDIR")
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
        })?;
    Some(runtime.join("paneflow").join("paneflow.sock"))
}

/// Windows default: the release named-pipe path. Mirrors
/// `runtime_paths::socket_path` on Windows.
#[cfg(windows)]
fn default_socket_path() -> Option<PathBuf> {
    Some(PathBuf::from(r"\\.\pipe\paneflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_has_jsonrpc_envelope() {
        let req = build_request(7, "surface.list", json!({}));
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["id"], 7);
        assert_eq!(req["method"], "surface.list");
        assert_eq!(req["params"], json!({}));
    }

    #[test]
    fn parse_response_extracts_result() {
        let line = r#"{"jsonrpc":"2.0","result":{"surfaces":[]},"id":1}"#;
        let result = parse_response(line).expect("ok");
        assert_eq!(result, json!({"surfaces": []}));
    }

    #[test]
    fn parse_response_translates_error_envelope() {
        let line = r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"surface_id 9 not found"},"id":1}"#;
        let err = parse_response(line).expect_err("err");
        assert!(err.contains("-32602"), "got: {err}");
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn parse_response_rejects_missing_result_and_error() {
        let line = r#"{"jsonrpc":"2.0","id":1}"#;
        assert!(parse_response(line).is_err());
    }

    #[test]
    fn parse_response_rejects_malformed_json() {
        assert!(parse_response("not json").is_err());
    }

    #[test]
    fn socket_path_from_env_requires_absolute() {
        assert_eq!(
            socket_path_from_env(Some("/run/user/1000/paneflow/paneflow.sock")),
            Some(PathBuf::from("/run/user/1000/paneflow/paneflow.sock"))
        );
        assert_eq!(socket_path_from_env(Some("relative/path.sock")), None);
        assert_eq!(socket_path_from_env(Some("")), None);
        assert_eq!(socket_path_from_env(None), None);
    }

    /// US-005 AC: a full request/response round-trip over a real local socket
    /// (not just the pure helpers). Spins up an `interprocess` listener that
    /// speaks the Paneflow framing — read one newline-delimited request, echo
    /// its `id` back in a JSON-RPC `result` envelope. Unix-only: the test path
    /// is a filesystem socket, not a Windows `\\.\pipe\` name.
    #[cfg(unix)]
    #[test]
    fn ipc_client_round_trips_against_a_live_socket() {
        use interprocess::local_socket::{Listener, ListenerOptions};
        use interprocess::TryClone;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("paneflow-test.sock");
        let name = path.as_path().to_fs_name::<GenericFilePath>().unwrap();
        let listener: Listener = ListenerOptions::new().name(name).create_sync().unwrap();

        let server = std::thread::spawn(move || {
            let stream = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("read request");
            let request: Value = serde_json::from_str(line.trim()).expect("parse request");
            // Echo the client's id back, mirroring the real server contract.
            let response = json!({
                "jsonrpc": "2.0",
                "id": request["id"].clone(),
                "result": {"surfaces": [{"surface_id": 1u64, "name": "cargo-run"}]},
            });
            let mut serialized = serde_json::to_string(&response).unwrap();
            serialized.push('\n');
            writer
                .write_all(serialized.as_bytes())
                .expect("write response");
            writer.flush().expect("flush");
        });

        let client = IpcClient::new(path);
        let result = client.call("surface.list", json!({})).expect("call ok");
        assert_eq!(result["surfaces"][0]["name"], "cargo-run");

        server.join().expect("server thread");
    }

    #[cfg(unix)]
    #[test]
    fn ipc_client_call_errors_when_socket_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist.sock");
        let client = IpcClient::new(path);
        let err = client
            .call("surface.list", json!({}))
            .expect_err("must fail with no listener");
        assert!(err.contains("unreachable"), "got: {err}");
    }
}
