//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar + main content area.
//! Terminal rendering and PTY integration are added in subsequent stories (US-002, US-004).

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, IntoElement, Render, Styled, Window,
    WindowBounds, WindowOptions,
};
use gpui_platform::application;

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

struct PaneFlowApp;

impl PaneFlowApp {
    fn new(_cx: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for PaneFlowApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .size_full()
            // Sidebar — 220px fixed width
            .child(
                div()
                    .w(px(220.))
                    .flex_shrink_0()
                    .h_full()
                    .bg(rgb(0x181825))
                    .border_r_1()
                    .border_color(rgb(0x313244))
                    .p_3()
                    .child(
                        div()
                            .text_color(rgb(0xcdd6f4))
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("PaneFlow"),
                    ),
            )
            // Main content area — flexible
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(rgb(0x1e1e2e))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .child("No terminal panes open"),
                    ),
            )
    }
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

fn main() {
    env_logger::init();

    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);

        let window_result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(800.0), px(500.0))),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("PaneFlow".into()),
                    ..Default::default()
                }),
                app_id: Some("paneflow".into()),
                ..Default::default()
            },
            |_window, cx| cx.new(PaneFlowApp::new),
        );

        match window_result {
            Ok(_) => cx.activate(true),
            Err(e) => {
                log::error!(
                    "Failed to open PaneFlow window: {e}\n\n\
                     This usually means your GPU driver does not support Vulkan (Linux) \
                     or Metal (macOS).\n\n\
                     Troubleshooting:\n\
                     - Linux: install mesa-vulkan-drivers or your GPU vendor's Vulkan ICD\n\
                     - Run `vulkaninfo` to verify Vulkan support\n\
                     - Try setting WGPU_BACKEND=gl for OpenGL fallback"
                );
                eprintln!(
                    "Error: Failed to open PaneFlow window.\n\n\
                     Your GPU driver may not support Vulkan. \
                     Install mesa-vulkan-drivers or run with RUST_LOG=error for details."
                );
                std::process::exit(1);
            }
        }
    });
}
