//! Scrollbar thumb paint pass.

use gpui::{Bounds, Pixels, Point, Window, fill, px};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

/// Paint the 4px right-edge scrollbar thumb, only when there is scroll-back
/// content and the user has scrolled away from the bottom.
pub fn paint_scrollbar(
    layout: &LayoutState,
    geom: &CellGeometry,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    if layout.display_offset == 0 || layout.history_size == 0 {
        return;
    }

    let CellGeometry {
        origin,
        line_height,
        ..
    } = *geom;

    let scrollbar_width = px(4.0);
    let visible_rows = (bounds.size.height / line_height).floor() as usize;
    let total_lines = layout.history_size + visible_rows;
    let visible_ratio = visible_rows as f32 / total_lines as f32;
    let thumb_height = (bounds.size.height * visible_ratio).max(px(16.0));
    let scroll_ratio = layout.display_offset as f32 / layout.history_size as f32;
    // display_offset=max → scrolled to top → thumb at top
    let thumb_y =
        bounds.size.height - thumb_height - (bounds.size.height - thumb_height) * scroll_ratio;
    let scrollbar_color = layout.scrollbar_thumb;
    let scrollbar_bounds = Bounds::new(
        Point {
            x: origin.x + bounds.size.width - scrollbar_width,
            y: origin.y + thumb_y,
        },
        gpui::Size {
            width: scrollbar_width,
            height: thumb_height,
        },
    );
    window.paint_quad(fill(scrollbar_bounds, scrollbar_color));
}
