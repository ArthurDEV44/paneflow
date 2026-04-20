//! Overlay paint passes — search highlights, hyperlink underline + tooltip,
//! IME preedit, process-exit banner, and the debug latency probe bookends.
//!
//! These layers draw on top of text and cursor; they're grouped here because
//! each one is conditional (rendered only when the relevant state is active)
//! and they all compose over the primary cell grid rather than participating
//! in cell-level layout.

use gpui::{App, Bounds, Font, Pixels, Point, SharedString, TextAlign, TextRun, Window, fill, px};

use super::super::LayoutState;
use super::super::TerminalElement;
use super::super::geometry::CellGeometry;

/// Search match highlight rects (`.floor()` / `.ceil()` matches background).
pub fn paint_search_highlights(layout: &LayoutState, geom: &CellGeometry, window: &mut Window) {
    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    for rect in &layout.search_rects {
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

/// Paint the Ctrl+hover hyperlink underline and, if a URI is hovered,
/// a tooltip near the link (auto-flipped above the link when the link
/// is near the bottom of the terminal).
pub fn paint_hyperlink_tooltip(
    element: &TerminalElement,
    layout: &LayoutState,
    geom: &CellGeometry,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((link_line, col_start, col_end)) = element.hovered_link_range else {
        return;
    };

    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    let display_offset = layout.display_offset as i32;
    let screen_line = link_line.0 + display_offset;
    if screen_line < 0 || (screen_line as usize) >= layout.desired_rows {
        return;
    }

    let x_start = origin.x + cell_width * col_start as f32;
    let x_end = origin.x + cell_width * (col_end + 1) as f32;
    let y = origin.y + line_height * (screen_line + 1) as f32 - gpui::px(1.0);
    let underline_bounds = Bounds::new(
        Point { x: x_start, y },
        gpui::Size {
            width: x_end - x_start,
            height: gpui::px(1.0),
        },
    );
    window.paint_quad(fill(underline_bounds, layout.link_text_color));

    // Paint URL tooltip near the underline
    let Some(ref uri) = element.hovered_link_uri else {
        return;
    };
    let tooltip_font_size = gpui::px(11.0);
    let tooltip_padding = gpui::px(4.0);
    // Char-safe truncation to avoid panics on multibyte URIs
    let display_uri: String = if uri.chars().count() > 80 {
        let mut s: String = uri.chars().take(77).collect();
        s.push_str("...");
        s
    } else {
        uri.clone()
    };
    let display_len = display_uri.len(); // UTF-8 byte count for TextRun
    let shaped = window.text_system().shape_line(
        SharedString::from(display_uri),
        tooltip_font_size,
        &[gpui::TextRun {
            len: display_len,
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
    let text_width = shaped.width;
    let tooltip_height = tooltip_font_size + tooltip_padding * 2.0;
    let tooltip_x = x_start;
    // Flip tooltip above the link when near the bottom of the terminal
    let tooltip_y = {
        let below = y + gpui::px(3.0);
        let bottom_edge = origin.y + line_height * layout.desired_rows as f32;
        if below + tooltip_height > bottom_edge {
            // Place above the link line
            origin.y + line_height * screen_line as f32 - tooltip_height - gpui::px(2.0)
        } else {
            below
        }
    };
    let bg_bounds = Bounds::new(
        Point {
            x: tooltip_x - tooltip_padding,
            y: tooltip_y,
        },
        gpui::Size {
            width: text_width + tooltip_padding * 2.0,
            height: tooltip_height,
        },
    );
    // Semi-transparent overlay background for visibility
    let mut tooltip_bg = layout.background_color;
    tooltip_bg.a = 0.92;
    window.paint_quad(fill(bg_bounds, tooltip_bg));
    let _ = shaped.paint(
        Point {
            x: tooltip_x,
            y: tooltip_y + tooltip_padding,
        },
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// Register the IME `InputHandler` for this element and paint the preedit
/// composition overlay (when focused and a composition is in progress).
///
/// `make_handler` is a closure that constructs the concrete input handler —
/// keeping the `TerminalInputHandler` type private to `mod.rs`.
#[allow(clippy::too_many_arguments)]
pub fn paint_ime_preedit<H, F>(
    element: &TerminalElement,
    layout: &LayoutState,
    geom: &CellGeometry,
    font_size: Pixels,
    base_font: &Font,
    window: &mut Window,
    cx: &mut App,
    make_handler: F,
) where
    H: gpui::InputHandler,
    F: FnOnce(Option<Bounds<Pixels>>) -> H,
{
    if !element.focused {
        return;
    }

    let CellGeometry {
        origin,
        cell_width,
        line_height,
    } = *geom;

    let cursor_bounds = layout.ime_cursor_bounds.map(|b| {
        Bounds::new(
            Point {
                x: b.origin.x + origin.x,
                y: b.origin.y + origin.y,
            },
            b.size,
        )
    });
    let handler = make_handler(cursor_bounds);
    window.handle_input(&element.focus_handle, handler, cx);

    // Paint preedit overlay
    if !element.ime_marked_text.is_empty()
        && let Some(cb) = cursor_bounds
    {
        let ime_run = TextRun {
            len: element.ime_marked_text.len(),
            font: base_font.clone(),
            color: layout.background_color,
            background_color: None,
            underline: Some(gpui::UnderlineStyle {
                color: None,
                thickness: px(1.0),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(element.ime_marked_text.clone()),
            font_size,
            &[ime_run],
            Some(cell_width),
        );
        // Background erase behind preedit
        let preedit_width = shaped.width();
        let preedit_bg = Bounds::new(
            cb.origin,
            gpui::Size {
                width: preedit_width,
                height: line_height,
            },
        );
        window.paint_quad(fill(preedit_bg, layout.background_color));
        // Paint preedit text
        let _ = shaped.paint(cb.origin, line_height, TextAlign::Left, None, window, cx);
    }
}

/// Paint the centered "[Process exited with code N]" message when the shell
/// child has exited. `exit_fg` is the Catppuccin Overlay6 grey passed in so
/// the overlay module stays free of color-helper imports.
#[allow(clippy::too_many_arguments)]
pub fn paint_exit_overlay(
    layout: &LayoutState,
    geom: &CellGeometry,
    bounds: Bounds<Pixels>,
    font_size: Pixels,
    base_font: &Font,
    exit_fg: gpui::Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(code) = layout.exited else {
        return;
    };

    let CellGeometry {
        origin,
        line_height,
        ..
    } = *geom;

    let msg = format!("[Process exited with code {code}]");
    let run = TextRun {
        len: msg.len(),
        font: base_font.clone(),
        color: exit_fg,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window
        .text_system()
        .shape_line(SharedString::from(msg), font_size, &[run], None);
    // Center the message in the terminal bounds
    let text_width = shaped.width();
    let x = origin.x + (bounds.size.width - text_width) * 0.5;
    let y = origin.y + (bounds.size.height - line_height) * 0.5;
    let _ = shaped.paint(
        Point { x, y },
        line_height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}
