//! Batched-text paint pass — one `shape_line` per `BatchedTextRun`.

use gpui::{App, Pixels, Point, SharedString, TextAlign, TextRun, Window};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

/// Compute the integer-pixel origin of a glyph run.
///
/// US-003: GPUI's `ShapedLine::paint(origin)` performs no internal pixel
/// snapping (verified against `crates/gpui/src/text_system/line.rs`), so
/// the caller must hand it a `Point<Pixels>` that is already on a pixel
/// boundary if pixel-perfect rendering is desired. After US-002 made
/// `cell_width` and `line_height` integer at measure time, `.round()`
/// here is normally a no-op — but it is a deliberate guardrail. If a
/// future change re-introduces a fractional residual (origin drift,
/// non-integer cell stride from a refactor, etc.), this snap keeps
/// glyphs aligned with their cell backgrounds without further fix.
pub(super) fn glyph_origin(
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    line: i32,
    col_start: usize,
) -> Point<Pixels> {
    let x = (origin.x + cell_width * col_start as f32).round();
    let y = (origin.y + line_height * line as f32).round();
    Point { x, y }
}

/// Paint all batched text runs produced during `build_layout`.
pub fn paint_text_runs(
    layout: &LayoutState,
    geom: &CellGeometry,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    for run in &layout.batched_runs {
        let Point { x, y } = glyph_origin(origin, cell_width, line_height, run.line, run.col_start);

        // PANEFLOW_PIXEL_PROBE: log glyph X/Y per run (sampled to first 16
        // columns of each row inside the probe). Post-US-003 these are
        // always integer; a fractional value here would mean the snap was
        // bypassed and the renderer is back to sub-pixel offsets.
        #[cfg(debug_assertions)]
        super::super::pixel_probe::record_glyph(run.line, run.col_start, x, y);

        let text_run = TextRun {
            len: run.text.len(),
            font: run.font.clone(),
            color: run.color,
            background_color: None,
            underline: run.underline,
            strikethrough: run.strikethrough,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(run.text.clone()),
            font_size,
            &[text_run],
            Some(cell_width),
        );
        let _ = shaped.paint(
            Point { x, y },
            line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }
}

// Gated on `debug_assertions` because the `pixel_probe` module — and its
// `assert_pixel_aligned` helper — are themselves debug-only. Without the
// extra cfg, `cargo test --release` (which turns `debug_assertions` off)
// would fail to resolve the import.
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    use crate::terminal::element::pixel_probe::assert_pixel_aligned;
    use gpui::px;

    #[test]
    fn glyph_origin_snaps_fractional_cell_width() {
        // 8.4 px is the canonical fractional cell_width from the PRD —
        // (DejaVu Sans Mono at 14 pt @ 1.0 DPI on Linux).
        let origin = Point {
            x: px(0.0),
            y: px(0.0),
        };
        let cell_width = px(8.4);
        let line_height = px(18.2);

        for col in [0usize, 1, 5, 17, 100, 1000] {
            let p = glyph_origin(origin, cell_width, line_height, 0, col);
            assert_pixel_aligned(p.x.as_f32(), "glyph_x");
        }
        for line in [0i32, 1, 10, 100] {
            let p = glyph_origin(origin, cell_width, line_height, line, 0);
            assert_pixel_aligned(p.y.as_f32(), "glyph_y");
        }
    }

    #[test]
    fn glyph_origin_no_op_for_integer_cell_width() {
        // Post-US-002 the typical case: cell_width and line_height are
        // already integer. The snap must produce exact arithmetic equality
        // — verifies `.round()` did not introduce drift.
        let origin = Point {
            x: px(0.0),
            y: px(0.0),
        };
        let cell_width = px(9.0);
        let line_height = px(18.0);
        let p = glyph_origin(origin, cell_width, line_height, 5, 7);
        assert_eq!(p.x, px(63.0)); // 9.0 * 7
        assert_eq!(p.y, px(90.0)); // 18.0 * 5
    }

    #[test]
    fn glyph_origin_handles_fractional_origin() {
        // Backstop guarantee: even when the ORIGIN drifts off-pixel (a
        // future regression in `paint()` gutter math, say), the snap still
        // produces an integer glyph X/Y. This is the entire point of
        // US-003 being a defensive guardrail rather than a primary fix.
        let origin = Point {
            x: px(0.4),
            y: px(0.5),
        };
        let cell_width = px(9.0);
        let line_height = px(18.0);
        let p = glyph_origin(origin, cell_width, line_height, 3, 11);
        assert_pixel_aligned(p.x.as_f32(), "glyph_x with fractional origin");
        assert_pixel_aligned(p.y.as_f32(), "glyph_y with fractional origin");
    }
}
