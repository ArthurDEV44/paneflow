//! Terminal cell renderer using GPUI's Element trait.
//!
//! Renders terminal cells from alacritty_terminal as batched text runs with
//! full ANSI color support, cell attributes, and background quads.

use std::sync::{Arc, LazyLock, Mutex};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::event_loop::{Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, Font, FontFallbacks, FontStyle, FontWeight,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Rgba,
    SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, px,
    relative,
};

use crate::terminal::{SpikeTermSize, ZedListener};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FONT_SIZE: f32 = 14.0;
const FONT_FAMILY: &str = "Noto Sans Mono";

const FONT_FALLBACK_EMOJI: &str = "Noto Color Emoji";
const FONT_FALLBACK_SYMBOLS: &str = "Symbols Nerd Font Mono";
const FONT_FALLBACK_SANS: &str = "Noto Sans";

/// WCAG 2.0 AA minimum contrast ratio for normal text.
const MIN_CONTRAST_RATIO: f32 = 4.5;

static FONT_FALLBACKS: LazyLock<FontFallbacks> = LazyLock::new(|| {
    FontFallbacks::from_fonts(vec![
        FONT_FALLBACK_EMOJI.to_string(),
        FONT_FALLBACK_SYMBOLS.to_string(),
        FONT_FALLBACK_SANS.to_string(),
    ])
});

// ---------------------------------------------------------------------------
// Minimum contrast (WCAG 2.0 luminance ratio)
// ---------------------------------------------------------------------------

/// sRGB relative luminance per WCAG 2.0 §1.4.3.
fn relative_luminance(color: Hsla) -> f32 {
    let rgba = Rgba::from(color);
    let linearize = |c: f32| -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linearize(rgba.r) + 0.7152 * linearize(rgba.g) + 0.0722 * linearize(rgba.b)
}

