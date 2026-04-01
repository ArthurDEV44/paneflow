use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use paneflow_terminal::bridge::{BridgeError, PtyBridge, TerminalEvent};

/// Shared bridge state. NOT behind a tokio::Mutex — the bridge manages
/// its own internal locking. This means write_pty can avoid any global lock.
pub type BridgeState = Arc<PtyBridge>;

fn parse_pane_id(pane_id: &str) -> Result<Uuid, BridgeError> {
    Uuid::parse_str(pane_id).map_err(|_| BridgeError::PaneNotFound(Uuid::nil()))
}

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to PaneFlow.", name)
}

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

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<TerminalEvent>();

    let env = vec![
        ("PANEFLOW".to_string(), "1".to_string()),
        ("PANEFLOW_PANE_ID".to_string(), pane_id.clone()),
    ];

    state.spawn_pane(id, None, cwd_path, env, rows, cols, event_tx).await?;

    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if channel.send(event).is_err() {
                break;
            }
        }
    });

    Ok(())
}

/// Write input to a pane's PTY.
/// **Zero global locks** — calls PtyBridge::write_pane which only locks
/// the per-pane std::sync::Mutex (~µs).
#[tauri::command]
pub fn write_pty(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
    data: String,
) -> Result<(), BridgeError> {
    let id = parse_pane_id(&pane_id)?;
    state.write_pane(id, data.as_bytes())
}

#[tauri::command]
pub async fn resize_pty(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
    rows: u16,
    cols: u16,
) -> Result<(), BridgeError> {
    let id = parse_pane_id(&pane_id)?;
    state.resize_pane(id, rows, cols).await
}

#[tauri::command]
pub async fn close_pane(
    state: tauri::State<'_, BridgeState>,
    pane_id: String,
) -> Result<(), BridgeError> {
    let id = parse_pane_id(&pane_id)?;
    state.close_pane(id).await
}
