//! Background paint pass — terminal fill, bell flash, per-cell background
//! rects with edge extension, and pixel-perfect block quads.
//!
//! Pixel alignment is load-bearing: `.floor()` on x / `.ceil()` on width
//! prevent sub-pixel gaps in block-drawing and Powerline artwork. Do not
//! change those calls without a regression test.

use gpui::{Bounds, Pixels, Point, Window, fill, px};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

/// Paint the terminal background fill + optional bell-flash overlay.
pub fn paint_base_fill(
    layout: &LayoutState,
    bounds: Bounds<Pixels>,
    bell_flash_active: bool,
    window: &mut Window,
) {
    window.paint_quad(fill(bounds, layout.background_color));

    // Bell flash: semi-transparent white overlay
    if bell_flash_active {
        window.paint_quad(fill(bounds, gpui::hsla(0., 0., 1., 0.12)));
    }
}

/// Paint per-cell background rects with edge extension (Ghostty-style
/// EXTEND_LEFT/RIGHT/UP/DOWN for neverExtendBg).
pub fn paint_cell_backgrounds(
    layout: &LayoutState,
    geom: &CellGeometry,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    let widget_left = bounds.origin.x;
    let widget_right = bounds.origin.x + bounds.size.width;
    let widget_top = bounds.origin.y;
    let widget_bottom = bounds.origin.y + bounds.size.height;
    let last_row = layout.desired_rows.saturating_sub(1) as i32;

    for rect in &layout.rects {
        // Zed-parity positioning: floor(x), ceil(width), raw y, single line_height.
        let mut x = (origin.x + cell_width * rect.col as f32).floor();
        let mut y = origin.y + line_height * rect.line as f32;
        let w = (cell_width * rect.num_cols as f32).ceil();
        let mut right = x + w;
        let mut bottom = y + line_height * rect.num_lines as f32;
        let last_rect_line = rect.line + rect.num_lines as i32 - 1;

        // Look up per-line extension flags (neverExtendBg, US-003)
        // For vertically merged rects, use the first line's flags for edges.
        let (extend_left, extend_right) =
            if rect.line >= 0 && (rect.line as usize) < layout.extend_line_flags.len() {
                layout.extend_line_flags[rect.line as usize]
            } else {
                (true, true)
            };

        // Extend left edge into gutter for column-0 rects
        if rect.col == 0 && extend_left {
            x = widget_left;
        }
        // Extend right edge to widget boundary for last-column rects
        if rect.col + rect.num_cols >= layout.desired_cols && extend_right {
            right = widget_right;
        }
        // Vertical extension is unconditional — Powerline glyphs only
        // create horizontal edge artifacts, not vertical ones.
        if rect.line == 0 {
            y = widget_top;
        }
        if last_rect_line == last_row {
            bottom = widget_bottom;
        }

        let rect_bounds = Bounds::new(
            Point { x, y },
            gpui::Size {
                width: (right - x).max(px(0.0)),
                height: (bottom - y).max(px(0.0)),
            },
        );
        window.paint_quad(fill(rect_bounds, rect.color));
    }
}

/// Paint block-element quads (half-blocks, 1/8-blocks, etc.) as filled rects
/// to avoid font-glyph sub-pixel gaps.
pub fn paint_block_quads(layout: &LayoutState, geom: &CellGeometry, window: &mut Window) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    for bq in &layout.block_quads {
        let cx_start = origin.x + cell_width * bq.col as f32;
        let cy_start = origin.y + line_height * bq.line as f32;
        let cw = cell_width * bq.num_cols as f32;
        let ch = line_height;
        let (fx, fy, fw, fh) = bq.coverage;
        let qx = (cx_start + cw * fx).floor();
        let qy = cy_start + ch * fy;
        let qw = (cw * fw).ceil();
        let qh = ch * fh;
        window.paint_quad(fill(
            Bounds::new(
                Point { x: qx, y: qy },
                gpui::Size {
                    width: qw,
                    height: qh,
                },
            ),
            bq.color,
        ));
    }
}
