//! Settings window — separate GPUI window for app-wide configuration
//! (keyboard shortcuts, theme, font).
//!
//! Layout:
//! - `window`   — `SettingsWindow` struct + lifecycle + `Render` shell
//! - `keyboard` — key-down handler + shortcut capture
//! - `tabs`     — per-panel bodies (`shortcuts`, `appearance`)
//!
//! The `open_or_focus` entry point is called from `PaneFlowApp` and either
//! focuses the existing settings window or opens a new one.

pub mod keyboard;
pub mod sidebar;
pub mod tabs;
pub mod window;

pub use window::SettingsWindow;

use gpui::{AppContext, Context, Window, WindowBounds, WindowDecorations, WindowOptions, px, size};

/// Open the settings window, or activate + focus the existing one.
pub fn open_or_focus<T>(_window: &mut Window, cx: &mut Context<T>) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<SettingsWindow>())
    {
        existing
            .update(cx, |settings_window, window, cx| {
                window.activate_window();
                settings_window.settings_focus.focus(window, cx);
            })
            .ok();
        return;
    }

    let config = paneflow_config::loader::load_config();
    let decorations = match config.window_decorations.as_deref() {
        Some("server") => WindowDecorations::Server,
        Some("client") | None => WindowDecorations::Client,
        Some(_) => WindowDecorations::Client,
    };

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(980.), px(720.)), cx)),
        window_min_size: Some(size(px(900.), px(520.))),
        window_decorations: Some(decorations),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("PaneFlow - Settings".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        app_id: Some("paneflow".into()),
        focus: true,
        show: true,
        ..Default::default()
    };

    if let Ok(settings_window) = cx.open_window(options, |window, cx| {
        let settings_window = cx.new(SettingsWindow::new);
        let focus = settings_window.read(cx).settings_focus.clone();
        focus.focus(window, cx);
        settings_window
    }) {
        let _ = settings_window.update(cx, |settings_window, window, cx| {
            window.activate_window();
            settings_window.settings_focus.focus(window, cx);
        });
    }
}
