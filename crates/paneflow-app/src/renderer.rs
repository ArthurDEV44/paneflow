// US-004: GPU-accelerated terminal cell renderer
//
// Uses iced's Canvas widget backed by the WGPU renderer.
// Each terminal pane is rendered as a grid of colored rectangles (backgrounds)
// with colored text (glyphs) on top. This leverages iced's built-in cosmic-text
// glyph atlas and WGPU pipeline for GPU-accelerated rendering.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry};
use iced::{Color, Font, Pixels, Point, Rectangle, Size, Theme};

// ─── Cell data ───────────────────────────────────────────────────────────────

/// A single terminal cell with character, colors, and attributes.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // underline/strikethrough used when full attribute rendering is enabled
pub struct CellData {
    pub character: char,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for CellData {
    fn default() -> Self {
        Self {
            character: ' ',
            fg: Color::from_rgb(0.8, 0.84, 0.96), // Catppuccin text
            bg: Color::from_rgb(0.12, 0.12, 0.18), // Catppuccin base
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
        }
    }
}

// ─── Terminal grid ───────────────────────────────────────────────────────────

/// The renderable state of a terminal pane.
#[derive(Debug, Clone)]
pub struct TerminalGrid {
    pub cells: Vec<CellData>,
    pub cols: usize,
    pub rows: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub cursor_visible: bool,
}

impl TerminalGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cells: vec![CellData::default(); cols * rows],
            cols,
            rows,
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
        }
    }

    pub fn cell(&self, row: usize, col: usize) -> &CellData {
        &self.cells[row * self.cols + col]
    }

    pub fn cell_mut(&mut self, row: usize, col: usize) -> &mut CellData {
        &mut self.cells[row * self.cols + col]
    }
}

// ─── Canvas program ──────────────────────────────────────────────────────────

/// iced Canvas program that renders a terminal grid.
/// Uses instanced colored-quad drawing for backgrounds and fill_text for glyphs,
/// both GPU-accelerated through iced's WGPU renderer.
#[allow(dead_code)] // focused used when cursor style varies by focus state
pub struct TerminalCanvas {
    pub grid: TerminalGrid,
    pub font_size: f32,
    pub focused: bool,
}

impl<Message> canvas::Program<Message> for TerminalCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let cell_width = bounds.width / self.grid.cols as f32;
        let cell_height = bounds.height / self.grid.rows as f32;
        let mono_font = Font::MONOSPACE;
        let text_size = Pixels(self.font_size.min(cell_height * 0.85));
        let default_bg = CellData::default().bg;

        let mut frame = Frame::new(renderer, bounds.size());

        // Pass 1: Background runs — merge consecutive cells with same bg color
        for row in 0..self.grid.rows {
            let y = row as f32 * cell_height;
            let mut col = 0;
            while col < self.grid.cols {
                let cell = self.grid.cell(row, col);
                if cell.bg == default_bg {
                    col += 1;
                    continue;
                }
                // Merge consecutive cells with same bg
                let start_col = col;
                let bg = cell.bg;
                while col < self.grid.cols && self.grid.cell(row, col).bg == bg {
                    col += 1;
                }
                let width = (col - start_col) as f32 * cell_width;
                frame.fill_rectangle(
                    Point::new(start_col as f32 * cell_width, y),
                    Size::new(width, cell_height),
                    bg,
                );
            }
        }

        // Pass 2: Text runs — batch consecutive chars with same fg/bold/italic
        // into single fill_text calls. Reduces ~1920 calls to ~50-100.
        for row in 0..self.grid.rows {
            let y = row as f32 * cell_height;
            let mut col = 0;
            while col < self.grid.cols {
                let cell = self.grid.cell(row, col);
                // Skip empty cells
                if cell.character == ' ' || cell.character == '\0' {
                    col += 1;
                    continue;
                }
                // Start a text run: same fg color, bold, italic
                let start_col = col;
                let fg = cell.fg;
                let bold = cell.bold;
                let italic = cell.italic;
                let mut run = String::new();

                while col < self.grid.cols {
                    let c = self.grid.cell(row, col);
                    // Break run on color/style change or end of visible text
                    if c.fg != fg || c.bold != bold || c.italic != italic {
                        break;
                    }
                    run.push(c.character);
                    col += 1;
                }

                // Trim trailing spaces from this run
                let trimmed = run.trim_end();
                if !trimmed.is_empty() {
                    let font = match (bold, italic) {
                        (true, true) => Font {
                            weight: iced::font::Weight::Bold,
                            style: iced::font::Style::Italic,
                            ..mono_font
                        },
                        (true, false) => Font {
                            weight: iced::font::Weight::Bold,
                            ..mono_font
                        },
                        (false, true) => Font {
                            style: iced::font::Style::Italic,
                            ..mono_font
                        },
                        _ => mono_font,
                    };

                    frame.fill_text(canvas::Text {
                        content: trimmed.to_string(),
                        position: Point::new(start_col as f32 * cell_width, y),
                        color: fg,
                        size: text_size,
                        font,
                        ..Default::default()
                    });
                }
            }
        }

        // Pass 3: Cursor
        if self.grid.cursor_visible
            && self.grid.cursor_row < self.grid.rows
            && self.grid.cursor_col < self.grid.cols
        {
            let cx = self.grid.cursor_col as f32 * cell_width;
            let cy = self.grid.cursor_row as f32 * cell_height;
            let cursor_color = Color::from_rgba(0.8, 0.84, 0.96, 0.7);

            frame.fill_rectangle(
                Point::new(cx, cy),
                Size::new(cell_width, cell_height),
                cursor_color,
            );
        }

        vec![frame.into_geometry()]
    }
}

