#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Workaround: WebKitGTK DMA-BUF renderer crashes on NVIDIA + Wayland.
    // Upstream: https://github.com/tauri-apps/tauri/issues/10702
    #[cfg(target_os = "linux")]
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    paneflow_app::run()
}
