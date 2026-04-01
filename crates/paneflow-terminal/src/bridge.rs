//! US-006: Async PTY I/O bridge.
//!
//! `PtyBridge` pairs each PTY session with a terminal emulator and an async
//! reader task. PTY output is read in a blocking thread (via
//! `tokio::task::spawn_blocking`), batched at ~16 ms intervals, and forwarded
//! through a caller-supplied callback.

use crate::emulator::TerminalEmulator;
use crate::pty_manager::{PtyError, PtyManager};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events emitted by the PTY bridge to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum TerminalEvent {
    /// Raw bytes received from the PTY.
    Data { pane_id: String, bytes: Vec<u8> },
    /// The child process has exited.
    Exit { pane_id: String, code: i32 },
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur in the bridge layer.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("PTY error: {0}")]
    Pty(#[from] PtyError),

    #[error("pane not found: {0}")]
    PaneNotFound(Uuid),

    #[error("reader already taken for pane: {0}")]
    ReaderTaken(Uuid),
}

/// Convenience alias for bridge results. Uses `String` as the error type so
/// it can be returned directly from `#[tauri::command]` handlers.
pub type Result<T> = std::result::Result<T, BridgeError>;

// We need a serialisable error for Tauri commands.
impl serde::Serialize for BridgeError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// ---------------------------------------------------------------------------
// Shutdown handles — one per reader task
// ---------------------------------------------------------------------------

/// Handle used to signal a reader task to stop.
struct PaneHandle {
    /// Dropping the sender causes the receiver in the reader task to resolve,
    /// which we use as a cancellation signal.
    _shutdown_tx: mpsc::Sender<()>,
}

// ---------------------------------------------------------------------------
// PtyBridge
// ---------------------------------------------------------------------------

/// Manages the lifecycle of PTY + emulator pairs and their async reader tasks.
pub struct PtyBridge {
    pty_manager: Arc<Mutex<PtyManager>>,
    emulators: Arc<Mutex<HashMap<Uuid, TerminalEmulator>>>,
    handles: Arc<Mutex<HashMap<Uuid, PaneHandle>>>,
}

impl PtyBridge {
    /// Create a new `PtyBridge`.
    pub fn new() -> Self {
        Self {
            pty_manager: Arc::new(Mutex::new(PtyManager::new())),
            emulators: Arc::new(Mutex::new(HashMap::new())),
            handles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Spawn a new pane: creates a PTY session and a terminal emulator,
    /// then starts an async reader task that forwards output via `event_tx`.
    ///
    /// The `event_tx` sender can be cloned and shared across multiple panes.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_pane(
        &self,
        pane_id: Uuid,
        command: Option<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        rows: u16,
        cols: u16,
        event_tx: mpsc::UnboundedSender<TerminalEvent>,
    ) -> Result<()> {
        // Spawn the PTY session.
        let mut mgr = self.pty_manager.lock().await;
        let session = mgr.spawn(
            pane_id,
            command.as_deref(),
            cwd,
            env,
            rows,
            cols,
            &[],
        )?;

        // Take the reader — the bridge owns it from now on.
        let reader = session
            .take_reader()
            .ok_or(BridgeError::ReaderTaken(pane_id))?;

        drop(mgr);

        // Create the emulator.
        let emulator = TerminalEmulator::new(rows, cols);
        self.emulators.lock().await.insert(pane_id, emulator);

        // Create a shutdown channel.
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        self.handles
            .lock()
            .await
            .insert(pane_id, PaneHandle { _shutdown_tx: shutdown_tx });

        // Start the reader thread + async forwarder.
        Self::start_reader(
            reader,
            pane_id,
            pane_id.to_string(),
            Arc::clone(&self.emulators),
            event_tx,
            shutdown_rx,
        );

        Ok(())
    }

    /// Spawn a dedicated blocking thread that reads PTY output and forwards
    /// raw bytes to the async event channel. A separate async task then
    /// processes bytes through the emulator and forwards to the frontend.
    ///
    /// Architecture (low-latency):
    /// - ONE persistent blocking thread per pane (no per-read spawn_blocking)
    /// - Raw bytes sent immediately via channel (no sleep)
    /// - Emulator processing is done on the async side
    fn start_reader(
        mut reader: Box<dyn Read + Send>,
        pane_id: Uuid,
        pane_id_str: String,
        emulators: Arc<Mutex<HashMap<Uuid, TerminalEmulator>>>,
        event_tx: mpsc::UnboundedSender<TerminalEvent>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        // Channel from the blocking reader thread to the async forwarder.
        let (raw_tx, mut raw_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Blocking reader thread — reads from PTY as fast as possible.
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,       // EOF
                    Ok(n) => {
                        if raw_tx.send(buf[..n].to_vec()).is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        // Async forwarder — processes bytes through emulator and sends events.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    bytes = raw_rx.recv() => {
                        match bytes {
                            Some(bytes) => {
                                // Process through the emulator.
                                {
                                    let mut ems = emulators.lock().await;
                                    if let Some(emu) = ems.get_mut(&pane_id) {
                                        emu.process_bytes(&bytes);
                                    }
                                }

                                // Forward to the frontend immediately.
                                if event_tx
                                    .send(TerminalEvent::Data {
                                        pane_id: pane_id_str.clone(),
                                        bytes,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            None => break, // reader thread exited
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::debug!(pane_id = %pane_id, "reader task: shutdown");
                        break;
                    }
                }
            }

            // Emit exit event.
            let _ = event_tx.send(TerminalEvent::Exit {
                pane_id: pane_id_str,
                code: 0,
            });
        });
    }

    /// Write input bytes to the PTY for a given pane.
    pub async fn write_pane(&self, pane_id: Uuid, data: &[u8]) -> Result<()> {
        let mut mgr = self.pty_manager.lock().await;
        let session = mgr
            .get_mut(&pane_id)
            .ok_or(BridgeError::PaneNotFound(pane_id))?;
        session.write(data)?;
        Ok(())
    }

    /// Resize both the PTY and the terminal emulator for a given pane.
    pub async fn resize_pane(&self, pane_id: Uuid, rows: u16, cols: u16) -> Result<()> {
        // Resize the PTY.
        {
            let mgr = self.pty_manager.lock().await;
            let session = mgr
                .get(&pane_id)
                .ok_or(BridgeError::PaneNotFound(pane_id))?;
            session.resize(rows, cols)?;
        }

        // Resize the emulator.
        {
            let mut ems = self.emulators.lock().await;
            if let Some(emu) = ems.get_mut(&pane_id) {
                emu.resize(rows, cols);
            }
        }

        Ok(())
    }

    /// Close a pane: kill the PTY, remove the emulator, and signal the reader
    /// task to stop.
    pub async fn close_pane(&self, pane_id: Uuid) -> Result<()> {
        // Remove the handle first — dropping the shutdown sender signals the
        // reader task.
        self.handles.lock().await.remove(&pane_id);

        // Kill the PTY session.
        {
            let mut mgr = self.pty_manager.lock().await;
            if let Some(session) = mgr.get_mut(&pane_id) {
                let _ = session.kill();
            }
            mgr.remove(&pane_id);
        }

        // Remove the emulator.
        self.emulators.lock().await.remove(&pane_id);

        tracing::info!(pane_id = %pane_id, "closed pane");
        Ok(())
    }
}

impl Default for PtyBridge {
    fn default() -> Self {
        Self::new()
    }
}
