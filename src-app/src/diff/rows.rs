//! Unified diff display-row model + per-row rendering (US-006,
//! prd-multi-worktree-diff-2026-Q3.md).
//!
//! `build_display_rows` reconstructs a standard unified diff (context / removed
//! / added lines, with file headers) as a flat row list keyed for
//! virtualization by `uniform_list`. Every row renders at a fixed height so the
//! uniform-list layout math holds. Side-by-side rendering (LHS/RHS with phantom
//! rows) is EP-003 (US-008/US-009); this is the EP-002 unified view.

use std::collections::HashMap;
use std::ops::Range;

use gpui::{Hsla, SharedString};

use super::engine::DiffHunkStatus;
use super::git::{FileChange, FileDiff};
use super::syntax::DiffSyntax;
use super::worddiff::{MAX_WORD_DIFF_LINE_COUNT, word_diff_ranges};

/// Per-file word-diff ranges, keyed by line index in each side's text. Only
/// populated for small modified hunks (US-010); other lines highlight at the
/// line level only.
struct WordMaps {
    old: HashMap<u32, Vec<Range<usize>>>,
    new: HashMap<u32, Vec<Range<usize>>>,
}

fn build_word_maps(file: &FileDiff, base_lines: &[&str], new_lines: &[&str]) -> WordMaps {
    let mut old: HashMap<u32, Vec<Range<usize>>> = HashMap::new();
    let mut new: HashMap<u32, Vec<Range<usize>>> = HashMap::new();
    for h in &file.hunks {
        let bcount = h.base_row_range.end - h.base_row_range.start;
        let ncount = h.new_row_range.end - h.new_row_range.start;
        // Word diff only for in-place modifications of bounded size (Zed's
        // MAX_WORD_DIFF_LINE_COUNT) — never on large or pure add/del hunks.
        if h.status != DiffHunkStatus::Modified
            || bcount != ncount
            || bcount > MAX_WORD_DIFF_LINE_COUNT
        {
            continue;
        }
        for k in 0..bcount {
            let bi = h.base_row_range.start + k;
            let ni = h.new_row_range.start + k;
            let (o, n) = word_diff_ranges(
                base_lines.get(bi as usize).copied().unwrap_or(""),
                new_lines.get(ni as usize).copied().unwrap_or(""),
            );
            if !o.is_empty() {
                old.insert(bi, o);
            }
            if !n.is_empty() {
                new.insert(ni, n);
            }
        }
    }
    WordMaps { old, new }
}

/// Content row height (CSS px) — compact, one diff line.
pub const ROW_HEIGHT: f32 = 18.0;

/// File-header row height. Taller than a content line so each file reads as a
/// padded, breathing "card" header (Zed buffer-subheader feel) while content
/// lines stay compact. The custom element lays rows out at variable heights
/// keyed off [`display_row_height`] / [`split_row_height`].
pub const FILE_HEADER_HEIGHT: f32 = 40.0;

/// Laid-out height of one unified row: a padded card for file headers, the
/// compact line height for everything else.
pub fn display_row_height(row: &DisplayRow) -> f32 {
    if matches!(row.kind, RowKind::FileHeader) {
        FILE_HEADER_HEIGHT
    } else {
        ROW_HEIGHT
    }
}

/// Side-by-side analog of [`display_row_height`].
pub fn split_row_height(row: &SplitRow) -> f32 {
    if matches!(row, SplitRow::Header(_)) {
        FILE_HEADER_HEIGHT
    } else {
        ROW_HEIGHT
    }
}

/// Cap on rendered rows across a whole column. Beyond this the column shows a
/// truncation notice instead of freezing the frame on a pathological diff.
pub const MAX_DISPLAY_ROWS: usize = 10_000;

/// Unchanged lines kept on each side of a hunk before the middle is collapsed
/// into a [`RowKind::Fold`] marker (mirrors Zed's default diff context).
pub const CONTEXT_LINES: u32 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    FileHeader,
    Context,
    Added,
    Removed,
    Binary,
    Truncated,
    /// Collapsed run of unchanged lines between/around hunks (Zed-style fold).
    Fold,
}

#[derive(Clone)]
pub struct DisplayRow {
    pub kind: RowKind,
    pub text: SharedString,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// Byte ranges of changed words within `text` (US-010); empty for context
    /// rows and modifications too large for word diff.
    pub word_ranges: Vec<Range<usize>>,
    /// Per-token foreground syntax runs (US-017), computed off-thread; empty
    /// when syntax highlighting is disabled or the line is plain.
    pub syntax_runs: Vec<(Range<usize>, Hsla)>,
}