/// WCAG 2.0 contrast ratio between two relative luminances.
fn contrast_ratio(l1: f32, l2: f32) -> f32 {
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Adjust `fg` lightness so that the WCAG 2.0 contrast ratio against `bg`
/// meets `min_ratio`. Returns `fg` unchanged if contrast is already sufficient.
fn ensure_minimum_contrast(fg: Hsla, bg: Hsla, min_ratio: f32) -> Hsla {
    let bg_lum = relative_luminance(bg);
    let fg_lum = relative_luminance(fg);

    if contrast_ratio(fg_lum, bg_lum) >= min_ratio {
        return fg;
    }

    // Lighten fg when bg is dark, darken when bg is light.
    let lighten = bg_lum < 0.5;
    let mut result = fg;
    let (mut lo, mut hi) = if lighten {
        (result.l, 1.0)
    } else {
        (0.0, result.l)
    };

    for _ in 0..16 {
        let mid = (lo + hi) * 0.5;
        result.l = mid;
        let new_lum = relative_luminance(result);
        if contrast_ratio(new_lum, bg_lum) >= min_ratio {
            // Found sufficient contrast — try to stay closer to original
            if lighten {
                hi = mid;
            } else {
                lo = mid;
            }
        } else if lighten {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    // Snap to the bound guaranteed to pass the contrast threshold.
    result.l = if lighten { hi } else { lo };
    result
}

/// Returns `true` for characters whose colors should be preserved exactly
/// (no contrast adjustment). Covers box-drawing, block elements, geometric
/// shapes, and Powerline separator symbols.
fn is_decorative_character(ch: char) -> bool {
    matches!(
        ch as u32,
        0x2500..=0x257F  // Box Drawing (─ │ ┌ ┐ └ ┘ etc.)
        | 0x2580..=0x259F // Block Elements (▀ ▄ █ ░ ▒ ▓ etc.)
        | 0x25A0..=0x25FF // Geometric Shapes (■ ▶ ● etc.)
        | 0xE0B0..=0xE0D7 // Powerline separators
    )
}

// ---------------------------------------------------------------------------
// Layout types
// ---------------------------------------------------------------------------

pub struct CellDimensions {
    pub cell_width: Pixels,
    pub line_height: Pixels,
}

struct BatchedTextRun {
    text: String,
    font: Font,
    color: Hsla,
    underline: Option<UnderlineStyle>,
    strikethrough: Option<StrikethroughStyle>,
    line: i32,
    col_start: usize,
}

struct LayoutRect {
    line: i32,
    col: usize,
    num_cols: usize,
    color: Hsla,
}

struct CursorInfo {
    line: i32,
    col: usize,
    shape: CursorShape,
    color: Hsla,
    wide: bool,
    /// Character under the cursor (None for whitespace or non-Block shapes).
    text: Option<char>,
    bold: bool,
    italic: bool,
}

pub struct LayoutState {
    batched_runs: Vec<BatchedTextRun>,
    rects: Vec<LayoutRect>,
    selection_rects: Vec<LayoutRect>,
    cursor: Option<CursorInfo>,
    dimensions: CellDimensions,
    background_color: Hsla,
    exited: Option<i32>,
    /// Scroll position for scrollbar indicator (0 = at bottom)
    display_offset: usize,
    /// Total scrollback history size
    history_size: usize,
}

// ---------------------------------------------------------------------------
// Cell style — used for batching comparison
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
struct CellStyle {
    font: Font,
    fg: Hsla,
    bg: Hsla,
    underline: bool,
    undercurl: bool,
    strikethrough: bool,
}

// ---------------------------------------------------------------------------
// TerminalElement
// ---------------------------------------------------------------------------

pub struct TerminalElement {
    term: Arc<FairMutex<Term<ZedListener>>>,
    notifier: Notifier,
    cursor_visible: bool,
    focused: bool,
    exited: Option<i32>,
    /// Shared origin — updated in paint() so mouse handlers know the element position.
    element_origin: Arc<Mutex<Point<Pixels>>>,
    /// Timestamp of the keystroke that triggered this render, for latency measurement.
    #[cfg(debug_assertions)]
    last_keystroke_at: Option<std::time::Instant>,
}

impl TerminalElement {
    pub fn new(
        term: Arc<FairMutex<Term<ZedListener>>>,
        notifier: Notifier,
        cursor_visible: bool,
        focused: bool,
        exited: Option<i32>,
        element_origin: Arc<Mutex<Point<Pixels>>>,
        #[cfg(debug_assertions)] last_keystroke_at: Option<std::time::Instant>,
    ) -> Self {
        Self {
            term,
            notifier,
            cursor_visible,
            focused,
            exited,
            element_origin,
            #[cfg(debug_assertions)]
            last_keystroke_at,
        }
    }

    fn base_font() -> Font {
        Font {
            family: SharedString::from(FONT_FAMILY),
            features: Default::default(),
            fallbacks: Some(FONT_FALLBACKS.clone()),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    fn font_size() -> Pixels {
        px(FONT_SIZE)
    }

    pub fn measure_cell(window: &mut Window, _cx: &mut App) -> CellDimensions {
        let font = Self::base_font();
        let font_size = Self::font_size();
        let font_id = window.text_system().resolve_font(&font);
        let cell_width = window
            .text_system()
            .advance(font_id, font_size, 'm')
            .unwrap()
            .width;
        let line_height = px(FONT_SIZE * 1.4);
        CellDimensions {
            cell_width,
            line_height,
        }
    }

    fn build_layout(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> LayoutState {
        let dims = Self::measure_cell(window, cx);
        let theme = crate::theme::active_theme();
        let background_color = theme.background;
        let default_bg = background_color;

        // Compute desired terminal grid size from pixel bounds (accounting for left gutter)
        let gutter = dims.cell_width;
        let available_width = (bounds.size.width - gutter).max(px(0.0));
        let desired_cols = (available_width / dims.cell_width).floor().max(1.0) as usize;
        let desired_rows = (bounds.size.height / dims.line_height).floor() as usize;

        // Snapshot the grid, cursor, and selection under lock to minimize FairMutex hold time.
        let cursor_color = theme.cursor;
        let selection_color = Hsla {
            h: 0.58,
            s: 0.6,
            l: 0.5,
            a: 0.35,
        }; // Semi-transparent blue highlight

        let (cells, cursor_snapshot, selection_range, display_offset, history_size): (
            Vec<_>,
            Option<CursorInfo>,
            Option<SelectionRange>,
            usize,
            usize,
        ) = {
            let mut term = self.term.lock();
            // Resize the terminal grid if bounds have changed
            let current_cols = term.columns();
            let current_rows = term.screen_lines();
            if desired_cols > 0
                && desired_rows > 0
                && (current_cols != desired_cols || current_rows != desired_rows)
            {
                term.resize(SpikeTermSize {
                    columns: desired_cols,
                    screen_lines: desired_rows,
                });
                // Notify PTY EventLoop to send SIGWINCH to the child process
                let _ = self.notifier.0.send(Msg::Resize(WindowSize {
                    num_cols: desired_cols as u16,
                    num_lines: desired_rows as u16,
                    cell_width: dims.cell_width.as_f32() as u16,
                    cell_height: dims.line_height.as_f32() as u16,
                }));
            }
            let content = term.renderable_content();
            let sel_range = content.selection.as_ref().map(|sel| SelectionRange {
                start: sel.start,
                end: sel.end,
                is_block: sel.is_block,
            });
            let cursor =
                if matches!(content.cursor.shape, CursorShape::Hidden) || !self.cursor_visible {
                    None
                } else {
                    let shape = if !self.focused {
                        CursorShape::HollowBlock
                    } else {
                        content.cursor.shape
                    };
                    let cursor_cell = &term.grid()[content.cursor.point];
                    let wide = cursor_cell.flags.contains(CellFlags::WIDE_CHAR);
                    let cursor_char = cursor_cell.c;
                    let cursor_flags = cursor_cell.flags;
                    let text = if matches!(shape, CursorShape::Block)
                        && cursor_char != ' '
                        && cursor_char != '\0'
                    {
                        Some(cursor_char)
                    } else {
                        None
                    };
                    Some(CursorInfo {
                        line: content.cursor.point.line.0,
                        col: content.cursor.point.column.0,
                        shape,
                        color: cursor_color,
                        wide,
                        text,
                        bold: cursor_flags.contains(CellFlags::BOLD)
                            || cursor_flags.contains(CellFlags::BOLD_ITALIC),
                        italic: cursor_flags.contains(CellFlags::ITALIC)
                            || cursor_flags.contains(CellFlags::BOLD_ITALIC),
                    })
                };
            let disp_offset = content.display_offset;
            let hist_size = term.history_size();
            let cells: Vec<_> = content
                .display_iter
                .map(|ic| {
                    let zw = ic.cell.zerowidth().map(|chars| chars.to_vec());
                    (
                        ic.point,
                        ic.cell.c,
                        ic.cell.fg,
                        ic.cell.bg,
                        ic.cell.flags,
                        zw,
                    )
                })
                .collect();
            (cells, cursor, sel_range, disp_offset, hist_size)
        };

        let mut batch = BatchAccumulator::new();
        let mut rects: Vec<LayoutRect> = Vec::new();
        let mut current_rect: Option<LayoutRect> = None;
        let mut last_line: i32 = i32::MIN;
        let mut previous_cell_had_extras = false;

        for (point, c, cell_fg, cell_bg, flags, zw) in &cells {
            let point = *point;
            let flags = *flags;

            // Skip wide char spacers (trailing cell of CJK chars)
            if flags.contains(CellFlags::WIDE_CHAR_SPACER) {
                continue;
            }

            // Line change → flush batch and rect
            if point.line.0 != last_line {
                batch.flush();
                if let Some(rect) = current_rect.take() {
                    rects.push(rect);
                }
                last_line = point.line.0;
            }

            // Compute colors
            let mut fg = convert_color(*cell_fg, &theme);
            let mut bg = convert_color(*cell_bg, &theme);

            // Handle inverse video
            if flags.contains(CellFlags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }

            // DIM/faint (SGR 2): reduce foreground opacity (applied after INVERSE)
            if flags.contains(CellFlags::DIM) {
                fg.a *= 0.7;
            }

            // Enforce minimum foreground/background contrast (skip decorative chars)
            if !is_decorative_character(*c) {
                fg = ensure_minimum_contrast(fg, bg, MIN_CONTRAST_RATIO);
            }

            // Background rect — only for non-default backgrounds
            let cell_cols = if flags.contains(CellFlags::WIDE_CHAR) {
                2
            } else {
                1
            };
            if bg != default_bg {
                match &mut current_rect {
                    Some(rect)
                        if rect.line == point.line.0
                            && rect.color == bg
                            && rect.col + rect.num_cols == point.column.0 =>
                    {
                        rect.num_cols += cell_cols;
                    }
                    _ => {
                        if let Some(rect) = current_rect.take() {
                            rects.push(rect);
                        }
                        current_rect = Some(LayoutRect {
                            line: point.line.0,
                            col: point.column.0,
                            num_cols: cell_cols,
                            color: bg,
                        });
                    }
                }
            } else if let Some(rect) = current_rect.take() {
                rects.push(rect);
            }

            // Skip space fillers following cells with zero-width extras (emoji sequences)
            let c = *c;
            if c == ' ' && previous_cell_had_extras {
                previous_cell_had_extras = false;
                continue;
            }

            // Track whether this cell has combining/zero-width characters
            let has_extras = matches!(zw, Some(chars) if !chars.is_empty());

            // Skip empty cells for text runs (space or NUL)
            if c == ' ' || c == '\0' {
                previous_cell_had_extras = has_extras;
                batch.flush();
                continue;
            }

            // Build cell style for batching comparison
            let mut font = Self::base_font();
            let is_underline = flags.contains(CellFlags::UNDERLINE)
                || flags.contains(CellFlags::DOUBLE_UNDERLINE)
                || flags.contains(CellFlags::UNDERCURL)
                || flags.contains(CellFlags::DOTTED_UNDERLINE)
                || flags.contains(CellFlags::DASHED_UNDERLINE);
            let is_undercurl = flags.contains(CellFlags::UNDERCURL);
            let is_strikethrough = flags.contains(CellFlags::STRIKEOUT);

            if flags.contains(CellFlags::BOLD) || flags.contains(CellFlags::BOLD_ITALIC) {
                font.weight = FontWeight::BOLD;
            }
            if flags.contains(CellFlags::ITALIC) || flags.contains(CellFlags::BOLD_ITALIC) {
                font.style = FontStyle::Italic;
            }

            let style = CellStyle {
                font: font.clone(),
                fg,
                bg,
                underline: is_underline,
                undercurl: is_undercurl,
                strikethrough: is_strikethrough,
            };

            // Check if we can append to current batch
            if batch.can_append(&style, point.line.0, point.column.0) {
                batch.append(c, cell_cols);
            } else {
                batch.flush();
                batch.start(
                    c,
                    cell_cols,
                    style,
                    font,
                    fg,
                    is_underline,
                    is_undercurl,
                    is_strikethrough,
                    point.line.0,
                    point.column.0,
                );
            }

            // Append zero-width combining characters (diacriticals, ZWJ, variation selectors)
            if let Some(chars) = zw {
                batch.append_zerowidth(chars);
            }
            previous_cell_had_extras = has_extras;
        }

        // Flush remaining
        batch.flush();
        if let Some(rect) = current_rect {
            rects.push(rect);
        }

        // Build selection highlight rects from the SelectionRange
        let mut selection_rects = Vec::new();
        if let Some(sel) = &selection_range {
            let start = sel.start;
            let end = sel.end;
            let num_cols = desired_cols.max(1);

            if start.line == end.line {
                // Single-line selection
                selection_rects.push(LayoutRect {
                    line: start.line.0,
                    col: start.column.0,
                    num_cols: end.column.0.saturating_sub(start.column.0) + 1,
                    color: selection_color,
                });
            } else {
                // Multi-line: first line from start.col to end of line
                selection_rects.push(LayoutRect {
                    line: start.line.0,
                    col: start.column.0,
                    num_cols: num_cols.saturating_sub(start.column.0),
                    color: selection_color,
                });
                // Middle full lines
                let mut line = start.line.0 + 1;
                while line < end.line.0 {
                    selection_rects.push(LayoutRect {
                        line,
                        col: 0,
                        num_cols,
                        color: selection_color,
                    });
                    line += 1;
                }
                // Last line from col 0 to end.col
                selection_rects.push(LayoutRect {
                    line: end.line.0,
                    col: 0,
                    num_cols: end.column.0 + 1,
                    color: selection_color,
                });
            }
        }

        LayoutState {
            batched_runs: batch.runs,
            rects,
            selection_rects,
            cursor: cursor_snapshot,
            dimensions: dims,
            background_color,
            exited: self.exited,
            display_offset,
            history_size,
        }
    }
}

struct BatchAccumulator {
    runs: Vec<BatchedTextRun>,
    text: String,
    style: Option<CellStyle>,
    font: Font,
    fg: Hsla,
    underline: bool,
    undercurl: bool,
    strikethrough: bool,
    line: i32,
    col_start: usize,
    col_end: usize, // next expected column (tracks wide chars correctly)
}

impl BatchAccumulator {
    fn new() -> Self {
        Self {
            runs: Vec::new(),
            text: String::new(),
            style: None,
            font: TerminalElement::base_font(),
            fg: Hsla::default(),
            underline: false,
            undercurl: false,
            strikethrough: false,
            line: 0,
            col_start: 0,
            col_end: 0,
        }
    }

    fn can_append(&self, style: &CellStyle, line: i32, col: usize) -> bool {
        match &self.style {
            Some(cs) => *cs == *style && self.line == line && col == self.col_end,
            None => false,
        }
    }

    fn append(&mut self, c: char, cell_cols: usize) {
        self.text.push(c);
        self.col_end += cell_cols;
    }

    fn append_zerowidth(&mut self, chars: &[char]) {
        debug_assert!(
            !self.text.is_empty(),
            "zero-width chars require a base character"
        );
        for &c in chars {
            self.text.push(c);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        c: char,
        cell_cols: usize,
        style: CellStyle,
        font: Font,
        fg: Hsla,
        underline: bool,
        undercurl: bool,
        strikethrough: bool,
        line: i32,
        col_start: usize,
    ) {
        self.text.push(c);
        self.style = Some(style);
        self.font = font;
        self.fg = fg;
        self.underline = underline;
        self.undercurl = undercurl;
        self.strikethrough = strikethrough;
        self.line = line;
        self.col_start = col_start;
        self.col_end = col_start + cell_cols;
    }

    fn flush(&mut self) {
        if self.text.is_empty() {
            return;
        }
        self.runs.push(BatchedTextRun {
            text: std::mem::take(&mut self.text),
            font: self.font.clone(),
            color: self.fg,
            underline: if self.underline {
                Some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(self.fg),
                    wavy: self.undercurl,
                })
            } else {
                None
            },
            strikethrough: if self.strikethrough {
                Some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(self.fg),
                })
            } else {
                None
            },
            line: self.line,
            col_start: self.col_start,
        });
        self.style = None;
    }
}

// ---------------------------------------------------------------------------
// Element trait implementation
// ---------------------------------------------------------------------------

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<LayoutState>;

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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        Some(self.build_layout(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(debug_assertions)]
        let _paint_start = if crate::terminal::probe_enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        let Some(layout) = prepaint.take() else {
            return;
        };

        let cell_width = layout.dimensions.cell_width;
        // Offset origin by left gutter (1 cell width)
        let origin = Point {
            x: bounds.origin.x + cell_width,
            y: bounds.origin.y,
        };
        // Store gutter-adjusted origin for mouse → grid coordinate conversion
        *self.element_origin.lock().unwrap() = origin;
        let line_height = layout.dimensions.line_height;
        let font_size = Self::font_size();

        // Clip to element bounds
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // 1. Paint full terminal background
            window.paint_quad(fill(bounds, layout.background_color));

            // 2. Paint per-cell background rects (pixel-aligned to prevent gaps)
            for rect in &layout.rects {
                let x = (origin.x + cell_width * rect.col as f32).floor();
                let y = origin.y + line_height * rect.line as f32;
                let w = (cell_width * rect.num_cols as f32).ceil();
                let rect_bounds = Bounds::new(
                    Point { x, y },
                    gpui::Size {
                        width: w,
                        height: line_height,
                    },
                );
                window.paint_quad(fill(rect_bounds, rect.color));
            }

            // 2b. Paint selection highlight rects (pixel-aligned)
            for rect in &layout.selection_rects {
                let x = (origin.x + cell_width * rect.col as f32).floor();
                let y = origin.y + line_height * rect.line as f32;
                let w = (cell_width * rect.num_cols as f32).ceil();
                let rect_bounds = Bounds::new(
                    Point { x, y },
                    gpui::Size {
                        width: w,
                        height: line_height,
                    },
                );
                window.paint_quad(fill(rect_bounds, rect.color));
            }

            // 3. Paint batched text runs
            for run in &layout.batched_runs {
                let x = origin.x + cell_width * run.col_start as f32;
                let y = origin.y + line_height * run.line as f32;
                let text_run = TextRun {
                    len: run.text.len(),
                    font: run.font.clone(),
                    color: run.color,
                    background_color: None,
                    underline: run.underline,
                    strikethrough: run.strikethrough,
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from(run.text.clone()),
                    font_size,
                    &[text_run],
                    Some(cell_width),
                );
                let _ = shaped.paint(
                    Point { x, y },
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            // 4. Paint cursor
            if let Some(cursor) = &layout.cursor {
                let cx_ = origin.x + cell_width * cursor.col as f32;
                let cy = origin.y + line_height * cursor.line as f32;
                let mut cw = if cursor.wide {
                    cell_width * 2.0
                } else {
                    cell_width
                };
                let ch = line_height;
                let color = cursor.color;

                match cursor.shape {
                    CursorShape::Block => {
                        // Shape the cursor character first so we can size the
                        // cursor quad to fit wide/emoji glyphs.
                        let shaped = cursor.text.map(|ch| {
                            let mut cursor_font = Self::base_font();
                            if cursor.bold {
                                cursor_font.weight = FontWeight::BOLD;
                            }
                            if cursor.italic {
                                cursor_font.style = FontStyle::Italic;
                            }
                            let text = ch.to_string();
                            let len = text.len();
                            window.text_system().shape_line(
                                SharedString::from(text),
                                Self::font_size(),
                                &[TextRun {
                                    len,
                                    font: cursor_font,
                                    color: layout.background_color,
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                }],
                                None,
                            )
                        });

                        // Widen cursor to fit glyphs that exceed cell_width * 2
                        if cursor.wide
                            && let Some(ref shaped) = shaped
                        {
                            cw = cw.max(shaped.width());
                        }

                        let cursor_bounds = Bounds::new(
                            Point { x: cx_, y: cy },
                            gpui::Size {
                                width: cw,
                                height: ch,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));

                        // Paint the character on top of the cursor quad
                        if let Some(shaped) = shaped {
                            let _ = shaped.paint(
                                Point { x: cx_, y: cy },
                                line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                    CursorShape::Beam => {
                        let beam_width = px(2.0);
                        let cursor_bounds = Bounds::new(
                            Point { x: cx_, y: cy },
                            gpui::Size {
                                width: beam_width,
                                height: ch,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));
                    }
                    CursorShape::Underline => {
                        let underline_height = px(2.0);
                        let cursor_bounds = Bounds::new(
                            Point {
                                x: cx_,
                                y: cy + ch - underline_height,
                            },
                            gpui::Size {
                                width: cw,
                                height: underline_height,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));
                    }
                    CursorShape::HollowBlock => {
                        let t = px(1.5);
                        // Top edge
                        window.paint_quad(fill(
                            Bounds::new(
                                Point { x: cx_, y: cy },
                                gpui::Size {
                                    width: cw,
                                    height: t,
                                },
                            ),
                            color,
                        ));
                        // Bottom edge
                        window.paint_quad(fill(
                            Bounds::new(
                                Point {
                                    x: cx_,
                                    y: cy + ch - t,
                                },
                                gpui::Size {
                                    width: cw,
                                    height: t,
                                },
                            ),
                            color,
                        ));
                        // Left edge
                        window.paint_quad(fill(
                            Bounds::new(
                                Point { x: cx_, y: cy },
                                gpui::Size {
                                    width: t,
                                    height: ch,
                                },
                            ),
                            color,
                        ));
                        // Right edge
                        window.paint_quad(fill(
                            Bounds::new(
                                Point {
                                    x: cx_ + cw - t,
                                    y: cy,
                                },
                                gpui::Size {
                                    width: t,
                                    height: ch,
                                },
                            ),
                            color,
                        ));
                    }
                    CursorShape::Hidden => {} // Already filtered in build_layout
                }
            }

            // 5. Paint scrollbar indicator (thin overlay, right edge)
            if layout.display_offset > 0 && layout.history_size > 0 {
                let scrollbar_width = px(4.0);
                let visible_rows = (bounds.size.height / line_height).floor() as usize;
                let total_lines = layout.history_size + visible_rows;
                let visible_ratio = visible_rows as f32 / total_lines as f32;
                let thumb_height = (bounds.size.height * visible_ratio).max(px(16.0));
                let scroll_ratio = layout.display_offset as f32 / layout.history_size as f32;
                // display_offset=max → scrolled to top → thumb at top
                let thumb_y = bounds.size.height
                    - thumb_height
                    - (bounds.size.height - thumb_height) * scroll_ratio;
                let scrollbar_color = Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.6,
                    a: 0.4,
                };
                let scrollbar_bounds = Bounds::new(
                    Point {
                        x: origin.x + bounds.size.width - scrollbar_width,
                        y: origin.y + thumb_y,
                    },
                    gpui::Size {
                        width: scrollbar_width,
                        height: thumb_height,
                    },
                );
                window.paint_quad(fill(scrollbar_bounds, scrollbar_color));
            }

            // 6. Paint exit overlay if process has exited
            if let Some(code) = layout.exited {
                let msg = format!("[Process exited with code {code}]");
                let exit_fg = rgb_to_hsla(0x6c, 0x70, 0x86); // Overlay6
                let run = TextRun {
                    len: msg.len(),
                    font: Self::base_font(),
                    color: exit_fg,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from(msg),
                    font_size,
                    &[run],
                    None,
                );
                // Center the message in the terminal bounds
                let text_width = shaped.width();
                let x = origin.x + (bounds.size.width - text_width) * 0.5;
                let y = origin.y + (bounds.size.height - line_height) * 0.5;
                let _ = shaped.paint(
                    Point { x, y },
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });

        #[cfg(debug_assertions)]
        if let Some(paint_start) = _paint_start {
            let paint_elapsed = paint_start.elapsed();
            let paint_ms = paint_elapsed.as_secs_f64() * 1000.0;

            // Phase 2: paint() duration
            if paint_ms > 1.0 {
                log::warn!("[latency] paint: {paint_ms:.2}ms");
            }

            // Phase 3: total keystroke → pixel with per-phase breakdown
            if let Some(keystroke_at) = self.last_keystroke_at {
                let total_elapsed = keystroke_at.elapsed();
                let total_ms = total_elapsed.as_secs_f64() * 1000.0;
                let pty_to_paint_ms = total_ms - paint_ms;
                if total_ms > 8.0 {
                    log::warn!(
                        "[latency] keystroke→pixel: {total_ms:.2}ms \
                         (pty_write→paint_start: {pty_to_paint_ms:.2}ms, \
                         paint: {paint_ms:.2}ms)"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// IntoElement implementation
// ---------------------------------------------------------------------------

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

// ---------------------------------------------------------------------------
// Color conversion
// ---------------------------------------------------------------------------

fn convert_color(color: AnsiColor, theme: &crate::theme::TerminalTheme) -> Hsla {
    match color {
        AnsiColor::Named(name) => named_color(name, theme),
        AnsiColor::Spec(rgb) => rgb_to_hsla(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(i) => indexed_color(i, theme),
    }
}

fn named_color(name: NamedColor, theme: &crate::theme::TerminalTheme) -> Hsla {
    match name {
        NamedColor::Black => theme.black,
        NamedColor::Red => theme.red,
        NamedColor::Green => theme.green,
        NamedColor::Yellow => theme.yellow,
        NamedColor::Blue => theme.blue,
        NamedColor::Magenta => theme.magenta,
        NamedColor::Cyan => theme.cyan,
        NamedColor::White => theme.white,
        NamedColor::BrightBlack => theme.bright_black,
        NamedColor::BrightRed => theme.bright_red,
        NamedColor::BrightGreen => theme.bright_green,
        NamedColor::BrightYellow => theme.bright_yellow,
        NamedColor::BrightBlue => theme.bright_blue,
        NamedColor::BrightMagenta => theme.bright_magenta,
        NamedColor::BrightCyan => theme.bright_cyan,
        NamedColor::BrightWhite => theme.bright_white,
        NamedColor::Foreground | NamedColor::BrightForeground => theme.foreground,
        NamedColor::Background => theme.ansi_background,
        NamedColor::DimBlack => theme.dim_black,
        NamedColor::DimRed => theme.dim_red,
        NamedColor::DimGreen => theme.dim_green,
        NamedColor::DimYellow => theme.dim_yellow,
        NamedColor::DimBlue => theme.dim_blue,
        NamedColor::DimMagenta => theme.dim_magenta,
        NamedColor::DimCyan => theme.dim_cyan,
        NamedColor::DimWhite => theme.dim_white,
        NamedColor::DimForeground => theme.dim_foreground,
        NamedColor::Cursor => theme.cursor,
    }
}

/// Convert the xterm-256color indexed palette to HSLA.
fn indexed_color(i: u8, theme: &crate::theme::TerminalTheme) -> Hsla {
    if i < 16 {
        // Standard 16 colors — map to named
        return named_color(
            match i {
                0 => NamedColor::Black,
                1 => NamedColor::Red,
                2 => NamedColor::Green,
                3 => NamedColor::Yellow,
                4 => NamedColor::Blue,
                5 => NamedColor::Magenta,
                6 => NamedColor::Cyan,
                7 => NamedColor::White,
                8 => NamedColor::BrightBlack,
                9 => NamedColor::BrightRed,
                10 => NamedColor::BrightGreen,
                11 => NamedColor::BrightYellow,
                12 => NamedColor::BrightBlue,
                13 => NamedColor::BrightMagenta,
                14 => NamedColor::BrightCyan,
                15 => NamedColor::BrightWhite,
                _ => unreachable!(),
            },
            theme,
        );
    }

    if i < 232 {
        // 6x6x6 color cube (indices 16-231)
        let idx = i - 16;
        let r_idx = idx / 36;
        let g_idx = (idx % 36) / 6;
        let b_idx = idx % 6;
        let r = if r_idx == 0 { 0 } else { 55 + 40 * r_idx };
        let g = if g_idx == 0 { 0 } else { 55 + 40 * g_idx };
        let b = if b_idx == 0 { 0 } else { 55 + 40 * b_idx };
        return rgb_to_hsla(r, g, b);
    }

    // Grayscale ramp (indices 232-255)
    let gray = 8 + 10 * (i - 232);
    rgb_to_hsla(gray, gray, gray)
}

fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
    Hsla::from(Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    })
}
