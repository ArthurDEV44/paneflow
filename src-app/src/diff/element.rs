//! Custom GPUI `Element` for the diff body - virtualized, direct-paint.
//!
//! Replaces the per-row `div`+`StyledText` `uniform_list`, which re-ran Taffy
//! flex layout AND line shaping for every visible row on every frame (~300
//! Taffy nodes/frame across 3 split columns) and scrolled choppily in debug
//! builds. This element is modeled on Paneflow's `TerminalElement` (which is
//! the same approach Zed's `EditorElement` uses): it reports its full content
//! height and is hosted inside an `overflow_y_scroll` div, computes the visible
//! window from `window.content_mask()` (pixel math, no flex), shapes ONLY the
//! visible lines via the frame-cached `text_system().shape_line`, and paints
//! background quads + glyphs directly. Row data (`DisplayRow`/`SplitRow`) is
//! built off-thread and consumed unchanged - zero changes to `rows.rs`.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, Font, FontFeatures, FontStyle,
    FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, Length, Pixels, Point, ShapedLine, SharedString, Style, TextAlign, TextRun, Window,
    fill, point, px, relative, size,
};

use super::align::CellKind;
use super::hscroll::{file_at_row, max_h_scroll};
use super::rows::{
    DisplayRow, FileSpan, HalfCell, HeaderParts, ROW_HEIGHT, RowKind, RowPalette, SplitRow,
};

const PAD: f32 = 6.0; // file-header text inset
const CHEVRON_W: f32 = 14.0; // collapsible-section chevron column on file headers
const SIGIL_GAP: f32 = 8.0; // gap between the status sigil and the path (US-006)
const STAT_GAP: f32 = 16.0; // min gap between the path region and the right-aligned diffstat (US-006)
const HALF_PAD: f32 = 4.0; // split half-cell left padding (after the bar)
const GUTTER_W: f32 = 36.0; // single line-number column FLOOR width (widened per max digits)
const NUM_GAP: f32 = 6.0; // right padding inside the gutter (number → code gap)
const GUTTER_PAD_L: f32 = 8.0; // left breathing room inside the derived gutter
/// Pinned sticky file-header height. Slimmer than the inline 40px header card
/// so the floating context bar stays unobtrusive while scrolling a file.
const STICKY_HEADER_HEIGHT: f32 = 24.0;
// Zed's gutter diff-hunk strip width: floor(0.275 * line_height) = 4px at ROW_HEIGHT 18.
const BAR_W: f32 = 4.0; // colored hunk-indicator bar
const DELETED_BAR_DASH_H: f32 = 1.0;
const DELETED_BAR_DASH_STEP: f32 = 2.0;
const PAD2: f32 = 6.0; // gap between the hunk bar and the line-number gutter

/// The row source for one column - either unified or side-by-side.
///
/// Each variant carries its precomputed cumulative row offsets (`offsets[i]` =
/// top of row `i`, `offsets[len]` = total content height) and widest line
/// number. Both are derived ONCE off the per-frame path - in
/// [`super::view::Column::recompute_display`] - and shared as an `Rc`, so
/// `request_layout` / `prepaint` never re-walk every row (previously two O(N)
/// `Vec<f32>` allocations per column per frame).
pub enum DiffBody {
    Unified {
        rows: Rc<Vec<DisplayRow>>,
        offsets: Rc<Vec<f32>>,
        max_line_no: u32,
        /// Per-file horizontal-scroll spans (widest line per file) + live
        /// per-file horizontal offsets (px), lockstep with `rows`. The element
        /// shifts each file's code left by its own offset; everything else
        /// (gutter, headers, sticky bar) stays pinned.
        spans: Rc<Vec<FileSpan>>,
        h_offsets: Rc<Vec<f32>>,
    },
    Split {
        rows: Rc<Vec<SplitRow>>,
        offsets: Rc<Vec<f32>>,
        max_line_no: u32,
        /// Per-file horizontal-scroll spans (widest line per file) + live
        /// per-file horizontal offsets (px), lockstep with `rows`. The element
        /// shifts each file's code left by its own offset; everything else
        /// (gutter, headers, sticky bar) stays pinned.
        spans: Rc<Vec<FileSpan>>,
        h_offsets: Rc<Vec<f32>>,
    },
}

impl DiffBody {
    fn len(&self) -> usize {
        match self {
            DiffBody::Unified { rows, .. } => rows.len(),
            DiffBody::Split { rows, .. } => rows.len(),
        }
    }

    /// Widest line number anywhere in the body (across both sides), used to size
    /// the gutter so wide line numbers never clip past its left edge. Precomputed
    /// at build time; `0` for an empty body.
    fn max_line_no(&self) -> u32 {
        match self {
            DiffBody::Unified { max_line_no, .. } | DiffBody::Split { max_line_no, .. } => {
                *max_line_no
            }
        }
    }

