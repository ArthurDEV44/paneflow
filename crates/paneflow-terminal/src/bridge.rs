//! Async PTY I/O bridge.
//!
//! Architecture (Ghostty-inspired):
//! - Per-pane writer handles behind `std::sync::Mutex` (no global lock for writes)
//! - Dedicated OS thread per pane for blocking PTY reads
//! - Coalescing forwarder: drains all pending chunks before sending one event
//! - Base64-encoded output to avoid JSON number array overhead
//! - Bounded channel for backpressure under fast output

use crate::emulator::TerminalEmulator;
use crate::pty_manager::{PtyError, PtyManager};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex as TokioMutex};
use uuid::Uuid;

/// Max bytes per output batch sent to the frontend.
/// Keeps individual `term.write()` calls small so the JS event loop
/// can process keyboard events between chunks.
const MAX_BATCH_BYTES: usize = 32 * 1024;

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// Raw PTY output bytes (no base64 encoding — direct in-process use).
    Data { pane_id: Uuid, data: Vec<u8> },
    /// PTY process exited with the given status code.
    Exit { pane_id: Uuid, code: i32 },
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("PTY error: {0}")]
    Pty(#[from] PtyError),

    #[error("pane not found: {0}")]
    PaneNotFound(Uuid),

    #[error("reader already taken for pane: {0}")]
    ReaderTaken(Uuid),

    #[error("write failed: {0}")]
    WriteFailed(String),
}

pub type Result<T> = std::result::Result<T, BridgeError>;

impl serde::Serialize for BridgeError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// ---------------------------------------------------------------------------
// Per-pane writer handle (lock-free from the caller's perspective)
// ---------------------------------------------------------------------------

/// A per-pane writer handle. Uses `std::sync::Mutex` (not tokio) because
/// the write syscall is fast (~µs) and we don't want tokio overhead.
struct PaneWriter {
    writer: std::sync::Mutex<Box<dyn Write + Send>>,
}

impl PaneWriter {
    fn write(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().map_err(|e| {
            BridgeError::WriteFailed(format!("writer mutex poisoned: {e}"))
        })?;
        w.write_all(data).map_err(|e| {
            BridgeError::WriteFailed(e.to_string())
        })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shutdown handle
// ---------------------------------------------------------------------------

struct PaneHandle {
    _shutdown_tx: mpsc::Sender<()>,
}

// ---------------------------------------------------------------------------
// PtyBridge
// ---------------------------------------------------------------------------

pub struct PtyBridge {
    pty_manager: TokioMutex<PtyManager>,
    emulators: TokioMutex<HashMap<Uuid, TerminalEmulator>>,
    /// Per-pane writers — NO global lock needed for writes.
    /// Arc<PaneWriter> uses std::sync::Mutex internally.
    writers: std::sync::RwLock<HashMap<Uuid, Arc<PaneWriter>>>,
    handles: std::sync::Mutex<HashMap<Uuid, PaneHandle>>,
}

impl PtyBridge {
    pub fn new() -> Self {
        Self {
            pty_manager: TokioMutex::new(PtyManager::new()),
            emulators: TokioMutex::new(HashMap::new()),
            writers: std::sync::RwLock::new(HashMap::new()),
            handles: std::sync::Mutex::new(HashMap::new()),
        }
    }

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

        // Take reader AND writer out of the session.
        let reader = session
            .take_reader()
            .ok_or(BridgeError::ReaderTaken(pane_id))?;
        let writer = session
            .take_writer()
            .ok_or(BridgeError::WriteFailed("writer already taken".into()))?;

        drop(mgr);

        // Store the writer in the per-pane map (no global lock on write path).
        let pane_writer = Arc::new(PaneWriter {
            writer: std::sync::Mutex::new(writer),
        });
        self.writers.write().unwrap().insert(pane_id, pane_writer);

        // Create the emulator (lazy, not on hot path).
        self.emulators
            .lock()
            .await
            .insert(pane_id, TerminalEmulator::new(rows, cols));

        // Shutdown channel.
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);
        self.handles
            .lock()
            .unwrap()
            .insert(pane_id, PaneHandle { _shutdown_tx: shutdown_tx });

        // Start reader pipeline.
        Self::start_reader(reader, pane_id, event_tx, shutdown_rx);

        Ok(())
    }

    /// Write input to a pane's PTY. **No global lock** — only locks the
    /// per-pane std::sync::Mutex (fast, ~µs).
    pub fn write_pane(&self, pane_id: Uuid, data: &[u8]) -> Result<()> {
        let writers = self.writers.read().unwrap();
        let writer = writers
            .get(&pane_id)
            .ok_or(BridgeError::PaneNotFound(pane_id))?;
        writer.write(data)
    }

    pub async fn resize_pane(&self, pane_id: Uuid, rows: u16, cols: u16) -> Result<()> {
        {
            let mgr = self.pty_manager.lock().await;
            let session = mgr
                .get(&pane_id)
                .ok_or(BridgeError::PaneNotFound(pane_id))?;
            session.resize(rows, cols)?;
        }
        {
            let mut ems = self.emulators.lock().await;
            if let Some(emu) = ems.get_mut(&pane_id) {
                emu.resize(rows, cols);
            }
        }
        Ok(())
    }

    pub async fn close_pane(&self, pane_id: Uuid) -> Result<()> {
        // Drop shutdown handle → signals reader task.
        self.handles.lock().unwrap().remove(&pane_id);

        // Remove writer.
        self.writers.write().unwrap().remove(&pane_id);

        // Kill PTY.
        {
            let mut mgr = self.pty_manager.lock().await;
            if let Some(session) = mgr.get_mut(&pane_id) {
                let _ = session.kill();
            }
            mgr.remove(&pane_id);
        }

        self.emulators.lock().await.remove(&pane_id);
        tracing::info!(pane_id = %pane_id, "closed pane");
        Ok(())
    }

    // ── Reader pipeline ────────────────────────────────────────────────

    fn start_reader(
        mut reader: Box<dyn Read + Send>,
        pane_id: Uuid,
        event_tx: mpsc::UnboundedSender<TerminalEvent>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        let (raw_tx, mut raw_rx) = mpsc::channel::<Vec<u8>>(64);

        // Blocking reader thread (dedicated OS thread per pane — US-008).
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if raw_tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        // Async coalescing forwarder.
        // Coalesces burst output and caps each batch at MAX_BATCH_BYTES.
        // Raw bytes are sent directly — no base64 encoding (v2: in-process use).
        tokio::spawn(async move {
            let mut batch = Vec::with_capacity(MAX_BATCH_BYTES);

            loop {
                tokio::select! {
                    chunk = raw_rx.recv() => {
                        match chunk {
                            Some(data) => batch.extend_from_slice(&data),
                            None => break,
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }

                // Drain pending chunks, but respect the batch size cap.
                while batch.len() < MAX_BATCH_BYTES {
                    match raw_rx.try_recv() {
                        Ok(data) => batch.extend_from_slice(&data),
                        Err(_) => break,
                    }
                }

                // Send raw bytes in MAX_BATCH_BYTES-sized chunks.
                for chunk in batch.chunks(MAX_BATCH_BYTES) {
                    if event_tx
                        .send(TerminalEvent::Data {
                            pane_id,
                            data: chunk.to_vec(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                batch.clear();
            }

            let _ = event_tx.send(TerminalEvent::Exit {
                pane_id,
                code: 0,
            });
        });
    }
}

impl Default for PtyBridge {
    fn default() -> Self {
        Self::new()
    }
}
