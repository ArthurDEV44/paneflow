//! Cursor paint pass - primary text cursor
//! (Vintage/Block/Beam/Underline/DoubleUnderline/HollowBlock) plus the
//! copy-mode selection anchor cursor.

use gpui::{
    App, BorderStyle, Bounds, Font, FontStyle, FontWeight, Pixels, Point, SharedString, TextAlign,
    TextRun, Window, fill, outline, px,
};

use super::super::geometry::CellGeometry;
use super::super::{CursorInfo, LayoutState};
use crate::terminal::types::CursorShape;

fn cursor_text_color(layout: &LayoutState) -> gpui::Hsla {
    if layout.background_color.a > 0.01 {
        layout.background_color
    } else {
        gpui::hsla(0.0, 0.0, 0.08, 1.0)
    }
}

fn paint_cursor_info(
    cursor: &CursorInfo,
    layout: &LayoutState,
    geom: &CellGeometry,
    base_font: &Font,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    let cx_ = origin.x + cell_width * cursor.col as f32;
    let cy = origin.y + line_height * cursor.line as f32;
    let mut cw = if cursor.wide {
        cell_width * 2.0
    } else {
        cell_width
    };
    let ch = line_height;
    let color = cursor.color;

    match cursor.shape {
        CursorShape::Vintage => {
            let vintage_height = (ch * 0.28).max(px(3.0));
            let cursor_bounds = Bounds::new(
                Point {
                    x: cx_,
                    y: cy + ch - vintage_height,
                },
                gpui::Size {
                    width: cw,
                    height: vintage_height,
                },
            );
            window.paint_quad(fill(cursor_bounds, color));
        }
        CursorShape::Block => {
            // Shape the cursor character first so we can size the
            // cursor quad to fit wide/emoji glyphs.
            let shaped = cursor.text.map(|ch| {
                let mut cursor_font = base_font.clone();
                if cursor.bold {
                    cursor_font.weight = FontWeight::BOLD;
                }
                if cursor.italic {
                    cursor_font.style = FontStyle::Italic;
                }
                let text = ch.to_string();
                let len = text.len();
                window.text_system().shape_line(
                    SharedString::from(text),
                    font_size,
                    &[TextRun {
                        len,
                        font: cursor_font,
                        color: cursor_text_color(layout),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                )
            });

            // Widen cursor to fit glyphs that exceed cell_width * 2
            if cursor.wide
                && let Some(ref shaped) = shaped
            {
                cw = cw.max(shaped.width());
            }

            let cursor_bounds = Bounds::new(
                Point { x: cx_, y: cy },
                gpui::Size {
                    width: cw,
                    height: ch,
                },
            );
            window.paint_quad(fill(cursor_bounds, color));

            // Paint the character on top of the cursor quad
            if let Some(shaped) = shaped {
                let _ = shaped.paint(
                    Point { x: cx_, y: cy },
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
        CursorShape::Beam => {
            let beam_width = px(2.0);
            let cursor_bounds = Bounds::new(
                Point { x: cx_, y: cy },
                gpui::Size {
                    width: beam_width,
                    height: ch,
                },
            );
            window.paint_quad(fill(cursor_bounds, color));
        }
        CursorShape::Underline => {
            let underline_height = px(2.0);
            let cursor_bounds = Bounds::new(
                Point {
                    x: cx_,
                    y: cy + ch - underline_height,
                },
                gpui::Size {
                    width: cw,
                    height: underline_height,
                },
            );
            window.paint_quad(fill(cursor_bounds, color));
        }
        CursorShape::DoubleUnderline => {
            let underline_height = px(2.0);
            let gap = px(2.0);
            let lower_y = cy + ch - underline_height;
            let upper_y = (lower_y - underline_height - gap).max(cy);
            for y in [upper_y, lower_y] {
                let cursor_bounds = Bounds::new(
                    Point { x: cx_, y },
                    gpui::Size {
                        width: cw,
                        height: underline_height,
                    },
                );
                window.paint_quad(fill(cursor_bounds, color));
            }
        }
        CursorShape::HollowBlock => {
            let cursor_bounds = Bounds::new(
                Point { x: cx_, y: cy },
                gpui::Size {
                    width: cw,
                    height: ch,
                },
            );
            window.paint_quad(
                outline(cursor_bounds, color, BorderStyle::Solid)
                    .border_widths(1.5)
                    .corner_radii(px(2.0)),
            );
        }
        CursorShape::Hidden => {} // Already filtered in build_layout
    }
}

/// Paint the primary cursor at its grid position using the shape dictated by
/// the terminal mode + config. For Block shapes, shapes the underlying
/// character on top in the terminal's background color.
pub fn paint_cursor(
    layout: &LayoutState,
    geom: &CellGeometry,
    base_font: &Font,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(cursor) = &layout.cursor else {
        return;
    };

    paint_cursor_info(cursor, layout, geom, base_font, font_size, window, cx);
}

/// Paint the secondary selection marker using the same glyph-aware cursor pass
/// as the primary cursor.
pub fn paint_anchor_cursor(
    layout: &LayoutState,
    geom: &CellGeometry,
    base_font: &Font,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(anchor) = &layout.anchor_cursor else {
        return;
    };

    paint_cursor_info(anchor, layout, geom, base_font, font_size, window, cx);
}