    /// Cheap clone of the shared cumulative-offset vector (length `len + 1`).
    /// Returned as an owned `Rc` so the caller can mutate other `self` fields
    /// (e.g. `gutter_w`) without holding a borrow of `self.body`.
    fn offsets_rc(&self) -> Rc<Vec<f32>> {
        match self {
            DiffBody::Unified { offsets, .. } | DiffBody::Split { offsets, .. } => offsets.clone(),
        }
    }

    /// Per-file scroll spans (widest line + header row per file).
    fn spans_rc(&self) -> Rc<Vec<FileSpan>> {
        match self {
            DiffBody::Unified { spans, .. } | DiffBody::Split { spans, .. } => spans.clone(),
        }
    }

    /// Live per-file horizontal offsets (px), indexed by file position.
    fn h_offsets_rc(&self) -> Rc<Vec<f32>> {
        match self {
            DiffBody::Unified { h_offsets, .. } | DiffBody::Split { h_offsets, .. } => {
                h_offsets.clone()
            }
        }
    }
}

/// One solid rectangle to paint (row / cell background, divider, word-diff bg).
struct Quad {
    bounds: Bounds<Pixels>,
    color: Hsla,
}

/// One shaped text fragment to paint at `origin`, optionally clipped to `clip`
/// (used to keep a split half's long line from bleeding past its column).
struct Glyphs {
    origin: Point<Pixels>,
    line: ShapedLine,
    clip: Option<Bounds<Pixels>>,
}

/// Prepaint output: the fully-resolved draw lists for the visible window only.
pub struct DiffPrepaint {
    quads: Vec<Quad>,
    glyphs: Vec<Glyphs>,
    /// Pinned sticky-header draw list, painted AFTER the body (and after
    /// `glyphs`) so it floats over the scrolling rows. Empty when the viewport
    /// top sits above the first file's inline header (nothing to pin yet).
    sticky_quads: Vec<Quad>,
    sticky_glyphs: Vec<Glyphs>,
    /// Hitboxes over the visible file-header rows so the cursor becomes a
    /// pointing hand there (the headers are click-to-collapse). `Normal`
    /// behavior - does not consume the click, which still bubbles to the
    /// hosting div's `on_click`.
    header_hitboxes: Vec<Hitbox>,
}

pub struct DiffElement {
    body: DiffBody,
    palette: RowPalette,
    font: Font,
    font_size: Pixels,
    line_height: Pixels,
    /// Gutter column width, derived each prepaint from the body's widest line
    /// number (floor [`GUTTER_W`]). A field so the layout helpers read one
    /// resolved value instead of threading it through every signature.
    gutter_w: Pixels,
}

impl DiffElement {
    pub fn new(body: DiffBody, palette: RowPalette) -> Self {
        // The mono family is constant (always the embedded default) and a fresh
        // `DiffElement` is built every frame, so resolve it once per thread and
        // clone the cheap `SharedString` handle instead of re-resolving + heap-
        // allocating a family string on every render.
        thread_local! {
            static MONO_FAMILY: SharedString =
                crate::terminal::element::resolve_font_family(None).into();
        }
        let family = MONO_FAMILY.with(|f| f.clone());
        Self {
            body,
            palette,
            font: Font {
                family,
                features: FontFeatures::disable_ligatures(),
                fallbacks: None,
                weight: FontWeight::NORMAL,
                style: FontStyle::Normal,
            },
            font_size: px(12.),
            line_height: px(ROW_HEIGHT),
            gutter_w: px(GUTTER_W),
        }
    }

