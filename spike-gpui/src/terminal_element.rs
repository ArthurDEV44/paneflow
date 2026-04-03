//! Terminal cell renderer using GPUI's Element trait.
//!
//! Renders terminal cells from alacritty_terminal as batched text runs with
//! full ANSI color support, cell attributes, and background quads.

use std::sync::Arc;

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags as CellFlags;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};

use gpui::{
    fill, px, relative, App, Bounds, ContentMask, Element, ElementId, Font, FontStyle, FontWeight,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Rgba,
    SharedString, StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window,
};

use crate::terminal::ZedListener;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FONT_SIZE: f32 = 14.0;
const FONT_FAMILY: &str = "monospace";

// Catppuccin Mocha palette (hardcoded until theme engine in US-013)
const BG_COLOR: Rgba = Rgba {
    r: 0.118,
    g: 0.118,
    b: 0.180,
    a: 1.0,
}; // #1e1e2e
const FG_COLOR: Rgba = Rgba {
    r: 0.804,
    g: 0.839,
    b: 0.957,
    a: 1.0,
}; // #cdd6f4

// ---------------------------------------------------------------------------
// Layout types
// ---------------------------------------------------------------------------

struct CellDimensions {
    cell_width: Pixels,
    line_height: Pixels,
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
}

pub struct LayoutState {
    batched_runs: Vec<BatchedTextRun>,
    rects: Vec<LayoutRect>,
    cursor: Option<CursorInfo>,
    dimensions: CellDimensions,
    background_color: Hsla,
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
    strikethrough: bool,
}

// ---------------------------------------------------------------------------
// TerminalElement
// ---------------------------------------------------------------------------

pub struct TerminalElement {
    term: Arc<FairMutex<Term<ZedListener>>>,
    cursor_visible: bool,
    focused: bool,
}

impl TerminalElement {
    pub fn new(
        term: Arc<FairMutex<Term<ZedListener>>>,
        cursor_visible: bool,
        focused: bool,
    ) -> Self {
        Self {
            term,
            cursor_visible,
            focused,
        }
    }

    fn base_font() -> Font {
        Font {
            family: SharedString::from(FONT_FAMILY),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
        }
    }

    fn font_size() -> Pixels {
        px(FONT_SIZE)
    }

