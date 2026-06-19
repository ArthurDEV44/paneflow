//! Unified diff display-row model + per-row rendering (US-006,
//! prd-multi-worktree-diff-2026-Q3.md).
//!
//! `build_display_rows` reconstructs a standard unified diff (context / removed
//! / added lines, with file headers) as a flat row list consumed by the custom
//! virtualized `super::element::DiffElement`, which culls to the visible window
//! using the precomputed per-row offsets (file headers are taller; other rows
//! use the compact line height). Side-by-side rendering (LHS/RHS with phantom
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
pub const FILE_HEADER_HEIGHT: f32 = 32.0;

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

/// Cumulative top offsets (px) for a unified row set: `offsets[i]` is the top of
/// row `i`, `offsets[len]` the total content height. Precomputed off the render
/// path (in `Column::recompute_display`) and shared with `DiffElement`, which
/// culls + lays out against it instead of re-walking every row each frame.
pub fn unified_offsets(rows: &[DisplayRow]) -> Vec<f32> {
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(rows.len() + 1);
    out.push(0.0);
    for r in rows {
        acc += display_row_height(r);
        out.push(acc);
    }
    out
}

/// Side-by-side analog of [`unified_offsets`].
pub fn split_offsets(rows: &[SplitRow]) -> Vec<f32> {
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(rows.len() + 1);
    out.push(0.0);
    for r in rows {
        acc += split_row_height(r);
        out.push(acc);
    }
    out
}

/// Cumulative top offsets (px) of each hunk's first changed row in a unified row
/// set. A "hunk start" is a change row (Added/Removed) whose predecessor is not
/// a change, so consecutive removed+added lines count as one hunk. Precomputed
/// in `Column::recompute_display` (US-046) so the toolbar's hunk counter and
/// hunk-nav read it per frame instead of re-walking every row.
pub fn unified_hunk_tops(rows: &[DisplayRow]) -> Vec<f32> {
    let mut tops = Vec::new();
    let mut acc = 0.0f32;
    let mut prev_change = false;
    for r in rows {
        let is_change = matches!(r.kind, RowKind::Added | RowKind::Removed);
        if is_change && !prev_change {
            tops.push(acc);
        }
        prev_change = is_change;
        acc += display_row_height(r);
    }
    tops
}

/// Side-by-side analog of [`unified_hunk_tops`].
pub fn split_hunk_tops(rows: &[SplitRow]) -> Vec<f32> {
    let mut tops = Vec::new();
    let mut acc = 0.0f32;
    let mut prev_change = false;
    for r in rows {
        let is_change = matches!(
            r,
            SplitRow::Pair { left, right }
                if matches!(left.kind, CellKind::Added | CellKind::Removed)
                    || matches!(right.kind, CellKind::Added | CellKind::Removed)
        );
        if is_change && !prev_change {
            tops.push(acc);
        }
        prev_change = is_change;
        acc += split_row_height(r);
    }
    tops
}

/// Widest line number across a unified row set (both sides); `0` when empty.
/// Used to size the line-number gutter once at build time.
pub fn unified_max_line_no(rows: &[DisplayRow]) -> u32 {
    rows.iter()
        .map(|r| r.new_no.unwrap_or(0).max(r.old_no.unwrap_or(0)))
        .max()
        .unwrap_or(0)
}