    /// Split a line into `TextRun`s: the syntax runs carry their own color, the
    /// gaps fall back to `default`. With syntax off, `syntax` is empty → one
    /// run. The run lengths sum to `text.len()`, which `shape_line` requires.
    fn text_runs(
        &self,
        text: &str,
        syntax: &[(Range<usize>, Hsla)],
        default: Hsla,
    ) -> Vec<TextRun> {
        let run = |len: usize, color: Hsla| TextRun {
            len,
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        if syntax.is_empty() {
            return vec![run(text.len(), default)];
        }
        let len = text.len();
        let mut runs = Vec::new();
        let mut ix = 0usize;
        for (r, color) in syntax {
            let start = r.start.min(len);
            let end = r.end.min(len);
            if start < ix || start >= end {
                continue; // defensive: skip malformed/overlapping ranges
            }
            if start > ix {
                runs.push(run(start - ix, default));
            }
            runs.push(run(end - start, *color));
            ix = end;
        }
        if ix < len {
            runs.push(run(len - ix, default));
        }
        runs
    }

    /// Shape a styled line (main text column).
    fn shape(
        &self,
        window: &mut Window,
        text: &SharedString,
        syntax: &[(Range<usize>, Hsla)],
        default: Hsla,
    ) -> ShapedLine {
        let runs = self.text_runs(text, syntax, default);
        window
            .text_system()
            .shape_line(text.clone(), self.font_size, &runs, None)
    }

    /// Shape a single line in a specific font weight (EP-002 US-006: the
    /// semibold basename segment of a file header). One run, one color.
    fn shape_weighted(
        &self,
        window: &mut Window,
        text: SharedString,
        color: Hsla,
        weight: FontWeight,
    ) -> ShapedLine {
        let font = Font {
            weight,
            ..self.font.clone()
        };
        let runs = [TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        window
            .text_system()
            .shape_line(text, self.font_size, &runs, None)
    }

    /// EP-002 US-006: paint a structured file-header row - colored status
    /// sigil, muted directory prefix, emphasized (semibold) basename, and a
    /// right-aligned green/red diffstat - replacing the old single fused
    /// monospace string. Drives BOTH the inline file card (`sticky = false`:
    /// taller, top separator, `header_bg`) and the pinned sticky bar
    /// (`sticky = true`: slim, bottom hairline, elevated `sticky_header_bg`),
    /// so the two never drift.
    #[allow(clippy::too_many_arguments)]
    fn paint_file_header(
        &self,
        window: &mut Window,
        parts: &HeaderParts,
        origin: Point<Pixels>,
        width: Pixels,
        row_h: Pixels,
        collapsed: bool,
        sticky: bool,
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let bounds = Bounds::new(origin, size(width, row_h));
        // Card / sticky background.
        quads.push(Quad {
            bounds,
            color: if sticky {
                p.sticky_header_bg
            } else {
                p.header_bg
            },
        });
        // A sticky bar floats with a bottom hairline; an inline card is divided
        // from the row above it by a top separator.
        let sep_y = if sticky {
            origin.y + row_h - px(1.)
        } else {
            origin.y
        };
        quads.push(Quad {
            bounds: Bounds::new(point(origin.x, sep_y), size(width, px(1.))),
            color: p.border,
        });

        let ty = origin.y + (row_h - lh) / 2.0;
        // Collapse chevron (a pinned/sticky file is, by definition, expanded).
        glyphs.push(Glyphs {
            origin: point(origin.x + px(PAD), ty),
            line: self.shape_plain(window, if collapsed { "▸" } else { "▾" }.into(), p.muted),
            clip: Some(bounds),
        });

        // Status sigil in its status color (A green, D red, M/R modified).
        let sigil_color = match parts.sigil {
            'A' => p.add_fg,
            'D' => p.del_fg,
            _ => p.mod_fg,
        };
        let sigil_x = origin.x + px(PAD + CHEVRON_W);
        let sigil_line = self.shape_plain(window, parts.sigil.to_string().into(), sigil_color);
        let sigil_w = sigil_line.width();
        glyphs.push(Glyphs {
            origin: point(sigil_x, ty),
            line: sigil_line,
            clip: Some(bounds),
        });

        // Right-aligned diffstat: "+N" (added, green) then "-N" (deleted, red).
        let stat_text = format!("+{} -{}", parts.added, parts.removed);
        let split = stat_text.find(" -").unwrap_or(stat_text.len());
        let stat_runs = [(0..split, p.add_fg), (split..stat_text.len(), p.del_fg)];
        let stat_ss: SharedString = stat_text.into();
        let stat_line = self.shape(window, &stat_ss, &stat_runs, p.muted);
        let stat_w = stat_line.width();
        let path_x = sigil_x + sigil_w + px(SIGIL_GAP);
        let stat_x = (bounds.right() - px(PAD) - stat_w).max(path_x);
        glyphs.push(Glyphs {
            origin: point(stat_x, ty),
            line: stat_line,
            clip: Some(bounds),
        });

        // Path region [path_x, region_right). The basename is emphasized and
        // NEVER truncated; the directory prefix is muted and gives way first -
        // trailing-aligned into its shrunken slot so the immediate parent dir
        // survives the clip.
        let region_right = (stat_x - px(STAT_GAP)).max(path_x);
        let avail = (region_right - path_x).max(px(0.));
        let base_line =
            self.shape_weighted(window, parts.basename.clone(), p.text, FontWeight::SEMIBOLD);
        let dir_line = self.shape_plain(window, parts.dir_prefix.clone(), p.muted);
        let bw = base_line.width().min(avail);
        let dir_avail = (avail - bw).max(px(0.));
        let dw = dir_line.width();
        if dw <= dir_avail {
            let base_x = path_x + dw;
            if dw > px(0.) {
                glyphs.push(Glyphs {
                    origin: point(path_x, ty),
                    line: dir_line,
                    clip: Some(bounds),
                });
            }
            glyphs.push(Glyphs {
                origin: point(base_x, ty),
                line: base_line,
                clip: Some(Bounds::new(point(base_x, origin.y), size(bw, row_h))),
            });
        } else {
            let base_x = path_x + dir_avail;
            let dir_origin_x = base_x - dw; // overflows left; masked by the clip
            glyphs.push(Glyphs {
                origin: point(dir_origin_x, ty),
                line: dir_line,
                clip: Some(Bounds::new(point(path_x, origin.y), size(dir_avail, row_h))),
            });
            glyphs.push(Glyphs {
                origin: point(base_x, ty),
                line: base_line,
                clip: Some(Bounds::new(point(base_x, origin.y), size(bw, row_h))),
            });
        }
    }

    /// Shape a single-color string (gutter numbers, signs, headers).
    fn shape_plain(&self, window: &mut Window, text: SharedString, color: Hsla) -> ShapedLine {
        let runs = [TextRun {
            len: text.len(),
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        window
            .text_system()
            .shape_line(text, self.font_size, &runs, None)
    }

    fn push_hunk_bar(
        origin: Point<Pixels>,
        height: Pixels,
        color: Hsla,
        dashed: bool,
        quads: &mut Vec<Quad>,
    ) {
        if !dashed {
            quads.push(Quad {
                bounds: Bounds::new(origin, size(px(BAR_W), height)),
                color,
            });
            return;
        }

        let height = f32::from(height).max(0.0);
        let mut y = 0.0;
        while y < height {
            let dash_h = DELETED_BAR_DASH_H.min(height - y);
            quads.push(Quad {
                bounds: Bounds::new(
                    point(origin.x, origin.y + px(y)),
                    size(px(BAR_W), px(dash_h)),
                ),
                color,
            });
            y += DELETED_BAR_DASH_STEP;
        }
    }

    fn push_phantom_hatches(
        &self,
        window: &mut Window,
        bounds: Bounds<Pixels>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return;
        }

        let count = ((f32::from(bounds.size.width) / 4.0).ceil() as usize).max(1) + 8;
        let phase = (f32::from(bounds.origin.y) / 3.0).rem_euclid(8.0);
        let pattern: SharedString = "/".repeat(count).into();
        glyphs.push(Glyphs {
            origin: point(bounds.origin.x - px(phase), bounds.origin.y),
            line: self.shape_plain(window, pattern, self.palette.phantom_hatch),
            clip: Some(bounds),
        });
    }

    /// Emit draw commands for one unified row at top-left `origin`.
    #[allow(clippy::too_many_arguments)]
    fn layout_unified(
        &self,
        window: &mut Window,
        row: &DisplayRow,
        origin: Point<Pixels>,
        width: Pixels,
        row_h: Pixels,
        collapsed: bool,
        h_offset: Pixels,
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let row_bounds = Bounds::new(origin, size(width, row_h));

        match row.kind {
            RowKind::FileHeader => match &row.header {
                // EP-002 US-006: structured header (sigil + dir + basename +
                // right-aligned diffstat). Always present for header rows.
                Some(parts) => self.paint_file_header(
                    window, parts, origin, width, row_h, collapsed, false, quads, glyphs,
                ),
                // Defensive fallback for the impossible header-less case.
                None => {
                    quads.push(Quad {
                        bounds: row_bounds,
                        color: p.header_bg,
                    });
                    glyphs.push(Glyphs {
                        origin: point(origin.x + px(PAD), origin.y + (row_h - lh) / 2.0),
                        line: self.shape_plain(window, row.text.clone(), p.text),
                        clip: Some(row_bounds),
                    });
                }
            },
            RowKind::Binary | RowKind::Truncated => {
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD), origin.y),
                    line: self.shape_plain(window, row.text.clone(), p.muted),
                    clip: Some(row_bounds),
                });
            }
            RowKind::Fold => {
                // Collapsed unchanged region: a faint separator band with a
                // quiet count, aligned under the code column. The label is
                // dimmed below `muted` (Codex redesign): it is pure metadata -
                // it must never compete with the code lines around it.
                quads.push(Quad {
                    bounds: row_bounds,
                    color: p.muted.opacity(0.06),
                });
                let text_x = origin.x + px(BAR_W + PAD2) + self.gutter_w;
                glyphs.push(Glyphs {
                    origin: point(text_x, origin.y),
                    line: self.shape_plain(window, row.text.clone(), p.muted.opacity(0.6)),
                    clip: Some(row_bounds),
                });
            }
            RowKind::Context | RowKind::Added | RowKind::Removed => {
                // (row-bg wash, opaque hunk-bar color, word-diff bg). EP-002
                // US-007: context now gets a faint document wash instead of the
                // bare window background.
                let (bg, gutter_bg, bar_color, word_bg) = match row.kind {
                    RowKind::Added => (
                        Some(p.add_bg),
                        p.add_gutter_bg,
                        Some(p.add_bar),
                        Some(p.add_word_bg),
                    ),
                    RowKind::Removed => (
                        Some(p.del_bg),
                        p.del_gutter_bg,
                        Some(p.del_bar),
                        Some(p.del_word_bg),
                    ),
                    _ => (Some(p.context_bg), p.gutter_bg, None, None),
                };
                if let Some(bg) = bg {
                    quads.push(Quad {
                        bounds: row_bounds,
                        color: bg,
                    });
                }
                // EP-002 US-007: gutter rail - a slightly stronger tint over the
                // line-number column (incl. the hunk-bar lane) so the gutter
                // reads as a structural rail on every content row.
                quads.push(Quad {
                    bounds: Bounds::new(origin, size(px(BAR_W + PAD2) + self.gutter_w, row_h)),
                    color: gutter_bg,
                });
                // Zed-style colored hunk-indicator bar at the far left.
                if let Some(c) = bar_color {
                    Self::push_hunk_bar(origin, lh, c, row.kind == RowKind::Removed, quads);
                }
                // One line-number gutter (Zed shows a single merged display-row
                // number: new line for adds/context, old line for deletes).
                let line_no = row.new_no.or(row.old_no);
                let num_color = match row.kind {
                    RowKind::Added => p.gutter_add,
                    RowKind::Removed => p.gutter_del,
                    _ => p.muted,
                };
                self.gutter(
                    window,
                    line_no,
                    origin.x + px(BAR_W + PAD2),
                    origin.y,
                    num_color,
                    glyphs,
                );
                // Text - no +/- sign column (the bar + wash convey status). The
                // code shifts left by this file's horizontal offset; the gutter
                // (painted above) stays pinned and the clip holds the shifted
                // text inside the code column.
                let text_x = origin.x + px(BAR_W + PAD2) + self.gutter_w;
                let code_x = text_x - h_offset;
                if !row.text.is_empty() {
                    let line = self.shape(window, &row.text, &row.syntax_runs, p.text);
                    if let Some(wbg) = word_bg {
                        self.push_word_quads(
                            &line,
                            &row.word_ranges,
                            code_x,
                            text_x,
                            origin.y,
                            lh,
                            wbg,
                            quads,
                        );
                    }
                    glyphs.push(Glyphs {
                        origin: point(code_x, origin.y),
                        line,
                        clip: Some(Bounds::new(
                            point(text_x, origin.y),
                            size((row_bounds.right() - text_x).max(px(0.)), lh),
                        )),
                    });
                }
            }
        }
    }

    /// Emit draw commands for one split row at top-left `origin`.
    #[allow(clippy::too_many_arguments)]
    fn layout_split(
        &self,
        window: &mut Window,
        row: &SplitRow,
        origin: Point<Pixels>,
        width: Pixels,
        row_h: Pixels,
        collapsed: bool,
        h_offset: Pixels,
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let row_bounds = Bounds::new(origin, size(width, row_h));

        match row {
            // EP-002 US-006: same structured header as the unified view.
            SplitRow::Header(parts) => {
                self.paint_file_header(
                    window, parts, origin, width, row_h, collapsed, false, quads, glyphs,
                );
            }
            SplitRow::Note(text) => {
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD), origin.y),
                    line: self.shape_plain(window, text.clone(), p.muted),
                    clip: Some(row_bounds),
                });
            }
            SplitRow::Fold(text) => {
                // Collapsed unchanged region: faint full-width band + muted count.
                quads.push(Quad {
                    bounds: row_bounds,
                    color: p.muted.opacity(0.06),
                });
                let text_x = origin.x + px(BAR_W + HALF_PAD) + self.gutter_w;
                glyphs.push(Glyphs {
                    origin: point(text_x, origin.y),
                    line: self.shape_plain(window, text.clone(), p.muted),
                    clip: Some(row_bounds),
                });
            }
            SplitRow::Pair { left, right } => {
                let half_w = ((width - px(1.)) / 2.0).max(px(0.));
                let left_x = origin.x;
                let right_x = origin.x + half_w + px(1.);
                self.layout_half(
                    window,
                    left,
                    point(left_x, origin.y),
                    half_w,
                    h_offset,
                    quads,
                    glyphs,
                );
                // Divider.
                quads.push(Quad {
                    bounds: Bounds::new(point(origin.x + half_w, origin.y), size(px(1.), lh)),
                    color: p.muted,
                });
                self.layout_half(
                    window,
                    right,
                    point(right_x, origin.y),
                    half_w,
                    h_offset,
                    quads,
                    glyphs,
                );
            }
        }
    }

    /// One half-cell of a split row, occupying `[origin.x, origin.x + width)`.
    #[allow(clippy::too_many_arguments)]
    fn layout_half(
        &self,
        window: &mut Window,
        cell: &HalfCell,
        origin: Point<Pixels>,
        width: Pixels,
        h_offset: Pixels,
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let half_bounds = Bounds::new(origin, size(width, lh));
        let (bg, gutter_bg, bar_color, word_bg) = match cell.kind {
            CellKind::Added => (
                Some(p.add_bg),
                p.add_gutter_bg,
                Some(p.add_bar),
                Some(p.add_word_bg),
            ),
            CellKind::Removed => (
                Some(p.del_bg),
                p.del_gutter_bg,
                Some(p.del_bar),
                Some(p.del_word_bg),
            ),
            CellKind::Phantom => (Some(p.phantom_bg), p.gutter_bg, None, None),
            // EP-002 US-007: faint document wash on unchanged code.
            CellKind::Context => (Some(p.context_bg), p.gutter_bg, None, None),
        };
        if let Some(bg) = bg {
            quads.push(Quad {
                bounds: half_bounds,
                color: bg,
            });
        }
        if matches!(cell.kind, CellKind::Phantom) {
            let hatch_x = origin.x + px(BAR_W + HALF_PAD) + self.gutter_w;
            let hatch_w = (half_bounds.right() - hatch_x).max(px(0.));
            self.push_phantom_hatches(
                window,
                Bounds::new(point(hatch_x, origin.y), size(hatch_w, lh)),
                glyphs,
            );
            return; // empty gap: no hunk bar, number, or code text
        }
        // EP-002 US-007: gutter rail over this half's line-number column.
        quads.push(Quad {
            bounds: Bounds::new(origin, size(px(BAR_W + HALF_PAD) + self.gutter_w, lh)),
            color: gutter_bg,
        });
        // Zed-style colored hunk-indicator bar at the half's left edge.
        if let Some(c) = bar_color {
            Self::push_hunk_bar(origin, lh, c, cell.kind == CellKind::Removed, quads);
        }
        let num_color = match cell.kind {
            CellKind::Added => p.gutter_add,
            CellKind::Removed => p.gutter_del,
            _ => p.muted,
        };
        self.gutter(
            window,
            cell.no,
            origin.x + px(BAR_W + HALF_PAD),
            origin.y,
            num_color,
            glyphs,
        );
        let text_x = origin.x + px(BAR_W + HALF_PAD) + self.gutter_w;
        let code_x = text_x - h_offset;
        if !cell.text.is_empty() {
            let line = self.shape(window, &cell.text, &cell.syntax_runs, p.text);
            if let Some(wbg) = word_bg {
                self.push_word_quads(
                    &line,
                    &cell.word_ranges,
                    code_x,
                    text_x,
                    origin.y,
                    lh,
                    wbg,
                    quads,
                );
            }
            glyphs.push(Glyphs {
                origin: point(code_x, origin.y),
                line,
                clip: Some(Bounds::new(
                    point(text_x, origin.y),
                    size((half_bounds.right() - text_x).max(px(0.)), lh),
                )),
            });
        }
    }

    fn gutter(
        &self,
        window: &mut Window,
        n: Option<u32>,
        x: Pixels,
        y: Pixels,
        color: Hsla,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let Some(n) = n else { return };
        let line = self.shape_plain(window, n.to_string().into(), color);
        // Right-align within the derived gutter column [x, x + gutter_w) (Zed
        // places line numbers flush-right with a small gap before the code),
        // clamped so very wide numbers don't underflow past the gutter's left
        // edge.
        let right_x = (x + self.gutter_w - px(NUM_GAP) - line.width()).max(x);
        glyphs.push(Glyphs {
            origin: point(right_x, y),
            line,
            clip: None,
        });
    }

    /// Push intra-line word-diff background quads. `ranges` are byte ranges into
    /// the same text that produced `line`, so `line.x_for_index(b)` (reachable via
    /// `ShapedLine`'s `Deref` to `LineLayout`) gives the glyph-aligned x relative
    /// to the line origin; add `text_x` to place it. These quads sit above the
    /// row wash and below the glyphs (all quads paint before all glyphs). No
    /// panics: byte ranges are clamped to the shaped length and degenerate spans
    /// are skipped.
    #[allow(clippy::too_many_arguments)]
    fn push_word_quads(
        &self,
        line: &ShapedLine,
        ranges: &[Range<usize>],
        code_x: Pixels,
        clip_left: Pixels,
        y: Pixels,
        lh: Pixels,
        color: Hsla,
        quads: &mut Vec<Quad>,
    ) {
        let len = line.len();
        for r in ranges {
            let start = r.start.min(len);
            let end = r.end.min(len);
            if start >= end {
                continue;
            }
            // The code is shifted left by the file's horizontal offset; clamp
            // the word-diff background to the viewport's left edge so a scrolled
            // span never bleeds over the pinned gutter (its right side stays
            // bounded by the element's content mask).
            let x0 = (code_x + line.x_for_index(start)).max(clip_left);
            let x1 = code_x + line.x_for_index(end);
            let w = (x1 - x0).max(px(0.));
            if w <= px(0.) {
                continue;
            }
            quads.push(Quad {
                bounds: Bounds::new(point(x0, y), size(w, lh)),
                color,
            });
        }
    }
}