// ─── ANSI color conversion (used in US-005) ─────────────────────────────────

/// Standard 16 ANSI colors (Catppuccin Mocha).
#[allow(dead_code)]
pub const ANSI_COLORS: [Color; 16] = [
    Color::from_rgb(0.271, 0.278, 0.353), // 0 black
    Color::from_rgb(0.953, 0.545, 0.659), // 1 red
    Color::from_rgb(0.651, 0.890, 0.631), // 2 green
    Color::from_rgb(0.976, 0.886, 0.686), // 3 yellow
    Color::from_rgb(0.537, 0.706, 0.980), // 4 blue
    Color::from_rgb(0.961, 0.761, 0.906), // 5 magenta
    Color::from_rgb(0.580, 0.886, 0.835), // 6 cyan
    Color::from_rgb(0.729, 0.761, 0.871), // 7 white
    Color::from_rgb(0.345, 0.357, 0.439), // 8 bright black
    Color::from_rgb(0.953, 0.545, 0.659), // 9 bright red
    Color::from_rgb(0.651, 0.890, 0.631), // 10 bright green
    Color::from_rgb(0.976, 0.886, 0.686), // 11 bright yellow
    Color::from_rgb(0.537, 0.706, 0.980), // 12 bright blue
    Color::from_rgb(0.961, 0.761, 0.906), // 13 bright magenta
    Color::from_rgb(0.580, 0.886, 0.835), // 14 bright cyan
    Color::from_rgb(0.651, 0.678, 0.784), // 15 bright white
];

/// Convert a 256-color index to an iced Color.
#[allow(dead_code)]
pub fn color_from_256(idx: u8) -> Color {
    if idx < 16 {
        ANSI_COLORS[idx as usize]
    } else if idx < 232 {
        // 216-color cube: 6x6x6
        let idx = idx - 16;
        let r = (idx / 36) as f32 / 5.0;
        let g = ((idx % 36) / 6) as f32 / 5.0;
        let b = (idx % 6) as f32 / 5.0;
        Color::from_rgb(r, g, b)
    } else {
        // Grayscale: 24 shades
        let v = ((idx - 232) as f32 * 10.0 + 8.0) / 255.0;
        Color::from_rgb(v, v, v)
    }
}

/// Parse a 6-digit hex color string to an iced Color.
#[allow(dead_code)]
pub fn color_from_hex(hex: &str) -> Option<Color> {
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    Some(Color::from_rgb(r, g, b))
}
