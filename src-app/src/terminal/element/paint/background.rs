//! Background paint pass — terminal fill, bell flash, per-cell background
//! rects with edge extension, and pixel-perfect block quads.
//!
//! ## Pixel alignment (US-004)
//!
//! Cell rects and block quads share a single pair of per-frame integer
//! pixel boundary arrays — `cell_x_boundaries` and `cell_y_boundaries`.
//! Looking up edges through those arrays makes adjacency exact by
//! construction: `next_rect.x == prev_rect.x + prev_rect.width` for any
//! two horizontally-adjacent rects, regardless of whether the underlying
//! `cell_width` is fractional or which way per-rect rounding would lean.
//! This replaces the previous per-rect `floor(x) + ceil(width)` pattern,
//! which could leave a 1-px gap or overlap at the seam when `cell_width`
//! was fractional. Block quads use the same arrays so a full-block (`█`,
//! `▀`, `▄`) coverage shares its outer edges with the cell background
//! underneath — the canonical anti-gap fix from `debug_block_char_rendering.md`.

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

/// Compute the integer pixel X boundaries for `col_count` cells, indexed
/// `0..=col_count`. Each entry is `floor(origin_x + cell_width * c)`. Two
/// adjacent runs of cells share the boundary at their join, so the right
/// edge of run `[a, b)` (i.e. `boundaries[b]`) equals the left edge of
/// run `[b, c)` exactly. See module doc for the full rationale.
pub(super) fn cell_x_boundaries(
    origin_x: Pixels,
    cell_width: Pixels,
    col_count: usize,
) -> Vec<Pixels> {
    (0..=col_count)
        .map(|c| (origin_x + cell_width * c as f32).floor())
        .collect()
}

/// Y-axis counterpart to [`cell_x_boundaries`]. Indexed `0..=row_count`.
pub(super) fn cell_y_boundaries(
    origin_y: Pixels,
    line_height: Pixels,
    row_count: usize,
) -> Vec<Pixels> {
    (0..=row_count)
        .map(|r| (origin_y + line_height * r as f32).floor())
        .collect()
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

    let widget_top = bounds.origin.y;
    let widget_bottom = bounds.origin.y + bounds.size.height;

    let col_count = layout.desired_cols;
    let row_count = layout.desired_rows;

    // Empty viewport (window minimised, mid-resize) — nothing to paint.
    if col_count == 0 || row_count == 0 {
        return;
    }

    let last_row = row_count.saturating_sub(1) as i32;
    let x_boundaries = cell_x_boundaries(origin.x, cell_width, col_count);
    let y_boundaries = cell_y_boundaries(origin.y, line_height, row_count);

    for rect in &layout.rects {
        let col_end = rect.col + rect.num_cols;
        let line_end_signed = rect.line + rect.num_lines as i32;

        // Defensive bounds check. `build_layout` should never emit a rect
        // outside the viewport or with zero extent — silent skip beats
        // indexing past the boundary arrays or queueing a zero-area quad
        // for the GPU. If this trips in practice, the layout pass has a
        // bug worth surfacing via the probe.
        if rect.num_cols == 0
            || rect.num_lines == 0
            || col_end > col_count
            || rect.line < 0
            || line_end_signed < 0
            || (line_end_signed as usize) > row_count
        {
            continue;
        }

        let line_start = rect.line as usize;
        let line_end = line_end_signed as usize;

        let x = x_boundaries[rect.col];
        let right = x_boundaries[col_end];
        let mut y = y_boundaries[line_start];
        let mut bottom = y_boundaries[line_end];
        let last_rect_line = rect.line + rect.num_lines as i32 - 1;

        // Horizontal extension into the gutter is intentionally NOT applied:
        // matching Zed (`crates/terminal_view/src/terminal_element.rs`
        // BackgroundRect::paint), bg rects stay strictly inside the cell
        // grid. Extending col-0 / last-col rects to the widget edges
        // caused the OpenAI Codex input-bar tint to leak into the gutter,
        // producing an unbounded gray bar instead of the inset Zed-style
        // band. The widget-wide `paint_base_fill` covers the gutter with
        // the theme background; cell rects sit on top of it inside the grid.
        //
        // Vertical extension remains unconditional — half-pixel residue at
        // the top and bottom of the grid would otherwise show a thin band
        // of widget-bg between the first/last cell row and the widget edge.
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

        // PANEFLOW_PIXEL_PROBE: capture the post-extension rect coordinates
        // so a future investigation can verify shared-boundary adjacency
        // (`prev.x + prev.width == next.x`) directly from the log.
        #[cfg(debug_assertions)]
        super::super::pixel_probe::record_background(
            rect.col,
            rect.line,
            rect_bounds.origin.x,
            rect_bounds.origin.y,
            rect_bounds.size.width,
            rect_bounds.size.height,
        );

        window.paint_quad(fill(rect_bounds, rect.color));
    }
}

