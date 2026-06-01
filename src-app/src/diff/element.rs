//! Custom GPUI `Element` for the diff body — virtualized, direct-paint.
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
//! built off-thread and consumed unchanged — zero changes to `rows.rs`.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, Element, ElementId, Font, FontFeatures, FontStyle,
    FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, Length, Pixels, Point, ShapedLine, SharedString, Style, TextAlign, TextRun, Window,
    fill, point, px, relative, size,
};

use super::align::CellKind;
use super::rows::{
    DisplayRow, HalfCell, ROW_HEIGHT, RowKind, RowPalette, SplitRow, display_row_height,
    split_row_height,
};

const PAD: f32 = 6.0; // file-header text inset
const CHEVRON_W: f32 = 14.0; // collapsible-section chevron column on file headers
const HALF_PAD: f32 = 4.0; // split half-cell left padding (after the bar)
const GUTTER_W: f32 = 36.0; // single line-number column FLOOR width (widened per max digits)
const NUM_GAP: f32 = 6.0; // right padding inside the gutter (number → code gap)
const GUTTER_PAD_L: f32 = 8.0; // left breathing room inside the derived gutter
/// Pinned sticky file-header height. Slimmer than the inline 40px header card
/// so the floating context bar stays unobtrusive while scrolling a file.
const STICKY_HEADER_HEIGHT: f32 = 24.0;
// Zed's gutter diff-hunk strip width: floor(0.275 * line_height) = 4px at ROW_HEIGHT 18.
const BAR_W: f32 = 4.0; // colored hunk-indicator bar
const PAD2: f32 = 6.0; // gap between the hunk bar and the line-number gutter

/// The row source for one column — either unified or side-by-side.
pub enum DiffBody {
    Unified(Rc<Vec<DisplayRow>>),
    Split(Rc<Vec<SplitRow>>),
}

impl DiffBody {
    fn len(&self) -> usize {
        match self {
            DiffBody::Unified(r) => r.len(),
            DiffBody::Split(r) => r.len(),
        }
    }

    /// Largest line number anywhere in the body (across both sides), used to
    /// size the gutter so wide line numbers never clip past its left edge. `0`
    /// for an empty body (no numbered rows). One linear pass — cheap relative to
    /// the per-frame `offsets()` walk it sits beside.
    fn max_line_no(&self) -> u32 {
        match self {
            DiffBody::Unified(rows) => rows
                .iter()
                .map(|r| r.new_no.unwrap_or(0).max(r.old_no.unwrap_or(0)))
                .max()
                .unwrap_or(0),
            DiffBody::Split(rows) => rows
                .iter()
                .map(|r| match r {
                    SplitRow::Pair { left, right } => {
                        left.no.unwrap_or(0).max(right.no.unwrap_or(0))
                    }
                    _ => 0,
                })
                .max()
                .unwrap_or(0),
        }
    }

