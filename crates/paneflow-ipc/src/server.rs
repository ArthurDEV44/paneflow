// US-012: Socket server framework

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

/// Type alias for the async handler callback.
///
/// Receives a raw JSON line from a client and returns a raw JSON response line.
pub type Handler =
    Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = String> + Send>> + Send + Sync>;

/// Unix domain socket server for PaneFlow IPC.
///
/// Listens on a well-known socket path, accepts concurrent connections,
/// and delegates each newline-delimited JSON message to a handler callback.
pub struct SocketServer {
    /// Resolved socket path.
    socket_path: PathBuf,
    /// Async handler invoked for every incoming JSON line.
    handler: Handler,
    /// Notifier used to signal graceful shutdown.
    shutdown: Arc<Notify>,
}

/// Errors produced by the socket server.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to resolve socket directory")]
    NoRuntimeDir,
}

impl SocketServer {
    /// Create a new `SocketServer`.
    ///
    /// The socket path is determined by [`resolve_socket_path`].  Pass a
    /// handler that will be called for every newline-delimited JSON message
    /// received from any connected client.
    pub fn new(handler: Handler) -> Result<Self, ServerError> {
        let socket_path = resolve_socket_path()?;
        Ok(Self {
            socket_path,
            handler,
            shutdown: Arc::new(Notify::new()),
        })
    }

    /// Create a `SocketServer` bound to an explicit socket path.
    ///
    /// Useful for tests that need an isolated temporary path.
    pub fn with_path(socket_path: PathBuf, handler: Handler) -> Self {
        Self {
            socket_path,
            handler,
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Return a handle that can be used to trigger a graceful shutdown.
    pub fn shutdown_handle(&self) -> Arc<Notify> {
        Arc::clone(&self.shutdown)
    }

    /// The path the server will listen on (or is currently listening on).
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Start accepting connections.
    ///
    /// This future runs until the shutdown notifier is triggered.  On exit it
    /// cleans up the socket file.
    pub async fn run(&self) -> Result<(), ServerError> {
        // Ensure the parent directory exists.
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Remove a stale socket file left by a previous crash.
        if self.socket_path.exists() {
            warn!(path = %self.socket_path.display(), "removing stale socket file");
            std::fs::remove_file(&self.socket_path)?;
        }

        // Bind the listener.
        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "socket server listening");

        // Set permissions to 0o600 (owner-only).
        set_owner_only_permissions(&self.socket_path)?;

        // Write the socket path to a discovery file for CLI tools.
        write_discovery_file(&self.socket_path)?;

        let shutdown = Arc::clone(&self.shutdown);

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, _addr)) => {
                            debug!("accepted new connection");
                            let handler = Arc::clone(&self.handler);
                            tokio::spawn(handle_connection(stream, handler));
                        }
                        Err(e) => {
                            error!(error = %e, "failed to accept connection");
                        }
                    }
                }
                _ = shutdown.notified() => {
                    info!("shutdown signal received, stopping server");
                    break;
                }
            }
        }

        // Cleanup: remove socket and discovery files.
        self.cleanup();
        Ok(())
    }

    /// Remove the socket file and the discovery file.
    pub fn cleanup(&self) {
        if self.socket_path.exists() {
            if let Err(e) = std::fs::remove_file(&self.socket_path) {
                warn!(error = %e, "failed to remove socket file");
            } else {
                debug!(path = %self.socket_path.display(), "socket file removed");
            }
        }

        if let Some(parent) = self.socket_path.parent() {
            let discovery = parent.join("last-socket-path");
            if discovery.exists() {
                let _ = std::fs::remove_file(&discovery);
            }
        }
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// ── Connection handler ──────────────────────────────────────────────────

/// Handle a single client connection.
///
/// Reads newline-delimited JSON lines, invokes the handler for each one,
/// and writes back the response followed by a newline.
async fn handle_connection(stream: tokio::net::UnixStream, handler: Handler) {
    let (reader, mut writer) = stream.into_split();
    let reader = BufReader::new(reader);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        debug!(len = line.len(), "received message");
        let response = (handler)(line).await;
        let mut response = response;
        if !response.ends_with('\n') {
            response.push('\n');
        }
        if let Err(e) = writer.write_all(response.as_bytes()).await {
            warn!(error = %e, "failed to write response to client");
            break;
        }
    }
    debug!("client disconnected");
}