impl Element for DiffElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<DiffPrepaint>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        // Full content height (sum of variable row heights) - the hosting
        // `overflow_y_scroll` div clips/scrolls. Reads the precomputed offsets
        // (no per-frame walk).
        let h = px(self.body.offsets_rc().last().copied().unwrap_or(0.0));
        style.size.height = Length::Definite(h.into());
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let row_count = self.body.len();

        // Variable-height layout: cumulative top offsets per row (precomputed,
        // shared). Cull against them (binary search) instead of a uniform line
        // height.
        let offsets = self.body.offsets_rc();
        let mask = window.content_mask();
        let vtop = f32::from(mask.bounds.origin.y - bounds.origin.y).max(0.0);
        let vbot = vtop + f32::from(mask.bounds.size.height);
        // First row whose top is at/above the viewport top; last row (exclusive)
        // whose top is still above the viewport bottom.
        let first = offsets
            .partition_point(|&o| o <= vtop)
            .saturating_sub(1)
            .min(row_count);
        let last = offsets.partition_point(|&o| o < vbot).min(row_count);
        // Diagnostic: fires only if the clip rect did NOT bound the visible
        // window (i.e. content_mask == full content height) → culling broken.
        if last.saturating_sub(first) > 200 {
            log::warn!(
                "diff element: visible {first}..{last} = {} of {row_count} rows - culling off?",
                last - first
            );
        }

        // Derive the gutter width from the body's widest line number so 5-digit
        // line numbers in large files never clip past the gutter's left edge.
        // One shaped digit gives the exact monospace advance.
        let digits = {
            // Count decimal digits by integer division - robust across libm
            // implementations, unlike `log10().floor()` whose rounding can be
            // off-by-one at exact powers of ten (log10(1000) may be 2.9999998).
            let mut n = self.body.max_line_no().max(1);
            let mut count = 0usize;
            while n > 0 {
                count += 1;
                n /= 10;
            }
            count.max(2)
        };
        let digit_w = f32::from(
            self.shape_plain(window, "0".into(), self.palette.muted)
                .width(),
        );
        self.gutter_w = px((GUTTER_PAD_L + digits as f32 * digit_w + NUM_GAP).max(GUTTER_W));

        // Per-file horizontal scroll inputs (spans + live offsets), read once so
        // the shaping loop offsets each file's code without re-borrowing `body`.
        let spans = self.body.spans_rc();
        let h_offsets = self.body.h_offsets_rc();
        let mut quads = Vec::new();
        let mut glyphs = Vec::new();
        let mut sticky_quads = Vec::new();
        let mut sticky_glyphs = Vec::new();
        let mut header_hitboxes = Vec::new();
        let width = bounds.size.width;

        // Clone the Rc so the borrow of `self.body` doesn't conflict with the
        // `&mut self` shaping calls below.
        match &self.body {
            DiffBody::Unified { rows, .. } => {
                let rows = rows.clone();
                for i in first..last {
                    let origin = point(bounds.origin.x, bounds.origin.y + px(offsets[i]));
                    let row_h = px(offsets[i + 1] - offsets[i]);
                    // A file is folded when the row after its header is another
                    // header (or EOF) - collapsed files emit a header-only row.
                    let collapsed = rows
                        .get(i + 1)
                        .is_none_or(|r| r.kind == RowKind::FileHeader);
                    if matches!(rows[i].kind, RowKind::FileHeader) {
                        header_hitboxes.push(window.insert_hitbox(
                            Bounds::new(origin, size(width, row_h)),
                            HitboxBehavior::Normal,
                        ));
                    }
                    let h_offset = px(file_at_row(&spans, i)
                        .map(|f| {
                            let raw = h_offsets.get(f).copied().unwrap_or(0.0);
                            let max = spans
                                .get(f)
                                .map(|s| max_h_scroll(s.max_chars, false, f32::from(width)))
                                .unwrap_or(0.0);
                            raw.clamp(0.0, max)
                        })
                        .unwrap_or(0.0));
                    self.layout_unified(
                        window,
                        &rows[i],
                        origin,
                        width,
                        row_h,
                        collapsed,
                        h_offset,
                        &mut quads,
                        &mut glyphs,
                    );
                }
                // Pinned sticky header for the file under the viewport top.
                let cur = spans
                    .partition_point(|span| span.header_row <= first)
                    .checked_sub(1)
                    .and_then(|file_idx| {
                        spans.get(file_idx).map(|span| (file_idx, span.header_row))
                    });
                if let Some((file_idx, hidx)) = cur
                    && offsets[hidx] < vtop
                {
                    // Next file header within the slide-up band → push the sticky
                    // up so the incoming file's inline header displaces it.
                    let nh = spans.get(file_idx + 1).map(|span| span.header_row);
                    let mut sticky_y = mask.bounds.origin.y;
                    if let Some(nh) = nh {
                        let nh_abs = bounds.origin.y + px(offsets[nh]);
                        if nh_abs < mask.bounds.origin.y + px(STICKY_HEADER_HEIGHT) {
                            sticky_y = nh_abs - px(STICKY_HEADER_HEIGHT);
                        }
                    }
                    if sticky_y + px(STICKY_HEADER_HEIGHT) > mask.bounds.origin.y
                        && let Some(parts) = rows[hidx].header.as_ref()
                    {
                        self.paint_file_header(
                            window,
                            parts,
                            point(bounds.origin.x, sticky_y),
                            width,
                            px(STICKY_HEADER_HEIGHT),
                            false,
                            true,
                            &mut sticky_quads,
                            &mut sticky_glyphs,
                        );
                    }
                }
            }
            DiffBody::Split { rows, .. } => {
                let rows = rows.clone();
                for i in first..last {
                    let origin = point(bounds.origin.x, bounds.origin.y + px(offsets[i]));
                    let row_h = px(offsets[i + 1] - offsets[i]);
                    let collapsed = rows
                        .get(i + 1)
                        .is_none_or(|r| matches!(r, SplitRow::Header(_)));
                    if matches!(rows[i], SplitRow::Header(_)) {
                        header_hitboxes.push(window.insert_hitbox(
                            Bounds::new(origin, size(width, row_h)),
                            HitboxBehavior::Normal,
                        ));
                    }
                    let h_offset = px(file_at_row(&spans, i)
                        .map(|f| {
                            let raw = h_offsets.get(f).copied().unwrap_or(0.0);
                            let max = spans
                                .get(f)
                                .map(|s| max_h_scroll(s.max_chars, true, f32::from(width)))
                                .unwrap_or(0.0);
                            raw.clamp(0.0, max)
                        })
                        .unwrap_or(0.0));
                    self.layout_split(
                        window,
                        &rows[i],
                        origin,
                        width,
                        row_h,
                        collapsed,
                        h_offset,
                        &mut quads,
                        &mut glyphs,
                    );
                }
                // Pinned sticky header for the file under the viewport top.
                let cur = spans
                    .partition_point(|span| span.header_row <= first)
                    .checked_sub(1)
                    .and_then(|file_idx| {
                        spans.get(file_idx).map(|span| (file_idx, span.header_row))
                    });
                if let Some((file_idx, hidx)) = cur
                    && offsets[hidx] < vtop
                {
                    let nh = spans.get(file_idx + 1).map(|span| span.header_row);
                    let mut sticky_y = mask.bounds.origin.y;
                    if let Some(nh) = nh {
                        let nh_abs = bounds.origin.y + px(offsets[nh]);
                        if nh_abs < mask.bounds.origin.y + px(STICKY_HEADER_HEIGHT) {
                            sticky_y = nh_abs - px(STICKY_HEADER_HEIGHT);
                        }
                    }
                    if sticky_y + px(STICKY_HEADER_HEIGHT) > mask.bounds.origin.y
                        && let SplitRow::Header(parts) = &rows[hidx]
                    {
                        self.paint_file_header(
                            window,
                            parts,
                            point(bounds.origin.x, sticky_y),
                            width,
                            px(STICKY_HEADER_HEIGHT),
                            false,
                            true,
                            &mut sticky_quads,
                            &mut sticky_glyphs,
                        );
                    }
                }
            }
        }

        Some(DiffPrepaint {
            quads,
            glyphs,
            sticky_quads,
            sticky_glyphs,
            header_hitboxes,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(layout) = prepaint.take() else {
            return;
        };
        // Pointer cursor over the clickable file-header cards.
        for hb in &layout.header_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hb);
        }
        let lh = self.line_height;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for q in &layout.quads {
                window.paint_quad(fill(q.bounds, q.color));
            }
            for g in layout.glyphs {
                if let Some(clip) = g.clip {
                    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                        let _ = g
                            .line
                            .paint(g.origin, lh, TextAlign::Left, None, window, cx);
                    });
                } else {
                    let _ = g
                        .line
                        .paint(g.origin, lh, TextAlign::Left, None, window, cx);
                }
            }
            // Sticky header floats above the scrolled body: paint its quads then
            // glyphs LAST so they overlay the rows that scroll underneath.
            for q in &layout.sticky_quads {
                window.paint_quad(fill(q.bounds, q.color));
            }
            for g in layout.sticky_glyphs {
                if let Some(clip) = g.clip {
                    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                        let _ = g
                            .line
                            .paint(g.origin, lh, TextAlign::Left, None, window, cx);
                    });
                } else {
                    let _ = g
                        .line
                        .paint(g.origin, lh, TextAlign::Left, None, window, cx);
                }
            }
        });
    }
}

impl IntoElement for DiffElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