    /// Cumulative top offsets (px) for every row, length `len + 1`. `offsets[i]`
    /// is the top of row `i`; `offsets[len]` is the total content height. Rows
    /// have variable height (taller file headers), so the element culls + lays
    /// out against this instead of a uniform line height.
    fn offsets(&self) -> Vec<f32> {
        let mut acc = 0.0;
        let mut out = Vec::with_capacity(self.len() + 1);
        out.push(0.0);
        match self {
            DiffBody::Unified(rows) => {
                for r in rows.iter() {
                    acc += display_row_height(r);
                    out.push(acc);
                }
            }
            DiffBody::Split(rows) => {
                for r in rows.iter() {
                    acc += split_row_height(r);
                    out.push(acc);
                }
            }
        }
        out
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
    /// behavior — does not consume the click, which still bubbles to the
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
        let family = crate::terminal::element::resolve_font_family(None);
        Self {
            body,
            palette,
            font: Font {
                family: family.into(),
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

    /// US-010 (prd-git-diff-mode-2026-Q3.md): color run for a file-header
    /// line's leading status sigil — `A`/`M`/`D` painted with the curated
    /// `vc_*` slot, the rest left at the header text color. Returned as a
    /// single `(0..1, color)` run consumed by [`Self::shape`], so the header
    /// stays one fixed-height shaped line (no layout change).
    fn header_sigil_runs(&self, text: &str) -> Vec<(std::ops::Range<usize>, Hsla)> {
        let p = &self.palette;
        let mut runs: Vec<(std::ops::Range<usize>, Hsla)> = Vec::new();
        // Leading A/M/D status sigil.
        match text.as_bytes().first() {
            Some(b'A') => runs.push((0..1, p.add_fg)),
            Some(b'D') => runs.push((0..1, p.del_fg)),
            Some(b'M') => runs.push((0..1, p.mod_fg)),
            _ => {}
        }
        // Trailing "+{added} -{removed}" diff stat: +N green, -N red. They are
        // the last two whitespace tokens, so `rfind` from the end is robust to
        // paths containing `+` / `-`. Pushed in byte order (sigil < +N < -N) so
        // the run list stays sorted + non-overlapping for `text_runs`.
        let is_digit = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_digit());
        if let Some(dash) = text.rfind(" -")
            && is_digit(&text[dash + 2..])
        {
            if let Some(plus) = text[..dash].rfind(" +")
                && is_digit(&text[plus + 2..])
            {
                runs.push((plus + 1..dash, p.add_fg));
            }
            runs.push((dash + 1..text.len(), p.del_fg));
        }
        runs
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
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let row_bounds = Bounds::new(origin, size(width, row_h));

        match row.kind {
            RowKind::FileHeader => {
                quads.push(Quad {
                    bounds: row_bounds,
                    color: p.header_bg,
                });
                // 1px separator above each file header so files read as distinct
                // collapsible "cards" (Zed buffer-subheader separation).
                quads.push(Quad {
                    bounds: Bounds::new(origin, size(width, px(1.))),
                    color: p.border,
                });
                // Vertically center the header content within the taller card.
                let ty = origin.y + (row_h - lh) / 2.0;
                // Collapsible-section chevron: ▸ when folded (next row is another
                // header / EOF), ▾ when expanded. Painted as a glyph; the header
                // text shifts right by CHEVRON_W.
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD), ty),
                    line: self.shape_plain(
                        window,
                        if collapsed { "▸" } else { "▾" }.into(),
                        p.muted,
                    ),
                    clip: Some(row_bounds),
                });
                // US-010: status-colored sigil, the rest at header text color.
                let runs = self.header_sigil_runs(&row.text);
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD + CHEVRON_W), ty),
                    line: self.shape(window, &row.text, &runs, p.text),
                    clip: Some(row_bounds),
                });
            }
            RowKind::Binary | RowKind::Truncated => {
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD), origin.y),
                    line: self.shape_plain(window, row.text.clone(), p.muted),
                    clip: Some(row_bounds),
                });
            }
            RowKind::Fold => {
                // Collapsed unchanged region: a faint separator band with a
                // muted count, aligned under the code column.
                quads.push(Quad {
                    bounds: row_bounds,
                    color: p.muted.opacity(0.06),
                });
                let text_x = origin.x + px(BAR_W + PAD2) + self.gutter_w;
                glyphs.push(Glyphs {
                    origin: point(text_x, origin.y),
                    line: self.shape_plain(window, row.text.clone(), p.muted),
                    clip: Some(row_bounds),
                });
            }
            RowKind::Context | RowKind::Added | RowKind::Removed => {
                // (row-bg wash, opaque hunk-bar color, word-diff bg). Context = none.
                let (bg, bar_color, word_bg) = match row.kind {
                    RowKind::Added => (Some(p.add_bg), Some(p.add_bar), Some(p.add_word_bg)),
                    RowKind::Removed => (Some(p.del_bg), Some(p.del_bar), Some(p.del_word_bg)),
                    _ => (None, None, None),
                };
                if let Some(bg) = bg {
                    quads.push(Quad {
                        bounds: row_bounds,
                        color: bg,
                    });
                }
                // Zed-style colored hunk-indicator bar at the far left.
                if let Some(c) = bar_color {
                    quads.push(Quad {
                        bounds: Bounds::new(origin, size(px(BAR_W), lh)),
                        color: c,
                    });
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
                // Text — no +/- sign column (the bar + wash convey status).
                let text_x = origin.x + px(BAR_W + PAD2) + self.gutter_w;
                if !row.text.is_empty() {
                    let line = self.shape(window, &row.text, &row.syntax_runs, p.text);
                    if let Some(wbg) = word_bg {
                        self.push_word_quads(
                            &line,
                            &row.word_ranges,
                            text_x,
                            origin.y,
                            lh,
                            wbg,
                            quads,
                        );
                    }
                    glyphs.push(Glyphs {
                        origin: point(text_x, origin.y),
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
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let row_bounds = Bounds::new(origin, size(width, row_h));

        match row {
            SplitRow::Header(text) => {
                quads.push(Quad {
                    bounds: row_bounds,
                    color: p.header_bg,
                });
                quads.push(Quad {
                    bounds: Bounds::new(origin, size(width, px(1.))),
                    color: p.border,
                });
                let ty = origin.y + (row_h - lh) / 2.0;
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD), ty),
                    line: self.shape_plain(
                        window,
                        if collapsed { "▸" } else { "▾" }.into(),
                        p.muted,
                    ),
                    clip: Some(row_bounds),
                });
                // US-010: status-colored sigil, the rest at header text color.
                let runs = self.header_sigil_runs(text);
                glyphs.push(Glyphs {
                    origin: point(origin.x + px(PAD + CHEVRON_W), ty),
                    line: self.shape(window, text, &runs, p.text),
                    clip: Some(row_bounds),
                });
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
                self.layout_half(window, left, point(left_x, origin.y), half_w, quads, glyphs);
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
                    quads,
                    glyphs,
                );
            }
        }
    }

    /// One half-cell of a split row, occupying `[origin.x, origin.x + width)`.
    fn layout_half(
        &self,
        window: &mut Window,
        cell: &HalfCell,
        origin: Point<Pixels>,
        width: Pixels,
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let half_bounds = Bounds::new(origin, size(width, lh));
        let (bg, bar_color, word_bg) = match cell.kind {
            CellKind::Added => (Some(p.add_bg), Some(p.add_bar), Some(p.add_word_bg)),
            CellKind::Removed => (Some(p.del_bg), Some(p.del_bar), Some(p.del_word_bg)),
            CellKind::Phantom => (Some(p.phantom_bg), None, None),
            CellKind::Context => (None, None, None),
        };
        if let Some(bg) = bg {
            quads.push(Quad {
                bounds: half_bounds,
                color: bg,
            });
        }
        if matches!(cell.kind, CellKind::Phantom) {
            return; // dimmed empty gap, no bar/gutter/text
        }
        // Zed-style colored hunk-indicator bar at the half's left edge.
        if let Some(c) = bar_color {
            quads.push(Quad {
                bounds: Bounds::new(origin, size(px(BAR_W), lh)),
                color: c,
            });
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
        if !cell.text.is_empty() {
            let line = self.shape(window, &cell.text, &cell.syntax_runs, p.text);
            if let Some(wbg) = word_bg {
                self.push_word_quads(&line, &cell.word_ranges, text_x, origin.y, lh, wbg, quads);
            }
            glyphs.push(Glyphs {
                origin: point(text_x, origin.y),
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
        text_x: Pixels,
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
            let x0 = text_x + line.x_for_index(start);
            let x1 = text_x + line.x_for_index(end);
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

    /// Emit the pinned sticky file header — an opaque bar at viewport-local
    /// `y_top` carrying the current file's status sigil + path, so the file you
    /// are reading never scrolls out of sight on a long diff. Painted last (over
    /// the body) by `paint`. `header_text` is the same pre-formatted header
    /// string the inline card uses, so the sigil/stat coloring matches.
    #[allow(clippy::too_many_arguments)]
    fn sticky_header(
        &self,
        window: &mut Window,
        header_text: &SharedString,
        x: Pixels,
        y_top: Pixels,
        width: Pixels,
        quads: &mut Vec<Quad>,
        glyphs: &mut Vec<Glyphs>,
    ) {
        let p = &self.palette;
        let lh = self.line_height;
        let h = px(STICKY_HEADER_HEIGHT);
        let bounds = Bounds::new(point(x, y_top), size(width, h));
        quads.push(Quad {
            bounds,
            color: p.sticky_header_bg,
        });
        // Bottom hairline so the pinned bar reads as a distinct, floating layer.
        quads.push(Quad {
            bounds: Bounds::new(point(x, y_top + h - px(1.)), size(width, px(1.))),
            color: p.border,
        });
        let ty = y_top + (h - lh) / 2.0;
        // A pinned file is, by definition, expanded (you are inside its body), so
        // the chevron is always the open glyph.
        glyphs.push(Glyphs {
            origin: point(x + px(PAD), ty),
            line: self.shape_plain(window, "▾".into(), p.muted),
            clip: Some(bounds),
        });
        let runs = self.header_sigil_runs(header_text);
        glyphs.push(Glyphs {
            origin: point(x + px(PAD + CHEVRON_W), ty),
            line: self.shape(window, header_text, &runs, p.text),
            clip: Some(bounds),
        });
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
        // Full content height (sum of variable row heights) — the hosting
        // `overflow_y_scroll` div clips/scrolls.
        let h = px(self.body.offsets().last().copied().unwrap_or(0.0));
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

        // Variable-height layout: cumulative top offsets per row. Cull against
        // them (binary search) instead of a uniform line height.
        let offsets = self.body.offsets();
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
                "diff element: visible {first}..{last} = {} of {row_count} rows — culling off?",
                last - first
            );
        }

        // Derive the gutter width from the body's widest line number so 5-digit
        // line numbers in large files never clip past the gutter's left edge.
        // One shaped digit gives the exact monospace advance.
        let digits = {
            let max_no = self.body.max_line_no().max(1);
            ((max_no as f64).log10().floor() as usize + 1).max(2)
        };
        let digit_w = f32::from(
            self.shape_plain(window, "0".into(), self.palette.muted)
                .width(),
        );
        self.gutter_w = px((GUTTER_PAD_L + digits as f32 * digit_w + NUM_GAP).max(GUTTER_W));

        let mut quads = Vec::new();
        let mut glyphs = Vec::new();
        let mut sticky_quads = Vec::new();
        let mut sticky_glyphs = Vec::new();
        let mut header_hitboxes = Vec::new();
        let width = bounds.size.width;

        // Clone the Rc so the borrow of `self.body` doesn't conflict with the
        // `&mut self` shaping calls below.
        match &self.body {
            DiffBody::Unified(rows) => {
                let rows = rows.clone();
                for i in first..last {
                    let origin = point(bounds.origin.x, bounds.origin.y + px(offsets[i]));
                    let row_h = px(offsets[i + 1] - offsets[i]);
                    // A file is folded when the row after its header is another
                    // header (or EOF) — collapsed files emit a header-only row.
                    let collapsed = rows
                        .get(i + 1)
                        .is_none_or(|r| r.kind == RowKind::FileHeader);
                    if matches!(rows[i].kind, RowKind::FileHeader) {
                        header_hitboxes.push(window.insert_hitbox(
                            Bounds::new(origin, size(width, row_h)),
                            HitboxBehavior::Normal,
                        ));
                    }
                    self.layout_unified(
                        window,
                        &rows[i],
                        origin,
                        width,
                        row_h,
                        collapsed,
                        &mut quads,
                        &mut glyphs,
                    );
                }
                // Pinned sticky header for the file under the viewport top.
                let cur = (0..=first.min(row_count.saturating_sub(1)))
                    .rev()
                    .find(|&i| rows[i].kind == RowKind::FileHeader);
                if let Some(hidx) = cur
                    && offsets[hidx] < vtop
                {
                    // Next file header within the slide-up band → push the sticky
                    // up so the incoming file's inline header displaces it.
                    let mut nh = None;
                    let mut j = hidx + 1;
                    while j < row_count && offsets[j] <= vtop + STICKY_HEADER_HEIGHT {
                        if rows[j].kind == RowKind::FileHeader {
                            nh = Some(j);
                            break;
                        }
                        j += 1;
                    }
                    let mut sticky_y = mask.bounds.origin.y;
                    if let Some(nh) = nh {
                        let nh_abs = bounds.origin.y + px(offsets[nh]);
                        if nh_abs < mask.bounds.origin.y + px(STICKY_HEADER_HEIGHT) {
                            sticky_y = nh_abs - px(STICKY_HEADER_HEIGHT);
                        }
                    }
                    if sticky_y + px(STICKY_HEADER_HEIGHT) > mask.bounds.origin.y {
                        let text = rows[hidx].text.clone();
                        self.sticky_header(
                            window,
                            &text,
                            bounds.origin.x,
                            sticky_y,
                            width,
                            &mut sticky_quads,
                            &mut sticky_glyphs,
                        );
                    }
                }
            }
            DiffBody::Split(rows) => {
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
                    self.layout_split(
                        window,
                        &rows[i],
                        origin,
                        width,
                        row_h,
                        collapsed,
                        &mut quads,
                        &mut glyphs,
                    );
                }
                // Pinned sticky header for the file under the viewport top.
                let cur = (0..=first.min(row_count.saturating_sub(1)))
                    .rev()
                    .find(|&i| matches!(rows[i], SplitRow::Header(_)));
                if let Some(hidx) = cur
                    && offsets[hidx] < vtop
                {
                    let mut nh = None;
                    let mut j = hidx + 1;
                    while j < row_count && offsets[j] <= vtop + STICKY_HEADER_HEIGHT {
                        if matches!(rows[j], SplitRow::Header(_)) {
                            nh = Some(j);
                            break;
                        }
                        j += 1;
                    }
                    let mut sticky_y = mask.bounds.origin.y;
                    if let Some(nh) = nh {
                        let nh_abs = bounds.origin.y + px(offsets[nh]);
                        if nh_abs < mask.bounds.origin.y + px(STICKY_HEADER_HEIGHT) {
                            sticky_y = nh_abs - px(STICKY_HEADER_HEIGHT);
                        }
                    }
                    if sticky_y + px(STICKY_HEADER_HEIGHT) > mask.bounds.origin.y
                        && let SplitRow::Header(text) = &rows[hidx]
                    {
                        let text = text.clone();
                        self.sticky_header(
                            window,
                            &text,
                            bounds.origin.x,
                            sticky_y,
                            width,
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
