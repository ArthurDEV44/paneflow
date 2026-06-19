//! Cross-column scroll sync + file-at-offset helpers for the Review view
//! (US-004 code-motion). See [`super`] for the `DiffView` definition.

use super::*;

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
}
