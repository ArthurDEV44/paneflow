//! Cursor paint pass — primary text cursor (Block/Beam/Underline/HollowBlock)
//! plus the copy-mode selection anchor cursor.

use alacritty_terminal::vte::ansi::CursorShape;
use gpui::{
    App, BorderStyle, Bounds, Font, FontStyle, FontWeight, Pixels, Point, SharedString, TextAlign,
    TextRun, Window, fill, outline, px,
};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

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
                        color: layout.background_color,
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

/// Paint the copy-mode selection anchor as a tmux-style distinct hollow block.
pub fn paint_anchor_cursor(layout: &LayoutState, geom: &CellGeometry, window: &mut Window) {
    let Some(anchor) = &layout.anchor_cursor else {
        return;
    };

    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    let ax = origin.x + cell_width * anchor.col as f32;
    let ay = origin.y + line_height * anchor.line as f32;
    let anchor_bounds = Bounds::new(
        Point { x: ax, y: ay },
        gpui::Size {
            width: cell_width,
            height: line_height,
        },
    );
    window.paint_quad(outline(anchor_bounds, anchor.color, BorderStyle::Solid).border_widths(1.5));
}
