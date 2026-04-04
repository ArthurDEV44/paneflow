use gpui::{
    div, prelude::*, px, rgb, svg, App, Context, Decorations, IntoElement, MouseButton, Render,
    Styled, Window,
};

/// Maximum workspace name display length before truncation.
const MAX_WORKSPACE_NAME_LEN: usize = 40;

pub struct TitleBar {
    should_move: bool,
    pub workspace_name: Option<String>,
}

impl TitleBar {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            should_move: false,
            workspace_name: None,
        }
    }
}

impl Render for TitleBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (1.75 * window.rem_size()).max(px(34.));

        // --- Left side: app title + workspace name ---
        let mut title = div().flex().flex_row().items_center().gap(px(8.)).child(
            div()
                .text_color(rgb(0xcdd6f4))
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .child("PaneFlow"),
        );

        if let Some(name) = &self.workspace_name {
            let display_name = if name.chars().count() > MAX_WORKSPACE_NAME_LEN {
                let truncated: String = name.chars().take(MAX_WORKSPACE_NAME_LEN).collect();
                format!("{truncated}...")
            } else {
                name.clone()
            };

            title = title
                .child(div().text_color(rgb(0x6c7086)).text_sm().child("\u{2014}"))
                .child(
                    div()
                        .text_color(rgb(0xa6adc8))
                        .text_sm()
                        .child(display_name),
                );
        }

        // --- Right side: window control buttons (CSD only) ---
        let is_csd = matches!(window.window_decorations(), Decorations::Client { .. });
        let controls = if is_csd {
            Some(render_window_controls(window))
        } else {
            None
        };

        div()
            .id("title-bar")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .w_full()
            .h(height)
            .bg(rgb(0x181825))
            .border_b_1()
            .border_color(rgb(0x313244))
            .px(px(12.))
            // Drag-to-move state machine
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.should_move = false;
                }),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.should_move = false;
            }))
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.should_move {
                    this.should_move = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.zoom_window();
                }
            })
            .child(title)
            .children(controls)
    }
}

/// Render close/minimize/maximize buttons for Linux CSD.
fn render_window_controls(window: &Window) -> impl IntoElement {
    let is_maximized = window.is_maximized();
    let supported = window.window_controls();

    let mut buttons = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.))
        // Prevent mouse-down on the button strip from triggering title bar drag
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

    if supported.minimize {
        buttons = buttons.child(window_control_button(
            "wc-minimize",
            "icons/generic_minimize.svg",
            |_, window: &mut Window, _cx| window.minimize_window(),
        ));
    }

    if supported.maximize {
        let icon = if is_maximized {
            "icons/generic_restore.svg"
        } else {
            "icons/generic_maximize.svg"
        };
        buttons = buttons.child(window_control_button(
            "wc-maximize",
            icon,
            |_, window: &mut Window, _cx| window.zoom_window(),
        ));
    }

    // Close is always shown
    buttons.child(window_control_button(
        "wc-close",
        "icons/generic_close.svg",
        |_, _window, cx: &mut App| cx.quit(),
    ))
}

/// A single window control button (close, minimize, or maximize/restore).
fn window_control_button(
    id: &'static str,
    icon_path: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(22.))
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x45475a)))
        .active(|s| s.bg(rgb(0x585b70)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |e, window, cx| {
            cx.stop_propagation();
            on_click(e, window, cx);
        })
        .child(
            svg()
                .size(px(16.))
                .flex_none()
                .path(icon_path)
                .text_color(rgb(0xcdd6f4)),
        )
}