/// Side-by-side analog of [`unified_max_line_no`].
pub fn split_max_line_no(rows: &[SplitRow]) -> u32 {
    rows.iter()
        .map(|r| match r {
            SplitRow::Pair { left, right } => left.no.unwrap_or(0).max(right.no.unwrap_or(0)),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// One file's extent in a display-row set: the index of its `FileHeader` row
/// and the width (in monospace cells) of its widest code line. The widest-line
/// width drives the file's horizontal-scroll bound; precomputed off the render
/// path (in `recompute_display` / `AgentsDiffData::recompute`) and shared with
/// `DiffElement`, which offsets each file's code by its own scroll position
/// instead of re-measuring every row per frame. `max_chars` counts `char`s (not
/// bytes), matching the monospace-cell estimate the element scrolls by; it is
/// `0` for a collapsed file (header only) or a binary/fold-only file.
#[derive(Clone, Copy)]
pub struct FileSpan {
    pub header_row: usize,
    pub max_chars: usize,
}

/// Per-file spans for a unified row set, one entry per `FileHeader` in file
/// order (so `partition_point` on `header_row` maps any row back to its file).
pub fn unified_file_spans(rows: &[DisplayRow]) -> Vec<FileSpan> {
    let mut spans: Vec<FileSpan> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        match r.kind {
            RowKind::FileHeader => spans.push(FileSpan {
                header_row: i,
                max_chars: 0,
            }),
            RowKind::Context | RowKind::Added | RowKind::Removed => {
                if let Some(span) = spans.last_mut() {
                    span.max_chars = span.max_chars.max(r.text.chars().count());
                }
            }
            // Folds, binary notes and the truncation row never scroll.
            RowKind::Fold | RowKind::Binary | RowKind::Truncated => {}
        }
    }
    spans
}

/// Side-by-side analog of [`unified_file_spans`]. Each `Pair` row's wider half
/// cell contributes to its file's `max_chars` (the offset applies per half, but
/// one file-level bound covers the widest cell on either side).
pub fn split_file_spans(rows: &[SplitRow]) -> Vec<FileSpan> {
    let mut spans: Vec<FileSpan> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        match r {
            SplitRow::Header(_) => spans.push(FileSpan {
                header_row: i,
                max_chars: 0,
            }),
            SplitRow::Pair { left, right } => {
                if let Some(span) = spans.last_mut() {
                    let w = left.text.chars().count().max(right.text.chars().count());
                    span.max_chars = span.max_chars.max(w);
                }
            }
            SplitRow::Note(_) | SplitRow::Fold(_) => {}
        }
    }
    spans
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
    /// EP-002 US-006: typed file-header segments for [`RowKind::FileHeader`]
    /// rows (`None` for every other kind). Decomposes the header into a status
    /// sigil + directory prefix + basename + diffstat so the element paints
    /// each as its own typed run instead of one fused monospace string.
    pub header: Option<HeaderParts>,
}

/// EP-002 US-006: the file-header row, split into typed segments at build time
/// (off the render path) so [`super::element::DiffElement`] paints a structured
/// header — colored status sigil, muted directory prefix, emphasized basename,
/// right-aligned green/red diffstat — instead of one undifferentiated mono
/// string. Shared by the Review view and the Agents diff dock.
#[derive(Clone)]
pub struct HeaderParts {
    /// Leading status sigil: `A`/`M`/`D`/`R`. Colored by status in the element.
    pub sigil: char,
    /// Directory portion including the trailing `/`, or `""` at the repo root.
    /// The element truncates HERE under width pressure, never on the basename.
    pub dir_prefix: SharedString,
    /// File basename (a rename shows `old → new`). Never truncated.
    pub basename: SharedString,
    pub added: u32,
    pub removed: u32,
}

/// Split a display path into `(dir_prefix_with_trailing_slash, basename)`.
/// For a rename (`"old → new"`) the last `/` lands inside the new path, so the
/// new file's basename is emphasized and the `old → newdir/` lead falls into the
/// muted directory prefix — readable, allocation-cheap, never panics.
fn split_header_path(shown_path: &str) -> (String, String) {
    match shown_path.rfind('/') {
        Some(i) => (
            shown_path[..=i].to_string(),
            shown_path[i + 1..].to_string(),
        ),
        None => (String::new(), shown_path.to_string()),
    }
}

/// Diff colors, snapshotted once per render and copied into the (`'static`)
/// `super::element::DiffElement`, which owns its row data for the frame.
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
    /// EP-002 US-007: faint document wash on context (unchanged) code so the
    /// diff body reads as a surface, not the bare window background.
    pub context_bg: Hsla,
    /// EP-002 US-007: persistent line-number-rail tint, painted over every
    /// content row's gutter region so the gutter reads as a structural column.
    pub gutter_bg: Hsla,
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
        header: None,
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

/// Resolve a [`RowPalette`] from the active theme's UI colors. The single color
/// source for [`super::element::DiffElement`], shared by the Review view
/// ([`super::view`]) and the Agents diff dock ([`crate::app::agents_diff`]) so
/// both render with identical washes.
pub fn palette(ui: crate::theme::UiColors) -> RowPalette {
    let diff = ui.diff_colors();
    let is_light = ui.base.l > 0.5;
    RowPalette {
        text: ui.text,
        muted: ui.muted,
        // EP-002 US-005: file card at the `surface` tier (one step off the
        // `base` body), so each file reads as an elevated card over the body.
        header_bg: ui.surface,
        // EP-002 US-005/US-008: the sticky bar is the file header pinned to the
        // viewport top; it must read as FLOATING above the inline card, not
        // identical to it. A faint text-tint lift shifts it off `surface` on
        // both themes (lighter on dark, defined on light); the bottom hairline
        // the element paints completes the "floating layer" read.
        sticky_header_bg: ui.surface.blend(ui.text.opacity(0.06)),
        border: ui.border,
        add_bg: diff.added_background,
        del_bg: diff.deleted_background,
        add_fg: diff.added,
        del_fg: diff.deleted,
        // Gutter numbers for changed lines: the status hue softened toward
        // the gutter's muted baseline so they tint without shouting over the
        // line wash they sit on.
        gutter_add: ui.muted.blend(diff.added.opacity(0.75)),
        gutter_del: ui.muted.blend(diff.deleted.opacity(0.75)),
        mod_fg: ui.vc_modified,
        // Zed paints the gutter hunk strip as `editor_background.blend(version_control_*)`
        // so it reads solid; pre-blend against the diff body surface (`ui.base`,
        // what context lines sit on) so the bar is opaque, not faint at the wash alpha.
        add_bar: ui.base.blend(diff.added),
        del_bar: ui.base.blend(diff.deleted),
        // Neutral alignment-row fill, derived from `muted` so it tracks the
        // theme instead of a hardcoded slate hex.
        phantom_bg: ui.muted.opacity(0.12),
        // EP-002 US-007: intra-line word emphasis. Light keeps the theme's 0.40
        // `vc_word_*` alpha; dark drops to 0.28 — 0.40 read too hot over the
        // Codex-sampled dark line wash.
        add_word_bg: diff.added.opacity(if is_light { 0.40 } else { 0.28 }),
        del_word_bg: diff.deleted.opacity(if is_light { 0.40 } else { 0.28 }),
        // EP-002 US-007: a 2-3% document wash on unchanged code and a slightly
        // stronger (~4.5%) gutter rail so the body reads as a surface with a
        // structural line-number column. Derived from `muted` to track theme.
        context_bg: ui.muted.opacity(0.025),
        gutter_bg: ui.muted.opacity(0.045),
    }
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
        let (dir_prefix, basename) = split_header_path(&shown_path);
        rows.push(DisplayRow {
            kind: RowKind::FileHeader,
            text: format!("{sigil}  {shown_path}   +{added} -{removed}").into(),
            old_no: None,
            new_no: None,
            word_ranges: Vec::new(),
            syntax_runs: Vec::new(),
            header: Some(HeaderParts {
                sigil,
                dir_prefix: dir_prefix.into(),
                basename: basename.into(),
                added,
                removed,
            }),
        });

        if file.is_binary {
            rows.push(DisplayRow {
                kind: RowKind::Binary,
                text: "Diff not shown (binary or large file)".into(),
                old_no: None,
                new_no: None,
                word_ranges: Vec::new(),
                syntax_runs: Vec::new(),
                header: None,
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
            header: None,
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
            header: None,
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
    /// File-section header. EP-002 US-006: carries the same typed
    /// [`HeaderParts`] as the unified [`DisplayRow::header`] so split + unified
    /// paint identical structured headers.
    Header(HeaderParts),
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
        let (dir_prefix, basename) = split_header_path(&shown_path);
        rows.push(SplitRow::Header(HeaderParts {
            sigil,
            dir_prefix: dir_prefix.into(),
            basename: basename.into(),
            added,
            removed,
        }));

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
                // US-058: lead/hidden/trail partition `run = j - i`, so every
                // `i + off` below is < j <= total. The `.get()` guards make that
                // fail-safe (no release panic) if the arithmetic ever drifts.
                for k in 0..lead as usize {
                    debug_assert!(i + k < total, "lead index out of bounds");
                    if let Some(a) = aligned.get(i + k) {
                        emit_pair(a, &mut rows, &mut dropped);
                    }
                }
                if hidden > 0 {
                    emit_fold(hidden, &mut rows, &mut dropped);
                }
                for k in 0..trail as usize {
                    let off = lead as usize + hidden as usize + k;
                    debug_assert!(i + off < total, "trail index out of bounds");
                    if let Some(a) = aligned.get(i + off) {
                        emit_pair(a, &mut rows, &mut dropped);
                    }
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

/// Filter a unified row set by a per-file collapse set: a collapsed file keeps
/// only its header row, an expanded file keeps its full segment. `anchors` maps
/// each file path to its header row index (file order). Returns the filtered
/// rows plus rebuilt anchors (header index in the output). Shared by the Review
/// view ([`super::view`]) and the Agents diff dock ([`crate::app::agents_diff`]).
pub fn apply_collapse_unified(
    rows: &[DisplayRow],
    anchors: &[(String, usize)],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<DisplayRow>, Vec<(String, usize)>) {
    let mut out = Vec::with_capacity(rows.len());
    let mut out_anchors = Vec::with_capacity(anchors.len());
    for (index, (path, start)) in anchors.iter().enumerate() {
        let Some(header) = rows.get(*start) else {
            continue;
        };
        let end = anchors
            .get(index + 1)
            .map(|(_, next_start)| *next_start)
            .unwrap_or(rows.len())
            .min(rows.len());
        out_anchors.push((path.clone(), out.len()));
        if collapsed.contains(path) {
            out.push(header.clone());
        } else if let Some(segment) = rows.get(*start..end) {
            out.extend_from_slice(segment);
        }
    }
    (out, out_anchors)
}

/// Split-view counterpart of [`apply_collapse_unified`].
pub fn apply_collapse_split(
    rows: &[SplitRow],
    anchors: &[(String, usize)],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<SplitRow>, Vec<(String, usize)>) {
    let mut out = Vec::with_capacity(rows.len());
    let mut out_anchors = Vec::with_capacity(anchors.len());
    for (index, (path, start)) in anchors.iter().enumerate() {
        let end = anchors
            .get(index + 1)
            .map(|(_, next_start)| *next_start)
            .unwrap_or(rows.len())
            .min(rows.len());
        out_anchors.push((path.clone(), out.len()));
        if collapsed.contains(path) {
            match rows.get(*start) {
                Some(row @ SplitRow::Header(_)) => out.push(row.clone()),
                _ => {
                    out_anchors.pop();
                    continue;
                }
            }
        } else if let Some(segment) = rows.get(*start..end) {
            out.extend_from_slice(segment);
        }
    }
    (out, out_anchors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_header_path_separates_dir_from_basename() {
        // EP-002 US-006: the directory prefix keeps its trailing slash and the
        // basename is the last segment — root files have an empty prefix.
        assert_eq!(
            split_header_path("src/app/view.rs"),
            ("src/app/".to_string(), "view.rs".to_string())
        );
        assert_eq!(
            split_header_path("Cargo.toml"),
            (String::new(), "Cargo.toml".to_string())
        );
        // A rename ("old → new"): the last `/` is in the new path, so the new
        // file's basename is emphasized and the arrow lead falls into the
        // (muted) directory prefix.
        assert_eq!(
            split_header_path("old/a.rs → new/b.rs"),
            ("old/a.rs → new/".to_string(), "b.rs".to_string())
        );
    }

    #[test]
    fn file_header_row_carries_typed_segments() {
        // EP-002 US-006: build_display_rows must populate the structured header
        // for the file-header row (sigil + split path + diffstat), so the
        // element paints typed segments instead of re-parsing a fused string.
        let file = FileDiff {
            path: "src/diff/rows.rs".into(),
            change: FileChange::Modified,
            old_path: None,
            base_text: "a\n".into(),
            new_text: "b\n".into(),
            hunks: crate::diff::engine::compute_hunks("a\n", "b\n"),
            is_binary: false,
        };
        let (added, removed) = file.line_counts();
        let (rows, _) = build_display_rows(&[file], None);
        let header = rows
            .iter()
            .find(|r| r.kind == RowKind::FileHeader)
            .and_then(|r| r.header.as_ref())
            .expect("file-header row must carry HeaderParts");
        assert_eq!(header.sigil, 'M');
        assert_eq!(header.dir_prefix.as_ref(), "src/diff/");
        assert_eq!(header.basename.as_ref(), "rows.rs");
        assert_eq!((header.added, header.removed), (added, removed));
    }

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
    fn unified_hunk_tops_marks_each_hunk_start_at_its_row_offset() {
        // US-046: the cached hunk tops MUST equal the precomputed row offset of
        // every hunk-start row (a change row whose predecessor is not a change).
        // Guards against the offsets and hunk_tops computations drifting apart.
        let base = "a\nb\nc\nd\ne\n".to_string();
        let new = "a\nB\nc\nd\nE\n".to_string(); // two separate single-line edits
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
        let (rows, _) = build_display_rows(&[file], None);
        let offsets = unified_offsets(&rows);

        let mut expected = Vec::new();
        let mut prev_change = false;
        for (i, r) in rows.iter().enumerate() {
            let is_change = matches!(r.kind, RowKind::Added | RowKind::Removed);
            if is_change && !prev_change {
                expected.push(offsets[i]);
            }
            prev_change = is_change;
        }

        assert_eq!(unified_hunk_tops(&rows), expected);
        assert_eq!(expected.len(), 2, "fixture has two distinct hunks");
    }

    #[test]
    fn split_hunk_tops_marks_each_hunk_start_at_its_row_offset() {
        // US-046 (EP-008 review): the split analog of the unified drift guard.
        // `split_hunk_tops` walks the rows with its own accumulator; it MUST
        // agree with the `split_offsets` prefix sum at every hunk-start row, or
        // side-by-side hunk-nav jumps to the wrong pixel. Catches a future
        // divergence between `split_row_height` and the hunk-tops walk that the
        // unified guard would NOT surface (the two heights are separate fns).
        let base = "a\nb\nc\nd\ne\n".to_string();
        let new = "a\nB\nc\nd\nE\n".to_string(); // two separate single-line edits
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
        let (rows, _) = build_split_rows(&[file], None);
        let offsets = split_offsets(&rows);

        let mut expected = Vec::new();
        let mut prev_change = false;
        for (i, r) in rows.iter().enumerate() {
            let is_change = matches!(
                r,
                SplitRow::Pair { left, right }
                    if matches!(left.kind, CellKind::Added | CellKind::Removed)
                        || matches!(right.kind, CellKind::Added | CellKind::Removed)
            );
            if is_change && !prev_change {
                expected.push(offsets[i]);
            }
            prev_change = is_change;
        }

        assert_eq!(split_hunk_tops(&rows), expected);
        assert!(
            !expected.is_empty(),
            "fixture must produce at least one split hunk for the guard to be meaningful"
        );
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