/// Diff colors, snapshotted once per render and copied into the (`'static`)
/// `uniform_list` row closure.
#[derive(Clone, Copy)]
pub struct RowPalette {
    pub text: Hsla,
    pub muted: Hsla,
    pub header_bg: Hsla,
    /// Background of the pinned sticky file header that tracks the viewport top
    /// while a file's hunks scroll under it. Opaque + slightly elevated so it
    /// reads as floating above the scrolling body.
    pub sticky_header_bg: Hsla,
    /// Separator line above each file header (file-section division).
    pub border: Hsla,
    pub add_bg: Hsla,
    pub del_bg: Hsla,
    pub add_fg: Hsla,
    pub del_fg: Hsla,
    /// Gutter line-number tint for changed lines — added/removed numbers read in
    /// a status hue instead of flat `muted`, so the eye finds the changed lines
    /// from the gutter alone (GitHub / Zed behaviour). Context numbers stay
    /// `muted`.
    pub gutter_add: Hsla,
    pub gutter_del: Hsla,
    /// Modified-status foreground (US-010): colors the `M` sigil on a
    /// modified file's header row.
    pub mod_fg: Hsla,
    /// Opaque hunk-indicator bar colors. Zed blends the raw `version_control_*`
    /// color with the editor background before painting the gutter strip so it
    /// reads solid; we pre-blend once in `view.rs::palette()`.
    pub add_bar: Hsla,
    pub del_bar: Hsla,
    /// Background for balancing phantom cells in the side-by-side view.
    pub phantom_bg: Hsla,
    /// Stronger backgrounds for word-diff-highlighted spans (US-010).
    pub add_word_bg: Hsla,
    pub del_word_bg: Hsla,
}

#[allow(clippy::too_many_arguments)]
fn content_row(
    lines: &[&str],
    idx: u32,
    kind: RowKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
    word_ranges: Vec<Range<usize>>,
    syntax_runs: Vec<(Range<usize>, Hsla)>,
) -> DisplayRow {
    DisplayRow {
        kind,
        text: lines
            .get(idx as usize)
            .copied()
            .unwrap_or("")
            .to_string()
            .into(),
        old_no,
        new_no,
        word_ranges,
        syntax_runs,
    }
}

