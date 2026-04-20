//! Selection highlight paint pass.

use gpui::{Bounds, Point, Window, fill};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

/// Paint the selection highlight as pixel-aligned square-corner rects.
pub fn paint_selection(layout: &LayoutState, geom: &CellGeometry, window: &mut Window) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    for rect in &layout.selection_rects {
        let x = (origin.x + cell_width * rect.col as f32).floor();
        let y = origin.y + line_height * rect.line as f32;
        let w = (cell_width * rect.num_cols as f32).ceil();
        let rect_bounds = Bounds::new(
            Point { x, y },
            gpui::Size {
                width: w,
                height: line_height,
            },
        );
        window.paint_quad(fill(rect_bounds, rect.color));
    }
}
