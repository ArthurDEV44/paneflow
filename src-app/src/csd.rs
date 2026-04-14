//! Shared CSD (Client-Side Decoration) utilities used by the main window and
//! settings window. Avoids duplicating resize-edge hit-testing and the default
//! window-button layout across multiple files.

use gpui::{
    Bounds, Pixels, Point, ResizeEdge, Size, WindowButton, WindowButtonLayout, point, px, size,
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
