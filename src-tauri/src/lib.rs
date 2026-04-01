mod commands;

use std::sync::Arc;
use uuid::Uuid;

use tauri::Listener;
use paneflow_terminal::bridge::PtyBridge;

/// Payload for the fire-and-forget PTY input event.
#[derive(Debug, Clone, serde::Deserialize)]
struct PtyInputEvent {
    pane_id: String,
    data: String,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "paneflow=debug".into()),
        )
        .init();

    let bridge_state: commands::BridgeState = Arc::new(PtyBridge::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(bridge_state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::spawn_pane,
            commands::write_pty,
            commands::resize_pty,
            commands::close_pane,
        ])
        .setup(move |app| {
            // Listen for fire-and-forget PTY input events from the frontend.
            // Events are one-way (no response), ~2x faster than invoke().
            let bridge = bridge_state.clone();
            app.listen("pty-input", move |event| {
                if let Ok(payload) = serde_json::from_str::<PtyInputEvent>(event.payload()) {
                    if let Ok(id) = Uuid::parse_str(&payload.pane_id) {
                        let _ = bridge.write_pane(id, payload.data.as_bytes());
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PaneFlow");
}