/// Paint block-element quads (half-blocks, 1/8-blocks, etc.) as filled
/// rects to avoid font-glyph sub-pixel gaps.
///
/// Uses the same shared-boundary arrays as [`paint_cell_backgrounds`] so
/// a full-block coverage (`█`, `▀`, `▄` with `fx=fy=0, fw=fh=1`) lines
/// up exactly with the cell background underneath — no sub-pixel seam.
/// Partial-block coverage applies floor on both inner edges to preserve
/// the same shared-boundary property between adjacent block cells.
pub fn paint_block_quads(layout: &LayoutState, geom: &CellGeometry, window: &mut Window) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    let col_count = layout.desired_cols;
    let row_count = layout.desired_rows;
    if col_count == 0 || row_count == 0 {
        return;
    }
    let x_boundaries = cell_x_boundaries(origin.x, cell_width, col_count);
    let y_boundaries = cell_y_boundaries(origin.y, line_height, row_count);

    for bq in &layout.block_quads {
        let col_end = bq.col + bq.num_cols;
        // Defensive: skip out-of-range or zero-extent quads. Symmetric
        // with the guard in `paint_cell_backgrounds` above.
        if bq.num_cols == 0 || col_end > col_count || bq.line < 0 || (bq.line as usize) >= row_count
        {
            continue;
        }
        let line = bq.line as usize;

        // Outer cell extents from the shared-boundary arrays — these are
        // identical to the cell background's edges, by construction.
        let cell_x_left = x_boundaries[bq.col];
        let cell_x_right = x_boundaries[col_end];
        let cell_y_top = y_boundaries[line];
        let cell_y_bottom = y_boundaries[line + 1];
        let cell_w = cell_x_right - cell_x_left;
        let cell_h = cell_y_bottom - cell_y_top;

        // Apply fractional coverage WITHIN the cell extents. Floor on
        // both inner edges (rather than `floor(start) + ceil(width)`) so
        // adjacent partial-block cells share their inner boundary the
        // same way full cells share their outer boundary. For full-cell
        // coverage (fx=fy=0, fw=fh=1) this collapses to the cell extents.
        let (fx, fy, fw, fh) = bq.coverage;
        let qx = (cell_x_left + cell_w * fx).floor();
        let qy = (cell_y_top + cell_h * fy).floor();
        let q_right = (cell_x_left + cell_w * (fx + fw)).floor();
        let q_bottom = (cell_y_top + cell_h * (fy + fh)).floor();
        let qw = (q_right - qx).max(px(0.0));
        let qh = (q_bottom - qy).max(px(0.0));

        // PANEFLOW_PIXEL_PROBE: block quads are the canonical "fix this
        // gap" surface from `debug_block_char_rendering.md` — log the
        // exact submitted geometry so a future investigation can compare
        // against the corresponding cell background and glyph X.
        #[cfg(debug_assertions)]
        super::super::pixel_probe::record_block_quad(bq.col, bq.line, qx, qy, qw, qh);

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

// Gated on `debug_assertions` because the test imports `pixel_probe`,
// which is itself debug-only. `cargo test --release` would otherwise fail
// to resolve the import (see the same pattern in `paint/text.rs`).
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use crate::terminal::element::pixel_probe::assert_pixel_aligned;

    #[test]
    fn cell_x_boundaries_count_matches_col_count_plus_one() {
        let b = cell_x_boundaries(px(0.0), px(9.0), 5);
        assert_eq!(b.len(), 6); // 0..=5
    }

    #[test]
    fn ten_cell_run_at_8_4_yields_84_px_total() {
        // PRD canonical scenario. A 10-cell same-color run at fractional
        // cell_width must produce a single quad whose width matches the
        // integer total `floor(10 * 8.4) = 84`, NOT `10 * floor(8.4) = 80`
        // (which would leave 4 px of internal gap).
        let cell_width = px(8.4);
        let b = cell_x_boundaries(px(0.0), cell_width, 10);
        let total_width = b[10] - b[0];
        assert_eq!(total_width, px(84.0));
        for boundary in &b {
            assert_pixel_aligned(boundary.as_f32(), "x boundary");
        }
    }

    #[test]
    fn adjacent_cells_have_non_negative_width() {
        // The shared-boundary invariant follows from the boundaries being
        // monotonically non-decreasing — pairwise differences are cell
        // widths, and a non-negative cell width per pair means no overlap
        // and (since both edges come from the same array) no gap either.
        // The previous-rect's right edge IS the next-rect's left edge by
        // construction; this test makes the monotonicity load-bearing.
        let b = cell_x_boundaries(px(0.0), px(8.4), 7);
        for window in b.windows(2) {
            let cell_width = window[1] - window[0];
            assert!(
                cell_width >= px(0.0),
                "boundaries must be monotonic; got cell width {cell_width:?}"
            );
        }
    }

    #[test]
    fn boundaries_are_integer_with_fractional_origin() {
        // Origin can be fractional in production (gutter offset adds the
        // integer `cell_width` to a possibly-fractional `bounds.origin.x`
        // from GPUI). Boundaries must still be integer-aligned because
        // `floor()` is applied per index.
        let b = cell_x_boundaries(px(0.4), px(8.4), 10);
        for boundary in &b {
            assert_pixel_aligned(boundary.as_f32(), "x boundary with fractional origin");
        }
    }

    #[test]
    fn cell_y_boundaries_18_2_yields_expected_values() {
        // 14 pt × 1.3 multiplier ≈ 18.2 px (default config). After
        // US-002 line_height is integer-snapped, but the helper itself
        // must handle fractional inputs correctly as a defensive measure.
        let b = cell_y_boundaries(px(0.0), px(18.2), 5);
        assert_eq!(b[0], px(0.0)); // floor(0)
        assert_eq!(b[1], px(18.0)); // floor(18.2)
        assert_eq!(b[2], px(36.0)); // floor(36.4)
        assert_eq!(b[3], px(54.0)); // floor(54.6)
        assert_eq!(b[4], px(72.0)); // floor(72.8)
        assert_eq!(b[5], px(91.0)); // floor(91.0)
    }

    #[test]
    fn single_cell_yields_two_element_array() {
        // PRD unhappy path: a single-cell row produces a 2-element array
        // (left edge + right edge), which is the minimum needed to emit
        // one quad. Behaves identically to the pre-US-004 code on this
        // input.
        let b = cell_x_boundaries(px(0.0), px(8.4), 1);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], px(0.0));
        assert_eq!(b[1], px(8.0));
    }

    #[test]
    fn integer_cell_width_is_no_op_post_us_002() {
        // Post-US-002 the typical case: cell_width is integer. Boundaries
        // collapse to exact arithmetic with no floor truncation.
        let b = cell_x_boundaries(px(0.0), px(9.0), 5);
        assert_eq!(
            b,
            vec![px(0.0), px(9.0), px(18.0), px(27.0), px(36.0), px(45.0)]
        );
    }
}
