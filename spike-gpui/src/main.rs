//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar + main content area.
//! Terminal cells rendered via TerminalElement with GPUI's Element trait.

mod terminal;
mod terminal_element;

use gpui::{
    div, prelude::*, px, rgb, size, App, Bounds, Context, Entity, Focusable, IntoElement, Render,
    Styled, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

use crate::terminal::TerminalView;

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

struct PaneFlowApp {
    pub terminal_view: Entity<TerminalView>,
}

impl PaneFlowApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let terminal_view = cx.new(TerminalView::new);
        // Focus the terminal on startup
        Self { terminal_view }
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
            // Main content area — terminal
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(rgb(0x1e1e2e))
                    .child(self.terminal_view.clone()),
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
            |window, cx| {
                let view = cx.new(PaneFlowApp::new);
                let focus = view.read(cx).terminal_view.read(cx).focus_handle(cx);
                focus.focus(window, cx);
                view
            },
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
