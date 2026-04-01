mod commands;

use std::sync::Arc;

use paneflow_terminal::bridge::PtyBridge;

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
        .manage(bridge_state)
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::spawn_pane,
            commands::write_pty,
            commands::resize_pty,
            commands::close_pane,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PaneFlow");
}
