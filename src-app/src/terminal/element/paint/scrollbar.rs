//! Scrollbar thumb paint pass.

use gpui::{Bounds, Pixels, Point, Window, fill, px};

use super::super::LayoutState;
use super::super::geometry::CellGeometry;

/// US-015: resolved pixel geometry of the scrollbar strip + thumb for a single
/// frame. Computed in [`scrollbar_metrics`] (the single source of truth shared
/// with [`paint_scrollbar`]) and stashed by `TerminalElement::paint` into a
/// shared cell so the view's mouse handlers can hit-test clicks/drags against
/// the exact same geometry that was painted — no formula duplication.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollbarMetrics {
    /// Absolute window-space X of the strip's left edge.
    pub(crate) strip_left: Pixels,
    /// Visual strip width (the painted thumb width).
    pub(crate) strip_width: Pixels,
    /// Absolute window-space Y of the track top.
    pub(crate) track_top: Pixels,
    /// Pixel height of the full track.
    pub(crate) track_height: Pixels,
    /// Absolute window-space Y of the thumb top (valid even when the thumb is
    /// not painted because `display_offset == 0`).
    pub(crate) thumb_top: Pixels,
    /// Pixel height of the thumb.
    pub(crate) thumb_height: Pixels,
    /// Scrollback line count, for pixel ↔ `display_offset` mapping.
    pub(crate) history_size: usize,
}

impl ScrollbarMetrics {
    /// Map an absolute pixel Y on the track to a scrollback `display_offset`.
    /// Track top → fully scrolled back (`history_size`); track bottom → `0`.
    /// Inverse of the thumb-position formula in [`scrollbar_metrics`].
    pub(crate) fn offset_for_y(&self, abs_y: Pixels) -> usize {
        if self.track_height.as_f32() <= 0.0 || self.history_size == 0 {
            return 0;
        }
        let rel = ((abs_y - self.track_top) / self.track_height).clamp(0.0, 1.0);
        let ratio = 1.0 - rel;
        (ratio * self.history_size as f32).round() as usize
    }

    /// Whether an absolute pixel X falls on the clickable strip. The hit zone
    /// is widened by `slop` pixels each side of the 4px visual strip so the
    /// thin strip is easy to grab without a pixel-perfect click.
    pub(crate) fn strip_contains_x(&self, x: Pixels, slop: Pixels) -> bool {
        x >= self.strip_left - slop && x <= self.strip_left + self.strip_width + slop
    }

    /// Whether an absolute pixel Y is on the thumb (a drag-grab) rather than
    /// the bare track (a click-to-jump).
    pub(crate) fn y_on_thumb(&self, abs_y: Pixels) -> bool {
        abs_y >= self.thumb_top && abs_y <= self.thumb_top + self.thumb_height
    }
}

/// Compute the scrollbar geometry for the current frame, or `None` when there
/// is no scrollback to scroll (`history_size == 0`). Single source of truth for
/// both the painted thumb ([`paint_scrollbar`]) and the view's mouse hit-test.
pub(crate) fn scrollbar_metrics(
    history_size: usize,
    display_offset: usize,
    geom: &CellGeometry,
    bounds: Bounds<Pixels>,
) -> Option<ScrollbarMetrics> {
    let line_height = geom.line_height;
    let visible_rows = (bounds.size.height / line_height).floor().max(1.0) as usize;
    let total_lines = history_size + visible_rows;
    if history_size == 0 || total_lines == 0 {
        return None;
    }
    let strip_width = px(4.0);
    // Sit against the element's right edge. Use `bounds.origin.x` (NOT
    // `geom.origin.x`, which is shifted right by the 1-cell left gutter) so the
    // strip lands `strip_width` inside the right edge instead of `cell_width`
    // past it — past the edge it gets scissored away by the `bounds` content
    // mask, making both the painted thumb and the hit zone invisible.
    let strip_left = bounds.origin.x + bounds.size.width - strip_width;
    let track_height = bounds.size.height;
    let visible_ratio = visible_rows as f32 / total_lines as f32;
    let thumb_height = (track_height * visible_ratio).max(px(16.0));
    let scroll_ratio = display_offset as f32 / history_size as f32;
    // display_offset = max → scrolled to top → thumb at top.
    let thumb_y = track_height - thumb_height - (track_height - thumb_height) * scroll_ratio;
    Some(ScrollbarMetrics {
        strip_left,
        strip_width,
        track_top: geom.origin.y,
        track_height,
        thumb_top: geom.origin.y + thumb_y,
        thumb_height,
        history_size,
    })
}