    fn measure_cell(window: &mut Window, _cx: &mut App) -> CellDimensions {
        let font = Self::base_font();
        let font_size = Self::font_size();
        let run = TextRun {
            len: "M".len(),
            font,
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped =
            window
                .text_system()
                .shape_line(SharedString::from("M"), font_size, &[run], None);
        let cell_width = shaped.width();
        // Line height = font_size * 1.4 (standard monospace ratio)
        let line_height = px(FONT_SIZE * 1.4);
        CellDimensions {
            cell_width,
            line_height,
        }
    }

    fn build_layout(
        &self,
        _bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> LayoutState {
        let dims = Self::measure_cell(window, cx);
        let background_color: Hsla = BG_COLOR.into();
        let default_fg: Hsla = FG_COLOR.into();
        let default_bg = background_color;

        // Snapshot the grid and cursor under lock to minimize FairMutex hold time.
        let cursor_color = named_color(NamedColor::Cursor, default_fg, default_bg);
        let (cells, cursor_snapshot): (Vec<_>, Option<CursorInfo>) = {
            let term = self.term.lock();
            let content = term.renderable_content();
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
                    Some(CursorInfo {
                        line: content.cursor.point.line.0,
                        col: content.cursor.point.column.0,
                        shape,
                        color: cursor_color,
                        wide,
                    })
                };
            let cells = content
                .display_iter
                .map(|ic| (ic.point, ic.cell.c, ic.cell.fg, ic.cell.bg, ic.cell.flags))
                .collect();
            (cells, cursor)
        };

        let mut batch = BatchAccumulator::new();
        let mut rects: Vec<LayoutRect> = Vec::new();
        let mut current_rect: Option<LayoutRect> = None;
        let mut last_line: i32 = i32::MIN;

        for (point, c, cell_fg, cell_bg, flags) in &cells {
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
            let mut fg = convert_color(*cell_fg, default_fg, default_bg);
            let mut bg = convert_color(*cell_bg, default_fg, default_bg);

            // Handle inverse video
            if flags.contains(CellFlags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
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

            // Skip empty cells for text runs (space or NUL)
            let c = *c;
            if c == ' ' || c == '\0' {
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
                    is_strikethrough,
                    point.line.0,
                    point.column.0,
                );
            }
        }

        // Flush remaining
        batch.flush();
        if let Some(rect) = current_rect {
            rects.push(rect);
        }

        LayoutState {
            batched_runs: batch.runs,
            rects,
            cursor: cursor_snapshot,
            dimensions: dims,
            background_color,
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

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        c: char,
        cell_cols: usize,
        style: CellStyle,
        font: Font,
        fg: Hsla,
        underline: bool,
        strikethrough: bool,
        line: i32,
        col_start: usize,
    ) {
        self.text.push(c);
        self.style = Some(style);
        self.font = font;
        self.fg = fg;
        self.underline = underline;
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
                    color: None,
                    wavy: false,
                })
            } else {
                None
            },
            strikethrough: if self.strikethrough {
                Some(StrikethroughStyle {
                    thickness: px(1.0),
                    color: None,
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
        let Some(layout) = prepaint.take() else {
            return;
        };

        let origin = bounds.origin;
        let cell_width = layout.dimensions.cell_width;
        let line_height = layout.dimensions.line_height;
        let font_size = Self::font_size();

        // Clip to element bounds
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // 1. Paint full terminal background
            window.paint_quad(fill(bounds, layout.background_color));

            // 2. Paint per-cell background rects
            for rect in &layout.rects {
                let x = origin.x + cell_width * rect.col as f32;
                let y = origin.y + line_height * rect.line as f32;
                let w = cell_width * rect.num_cols as f32;
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
                let cw = if cursor.wide {
                    cell_width * 2.0
                } else {
                    cell_width
                };
                let ch = line_height;
                let color = cursor.color;

                match cursor.shape {
                    CursorShape::Block => {
                        let cursor_bounds = Bounds::new(
                            Point { x: cx_, y: cy },
                            gpui::Size {
                                width: cw,
                                height: ch,
                            },
                        );
                        window.paint_quad(fill(cursor_bounds, color));
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
        });
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

fn convert_color(color: AnsiColor, default_fg: Hsla, default_bg: Hsla) -> Hsla {
    match color {
        AnsiColor::Named(name) => named_color(name, default_fg, default_bg),
        AnsiColor::Spec(rgb) => rgb_to_hsla(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(i) => indexed_color(i, default_fg, default_bg),
    }
}

fn named_color(name: NamedColor, default_fg: Hsla, default_bg: Hsla) -> Hsla {
    // Catppuccin Mocha palette.
    // Note: bright variants intentionally match normal variants — this is how
    // Catppuccin defines them. The theme engine (US-013) will allow distinct values.
    match name {
        NamedColor::Black => rgb_to_hsla(0x45, 0x47, 0x5a),
        NamedColor::Red => rgb_to_hsla(0xf3, 0x8b, 0xa8),
        NamedColor::Green => rgb_to_hsla(0xa6, 0xe3, 0xa1),
        NamedColor::Yellow => rgb_to_hsla(0xf9, 0xe2, 0xaf),
        NamedColor::Blue => rgb_to_hsla(0x89, 0xb4, 0xfa),
        NamedColor::Magenta => rgb_to_hsla(0xf5, 0xc2, 0xe7),
        NamedColor::Cyan => rgb_to_hsla(0x94, 0xe2, 0xd5),
        NamedColor::White => rgb_to_hsla(0xba, 0xc2, 0xde),
        NamedColor::BrightBlack => rgb_to_hsla(0x58, 0x5b, 0x70),
        NamedColor::BrightRed => rgb_to_hsla(0xf3, 0x8b, 0xa8),
        NamedColor::BrightGreen => rgb_to_hsla(0xa6, 0xe3, 0xa1),
        NamedColor::BrightYellow => rgb_to_hsla(0xf9, 0xe2, 0xaf),
        NamedColor::BrightBlue => rgb_to_hsla(0x89, 0xb4, 0xfa),
        NamedColor::BrightMagenta => rgb_to_hsla(0xf5, 0xc2, 0xe7),
        NamedColor::BrightCyan => rgb_to_hsla(0x94, 0xe2, 0xd5),
        NamedColor::BrightWhite => rgb_to_hsla(0xa6, 0xad, 0xc8),
        NamedColor::Foreground | NamedColor::BrightForeground => default_fg,
        NamedColor::Background => default_bg,
        NamedColor::DimBlack => rgb_to_hsla(0x33, 0x35, 0x44),
        NamedColor::DimRed => rgb_to_hsla(0xb3, 0x67, 0x7e),
        NamedColor::DimGreen => rgb_to_hsla(0x7b, 0xaa, 0x78),
        NamedColor::DimYellow => rgb_to_hsla(0xb9, 0xa8, 0x82),
        NamedColor::DimBlue => rgb_to_hsla(0x66, 0x87, 0xba),
        NamedColor::DimMagenta => rgb_to_hsla(0xb5, 0x90, 0xab),
        NamedColor::DimCyan => rgb_to_hsla(0x6e, 0xa9, 0x9e),
        NamedColor::DimWhite => rgb_to_hsla(0x8b, 0x91, 0xa6),
        NamedColor::DimForeground => rgb_to_hsla(0x8b, 0x91, 0xa6),
        NamedColor::Cursor => rgb_to_hsla(0xf5, 0xe0, 0xdc),
    }
}

/// Convert the xterm-256color indexed palette to HSLA.
fn indexed_color(i: u8, default_fg: Hsla, default_bg: Hsla) -> Hsla {
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
            default_fg,
            default_bg,
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
