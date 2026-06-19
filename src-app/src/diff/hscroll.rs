//! Per-file horizontal scroll for the diff body.
//!
//! The unified diff pipeline (refactor `ee1a042`) dropped the dock's bespoke
//! per-file horizontal scroll when it moved to the direct-paint
//! [`super::element::DiffElement`]. This restores it as shared geometry: each
//! file owns one horizontal offset (px), bounded by [`max_h_scroll`]. The
//! element offsets each file's code by its own slot; the two hosts (Agents dock
//! and Review column) clamp the wheel/drag against the same bound, and render
//! the scrollbar.
//!
//! The cell-advance estimate is intentionally coarse - it only *bounds* the
//! scroll, never lays out glyphs - so a few px of slop just lets a sliver of
//! trailing whitespace scroll into view rather than clipping a line short.

use super::rows::FileSpan;

/// Estimated advance width of one monospace cell at the panel's 12px code text
/// (~0.6em). The small margin keeps the longest line fully reachable.
pub(crate) const DIFF_CHAR_WIDTH: f32 = 7.5;

/// Estimated visible width (px) of the code text column for the current mode.
/// Unified subtracts the fixed prefix (hunk bar + line-number gutter + pads);
/// split halves the panel and subtracts one gutter per side. Coarse on purpose.
pub(crate) fn h_text_viewport(split: bool, panel_width: f32) -> f32 {
    if split {
        (panel_width - 1.0) / 2.0 - 55.0
    } else {
        panel_width - 92.0
    }
}

/// Max horizontal scroll (px) for a file whose widest line is `max_chars`, in
/// the current mode + panel width. Zero when everything fits, so a short file
/// never overscrolls into empty space.
pub(crate) fn max_h_scroll(max_chars: usize, split: bool, panel_width: f32) -> f32 {
    let content_w = max_chars as f32 * DIFF_CHAR_WIDTH + 12.0;
    (content_w - h_text_viewport(split, panel_width)).max(0.0)
}

/// File index owning display row `row` - `partition_point` on the per-file
/// header rows (ascending). `None` for an empty span set.
pub(crate) fn file_at_row(spans: &[FileSpan], row: usize) -> Option<usize> {
    if spans.is_empty() {
        return None;
    }
    Some(
        spans
            .partition_point(|s| s.header_row <= row)
            .saturating_sub(1),
    )
}

/// Grow/shrink a per-file offset vector to `n` files, zeroing new slots. Called
/// lazily at render when a fresh diff changed the file count, so offsets stay
/// indexed by stable file position across collapse/split (which never change
/// the file count).
pub(crate) fn ensure_len(offsets: &mut Vec<f32>, n: usize) {
    if offsets.len() != n {
        offsets.resize(n, 0.0);
    }
}

/// Clamp + store `value` as file `idx`'s offset, bounded to its own
/// `max_h_scroll`. Resizes `offsets` to the span set first so a stale length
/// (file count just changed) never indexes the wrong file. Shared by both
/// hosts' wheel + scrollbar handlers.
pub(crate) fn set_file_offset(
    offsets: &mut Vec<f32>,
    spans: &[FileSpan],
    idx: usize,
    value: f32,
    split: bool,
    panel_width: f32,
) {
    ensure_len(offsets, spans.len());
    let max_chars = spans.get(idx).map(|s| s.max_chars).unwrap_or(0);
    if let Some(slot) = offsets.get_mut(idx) {
        *slot = value.clamp(0.0, max_h_scroll(max_chars, split, panel_width));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(header_row: usize, max_chars: usize) -> FileSpan {
        FileSpan {
            header_row,
            max_chars,
        }
    }

    #[test]
    fn max_h_scroll_zero_when_fits_then_grows() {
        // Short lines fit the text column → no scroll; a long line overflows.
        assert_eq!(max_h_scroll(10, false, 600.0), 0.0);
        assert!(max_h_scroll(400, false, 600.0) > 0.0);
        // Narrower panel → more overflow for the same line.
        assert!(max_h_scroll(120, false, 300.0) > max_h_scroll(120, false, 600.0));
    }

    #[test]
    fn file_at_row_maps_rows_to_files() {
        let spans = [span(0, 50), span(8, 50), span(20, 50)];
        assert_eq!(file_at_row(&spans, 0), Some(0));
        assert_eq!(file_at_row(&spans, 7), Some(0));
        assert_eq!(file_at_row(&spans, 8), Some(1));
        assert_eq!(file_at_row(&spans, 25), Some(2));
        assert_eq!(file_at_row(&[], 3), None);
    }

    #[test]
    fn set_file_offset_clamps_and_resizes() {
        let spans = [span(0, 400), span(10, 5)];
        let mut offsets = Vec::new();
        // File 0 (long line) clamps to its positive max; resized to 2 slots.
        set_file_offset(&mut offsets, &spans, 0, 1_000_000.0, false, 600.0);
        assert_eq!(offsets.len(), 2);
        assert!(offsets[0] > 0.0 && offsets[0] <= max_h_scroll(400, false, 600.0));
        // File 1 (short line) can't scroll → pinned at 0.
        set_file_offset(&mut offsets, &spans, 1, 500.0, false, 600.0);
        assert_eq!(offsets[1], 0.0);
        // Negative candidate clamps up to 0.
        set_file_offset(&mut offsets, &spans, 0, -50.0, false, 600.0);
        assert_eq!(offsets[0], 0.0);
    }
}
