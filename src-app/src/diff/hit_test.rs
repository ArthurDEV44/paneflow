//! Single source for diff-body hit-testing (US-050): map between a vertical
//! pixel offset and a displayed-row index using a column's precomputed
//! prefix-sum offsets, replacing the per-call O(rows) height walks that several
//! call sites (`handle_body_click`, `row_at_point`, `jump_to_file`) previously
//! re-implemented inline.

/// Displayed-row index whose vertical band `[offsets[i], offsets[i + 1])`
/// contains `y` (scroll-adjusted, clamped ≥ 0), or `None` when `y` is at/past
/// the total content height (a click below the last row). `offsets` is the
/// `len + 1` prefix sum from `Column::recompute_display`; the lookup is
/// O(log rows) instead of the previous linear walk.
pub(super) fn row_at_offset(offsets: &[f32], y: f32) -> Option<usize> {
    // `partition_point` is the count of offsets ≤ y = index of the first offset
    // strictly greater than y, so the containing row is `pp - 1`. `pp == len`
    // means y is past the last band (no hit); `pp == 0` can't happen for y ≥ 0
    // since `offsets[0] == 0`.
    let pp = offsets.partition_point(|&o| o <= y);
    (1..offsets.len()).contains(&pp).then(|| pp - 1)
}

/// Cumulative top offset (px) of displayed row `idx` — an O(1) lookup into the
/// same prefix sum, replacing `rows[..idx].iter().map(height).sum()`. Returns
/// `0.0` when `idx` is out of range.
pub(super) fn row_top(offsets: &[f32], idx: usize) -> f32 {
    offsets.get(idx).copied().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // offsets for 3 rows of heights [10, 40, 18] -> prefix sum (len 4).
    const OFFSETS: &[f32] = &[0.0, 10.0, 50.0, 68.0];

    #[test]
    fn row_at_offset_maps_each_band() {
        assert_eq!(row_at_offset(OFFSETS, 0.0), Some(0));
        assert_eq!(row_at_offset(OFFSETS, 9.9), Some(0));
        assert_eq!(row_at_offset(OFFSETS, 10.0), Some(1)); // band boundary -> next row
        assert_eq!(row_at_offset(OFFSETS, 49.9), Some(1));
        assert_eq!(row_at_offset(OFFSETS, 50.0), Some(2));
        assert_eq!(row_at_offset(OFFSETS, 67.9), Some(2));
    }

    #[test]
    fn row_at_offset_past_end_is_none() {
        assert_eq!(row_at_offset(OFFSETS, 68.0), None); // exactly total height
        assert_eq!(row_at_offset(OFFSETS, 1000.0), None);
    }

    #[test]
    fn row_top_is_o1_prefix_lookup() {
        assert_eq!(row_top(OFFSETS, 0), 0.0);
        assert_eq!(row_top(OFFSETS, 2), 50.0);
        assert_eq!(row_top(OFFSETS, 99), 0.0); // out of range
    }
}