/// File extension (lowercased) used to pick a `syntect` grammar.
fn file_ext(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Per-line syntax runs for one side of a file, or an empty vec when syntax
/// highlighting is off / the file is binary. Indexed to match `str::lines()`.
fn side_syntax(
    syntax: Option<&DiffSyntax>,
    file: &FileDiff,
    text: &str,
) -> Vec<Vec<(Range<usize>, Hsla)>> {
    match syntax {
        Some(s) if !file.is_binary => {
            super::highlighter::highlight_lines(text, &file_ext(&file.path), s)
        }
        _ => Vec::new(),
    }
}

fn line_syntax(side: &[Vec<(Range<usize>, Hsla)>], idx: u32) -> Vec<(Range<usize>, Hsla)> {
    side.get(idx as usize).cloned().unwrap_or_default()
}

/// Build the flat, virtualization-ready row list for a column's files. Returns
/// the rows plus the number of content lines dropped by the `MAX_DISPLAY_ROWS`
/// cap (0 when nothing was truncated).
pub fn build_display_rows(
    files: &[FileDiff],
    syntax: Option<&DiffSyntax>,
) -> (Vec<DisplayRow>, usize) {
    let mut rows: Vec<DisplayRow> = Vec::new();
    let mut dropped = 0usize;

    for file in files {
        if rows.len() >= MAX_DISPLAY_ROWS {
            dropped += 1;
            continue;
        }
        let (added, removed) = file.line_counts();
        let sigil = match file.change {
            FileChange::Added => 'A',
            FileChange::Modified => 'M',
            FileChange::Deleted => 'D',
            FileChange::Renamed => 'R',
        };
        let shown_path = match (file.change, &file.old_path) {
            (FileChange::Renamed, Some(old)) => format!("{old} → {}", file.path),
            _ => file.path.clone(),
        };
        rows.push(DisplayRow {
            kind: RowKind::FileHeader,
            text: format!("{sigil}  {shown_path}   +{added} -{removed}").into(),
            old_no: None,
            new_no: None,
            word_ranges: Vec::new(),
            syntax_runs: Vec::new(),
        });

        if file.is_binary {
            rows.push(DisplayRow {
                kind: RowKind::Binary,
                text: "Diff not shown (binary or large file)".into(),
                old_no: None,
                new_no: None,
                word_ranges: Vec::new(),
                syntax_runs: Vec::new(),
            });
            continue;
        }

        let base_lines: Vec<&str> = file.base_text.lines().collect();
        let new_lines: Vec<&str> = file.new_text.lines().collect();
        let words = build_word_maps(file, &base_lines, &new_lines);
        let syn_old = side_syntax(syntax, file, &file.base_text);
        let syn_new = side_syntax(syntax, file, &file.new_text);
        let mut bc = 0u32; // base cursor (next unconsumed base row)
        let mut nc = 0u32; // new cursor (next unconsumed new row)

        let push = |row: DisplayRow, rows: &mut Vec<DisplayRow>, dropped: &mut usize| {
            if rows.len() >= MAX_DISPLAY_ROWS {
                *dropped += 1;
            } else {
                rows.push(row);
            }
        };

        // One equal/context line at (base `bi`, new `ni`).
        let ctx = |bi: u32, ni: u32| {
            content_row(
                &new_lines,
                ni,
                RowKind::Context,
                Some(bi + 1),
                Some(ni + 1),
                Vec::new(),
                line_syntax(&syn_new, ni),
            )
        };
        // A collapsed-context marker hiding `n` unchanged lines (Zed-style fold).
        let fold = |n: u32| DisplayRow {
            kind: RowKind::Fold,
            text: if n == 1 {
                "⋯ 1 unchanged line".into()
            } else {
                format!("⋯ {n} unchanged lines").into()
            },
            old_no: None,
            new_no: None,
            word_ranges: Vec::new(),
            syntax_runs: Vec::new(),
        };

        let mut first_gap = true;
        for h in &file.hunks {
            // Equal region before this hunk: keep CONTEXT_LINES bordering the
            // hunk and collapse the middle. The very first gap has no preceding
            // hunk, so it shows no lead context (Zed folds the top of the file).
            let gap = h.new_row_range.start - nc;
            let lead = if first_gap { 0 } else { CONTEXT_LINES.min(gap) };
            let trail = CONTEXT_LINES.min(gap - lead);
            let hidden = gap - lead - trail;
            for k in 0..lead {
                push(ctx(bc + k, nc + k), &mut rows, &mut dropped);
            }
            if hidden > 0 {
                push(fold(hidden), &mut rows, &mut dropped);
            }
            for k in 0..trail {
                let off = lead + hidden + k;
                push(ctx(bc + off, nc + off), &mut rows, &mut dropped);
            }
            for r in h.base_row_range.clone() {
                push(
                    content_row(
                        &base_lines,
                        r,
                        RowKind::Removed,
                        Some(r + 1),
                        None,
                        words.old.get(&r).cloned().unwrap_or_default(),
                        line_syntax(&syn_old, r),
                    ),
                    &mut rows,
                    &mut dropped,
                );
            }
            bc = h.base_row_range.end;
            for r in h.new_row_range.clone() {
                push(
                    content_row(
                        &new_lines,
                        r,
                        RowKind::Added,
                        None,
                        Some(r + 1),
                        words.new.get(&r).cloned().unwrap_or_default(),
                        line_syntax(&syn_new, r),
                    ),
                    &mut rows,
                    &mut dropped,
                );
            }
            nc = h.new_row_range.end;
            first_gap = false;
        }
        // Trailing equal region after the last hunk: keep CONTEXT_LINES, collapse
        // the rest down to EOF.
        let tail = new_lines.len() as u32 - nc;
        let lead = CONTEXT_LINES.min(tail);
        for k in 0..lead {
            push(ctx(bc + k, nc + k), &mut rows, &mut dropped);
        }
        let hidden = tail - lead;
        if hidden > 0 {
            push(fold(hidden), &mut rows, &mut dropped);
        }
    }

    if dropped > 0 {
        rows.push(DisplayRow {
            kind: RowKind::Truncated,
            text: format!("diff truncated — {dropped} more lines not shown").into(),
            old_no: None,
            new_no: None,
            word_ranges: Vec::new(),
            syntax_runs: Vec::new(),
        });
    }
    (rows, dropped)
}

// ── Side-by-side (split) rows (US-009) ──────────────────────────────────────

use super::align::{AlignedRow, Cell, CellKind, align_rows};

/// One resolved half (left=base or right=new) of a side-by-side row.
#[derive(Clone)]
pub struct HalfCell {
    pub kind: CellKind,
    pub no: Option<u32>,
    pub text: SharedString,
    pub word_ranges: Vec<Range<usize>>,
    pub syntax_runs: Vec<(Range<usize>, Hsla)>,
}

/// A row of the side-by-side view. `Pair` holds both halves so the two sides
/// share one row (and therefore one scroll offset — US-011 sync scroll is free).
#[derive(Clone)]
pub enum SplitRow {
    Header(SharedString),
    Note(SharedString),
    /// Collapsed run of unchanged lines (Zed-style fold), spanning both halves.
    Fold(SharedString),
    Pair {
        left: HalfCell,
        right: HalfCell,
    },
}

fn resolve_half(
    cell: Cell,
    lines: &[&str],
    words: &HashMap<u32, Vec<Range<usize>>>,
    syntax: &[Vec<(Range<usize>, Hsla)>],
) -> HalfCell {
    let (no, text, word_ranges, syntax_runs) = match cell.kind {
        CellKind::Phantom => (
            None,
            SharedString::default(),
            Vec::<Range<usize>>::new(),
            Vec::<(Range<usize>, Hsla)>::new(),
        ),
        _ => {
            let idx = cell.line.unwrap_or(0);
            (
                Some(idx + 1),
                lines
                    .get(idx as usize)
                    .copied()
                    .unwrap_or("")
                    .to_string()
                    .into(),
                words.get(&idx).cloned().unwrap_or_default(),
                line_syntax(syntax, idx),
            )
        }
    };
    HalfCell {
        kind: cell.kind,
        no,
        text,
        word_ranges,
        syntax_runs,
    }
}

/// Build the side-by-side row list for a column's files (US-009). Aligns each
/// file with [`align_rows`] and resolves cells to text. Honors the same
/// `MAX_DISPLAY_ROWS` cap as the unified builder.
pub fn build_split_rows(files: &[FileDiff], syntax: Option<&DiffSyntax>) -> (Vec<SplitRow>, usize) {
    let mut rows: Vec<SplitRow> = Vec::new();
    let mut dropped = 0usize;

    for file in files {
        if rows.len() >= MAX_DISPLAY_ROWS {
            dropped += 1;
            continue;
        }
        let (added, removed) = file.line_counts();
        let sigil = match file.change {
            FileChange::Added => 'A',
            FileChange::Modified => 'M',
            FileChange::Deleted => 'D',
            FileChange::Renamed => 'R',
        };
        let shown_path = match (file.change, &file.old_path) {
            (FileChange::Renamed, Some(old)) => format!("{old} → {}", file.path),
            _ => file.path.clone(),
        };
        rows.push(SplitRow::Header(
            format!("{sigil}  {shown_path}   +{added} -{removed}").into(),
        ));

        if file.is_binary {
            rows.push(SplitRow::Note(
                "Diff not shown (binary or large file)".into(),
            ));
            continue;
        }

        let base_lines: Vec<&str> = file.base_text.lines().collect();
        let new_lines: Vec<&str> = file.new_text.lines().collect();
        let words = build_word_maps(file, &base_lines, &new_lines);
        let syn_old = side_syntax(syntax, file, &file.base_text);
        let syn_new = side_syntax(syntax, file, &file.new_text);
        let aligned = align_rows(&file.hunks, base_lines.len() as u32, new_lines.len() as u32);
        // Collapse runs of unchanged (context-on-both-sides) aligned rows the
        // same way the unified builder does: keep CONTEXT_LINES bordering each
        // change, fold the middle (and the file head/tail) into one marker.
        let emit_pair = |a: &AlignedRow, rows: &mut Vec<SplitRow>, dropped: &mut usize| {
            if rows.len() >= MAX_DISPLAY_ROWS {
                *dropped += 1;
            } else {
                rows.push(SplitRow::Pair {
                    left: resolve_half(a.left, &base_lines, &words.old, &syn_old),
                    right: resolve_half(a.right, &new_lines, &words.new, &syn_new),
                });
            }
        };
        let emit_fold = |n: u32, rows: &mut Vec<SplitRow>, dropped: &mut usize| {
            if rows.len() >= MAX_DISPLAY_ROWS {
                *dropped += 1;
            } else {
                rows.push(SplitRow::Fold(if n == 1 {
                    "⋯ 1 unchanged line".into()
                } else {
                    format!("⋯ {n} unchanged lines").into()
                }));
            }
        };
        let is_ctx =
            |a: &AlignedRow| a.left.kind == CellKind::Context && a.right.kind == CellKind::Context;
        let total = aligned.len();
        let mut i = 0;
        while i < total {
            if is_ctx(&aligned[i]) {
                let mut j = i;
                while j < total && is_ctx(&aligned[j]) {
                    j += 1;
                }
                let run = (j - i) as u32;
                let lead = if i == 0 { 0 } else { CONTEXT_LINES.min(run) };
                let trail = if j == total {
                    0
                } else {
                    CONTEXT_LINES.min(run - lead)
                };
                let hidden = run - lead - trail;
                for k in 0..lead as usize {
                    emit_pair(&aligned[i + k], &mut rows, &mut dropped);
                }
                if hidden > 0 {
                    emit_fold(hidden, &mut rows, &mut dropped);
                }
                for k in 0..trail as usize {
                    let off = lead as usize + hidden as usize + k;
                    emit_pair(&aligned[i + off], &mut rows, &mut dropped);
                }
                i = j;
            } else {
                emit_pair(&aligned[i], &mut rows, &mut dropped);
                i += 1;
            }
        }
    }

    if dropped > 0 {
        rows.push(SplitRow::Note(
            format!("diff truncated — {dropped} more lines not shown").into(),
        ));
    }
    (rows, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_collapses_distant_context_into_fold() {
        // One change on line 0, then 29 unchanged lines. The trailing context
        // must collapse to CONTEXT_LINES kept + a single Fold marker, instead
        // of dumping the whole file (the pre-fold behaviour).
        let mut base = String::from("OLD\n");
        let mut new = String::from("NEW\n");
        for i in 1..30 {
            base.push_str(&format!("ctx{i}\n"));
            new.push_str(&format!("ctx{i}\n"));
        }
        let hunks = crate::diff::engine::compute_hunks(&base, &new);
        let file = FileDiff {
            path: "a.txt".into(),
            change: FileChange::Modified,
            old_path: None,
            base_text: base,
            new_text: new,
            hunks,
            is_binary: false,
        };
        let (rows, dropped) = build_display_rows(&[file], None);
        assert_eq!(dropped, 0);

        let folds: Vec<&DisplayRow> = rows.iter().filter(|r| r.kind == RowKind::Fold).collect();
        assert_eq!(folds.len(), 1, "exactly one collapsed region");
        assert!(
            folds[0].text.contains("26"),
            "fold hides 29 - 3 = 26 lines, got {:?}",
            folds[0].text
        );

        let ctx = rows.iter().filter(|r| r.kind == RowKind::Context).count();
        assert_eq!(ctx, CONTEXT_LINES as usize, "only bordering context kept");

        // 30-line file → folded output is a handful of rows, not 1 + 30.
        assert!(rows.len() < 10, "folded row count {} too large", rows.len());
    }

    #[test]
    fn split_collapses_distant_context_into_fold() {
        // Same fixture as the unified test: side-by-side must collapse the
        // unchanged tail too, instead of dumping every aligned row.
        let mut base = String::from("OLD\n");
        let mut new = String::from("NEW\n");
        for i in 1..30 {
            base.push_str(&format!("ctx{i}\n"));
            new.push_str(&format!("ctx{i}\n"));
        }
        let hunks = crate::diff::engine::compute_hunks(&base, &new);
        let file = FileDiff {
            path: "a.txt".into(),
            change: FileChange::Modified,
            old_path: None,
            base_text: base,
            new_text: new,
            hunks,
            is_binary: false,
        };
        let (rows, dropped) = build_split_rows(&[file], None);
        assert_eq!(dropped, 0);

        let folds = rows
            .iter()
            .filter(|r| matches!(r, SplitRow::Fold(_)))
            .count();
        assert_eq!(folds, 1, "exactly one collapsed region");

        let pairs = rows
            .iter()
            .filter(|r| matches!(r, SplitRow::Pair { .. }))
            .count();
        // 1 changed pair + 3 trailing context pairs kept; the rest folded.
        assert_eq!(pairs, 1 + CONTEXT_LINES as usize);
        assert!(rows.len() < 10, "folded row count {} too large", rows.len());
    }
}
