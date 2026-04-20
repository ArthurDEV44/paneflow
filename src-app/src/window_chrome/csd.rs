//! Shared CSD (Client-Side Decoration) utilities used by the main window and
//! settings window. Avoids duplicating resize-edge hit-testing and the default
//! window-button layout across multiple files.

use gpui::{
    AnyElement, App, Bounds, ClickEvent, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, ResizeEdge, SharedString, Size, Styled, Window, WindowButton,
    WindowButtonLayout, WindowControlArea, WindowControls, div, point, prelude::*, px, size, svg,
};

/// Default button layout when the DE doesn't provide one.
pub fn default_button_layout() -> WindowButtonLayout {
    WindowButtonLayout {
        left: [None, None, None],
        right: [
            Some(WindowButton::Minimize),
            Some(WindowButton::Maximize),
            Some(WindowButton::Close),
        ],
    }
}

/// Hit-test a mouse position against the CSD resize border.
///
/// Returns `Some(edge)` if the cursor is in a resize zone, respecting the
/// current tiling state (tiled edges are not resizable).
pub fn resize_edge(
    pos: Point<Pixels>,
    border: Pixels,
    window_size: Size<Pixels>,
    tiling: gpui::Tiling,
) -> Option<ResizeEdge> {
    let inner = Bounds::new(Point::default(), window_size).inset(border * 1.5);
    if inner.contains(&pos) {
        return None;
    }

    let corner = size(border * 1.5, border * 1.5);

    // Corners first (larger hit zone = 1.5× border)
    if !tiling.top && !tiling.left && Bounds::new(point(px(0.), px(0.)), corner).contains(&pos) {
        return Some(ResizeEdge::TopLeft);
    }
    if !tiling.top
        && !tiling.right
        && Bounds::new(point(window_size.width - corner.width, px(0.)), corner).contains(&pos)
    {
        return Some(ResizeEdge::TopRight);
    }
    if !tiling.bottom
        && !tiling.left
        && Bounds::new(point(px(0.), window_size.height - corner.height), corner).contains(&pos)
    {
        return Some(ResizeEdge::BottomLeft);
    }
    if !tiling.bottom
        && !tiling.right
        && Bounds::new(
            point(
                window_size.width - corner.width,
                window_size.height - corner.height,
            ),
            corner,
        )
        .contains(&pos)
    {
        return Some(ResizeEdge::BottomRight);
    }

    // Edges
    if !tiling.top && pos.y < border {
        Some(ResizeEdge::Top)
    } else if !tiling.bottom && pos.y > window_size.height - border {
        Some(ResizeEdge::Bottom)
    } else if !tiling.left && pos.x < border {
        Some(ResizeEdge::Left)
    } else if !tiling.right && pos.x > window_size.width - border {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

/// Render a group of window control buttons for one side (left or right).
///
/// Returns `None` if no buttons are active on this side (all slots are `None`
/// or all are filtered out by the compositor's supported controls).
///
/// `on_close` is invoked when the Close button is clicked, allowing each
/// caller (main title bar vs settings window) to dispatch its own close
/// semantics (event emission vs `window.remove_window()`).
pub(crate) fn render_button_group(
    side: &'static str,
    buttons: &[Option<WindowButton>; 3],
    is_maximized: bool,
    supported: &WindowControls,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> Option<AnyElement> {
    let children: Vec<AnyElement> = buttons
        .iter()
        .filter_map(|slot| *slot)
        .filter(|button| match button {
            WindowButton::Minimize => supported.minimize,
            WindowButton::Maximize => supported.maximize,
            WindowButton::Close => true,
        })
        .map(|button| render_window_button(side, button, is_maximized, on_close.clone()))
        .collect();

    if children.is_empty() {
        return None;
    }

    Some(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(children)
            .into_any_element(),
    )
}

/// Render a single window control button. Close button clicks dispatch to
/// the `on_close` callback; Min/Max call directly into `Window`.
pub(crate) fn render_window_button(
    side: &'static str,
    button: WindowButton,
    is_maximized: bool,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let id = match button {
        WindowButton::Minimize => "wc-minimize",
        WindowButton::Maximize => "wc-maximize",
        WindowButton::Close => "wc-close",
    };

    let icon_path = match button {
        WindowButton::Minimize => "icons/generic_minimize.svg",
        WindowButton::Maximize if is_maximized => "icons/generic_restore.svg",
        WindowButton::Maximize => "icons/generic_maximize.svg",
        WindowButton::Close => "icons/generic_close.svg",
    };

    let control_area = match button {
        WindowButton::Minimize => WindowControlArea::Min,
        WindowButton::Maximize => WindowControlArea::Max,
        WindowButton::Close => WindowControlArea::Close,
    };

    let element_id = format!("{id}-{side}");

    div()
        .id(SharedString::from(element_id))
        .window_control_area(control_area)
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(22.))
        .rounded_sm()
        .cursor_pointer()
        .hover(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle)
        })
        .active(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_: &ClickEvent, window, cx| {
            cx.stop_propagation();
            match button {
                WindowButton::Minimize => window.minimize_window(),
                WindowButton::Maximize => window.zoom_window(),
                WindowButton::Close => on_close(window, cx),
            }
        })
        .child({
            let ui = crate::theme::ui_colors();
            svg()
                .size(px(16.))
                .flex_none()
                .path(icon_path)
                .text_color(ui.text)
        })
        .into_any_element()
}
