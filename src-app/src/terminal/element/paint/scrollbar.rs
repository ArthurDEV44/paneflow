//! Scrollbar thumb paint pass.

use gpui::{Bounds, Hsla, Pixels, Point, Window, fill, px};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

/// Amber tick color for OSC 133 prompt-start marks in the scrollbar gutter.
/// Chosen for contrast against typical dark themes' scrollbar_thumb without
/// pulling from the theme palette — the mark is a navigation affordance, not
/// part of the terminal color model, so it stays visually constant when the
/// user swaps themes (less surprising when scrolling through history).
const PROMPT_MARK_COLOR: Hsla = Hsla {
    h: 0.12,
    s: 0.95,
    l: 0.6,
    a: 0.9,
};

/// Minimum vertical pixel size of a single prompt-mark tick. Below this the
/// mark would be invisible at terminal heights of a few hundred pixels.
const MIN_MARK_HEIGHT: Pixels = px(2.0);

/// Paint the 4px right-edge scrollbar thumb plus one tick per OSC 133
/// PromptStart marker. The thumb only shows while scrolled away from the
/// bottom; marks are always painted whenever there is scrollback history
/// (so the user sees them in the gutter even before scrolling, making
/// "jump to previous prompt" predictable).
pub fn paint_scrollbar(
    layout: &LayoutState,
    geom: &CellGeometry,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    let CellGeometry {
        origin,
        line_height,
        ..
    } = *geom;

    let scrollbar_width = px(4.0);
    let visible_rows = (bounds.size.height / line_height).floor().max(1.0) as usize;
    let total_lines = layout.history_size + visible_rows;
    let scrollbar_x = origin.x + bounds.size.width - scrollbar_width;

    // Prompt-mark ticks. Each `PromptStart` line is in grid coords (negative
    // = scrollback). Project onto the scrollbar by mapping to an absolute row
    // 0..total_lines, then to vertical pixels. Drawn first so the thumb sits
    // on top of any marks that fall under it (the thumb is the user's
    // current focus, marks are reference markers).
    if total_lines > 0 && !layout.prompt_mark_lines.is_empty() {
        for &line in &layout.prompt_mark_lines {
            // Grid line `line` lives at absolute row `line + history_size`
            // (top of scrollback = row 0). Clamp to [0, total_lines] in case
            // a stale mark survives a `clear_scroll_history` or a partial
            // resize during a fast scroll.
            let abs_row = (line + layout.history_size as i32).clamp(0, total_lines as i32) as usize;
            let mark_y = (abs_row as f32 / total_lines as f32) * bounds.size.height.as_f32();
            let mark_bounds = Bounds::new(
                Point {
                    x: scrollbar_x,
                    y: origin.y + Pixels::from(mark_y),
                },
                gpui::Size {
                    width: scrollbar_width,
                    height: MIN_MARK_HEIGHT,
                },
            );
            window.paint_quad(fill(mark_bounds, PROMPT_MARK_COLOR));
        }
    }

    // Thumb — short-circuit when scrolled to the bottom and there is no
    // history (legacy behaviour preserved; the thumb is purely a position
    // indicator).
    if layout.display_offset == 0 || layout.history_size == 0 {
        return;
    }
    let visible_ratio = visible_rows as f32 / total_lines as f32;
    let thumb_height = (bounds.size.height * visible_ratio).max(px(16.0));
    let scroll_ratio = layout.display_offset as f32 / layout.history_size as f32;
    // display_offset=max → scrolled to top → thumb at top
    let thumb_y =
        bounds.size.height - thumb_height - (bounds.size.height - thumb_height) * scroll_ratio;
    let scrollbar_color = layout.scrollbar_thumb;
    let scrollbar_bounds = Bounds::new(
        Point {
            x: scrollbar_x,
            y: origin.y + thumb_y,
        },
        gpui::Size {
            width: scrollbar_width,
            height: thumb_height,
        },
    );
    window.paint_quad(fill(scrollbar_bounds, scrollbar_color));
}
