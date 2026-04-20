//! Batched-text paint pass — one `shape_line` per `BatchedTextRun`.

use gpui::{App, Pixels, Point, SharedString, TextAlign, TextRun, Window};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

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
        let x = origin.x + cell_width * run.col_start as f32;
        let y = origin.y + line_height * run.line as f32;
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
