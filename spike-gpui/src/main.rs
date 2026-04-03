//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar + main content area.
//! Terminal cells rendered via TerminalElement with GPUI's Element trait.

mod keys;
mod split;
mod terminal;
mod terminal_element;

use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Bounds, Context, Focusable, IntoElement,
    KeyBinding, Render, Styled, Window, WindowBounds, WindowOptions,
};
use gpui_platform::application;

use crate::split::{FocusDirection, SplitDirection, SplitNode};
use crate::terminal::TerminalView;

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

actions!(
    paneflow,
    [
        SplitHorizontally,
        SplitVertically,
        ClosePane,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown
    ]
);

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

struct PaneFlowApp {
    root: Option<SplitNode>,
}

impl PaneFlowApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let terminal_view = cx.new(TerminalView::new);
        Self {
            root: Some(SplitNode::Leaf(terminal_view)),
        }
    }

    fn split(&mut self, direction: SplitDirection, window: &mut Window, cx: &mut Context<Self>) {
        const MAX_PANES: usize = 32;
        if let Some(root) = &self.root
            && root.leaf_count() >= MAX_PANES
        {
            return;
        }
        let new_terminal = cx.new(TerminalView::new);
        if let Some(root) = &mut self.root
            && root.split_at_focused(direction, new_terminal.clone(), window, cx)
        {
            new_terminal.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn handle_split_h(
        &mut self,
        _: &SplitHorizontally,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split(SplitDirection::Horizontal, window, cx);
    }

    fn handle_split_v(
        &mut self,
        _: &SplitVertically,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split(SplitDirection::Vertical, window, cx);
    }

    fn handle_close_pane(
        &mut self,
        _: &ClosePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(root) = self.root.take() {
            let (new_root, _closed) = root.close_focused(window, cx);
            self.root = new_root;
            if let Some(ref root) = self.root {
                root.focus_first(window, cx);
            }
        }
        cx.notify();
    }

    fn handle_focus(&mut self, dir: FocusDirection, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = &self.root {
            root.focus_in_direction(dir, window, cx);
        }
        cx.notify();
    }

    fn handle_focus_left(&mut self, _: &FocusLeft, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Left, w, cx);
    }
    fn handle_focus_right(&mut self, _: &FocusRight, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Right, w, cx);
    }
    fn handle_focus_up(&mut self, _: &FocusUp, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Up, w, cx);
    }
    fn handle_focus_down(&mut self, _: &FocusDown, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Down, w, cx);
    }
}

impl Render for PaneFlowApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main_content = if let Some(root) = &self.root {
            root.render(window, cx)
        } else {
            // Empty state — no terminal panes
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(
                    div()
                        .text_color(rgb(0x6c7086))
                        .child("No terminal panes open"),
                )
                .into_any_element()
        };

        div()
            .flex()
            .flex_row()
            .size_full()
            .on_action(cx.listener(Self::handle_split_h))
            .on_action(cx.listener(Self::handle_split_v))
            .on_action(cx.listener(Self::handle_close_pane))
            .on_action(cx.listener(Self::handle_focus_left))
            .on_action(cx.listener(Self::handle_focus_right))
            .on_action(cx.listener(Self::handle_focus_up))
            .on_action(cx.listener(Self::handle_focus_down))
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
            // Main content area
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(rgb(0x1e1e2e))
                    .overflow_hidden()
                    .child(main_content),
            )
    }
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

fn main() {
    env_logger::init();

    application().run(|cx: &mut App| {
        // Register keybindings
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-d", SplitHorizontally, None),
            KeyBinding::new("ctrl-shift-e", SplitVertically, None),
            KeyBinding::new("ctrl-shift-w", ClosePane, None),
            KeyBinding::new("alt-left", FocusLeft, None),
            KeyBinding::new("alt-right", FocusRight, None),
            KeyBinding::new("alt-up", FocusUp, None),
            KeyBinding::new("alt-down", FocusDown, None),
        ]);

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
                // Focus the first terminal pane
                view.update(cx, |app, cx| {
                    if let Some(root) = &app.root {
                        root.focus_first(window, cx);
                    }
                });
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
