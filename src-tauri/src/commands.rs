use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use paneflow_terminal::bridge::{BridgeError, PtyBridge, TerminalEvent};

/// Shared bridge state managed by Tauri.
pub type BridgeState = Arc<Mutex<PtyBridge>>;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_pane_id(pane_id: &str) -> Result<Uuid, BridgeError> {
    Uuid::parse_str(pane_id).map_err(|_| BridgeError::PaneNotFound(Uuid::nil()))
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to PaneFlow.", name)
}

/// Spawn a PTY process for a pane and stream its output to the given Tauri Channel.
///
/// This is the primary command that connects a frontend terminal pane to a real
/// shell process. It:
/// 1. Spawns a PTY via the bridge (creates a shell process)
/// 2. Starts a background reader task that reads PTY output
/// 3. Forwards all TerminalEvents to the provided Tauri Channel
#[tauri::command]
pub async fn spawn_pane(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
    cwd: Option<String>,
    rows: u16,
    cols: u16,
    channel: tauri::ipc::Channel<TerminalEvent>,
) -> Result<(), BridgeError> {
    let id = parse_pane_id(&pane_id)?;
    let cwd_path = cwd.map(PathBuf::from);

    // Create an unbounded channel for the bridge to emit events into.
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<TerminalEvent>();

    // Inject PaneFlow env vars (similar to cmux's CMUX_SURFACE_ID, etc.)
    let env = vec![
        ("PANEFLOW".to_string(), "1".to_string()),
        ("PANEFLOW_PANE_ID".to_string(), pane_id.clone()),
    ];

    // Spawn the PTY session + reader task.
    let bridge = state.lock().await;
    bridge.spawn_pane(id, None, cwd_path, env, rows, cols, event_tx).await?;
    drop(bridge);

    // Forward events from the mpsc receiver to the Tauri Channel.
    // This task runs for the lifetime of the pane.
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if channel.send(event).is_err() {
                // Channel closed — frontend disconnected.
                break;
            }
        }
    });

    Ok(())
}

/// Send input bytes to the PTY for a given pane.
#[tauri::command]
pub async fn write_pty(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
    bytes: Vec<u8>,
) -> Result<(), BridgeError> {
    let id = parse_pane_id(&pane_id)?;
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
    let id = parse_pane_id(&pane_id)?;
    let bridge = state.lock().await;
    bridge.resize_pane(id, rows, cols).await
}

/// Close a pane: kill the PTY process and clean up resources.
#[tauri::command]
pub async fn close_pane(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
) -> Result<(), BridgeError> {
    let id = parse_pane_id(&pane_id)?;
    let bridge = state.lock().await;
    bridge.close_pane(id).await
}
