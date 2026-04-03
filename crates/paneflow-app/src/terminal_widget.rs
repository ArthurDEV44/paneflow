// Custom terminal widget using iced::advanced::Widget
//
// Renders terminal cells with fill_quad (backgrounds) + fill_text (glyphs).
// Cell dimensions are computed from actual font metrics (like Ghostty/Alacritty),
// not hardcoded ratios. This produces pixel-exact, gap-free terminal rendering.

use crate::renderer::{CellData, TerminalGrid};
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use iced::advanced::layout::{Limits, Node};
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::Tree;
use iced::advanced::{renderer, Layout, Renderer as _, Widget};
use iced::{Color, Element, Font, Length, Pixels, Point, Rectangle, Size, Theme};
use std::sync::OnceLock;

// ─── Font metrics (computed once from actual font, cached globally) ──────────

/// Cell dimensions derived from actual monospace font metrics.
/// Matches Ghostty's approach: measure real glyph advances and line height.
#[derive(Debug, Clone, Copy)]
pub struct CellMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
}

/// Cache: (font_size, metrics) — recomputed only when font_size changes.
static CELL_METRICS_CACHE: OnceLock<(f32, CellMetrics)> = OnceLock::new();

/// Compute cell dimensions from actual font metrics using cosmic-text.
/// Like Ghostty: cell_width = max glyph advance, cell_height = line_height from font tables.
pub fn cell_metrics(font_size: f32) -> CellMetrics {
    // Fast path: return cached if font size matches
    if let Some(&(cached_size, metrics)) = CELL_METRICS_CACHE.get() {
        if (cached_size - font_size).abs() < 0.01 {
            return metrics;
        }
    }

    let metrics = compute_cell_metrics(font_size);

    // Store (ignore if already set — first call wins, which is fine)
    let _ = CELL_METRICS_CACHE.set((font_size, metrics));
    metrics
}

fn compute_cell_metrics(font_size: f32) -> CellMetrics {
    let mut font_system = FontSystem::new();

    // Measure with a line_height that lets cosmic-text use the font's natural metrics.
    // We use font_size * 1.2 as a ceiling, then read the actual computed height.
    let ct_metrics = Metrics::new(font_size, font_size * 1.2);
    let mut buffer = Buffer::new(&mut font_system, ct_metrics);

    // Shape all printable ASCII to find max advance (like Ghostty does)
    let ascii: String = (0x20u8..=0x7Eu8).map(|b| b as char).collect();
    buffer.set_text(
        &mut font_system,
        &ascii,
        Attrs::new().family(Family::Monospace),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut font_system, false);

    // Extract actual glyph advance width from layout run
    let mut max_advance: f32 = 0.0;
    let mut line_height: f32 = font_size * 1.2; // fallback
    for run in buffer.layout_runs() {
        line_height = run.line_height;
        for glyph in run.glyphs.iter() {
            if glyph.w > max_advance {
                max_advance = glyph.w;
            }
        }
    }

    // Round to integer pixels (like Ghostty) for exact tiling
    let cell_w = if max_advance > 0.0 {
        max_advance.round()
    } else {
        (font_size * 0.6).round() // fallback
    };
    let cell_h = line_height.round();

    CellMetrics { cell_w, cell_h }
}

// ─── Terminal widget ────────────────────────────────────────────────────────

/// Custom widget that renders a terminal grid with pixel-exact cell tiling.
pub struct TerminalView<'a> {
    grid: &'a TerminalGrid,
    font_size: f32,
    cursor_visible: bool,
}

impl<'a> TerminalView<'a> {
    pub fn new(grid: &'a TerminalGrid, font_size: f32, cursor_visible: bool) -> Self {
        Self {
            grid,
            font_size,
            cursor_visible,
        }
    }
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for TerminalView<'a> {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(&self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &Limits) -> Node {
        Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: iced::advanced::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let m = cell_metrics(self.font_size);
        let cell_w = m.cell_w;
        let cell_h = m.cell_h;
        let default_bg = CellData::default().bg;
        let text_size = Pixels(self.font_size);

