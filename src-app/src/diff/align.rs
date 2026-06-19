//! Phantom-row alignment for the side-by-side diff view (US-008,
//! prd-multi-worktree-diff-2026-Q3.md).
//!
//! Zed keeps the two sides of a split diff vertically aligned by inserting
//! balancing blank ("phantom") rows on the shorter side of each hunk - the
//! `Companion` mechanism welded into its `DisplayMap` block pipeline. Paneflow
//! has no editor, so this reimplements the *insight* as ~100 lines of pure
//! layout math over a flat row list: a `Vec<AlignedRow>` where every row pairs
//! a left (base) cell with a right (new) cell, padding with `Phantom` where one
//! side has fewer lines. Rendering each pair as a single row (US-009) makes
//! synchronized scroll automatic - both halves live in one row of one list.

use super::engine::DiffHunk;

/// What a single side's cell shows on one aligned row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// Unchanged line present on both sides.
    Context,
    /// Added line (new side only).
    Added,
    /// Removed line (base side only).
    Removed,
    /// Balancing blank inserted to keep the sides aligned. Never selectable.
    Phantom,
}

/// One side's cell: its kind and the 0-based row it references in that side's
/// text (`None` for a phantom).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub kind: CellKind,
    pub line: Option<u32>,
}

impl Cell {
    const PHANTOM: Cell = Cell {
        kind: CellKind::Phantom,
        line: None,
    };

    fn context(line: u32) -> Cell {
        Cell {
            kind: CellKind::Context,
            line: Some(line),
        }
    }
}

/// A single side-by-side row: a left (base) cell and a right (new) cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignedRow {
    pub left: Cell,
    pub right: Cell,
}

/// Produce the aligned side-by-side row plan for one file.
///
/// Context lines pair 1:1 (`left=base row`, `right=new row`). Within each hunk,
/// removed line `k` pairs with added line `k`; the shorter side is padded with
/// `Phantom` cells. A pure addition therefore yields phantom left cells, a pure
/// deletion phantom right cells. Pure: takes hunks + line counts, returns rows.
pub fn align_rows(
    hunks: &[DiffHunk],
    base_line_count: u32,
    new_line_count: u32,
) -> Vec<AlignedRow> {
    let mut rows = Vec::new();
    let mut bc = 0u32; // next unconsumed base row
    let mut nc = 0u32; // next unconsumed new row

    for h in hunks {
        // Context before the hunk: both sides advance in lockstep.
        while nc < h.new_row_range.start && bc < h.base_row_range.start {
            rows.push(AlignedRow {
                left: Cell::context(bc),
                right: Cell::context(nc),
            });
            bc += 1;
            nc += 1;
        }

        // Hunk body: pair removed (left) with added (right), pad with phantoms.
        // Index directly into the ranges - no per-hunk `Vec<u32>` allocation.
        let rem_start = h.base_row_range.start;
        let add_start = h.new_row_range.start;
        let rem_len = h.base_row_range.end - rem_start;
        let add_len = h.new_row_range.end - add_start;
        let pairs = rem_len.max(add_len);
        for k in 0..pairs {
            let left = if k < rem_len {
                Cell {
                    kind: CellKind::Removed,
                    line: Some(rem_start + k),
                }
            } else {
                Cell::PHANTOM
            };
            let right = if k < add_len {
                Cell {
                    kind: CellKind::Added,
                    line: Some(add_start + k),
                }
            } else {
                Cell::PHANTOM
            };
            rows.push(AlignedRow { left, right });
        }

        bc = h.base_row_range.end;
        nc = h.new_row_range.end;
    }

    // Trailing context after the last hunk.
    while nc < new_line_count && bc < base_line_count {
        rows.push(AlignedRow {
            left: Cell::context(bc),
            right: Cell::context(nc),
        });
        bc += 1;
        nc += 1;
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::super::engine::DiffHunkStatus;
    use super::*;

    fn hunk(b: std::ops::Range<u32>, n: std::ops::Range<u32>, status: DiffHunkStatus) -> DiffHunk {
        DiffHunk {
            base_row_range: b,
            new_row_range: n,
            status,
        }
    }

    #[test]
    fn no_hunks_all_context() {
        let rows = align_rows(&[], 3, 3);
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter()
                .all(|r| r.left.kind == CellKind::Context && r.right.kind == CellKind::Context)
        );
        assert_eq!(rows[0].left.line, Some(0));
        assert_eq!(rows[0].right.line, Some(0));
    }

    #[test]
    fn pure_addition_pads_left_with_phantoms() {
        // base "a\nb" (2 lines), new "a\nX\nY\nb" → added rows 1..3 at new index 1.
        let rows = align_rows(&[hunk(1..1, 1..3, DiffHunkStatus::Added)], 2, 4);
        // row0: context a/a; rows1-2: phantom|Added; row3: context b/b
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].left.kind, CellKind::Context);
        assert_eq!(rows[1].left.kind, CellKind::Phantom);
        assert_eq!(rows[1].right.kind, CellKind::Added);
        assert_eq!(rows[2].left.kind, CellKind::Phantom);
        assert_eq!(rows[2].right.kind, CellKind::Added);
        assert_eq!(rows[3].left.kind, CellKind::Context);
    }

    #[test]
    fn pure_deletion_pads_right_with_phantoms() {
        let rows = align_rows(&[hunk(1..3, 1..1, DiffHunkStatus::Deleted)], 4, 2);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].left.kind, CellKind::Removed);
        assert_eq!(rows[1].right.kind, CellKind::Phantom);
        assert_eq!(rows[2].left.kind, CellKind::Removed);
        assert_eq!(rows[2].right.kind, CellKind::Phantom);
    }

    #[test]
    fn modification_pairs_removed_with_added() {
        // One line changed in place: removed row 1 pairs with added row 1.
        let rows = align_rows(&[hunk(1..2, 1..2, DiffHunkStatus::Modified)], 3, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].left.kind, CellKind::Removed);
        assert_eq!(rows[1].left.line, Some(1));
        assert_eq!(rows[1].right.kind, CellKind::Added);
        assert_eq!(rows[1].right.line, Some(1));
    }

    #[test]
    fn uneven_modification_pads_shorter_side() {
        // 1 base line replaced by 3 new lines: 1 paired row + 2 phantom-left rows.
        let rows = align_rows(&[hunk(0..1, 0..3, DiffHunkStatus::Modified)], 1, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].left.kind, CellKind::Removed);
        assert_eq!(rows[0].right.kind, CellKind::Added);
        assert_eq!(rows[1].left.kind, CellKind::Phantom);
        assert_eq!(rows[1].right.kind, CellKind::Added);
        assert_eq!(rows[2].left.kind, CellKind::Phantom);
        assert_eq!(rows[2].right.kind, CellKind::Added);
    }
}