// ── Path resolution ─────────────────────────────────────────────────────

/// Determine the socket path using platform conventions.
///
/// - Linux: `$XDG_RUNTIME_DIR/paneflow/paneflow.sock`
/// - Fallback: `/tmp/paneflow-{uid}/paneflow.sock`
pub fn resolve_socket_path() -> Result<PathBuf, ServerError> {
    // Prefer XDG_RUNTIME_DIR if set.
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return Ok(dir.join("paneflow").join("paneflow.sock"));
        }
    }

    // Fallback: /tmp/paneflow-{uid}/
    let uid = get_uid();
    Ok(PathBuf::from(format!("/tmp/paneflow-{uid}")).join("paneflow.sock"))
}

// ── Platform helpers ────────────────────────────────────────────────────

/// Get the current user's UID without depending on the `libc` crate.
fn get_uid() -> u32 {
    // SAFETY: getuid() is a POSIX syscall that requires no arguments, never
    // fails, and has no side effects.
    extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Set file permissions to `0o600` (owner read/write only).
fn set_owner_only_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

/// Write the socket path to a sibling file for CLI discovery.
fn write_discovery_file(socket_path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = socket_path.parent() {
        let discovery = parent.join("last-socket-path");
        std::fs::write(&discovery, socket_path.to_string_lossy().as_bytes())?;
        debug!(path = %discovery.display(), "wrote discovery file");
    }
    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    /// Atomic counter to generate unique temp directory names across tests.
    static TEST_ID: AtomicU32 = AtomicU32::new(0);

    /// Create a unique temporary directory for a test and return its path.
    /// The caller is responsible for cleaning up via the returned `TempDir` guard.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("paneflow-test-{pid}-{id}"));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Build a simple echo handler for testing.
    fn echo_handler() -> Handler {
        Arc::new(|msg: String| {
            Box::pin(async move {
                // Echo back whatever the client sent.
                msg
            })
        })
    }

    /// Helper: create a server in a temp directory with an echo handler.
    fn temp_server() -> (SocketServer, TempDir) {
        let dir = TempDir::new();
        let sock = dir.path().join("paneflow.sock");
        let server = SocketServer::with_path(sock, echo_handler());
        (server, dir)
    }

    #[tokio::test]
    async fn server_starts_and_client_connects() {
        let (server, _dir) = temp_server();
        let shutdown = server.shutdown_handle();
        let path = server.socket_path().to_path_buf();

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        // Give the server a moment to bind.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connect a client.
        let stream: UnixStream = UnixStream::connect(&path).await.expect("connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Send a message.
        writer
            .write_all(b"hello world\n")
            .await
            .expect("write");

        // Read the echoed response.
        let mut response = String::new();
        reader.read_line(&mut response).await.expect("read");
        assert_eq!(response.trim(), "hello world");

        // Shutdown.
        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");
    }

    #[tokio::test]
    async fn socket_file_cleaned_up_on_shutdown() {
        let (server, _dir) = temp_server();
        let shutdown = server.shutdown_handle();
        let path = server.socket_path().to_path_buf();

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(path.exists(), "socket should exist while running");

        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");

        assert!(!path.exists(), "socket should be removed after shutdown");
    }

    #[tokio::test]
    async fn stale_socket_is_removed_on_startup() {
        let dir = TempDir::new();
        let sock = dir.path().join("paneflow.sock");

        // Create a fake stale socket file.
        std::fs::write(&sock, b"stale").expect("write stale file");
        assert!(sock.exists());

        let server = SocketServer::with_path(sock.clone(), echo_handler());
        let shutdown = server.shutdown_handle();

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        // The server should have replaced the stale file.
        // Verify we can actually connect.
        let _stream: UnixStream =
            UnixStream::connect(&sock).await.expect("connect after stale removal");

        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");
    }

    #[tokio::test]
    async fn socket_permissions_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let (server, _dir) = temp_server();
        let shutdown = server.shutdown_handle();
        let path = server.socket_path().to_path_buf();

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let meta = std::fs::metadata(&path).expect("metadata");
        let mode = meta.permissions().mode() & 0o777;
        // On most systems, Unix sockets respect the permission bits we set,
        // though some may mask differently.  Accept 0o600 or 0o755-style if
        // the OS doesn't honour socket chmod (very rare).
        assert_eq!(mode, 0o600, "socket should have 0o600 permissions");

        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");
    }

    #[tokio::test]
    async fn discovery_file_is_written() {
        let (server, _dir) = temp_server();
        let shutdown = server.shutdown_handle();
        let path = server.socket_path().to_path_buf();
        let discovery = path.parent().unwrap().join("last-socket-path");

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(discovery.exists(), "discovery file should exist");
        let content = std::fs::read_to_string(&discovery).expect("read discovery");
        assert_eq!(content, path.to_string_lossy());

        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");
    }

    #[tokio::test]
    async fn concurrent_clients_do_not_block_each_other() {
        // Use a handler that deliberately sleeps for slow messages.
        let handler: Handler = Arc::new(|msg: String| {
            Box::pin(async move {
                if msg.contains("slow") {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                msg
            })
        });

        let dir = TempDir::new();
        let sock = dir.path().join("paneflow.sock");
        let server = SocketServer::with_path(sock.clone(), handler);
        let shutdown = server.shutdown_handle();

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        // Client 1 sends a slow message.
        let sock1 = sock.clone();
        let c1 = tokio::spawn(async move {
            let stream: UnixStream = UnixStream::connect(&sock1).await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            writer.write_all(b"slow request\n").await.unwrap();
            let mut resp = String::new();
            reader.read_line(&mut resp).await.unwrap();
            resp
        });

        // Client 2 sends a fast message immediately after.
        let sock2 = sock.clone();
        let c2 = tokio::spawn(async move {
            // Small delay so c1 connects first.
            tokio::time::sleep(Duration::from_millis(10)).await;
            let stream: UnixStream = UnixStream::connect(&sock2).await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            writer.write_all(b"fast request\n").await.unwrap();
            let mut resp = String::new();
            reader.read_line(&mut resp).await.unwrap();
            resp
        });

        // The fast client should finish well before the slow one.
        let fast_result = tokio::time::timeout(Duration::from_millis(150), c2)
            .await
            .expect("fast client should not be blocked by slow client")
            .expect("join");
        assert_eq!(fast_result.trim(), "fast request");

        // Wait for slow client to finish too.
        let slow_result = c1.await.expect("join");
        assert_eq!(slow_result.trim(), "slow request");

        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");
    }

    #[tokio::test]
    async fn multiple_messages_on_same_connection() {
        let (server, _dir) = temp_server();
        let shutdown = server.shutdown_handle();
        let path = server.socket_path().to_path_buf();

        let server_handle = tokio::spawn(async move {
            server.run().await.expect("server run");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let stream: UnixStream = UnixStream::connect(&path).await.expect("connect");
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        for i in 0..5 {
            let msg = format!("message {i}\n");
            writer.write_all(msg.as_bytes()).await.expect("write");
            let mut resp = String::new();
            reader.read_line(&mut resp).await.expect("read");
            assert_eq!(resp.trim(), format!("message {i}"));
        }

        shutdown.notify_one();
        tokio::time::timeout(Duration::from_secs(2), server_handle)
            .await
            .expect("timeout")
            .expect("join");
    }
}