/// Paint the 4px right-edge scrollbar thumb. The thumb only shows while
/// scrolled away from the bottom; it is purely a position indicator.
pub fn paint_scrollbar(
    layout: &LayoutState,
    geom: &CellGeometry,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    // Thumb — short-circuit when scrolled to the bottom and there is no
    // history (legacy behaviour preserved; the thumb is purely a position
    // indicator). Geometry comes from `scrollbar_metrics` so the painted
    // thumb and the view's interactive hit-test never diverge (US-015).
    if layout.display_offset == 0 || layout.history_size == 0 {
        return;
    }
    let Some(metrics) = scrollbar_metrics(layout.history_size, layout.display_offset, geom, bounds)
    else {
        return;
    };
    let scrollbar_bounds = Bounds::new(
        Point {
            x: metrics.strip_left,
            y: metrics.thumb_top,
        },
        gpui::Size {
            width: metrics.strip_width,
            height: metrics.thumb_height,
        },
    );
    window.paint_quad(fill(scrollbar_bounds, layout.scrollbar_thumb));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(track_top: f32, track_height: f32, history: usize) -> ScrollbarMetrics {
        ScrollbarMetrics {
            strip_left: px(796.0),
            strip_width: px(4.0),
            track_top: px(track_top),
            track_height: px(track_height),
            thumb_top: px(track_top),
            thumb_height: px(16.0),
            history_size: history,
        }
    }

    // US-015: top of track = fully scrolled back; bottom = at the live edge.
    #[test]
    fn offset_for_y_top_is_full_history() {
        let m = metrics(0.0, 400.0, 1000);
        assert_eq!(m.offset_for_y(px(0.0)), 1000);
    }

    #[test]
    fn offset_for_y_bottom_is_zero() {
        let m = metrics(0.0, 400.0, 1000);
        assert_eq!(m.offset_for_y(px(400.0)), 0);
    }

    #[test]
    fn offset_for_y_midpoint_is_half_history() {
        let m = metrics(0.0, 400.0, 1000);
        let offset = m.offset_for_y(px(200.0));
        assert!(
            (offset as i64 - 500).abs() <= 1,
            "got {offset}, expected ~500"
        );
    }

    // Origin offset (non-zero track_top) is honoured.
    #[test]
    fn offset_for_y_respects_track_top() {
        let m = metrics(100.0, 400.0, 1000);
        assert_eq!(m.offset_for_y(px(100.0)), 1000); // at track top
        assert_eq!(m.offset_for_y(px(500.0)), 0); // at track bottom
    }

    // Out-of-range Y is clamped, never panics, never exceeds history.
    #[test]
    fn offset_for_y_clamps_out_of_range() {
        let m = metrics(0.0, 400.0, 500);
        assert_eq!(m.offset_for_y(px(-50.0)), 500); // above top → clamped
        assert_eq!(m.offset_for_y(px(9999.0)), 0); // below bottom → clamped
    }

    #[test]
    fn offset_for_y_zero_history_is_zero() {
        let m = metrics(0.0, 400.0, 0);
        assert_eq!(m.offset_for_y(px(0.0)), 0);
    }

    // Widened hit zone straddles the 4px visual strip.
    #[test]
    fn strip_contains_x_widened_hit_zone() {
        let m = metrics(0.0, 400.0, 1000); // strip_left=796, width=4 → [796,800]
        assert!(m.strip_contains_x(px(798.0), px(6.0))); // on the strip
        assert!(m.strip_contains_x(px(791.0), px(6.0))); // within left slop
        assert!(!m.strip_contains_x(px(780.0), px(6.0))); // outside
    }

    #[test]
    fn y_on_thumb_discriminates_track_from_thumb() {
        let mut m = metrics(0.0, 400.0, 1000);
        m.thumb_top = px(100.0);
        m.thumb_height = px(40.0); // thumb spans [100,140]
        assert!(m.y_on_thumb(px(120.0))); // on thumb → drag-grab
        assert!(!m.y_on_thumb(px(50.0))); // above thumb → track jump
        assert!(!m.y_on_thumb(px(300.0))); // below thumb → track jump
    }
}
