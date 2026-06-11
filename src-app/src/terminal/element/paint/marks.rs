//! Exit-code dots + jump-to-prompt tooltip (EP-003 US-008,
//! prd-cli-cockpit-ergonomics-2026-Q3.md).
//!
//! `133;D;<code>` marks paint a small dot in the existing 1-cell left
//! gutter (the grid origin already reserves it — `geometry.rs`), on the
//! viewport row their absolute line currently maps to. Pure `paint_quad`
//! work: no reflow, bounded by the viewport (never by the mark count).
//! Colors are `UiColors` status slots resolved in `build_layout` (FR-08 —
//! no inline hex).

use gpui::{Bounds, Hsla, Pixels, Point, SharedString, Window, fill, px};

use super::super::{CellGeometry, LayoutState};

/// One dot ready to paint: viewport row + resolved status color.
pub(crate) struct MarkDot {
    pub row: usize,
    pub color: Hsla,
}

/// Tooltip payload for the hovered dot (computed by the view's mouse-move
/// hit-test against the gutter).
#[derive(Clone, Copy, PartialEq)]
pub struct HoveredMark {
    /// Viewport row the hovered dot was on at hit-test time.
    pub row: i32,
    pub exit_code: i32,
    pub at: std::time::Instant,
}

/// Project absolute-line `133;D` marks onto viewport rows. Pure — the
/// inputs mirror alacritty's coordinate model (`viewport_row = abs_line -
/// history_size + display_offset`). Off-viewport marks are culled, so the
/// paint cost is bounded by `desired_rows` regardless of ring size.
pub(crate) fn compute_mark_dots(
    marks: &[(i64, i32)],
    history_size: usize,
    display_offset: usize,
    desired_rows: usize,
    success_color: Hsla,
    error_color: Hsla,
) -> Vec<MarkDot> {
    let mut dots = Vec::new();
    for &(abs_line, exit_code) in marks {
        let row = abs_line - history_size as i64 + display_offset as i64;
        if row >= 0 && (row as usize) < desired_rows {
            dots.push(MarkDot {
                row: row as usize,
                color: if exit_code == 0 {
                    success_color
                } else {
                    error_color
                },
            });
        }
    }
    dots
}

const DOT_SIZE: f32 = 6.0;

/// Paint the dots into the left gutter. `bounds.origin` is the ELEMENT
/// origin (gutter left edge); `geom.origin` already includes the gutter.
pub(crate) fn paint_mark_dots(
    layout: &LayoutState,
    bounds: Bounds<Pixels>,
    geom: &CellGeometry,
    window: &mut Window,
) {
    if layout.mark_dots.is_empty() {
        return;
    }
    let dot = px(DOT_SIZE);
    let x = bounds.origin.x + (geom.cell_width - dot) / 2.0;
    for d in &layout.mark_dots {
        let y = geom.origin.y + geom.line_height * d.row as f32 + (geom.line_height - dot) / 2.0;
        let dot_bounds = Bounds::new(
            Point { x, y },
            gpui::Size {
                width: dot,
                height: dot,
            },
        );
        window.paint_quad(fill(dot_bounds, d.color).corner_radii(dot / 2.0));
    }
}

/// Paint the `exit <code> · <when>` tooltip next to the hovered dot.
/// Mirrors the hyperlink tooltip's shaping + bottom-flip mechanics.
pub(crate) fn paint_mark_tooltip(
    hovered: Option<&HoveredMark>,
    layout: &LayoutState,
    geom: &CellGeometry,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let Some(mark) = hovered else {
        return;
    };
    if mark.row < 0 || (mark.row as usize) >= layout.desired_rows {
        return;
    }
    let text = format!("exit {} · {}", mark.exit_code, ago_label(mark.at.elapsed()));

    let tooltip_font_size = px(11.0);
    let tooltip_padding = px(4.0);
    let len = text.len();
    let shaped = window.text_system().shape_line(
        SharedString::from(text),
        tooltip_font_size,
        &[gpui::TextRun {
            len,
            font: gpui::Font {
                family: "monospace".into(),
                ..Default::default()
            },
            color: layout.link_text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    );
    let tooltip_height = tooltip_font_size + tooltip_padding * 2.0;
    let x = geom.origin.x + px(2.0);
    let row_top = geom.origin.y + geom.line_height * mark.row as f32;
    let y = {
        let below = row_top + geom.line_height + px(2.0);
        let bottom_edge = geom.origin.y + geom.line_height * layout.desired_rows as f32;
        if below + tooltip_height > bottom_edge {
            row_top - tooltip_height - px(2.0)
        } else {
            below
        }
    };
    let mut bg = layout.background_color;
    bg.a = 0.92;
    window.paint_quad(fill(
        Bounds::new(
            Point {
                x: x - tooltip_padding,
                y,
            },
            gpui::Size {
                width: shaped.width + tooltip_padding * 2.0,
                height: tooltip_height,
            },
        ),
        bg,
    ));
    let _ = shaped.paint(
        Point {
            x,
            y: y + tooltip_padding,
        },
        geom.line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// Compact relative timestamp for the tooltip ("just now", "3m ago", …).
fn ago_label(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3_600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREEN: Hsla = gpui::green();
    const RED: Hsla = gpui::red();

    #[test]
    fn dots_project_to_viewport_rows_and_cull() {
        // hist=100, offset=0: viewport shows abs [100, 100+rows).
        let dots = compute_mark_dots(
            &[(99, 0), (100, 0), (105, 1), (124, 0), (125, 0)],
            100,
            0,
            25,
            GREEN,
            RED,
        );
        let rows: Vec<(usize, bool)> = dots.iter().map(|d| (d.row, d.color == RED)).collect();
        assert_eq!(rows, vec![(0, false), (5, true), (24, false)]);
        // Scrolled up by 10: the same abs lines shift down by 10.
        let dots = compute_mark_dots(&[(95, 1)], 100, 10, 25, GREEN, RED);
        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0].row, 5);
    }
}
