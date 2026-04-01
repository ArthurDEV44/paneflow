use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Default timeout for socket operations.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// A minimal JSON-RPC 2.0 request.
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest<'a> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// A minimal JSON-RPC 2.0 response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

/// Client that communicates with the PaneFlow daemon over a Unix socket.
pub struct SocketClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl SocketClient {
    /// Create a new client targeting the given socket path.
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Send a JSON-RPC request and return the parsed response.
    pub async fn request(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse> {
        let stream = timeout(self.timeout, UnixStream::connect(&self.socket_path))
            .await
            .context("connection timed out")?
            .with_context(|| {
                format!(
                    "failed to connect to PaneFlow socket at {}",
                    self.socket_path.display()
                )
            })?;

        let (reader, mut writer) = stream.into_split();

        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        };

        let mut payload = serde_json::to_vec(&req).context("failed to serialize request")?;
        payload.push(b'\n');

        timeout(self.timeout, writer.write_all(&payload))
            .await
            .context("write timed out")?
            .context("failed to write request")?;

        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();

        timeout(self.timeout, buf_reader.read_line(&mut line))
            .await
            .context("read timed out")?
            .context("failed to read response")?;

        if line.is_empty() {
            bail!("server closed connection without responding");
        }

        let response: JsonRpcResponse =
            serde_json::from_str(&line).context("failed to parse JSON-RPC response")?;

        Ok(response)
    }
}

/// Discover the PaneFlow daemon socket path.
///
/// Resolution order:
/// 1. `PANEFLOW_SOCKET_PATH` environment variable
/// 2. `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`
/// 3. `$XDG_RUNTIME_DIR/paneflow/last-socket-path` file content
/// 4. `/tmp/paneflow-{uid}/paneflow.sock`
pub fn discover_socket() -> Option<PathBuf> {
    // 1. Explicit env override.
    if let Ok(path) = std::env::var("PANEFLOW_SOCKET_PATH") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. XDG_RUNTIME_DIR default location.
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let sock = PathBuf::from(&runtime_dir)
            .join("paneflow")
            .join("paneflow.sock");
        if sock.exists() {
            return Some(sock);
        }

        // 3. last-socket-path file.
        let last_path_file = PathBuf::from(&runtime_dir)
            .join("paneflow")
            .join("last-socket-path");
        if let Ok(contents) = std::fs::read_to_string(&last_path_file) {
            let p = PathBuf::from(contents.trim());
            if p.exists() {
                return Some(p);
            }
        }
    }

    // 4. Fallback: /tmp/paneflow-{uid}/paneflow.sock
    let uid = unsafe { libc::getuid() };
    let fallback = PathBuf::from(format!("/tmp/paneflow-{uid}/paneflow.sock"));
    if fallback.exists() {
        return Some(fallback);
    }

    None
}

/// Return the socket path to use, preferring an explicit override, then discovery.
pub fn resolve_socket(override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }

    discover_socket().with_context(|| {
        "PaneFlow is not running. No socket found.\n\
         Searched:\n  \
           - $PANEFLOW_SOCKET_PATH\n  \
           - $XDG_RUNTIME_DIR/paneflow/paneflow.sock\n  \
           - $XDG_RUNTIME_DIR/paneflow/last-socket-path\n  \
           - /tmp/paneflow-<uid>/paneflow.sock\n\n\
         Start PaneFlow first, or pass --socket <path> to specify the socket manually."
    })
}
