use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use paneflow_terminal::bridge::{BridgeError, PtyBridge, TerminalEvent};

/// Shared bridge state managed by Tauri.
pub type BridgeState = Arc<Mutex<PtyBridge>>;

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to PaneFlow.", name)
}

/// Stub for attaching a Tauri Channel to a pane's output stream.
///
/// The full implementation will subscribe the channel to `TerminalEvent`s
/// for the given pane. For now this is a no-op placeholder.
#[tauri::command]
pub async fn attach_pty(
    _pane_id: String,
    _channel: tauri::ipc::Channel<TerminalEvent>,
) -> Result<(), String> {
    // TODO: Wire up the channel to the bridge's event stream in a future US.
    Ok(())
}

/// Send input bytes to the PTY for a given pane.
#[tauri::command]
pub async fn write_pty(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
    bytes: Vec<u8>,
) -> Result<(), BridgeError> {
    let id = Uuid::parse_str(&pane_id)
        .map_err(|_| BridgeError::PaneNotFound(Uuid::nil()))?;
    let bridge = state.lock().await;
    bridge.write_pane(id, &bytes).await
}

/// Resize the PTY and terminal emulator for a given pane.
#[tauri::command]
pub async fn resize_pty(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
    rows: u16,
    cols: u16,
) -> Result<(), BridgeError> {
    let id = Uuid::parse_str(&pane_id)
        .map_err(|_| BridgeError::PaneNotFound(Uuid::nil()))?;
    let bridge = state.lock().await;
    bridge.resize_pane(id, rows, cols).await
}
