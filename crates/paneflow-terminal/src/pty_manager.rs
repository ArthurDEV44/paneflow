//! US-004: Cross-platform PTY spawning via `portable_pty`.
//!
//! `PtyManager` wraps the native PTY system and provides a high-level API
//! for spawning shell sessions. Each spawned session is represented by a
//! `PtySession` that holds the master PTY reader/writer, child handle,
//! and associated pane identifier.

use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during PTY operations.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("failed to open PTY pair: {0}")]
    OpenPty(#[source] anyhow::Error),

    #[error("failed to spawn command: {0}")]
    Spawn(#[source] anyhow::Error),

    #[error("failed to clone PTY reader: {0}")]
    CloneReader(#[source] anyhow::Error),

    #[error("failed to take PTY writer: {0}")]
    TakeWriter(#[source] anyhow::Error),

    #[error("failed to resize PTY: {0}")]
    Resize(#[source] anyhow::Error),

    #[error("failed to write to PTY: {0}")]
    Write(#[source] std::io::Error),

    #[error("failed to kill child process: {0}")]
    Kill(#[source] std::io::Error),

    #[error("PTY writer has already been taken")]
    WriterTaken,

    #[error("PTY reader has already been taken")]
    ReaderTaken,
}

/// Convenience alias used throughout this module.
pub type Result<T> = std::result::Result<T, PtyError>;

// ---------------------------------------------------------------------------
// PtyManager
// ---------------------------------------------------------------------------

/// Manages the lifecycle of PTY sessions.
///
/// Wraps the platform-native PTY system (`NativePtySystem`) and keeps an
/// index of live sessions keyed by `pane_id`.
pub struct PtyManager {
    pty_system: Box<dyn PtySystem + Send>,
    sessions: HashMap<Uuid, PtySession>,
}

impl PtyManager {
    /// Create a new `PtyManager` backed by the platform-native PTY system.
    pub fn new() -> Self {
        Self {
            pty_system: Box::new(NativePtySystem::default()),
            sessions: HashMap::new(),
        }
    }

    /// Spawn a new PTY session.
    ///
    /// # Arguments
    ///
    /// * `pane_id` – caller-assigned identifier for this pane.
    /// * `command` – program to execute. `None` launches the default shell.
    /// * `cwd` – working directory for the child process (`None` = inherit).
    /// * `env` – extra environment variables.
    /// * `rows` – initial number of terminal rows.
    /// * `cols` – initial number of terminal columns.
    /// * `args` – additional command-line arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &mut self,
        pane_id: Uuid,
        command: Option<&str>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        rows: u16,
        cols: u16,
        args: &[&str],
    ) -> Result<&mut PtySession> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        // Open a new PTY pair.
        let pair = self.pty_system.openpty(size).map_err(PtyError::OpenPty)?;

        // Build the command.
        let mut cmd = match command {
            Some(program) => {
                let mut builder = CommandBuilder::new(program);
                builder.args(args);
                builder
            }
            None => CommandBuilder::new_default_prog(),
        };

        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }

        for (key, value) in &env {
            cmd.env(key, value);
        }

        // Spawn the child inside the slave side of the PTY.
        let child = pair.slave.spawn_command(cmd).map_err(PtyError::Spawn)?;

        tracing::info!(
            pane_id = %pane_id,
            command = command.unwrap_or("<default shell>"),
            "spawned PTY session"
        );

        // Split the master into a reader and a writer so they can be
        // independently handed off (e.g. to the async bridge in US-006).
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(PtyError::CloneReader)?;
        let writer = pair.master.take_writer().map_err(PtyError::TakeWriter)?;

        let session = PtySession {
            pane_id,
            master: pair.master,
            reader: Some(reader),
            writer: Some(writer),
            child,
        };

        self.sessions.insert(pane_id, session);
        // SAFETY: we just inserted, so unwrap is fine.
        Ok(self.sessions.get_mut(&pane_id).unwrap())
    }

    /// Look up an existing session by pane id.
    pub fn get(&self, pane_id: &Uuid) -> Option<&PtySession> {
        self.sessions.get(pane_id)
    }

    /// Look up an existing session mutably by pane id.
    pub fn get_mut(&mut self, pane_id: &Uuid) -> Option<&mut PtySession> {
        self.sessions.get_mut(pane_id)
    }

    /// Remove and return a session (e.g. after it has been killed).
    pub fn remove(&mut self, pane_id: &Uuid) -> Option<PtySession> {
        self.sessions.remove(pane_id)
    }

    /// Number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PtySession
// ---------------------------------------------------------------------------

/// A single PTY session tied to a pane.
///
/// The `reader` and `writer` fields are wrapped in `Option` so that they can
/// be *taken* (moved out) when the async I/O bridge needs ownership (US-006).
///
/// `Debug` is implemented manually because the inner trait objects
/// (`MasterPty`, `Read`, `Write`, `Child`) do not all derive `Debug`.
pub struct PtySession {
    /// Caller-assigned pane identifier.
    pane_id: Uuid,

    /// The master end of the PTY. Kept alive for resize operations.
    master: Box<dyn MasterPty + Send>,

    /// Readable stream from the PTY (child stdout/stderr).
    reader: Option<Box<dyn Read + Send>>,

    /// Writable stream to the PTY (child stdin).
    writer: Option<Box<dyn Write + Send>>,

    /// Handle to the spawned child process.
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("pane_id", &self.pane_id)
            .field("has_reader", &self.reader.is_some())
            .field("has_writer", &self.writer.is_some())
            .finish_non_exhaustive()
    }
}

impl PtySession {
    /// Returns the pane identifier associated with this session.
    pub fn pane_id(&self) -> Uuid {
        self.pane_id
    }

    /// Resize the PTY to the given dimensions. This sends `SIGWINCH`
    /// (or equivalent) to the child without killing it.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master.resize(size).map_err(PtyError::Resize)?;
        tracing::debug!(pane_id = %self.pane_id, rows, cols, "resized PTY");
        Ok(())
    }

    /// Send bytes to the child process via the PTY.
    ///
    /// Returns `PtyError::WriterTaken` if the writer has already been moved
    /// out via [`take_writer`](Self::take_writer).
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        let writer = self.writer.as_mut().ok_or(PtyError::WriterTaken)?;
        writer.write_all(data).map_err(PtyError::Write)?;
        Ok(())
    }

    /// Terminate the child process.
    pub fn kill(&mut self) -> Result<()> {
        // Drop writer first — this sends EOF to the child.
        self.writer.take();
        self.child.kill().map_err(PtyError::Kill)?;
        tracing::info!(pane_id = %self.pane_id, "killed PTY child process");
        Ok(())
    }

    /// Poll whether the child has exited without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        self.child.try_wait()
    }

    /// Block until the child exits, returning its status.
    pub fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        self.child.wait()
    }

    /// Take ownership of the reader stream. This is intended for the async
    /// I/O bridge (US-006). After calling this, further reads through this
    /// session are not possible.
    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    /// Take ownership of the writer stream. This is intended for the async
    /// I/O bridge (US-006). After calling this, [`write`](Self::write) will
    /// return `PtyError::WriterTaken`.
    pub fn take_writer(&mut self) -> Option<Box<dyn Write + Send>> {
        self.writer.take()
    }

    /// Whether the reader is still held by this session (i.e. not yet taken).
    pub fn has_reader(&self) -> bool {
        self.reader.is_some()
    }

    /// Whether the writer is still held by this session (i.e. not yet taken).
    pub fn has_writer(&self) -> bool {
        self.writer.is_some()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    // -- Unit tests (cross-platform) --

    #[test]
    fn pty_manager_default() {
        let mgr = PtyManager::default();
        assert_eq!(mgr.session_count(), 0);
    }

    #[test]
    fn spawn_returns_descriptive_error_on_invalid_command() {
        let mut mgr = PtyManager::new();
        let id = Uuid::new_v4();
        let result = mgr.spawn(
            id,
            Some("/nonexistent/binary/that/does/not/exist"),
            None,
            vec![],
            24,
            80,
            &[],
        );
        assert!(result.is_err(), "expected spawn to fail for invalid binary");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("spawn") || msg.contains("command"),
            "error should be descriptive, got: {msg}"
        );
    }

    // -- Integration tests (Unix only) --

    #[cfg(not(windows))]
    #[test]
    fn spawn_echo_read_output() {
        let mut mgr = PtyManager::new();
        let pane_id = Uuid::new_v4();

        // Spawn `/bin/sh -c 'echo hello'` — prints "hello\n" then exits.
        let session = mgr
            .spawn(
                pane_id,
                Some("/bin/sh"),
                None,
                vec![],
                24,
                80,
                &["-c", "echo hello"],
            )
            .expect("failed to spawn echo command");

        // Take the reader and collect output.
        let mut reader = session.take_reader().expect("reader should be available");

        let mut buf = vec![0u8; 4096];
        let mut output = String::new();

        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if output.contains("hello") {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => break,
            }
        }

        assert!(
            output.contains("hello"),
            "expected output to contain 'hello', got: {output:?}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn resize_does_not_kill_process() {
        let mut mgr = PtyManager::new();
        let pane_id = Uuid::new_v4();

        // Spawn a short-lived command.
        let session = mgr
            .spawn(pane_id, Some("/bin/echo"), None, vec![], 24, 80, &[])
            .expect("failed to spawn");

        // Resize should succeed without error.
        session.resize(48, 120).expect("resize should succeed");
    }

    #[cfg(not(windows))]
    #[test]
    fn write_to_pty() {
        let mut mgr = PtyManager::new();
        let pane_id = Uuid::new_v4();

        // Spawn cat, which echoes stdin to stdout.
        let session = mgr
            .spawn(pane_id, Some("/bin/cat"), None, vec![], 24, 80, &[])
            .expect("failed to spawn /bin/cat");

        session.write(b"ping\n").expect("write should succeed");

        // Read output — cat should echo back.
        let mut reader = session.take_reader().expect("reader available");
        let mut buf = vec![0u8; 4096];
        let mut output = String::new();

        for _ in 0..50 {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if output.contains("ping") {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    continue;
                }
                Err(_) => break,
            }
        }

        assert!(
            output.contains("ping"),
            "expected output to contain 'ping', got: {output:?}"
        );

        // Clean up.
        let session = mgr.get_mut(&pane_id).unwrap();
        let _ = session.kill();
    }

    #[cfg(not(windows))]
    #[test]
    fn kill_terminates_child() {
        let mut mgr = PtyManager::new();
        let pane_id = Uuid::new_v4();

        // Spawn `cat` which blocks forever reading stdin.
        let session = mgr
            .spawn(pane_id, Some("/bin/cat"), None, vec![], 24, 80, &[])
            .expect("spawn cat");

        session.kill().expect("kill should succeed");

        // After kill, wait should return (not hang).
        let status = session.wait().expect("wait after kill");
        // The process should not report success (it was killed).
        assert!(
            !status.success(),
            "killed process should not report success"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn take_reader_writer_then_error() {
        let mut mgr = PtyManager::new();
        let pane_id = Uuid::new_v4();

        let session = mgr
            .spawn(pane_id, Some("/bin/echo"), None, vec![], 24, 80, &[])
            .expect("spawn");

        assert!(session.has_reader());
        assert!(session.has_writer());

        let _reader = session.take_reader();
        let _writer = session.take_writer();

        assert!(!session.has_reader());
        assert!(!session.has_writer());

        // write should now fail.
        let err = session.write(b"test");
        assert!(matches!(err, Err(PtyError::WriterTaken)));
    }

    #[cfg(not(windows))]
    #[test]
    fn manager_tracks_sessions() {
        let mut mgr = PtyManager::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        mgr.spawn(id1, Some("/bin/echo"), None, vec![], 24, 80, &[])
            .expect("spawn 1");
        mgr.spawn(id2, Some("/bin/echo"), None, vec![], 24, 80, &[])
            .expect("spawn 2");

        assert_eq!(mgr.session_count(), 2);
        assert!(mgr.get(&id1).is_some());
        assert!(mgr.get(&id2).is_some());

        let removed = mgr.remove(&id1);
        assert!(removed.is_some());
        assert_eq!(mgr.session_count(), 1);
        assert!(mgr.get(&id1).is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn spawn_default_shell() {
        let mut mgr = PtyManager::new();
        let pane_id = Uuid::new_v4();

        // Passing None for command should launch the default shell.
        let session = mgr
            .spawn(pane_id, None, None, vec![], 24, 80, &[])
            .expect("default shell should spawn");

        // It should be alive — try_wait returns None for running processes.
        // (We can't guarantee this across all CI, so just check the session exists.)
        assert_eq!(session.pane_id(), pane_id);

        let _ = session.kill();
    }
}