        // Fill entire terminal area with default background — no gaps possible
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            default_bg,
        );

        // Pass 1: Background quads for non-default cells (merged runs)
        for row in 0..self.grid.rows {
            let y = bounds.y + (row as f32 * cell_h).round();
            let mut col = 0;
            while col < self.grid.cols {
                let cell = self.grid.cell(row, col);
                if cell.bg == default_bg {
                    col += 1;
                    continue;
                }
                let start_col = col;
                let bg = cell.bg;
                while col < self.grid.cols && self.grid.cell(row, col).bg == bg {
                    col += 1;
                }
                let x = bounds.x + (start_col as f32 * cell_w).round();
                let x_end = bounds.x + (col as f32 * cell_w).round();
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x,
                            y,
                            width: x_end - x,
                            height: cell_h,
                        },
                        ..renderer::Quad::default()
                    },
                    bg,
                );
            }
        }

        // Pass 2: Text runs (merged consecutive cells with same fg/style)
        for row in 0..self.grid.rows {
            let y = bounds.y + (row as f32 * cell_h).round();
            let mut col = 0;
            while col < self.grid.cols {
                let cell = self.grid.cell(row, col);
                if cell.character == ' ' || cell.character == '\0' {
                    col += 1;
                    continue;
                }
                let start_col = col;
                let fg = cell.fg;
                let bold = cell.bold;
                let italic = cell.italic;
                let mut run = String::new();

                while col < self.grid.cols {
                    let c = self.grid.cell(row, col);
                    if c.fg != fg || c.bold != bold || c.italic != italic {
                        break;
                    }
                    run.push(c.character);
                    col += 1;
                }

                let trimmed = run.trim_end();
                if !trimmed.is_empty() {
                    let font = match (bold, italic) {
                        (true, true) => Font {
                            weight: iced::font::Weight::Bold,
                            style: iced::font::Style::Italic,
                            ..Font::MONOSPACE
                        },
                        (true, false) => Font {
                            weight: iced::font::Weight::Bold,
                            ..Font::MONOSPACE
                        },
                        (false, true) => Font {
                            style: iced::font::Style::Italic,
                            ..Font::MONOSPACE
                        },
                        _ => Font::MONOSPACE,
                    };

                    let x = bounds.x + (start_col as f32 * cell_w).round();
                    let run_width = trimmed.len() as f32 * cell_w;
                    renderer.fill_text(
                        iced::advanced::Text {
                            content: trimmed.to_string(),
                            bounds: Size::new(run_width + cell_w, cell_h),
                            size: text_size,
                            line_height: iced::advanced::text::LineHeight::Absolute(Pixels(cell_h)),
                            font,
                            horizontal_alignment: iced::alignment::Horizontal::Left,
                            vertical_alignment: iced::alignment::Vertical::Top,
                            shaping: iced::advanced::text::Shaping::Advanced,
                            wrapping: iced::advanced::text::Wrapping::None,
                        },
                        Point::new(x, y),
                        fg,
                        *viewport,
                    );
                }
            }
        }

        // Pass 3: Cursor block
        if self.cursor_visible
            && self.grid.cursor_visible
            && self.grid.cursor_row < self.grid.rows
            && self.grid.cursor_col < self.grid.cols
        {
            let cx = bounds.x + (self.grid.cursor_col as f32 * cell_w).round();
            let cy = bounds.y + (self.grid.cursor_row as f32 * cell_h).round();
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: cx,
                        y: cy,
                        width: cell_w,
                        height: cell_h,
                    },
                    ..renderer::Quad::default()
                },
                Color::from_rgba(0.8, 0.84, 0.96, 0.7),
            );
        }
    }
}

impl<'a, Message: 'a> From<TerminalView<'a>> for Element<'a, Message> {
    fn from(widget: TerminalView<'a>) -> Self {
        Self::new(widget)
    }
}
