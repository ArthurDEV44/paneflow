use gpui::{
    div, prelude::*, px, rgb, svg, App, Decorations, IntoElement, MouseButton, Styled, Window,
};

/// Maximum workspace name display length before truncation.
const MAX_WORKSPACE_NAME_LEN: usize = 40;

/// Render the PaneFlow title bar with window controls.
///
/// Height follows Zed's formula: `(1.75 * rem_size).max(34px)`.
/// Window control buttons are only rendered in CSD mode.
pub fn render(workspace_name: Option<&str>, window: &Window) -> impl IntoElement {
    let height = (1.75 * window.rem_size()).max(px(34.));

    // --- Left side: app title + workspace name ---
    let mut title = div().flex().flex_row().items_center().gap(px(8.)).child(
        div()
            .text_color(rgb(0xcdd6f4))
            .text_sm()
            .font_weight(gpui::FontWeight::BOLD)
            .child("PaneFlow"),
    );

    if let Some(name) = workspace_name {
        let display_name = if name.chars().count() > MAX_WORKSPACE_NAME_LEN {
            let truncated: String = name.chars().take(MAX_WORKSPACE_NAME_LEN).collect();
            format!("{truncated}...")
        } else {
            name.to_string()
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
        .child(title)
        .children(controls)
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
        .hover(|s| s.bg(rgb(0x45475a))) // ghost_element_hover
        .active(|s| s.bg(rgb(0x585b70))) // ghost_element_active
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
