//! Cross-column scroll sync + file-at-offset helpers for the Review view
//! (US-004 code-motion). See [`super`] for the `DiffView` definition.

use super::*;
use std::rc::Rc;

impl DiffView {
    /// Toggle cross-column scroll synchronization (toolbar control).
    pub(super) fn toggle_sync(&mut self, cx: &mut Context<Self>) {
        self.sync_scroll = !self.sync_scroll;
        cx.notify();
    }

    /// Scroll the selected column's body so `path`'s file header is at the top.
    /// With sync on, the per-render broadcast carries the other columns to the
    /// same row offset (file-aligned where the columns share that file).
    pub fn jump_to_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.effective_mode(window);
        let target = self
            .columns
            .get(self.selected_column)
            .filter(|c| c.visible)
            .and_then(|col| {
                // Index against the *displayed* anchors so the jump lands right
                // even when files above are collapsed.
                let anchors = match mode {
                    ViewMode::Unified => &col.disp_anchors_unified,
                    ViewMode::Split => &col.disp_anchors_split,
                };
                let idx = anchors.iter().find(|(p, _)| p == path).map(|(_, i)| *i)?;
                // US-050: O(1) prefix-sum lookup of the header row's top offset.
                let offsets = match mode {
                    ViewMode::Unified => &col.disp_unified_offsets,
                    ViewMode::Split => &col.disp_split_offsets,
                };
                let y = hit_test::row_top(offsets, idx);
                Some((col.el_scroll.clone(), y))
            });
        let Some((handle, y)) = target else {
            return;
        };
        let x = handle.offset().x;
        handle.set_offset(point(x, px(-y)));
        // Drive the sync broadcast from the selected column this frame.
        self.scroll_driver = self.selected_column;
        cx.notify();
    }
    /// Cross-column scroll sync, FILE-ANCHORED. Always sources from the explicit
    /// `scroll_driver` (the last column the pointer scrolled), never from a
    /// follower - a short column whose offset got clamped to its own end never
    /// drags the others back, so the sync is drift-free across columns of
    /// differing height. Rather than copy the raw pixel offset (which drifts
    /// mid-file when the same file has different line counts across branches),
    /// it finds the file at the driver's viewport top + the intra-file delta and
    /// re-anchors each follower on THAT file's header, so "same file, two
    /// branches" stays truly lockstep. Falls back to the raw offset for a
    /// follower that doesn't contain the driver's top file.
    pub(super) fn broadcast_scroll(&self, mode: ViewMode) {
        if !self.sync_scroll {
            return;
        }
        let driver = if self
            .columns
            .get(self.scroll_driver)
            .map(|c| c.visible)
            .unwrap_or(false)
        {
            self.scroll_driver
        } else {
            match self.columns.iter().position(|c| c.visible) {
                Some(i) => i,
                None => return,
            }
        };
        let Some(driver_col) = self.columns.get(driver) else {
            return;
        };
        let driver_y = f32::from(-driver_col.el_scroll.offset().y).max(0.0);
        let (top_file, intra) = self.file_at_offset(driver_col, mode, driver_y);
        for (i, col) in self.columns.iter().enumerate() {
            if i == driver || !col.visible {
                continue;
            }
            let target_y = match &top_file {
                // Align on the same file's header across branches; the intra-file
                // delta keeps the relative position within the file.
                Some(path) => self
                    .file_top_offset(col, mode, path)
                    .map(|fy| fy + intra)
                    .unwrap_or(driver_y),
                None => driver_y,
            };
            let cur = col.el_scroll.offset();
            if f32::from(-cur.y) != target_y {
                col.el_scroll.set_offset(point(cur.x, px(-target_y)));
            }
        }
    }

    /// The file (header anchor path) at scrolled offset `y` in `col`, plus the
    /// intra-file delta (`y` minus that file header's top). Walks the displayed
    /// rows accumulating their variable heights, tracking the most recent file
    /// header, stopping once the accumulated height passes `y`. `(None, y)` when
    /// the column has no file header at/above `y` (empty / pre-first-header).
    pub(super) fn file_at_offset(
        &self,
        col: &Column,
        mode: ViewMode,
        y: f32,
    ) -> (Option<String>, f32) {
        // US-046: binary-search the precomputed prefix-sum offsets + the
        // row-sorted anchors instead of re-walking every row. `broadcast_scroll`
        // calls this per scroll-sync event, so the old O(rows) walk ran on every
        // wheel tick of a large diff.
        let (offsets, anchors) = match mode {
            ViewMode::Unified => (&col.disp_unified_offsets, &col.disp_anchors_unified),
            ViewMode::Split => (&col.disp_split_offsets, &col.disp_anchors_split),
        };
        // Row whose vertical band [offsets[r], offsets[r+1]) contains `y` - the
        // last offset that is still ≤ y (offsets is a len+1 prefix sum).
        let row = offsets.partition_point(|&o| o <= y).saturating_sub(1);
        // Most recent file header at or above that row (anchors sorted by row).
        match anchors
            .partition_point(|(_, ri)| *ri <= row)
            .checked_sub(1)
            .and_then(|i| anchors.get(i))
        {
            Some((path, anchor_row)) => {
                let top = offsets.get(*anchor_row).copied().unwrap_or(0.0);
                (Some(path.clone()), (y - top).max(0.0))
            }
            None => (None, y),
        }
    }

    /// Cumulative top offset (px) of `path`'s file header in `col`, or `None` if
    /// that column doesn't contain the file. Mirrors `jump_to_file`'s sum.
    pub(super) fn file_top_offset(&self, col: &Column, mode: ViewMode, path: &str) -> Option<f32> {
        // US-046: O(1) prefix-sum lookup of the anchor row's top instead of
        // re-summing every preceding row's height. The anchor lookup stays a
        // linear scan over file headers (few, and not row-sorted by path).
        let (offsets, anchors) = match mode {
            ViewMode::Unified => (&col.disp_unified_offsets, &col.disp_anchors_unified),
            ViewMode::Split => (&col.disp_split_offsets, &col.disp_anchors_split),
        };
        let idx = anchors.iter().find(|(p, _)| p == path).map(|(_, i)| *i)?;
        offsets.get(idx).copied()
    }

    fn scrollbar_segments_for_column(
        &self,
        col_idx: usize,
        mode: ViewMode,
    ) -> Option<Vec<HScrollbarSegment>> {
        let col = self.columns.get(col_idx)?;
        let bounds = col.el_scroll.bounds();
        let viewport_h = f32::from(bounds.size.height);
        let panel_width = f32::from(bounds.size.width);
        if viewport_h <= 0.0 || panel_width <= 0.0 {
            return None;
        }

        let visible_top = f32::from(-col.el_scroll.offset().y).max(0.0);
        let visible_bottom = visible_top + viewport_h;
        let (offsets, spans) = match mode {
            ViewMode::Unified => (&col.disp_unified_offsets, &col.disp_unified_spans),
            ViewMode::Split => (&col.disp_split_offsets, &col.disp_split_spans),
        };
        Some(super::super::hscroll::h_scrollbar_segments(
            spans,
            offsets,
            &col.h_offsets,
            mode == ViewMode::Split,
            panel_width,
            visible_top,
            visible_bottom,
        ))
    }

    fn h_scrollbar_local_point(&self, col_idx: usize, point: Point<Pixels>) -> Option<(f32, f32)> {
        let col = self.columns.get(col_idx)?;
        let bounds = col.el_scroll.bounds();
        if point.x < bounds.left()
            || point.x > bounds.right()
            || point.y < bounds.top()
            || point.y > bounds.bottom()
        {
            return None;
        }
        Some((
            f32::from(point.x - bounds.left()),
            f32::from(point.y - bounds.top() - col.el_scroll.offset().y).max(0.0),
        ))
    }

    pub(super) fn handle_horizontal_scrollbar_mouse_down(
        &mut self,
        col_idx: usize,
        point: Point<Pixels>,
        mode: ViewMode,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((x, y)) = self.h_scrollbar_local_point(col_idx, point) else {
            return false;
        };
        let Some(segments) = self.scrollbar_segments_for_column(col_idx, mode) else {
            return false;
        };
        let Some(segment) = segments.iter().find(|segment| {
            x >= segment.x
                && x <= segment.x + segment.width
                && y >= segment.y
                && y <= segment.y + super::super::hscroll::H_SCROLLBAR_TRACK_HEIGHT
        }) else {
            return false;
        };

        let thumb_left = segment.x + segment.thumb_x;
        let thumb_right = thumb_left + segment.thumb_width;
        let target = if x >= thumb_left && x <= thumb_right {
            segment.offset
        } else {
            super::super::hscroll::h_scrollbar_click_offset(&segments, x, y)
                .map(|(_, offset)| offset)
                .unwrap_or(segment.offset)
        };

        if let Some(col) = self.columns.get_mut(col_idx) {
            let offsets = Rc::make_mut(&mut col.h_offsets);
            if offsets.len() <= segment.offset_idx {
                offsets.resize(segment.offset_idx + 1, 0.0);
            }
            offsets[segment.offset_idx] = target.clamp(0.0, segment.max_scroll);
        }

        self.h_scroll_drag = Some(DiffHScrollDrag {
            col_idx,
            offset_idx: segment.offset_idx,
            start_mouse_x: point.x,
            start_offset: target.clamp(0.0, segment.max_scroll),
            max_scroll: segment.max_scroll,
            track_width: segment.width,
            thumb_width: segment.thumb_width,
        });
        cx.notify();
        true
    }

    pub(super) fn handle_horizontal_scrollbar_click(
        &mut self,
        col_idx: usize,
        point: Point<Pixels>,
        mode: ViewMode,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((x, y)) = self.h_scrollbar_local_point(col_idx, point) else {
            return false;
        };
        let Some(segments) = self.scrollbar_segments_for_column(col_idx, mode) else {
            return false;
        };
        let Some(segment) = segments.iter().find(|segment| {
            x >= segment.x
                && x <= segment.x + segment.width
                && y >= segment.y
                && y <= segment.y + super::super::hscroll::H_SCROLLBAR_TRACK_HEIGHT
        }) else {
            return false;
        };

        let thumb_left = segment.x + segment.thumb_x;
        let thumb_right = thumb_left + segment.thumb_width;
        if x >= thumb_left && x <= thumb_right {
            return true;
        }

        let target = super::super::hscroll::h_scrollbar_click_offset(&segments, x, y)
            .map(|(_, offset)| offset)
            .unwrap_or(segment.offset);
        if let Some(col) = self.columns.get_mut(col_idx) {
            let offsets = Rc::make_mut(&mut col.h_offsets);
            if offsets.len() <= segment.offset_idx {
                offsets.resize(segment.offset_idx + 1, 0.0);
            }
            offsets[segment.offset_idx] = target;
        }
        cx.notify();
        true
    }

    pub(super) fn drag_horizontal_scrollbar(&mut self, mouse_x: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.h_scroll_drag else {
            return;
        };
        let Some(col) = self.columns.get_mut(drag.col_idx) else {
            self.h_scroll_drag = None;
            cx.notify();
            return;
        };

        let track_range = (drag.track_width - drag.thumb_width).max(1.0);
        let delta = f32::from(mouse_x - drag.start_mouse_x);
        let next =
            (drag.start_offset + delta * drag.max_scroll / track_range).clamp(0.0, drag.max_scroll);
        let offsets = Rc::make_mut(&mut col.h_offsets);
        if offsets.len() <= drag.offset_idx {
            offsets.resize(drag.offset_idx + 1, 0.0);
        }
        if (offsets[drag.offset_idx] - next).abs() > 0.1 {
            offsets[drag.offset_idx] = next;
            cx.notify();
        }
    }

    pub(super) fn end_horizontal_scrollbar_drag(&mut self, cx: &mut Context<Self>) {
        if self.h_scroll_drag.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn apply_horizontal_wheel(
        &mut self,
        col_idx: usize,
        ev: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = ev.delta.pixel_delta(window.line_height());
        let dx = f32::from(delta.x);
        if dx == 0.0 {
            return;
        }

        let mode = self.effective_mode(window);
        let Some(col) = self.columns.get_mut(col_idx) else {
            return;
        };
        let bounds = col.el_scroll.bounds();
        if ev.position.x < bounds.left()
            || ev.position.x > bounds.right()
            || ev.position.y < bounds.top()
            || ev.position.y > bounds.bottom()
        {
            return;
        }

        let content_y = f32::from(ev.position.y - bounds.top() - col.el_scroll.offset().y).max(0.0);
        let (offsets, spans) = match mode {
            ViewMode::Unified => (&col.disp_unified_offsets, &col.disp_unified_spans),
            ViewMode::Split => (&col.disp_split_offsets, &col.disp_split_spans),
        };
        let Some(row) = hit_test::row_at_offset(offsets, content_y) else {
            return;
        };
        let Some(file_idx) = super::super::hscroll::file_at_row(spans, row) else {
            return;
        };

        let panel_width = f32::from(bounds.size.width);
        let split = mode == ViewMode::Split;
        let local_x = f32::from(ev.position.x - bounds.left());
        let right = split && super::super::hscroll::split_right_side_at_x(local_x, panel_width);
        let offset_idx = super::super::hscroll::h_offset_index(spans.len(), file_idx, split, right);
        let current = col.h_offsets.get(offset_idx).copied().unwrap_or(0.0);
        super::super::hscroll::set_file_side_offset(
            Rc::make_mut(&mut col.h_offsets),
            spans,
            file_idx,
            right,
            current - dx,
            split,
            panel_width,
        );
        cx.notify();
    }
}
