// US-005: VT emulation with alacritty_terminal

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

// ---------------------------------------------------------------------------
// Public screen-content types
// ---------------------------------------------------------------------------

/// Color representation for a terminal cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellColor {
    /// The terminal's default foreground or background.
    Default,
    /// An indexed color (0-255 from the 256-color palette).
    Indexed(u8),
    /// A true-color RGB value.
    Rgb(u8, u8, u8),
}

/// A single cell in the terminal grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCell {
    pub character: char,
    pub fg_color: CellColor,
    pub bg_color: CellColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// A single row in the terminal grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRow {
    pub cells: Vec<TerminalCell>,
}

/// Snapshot of the visible terminal screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalScreen {
    pub rows: Vec<TerminalRow>,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

// ---------------------------------------------------------------------------
// Color conversion
// ---------------------------------------------------------------------------

fn convert_color(color: Color) -> CellColor {
    match color {
        Color::Named(NamedColor::Foreground | NamedColor::Background) => CellColor::Default,
        Color::Named(named) => CellColor::Indexed(named as u8),
        Color::Indexed(idx) => CellColor::Indexed(idx),
        Color::Spec(rgb) => CellColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

// ---------------------------------------------------------------------------
// TerminalEmulator
// ---------------------------------------------------------------------------

/// Wraps `alacritty_terminal::Term` with a VT parser to provide a
/// self-contained terminal emulator that can process raw PTY bytes and
/// expose structured screen content.
pub struct TerminalEmulator {
    term: Term<VoidListener>,
    processor: Processor,
}

impl TerminalEmulator {
    /// Create a new terminal emulator with the given visible dimensions.
    pub fn new(rows: u16, cols: u16) -> Self {
        let size = TermSize::new(cols as usize, rows as usize);
        let term = Term::new(TermConfig::default(), &size, VoidListener);
        let processor = Processor::new();

        Self { term, processor }
    }

    /// Feed raw PTY output bytes through the VT parser into the terminal.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    /// Resize the terminal grid to new dimensions.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let size = TermSize::new(cols as usize, rows as usize);
        self.term.resize(size);
    }

    /// Return a snapshot of the current visible screen content.
    pub fn screen_content(&self) -> TerminalScreen {
        let grid = self.term.grid();
        let num_lines = grid.screen_lines();
        let num_cols = grid.columns();

        let mut rows = Vec::with_capacity(num_lines);

        for line_idx in 0..num_lines {
            let line = Line(line_idx as i32);
            let row = &grid[line];
            let mut cells = Vec::with_capacity(num_cols);

            for col_idx in 0..num_cols {
                let cell = &row[Column(col_idx)];
                cells.push(TerminalCell {
                    character: cell.c,
                    fg_color: convert_color(cell.fg),
                    bg_color: convert_color(cell.bg),
                    bold: cell.flags.contains(Flags::BOLD),
                    italic: cell.flags.contains(Flags::ITALIC),
                    underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                });
            }

            rows.push(TerminalRow { cells });
        }

        let cursor = grid.cursor.point;
        TerminalScreen {
            rows,
            cursor_row: cursor.line.0 as usize,
            cursor_col: cursor.column.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_emulator() {
        let emu = TerminalEmulator::new(24, 80);
        let screen = emu.screen_content();

        assert_eq!(screen.rows.len(), 24);
        assert_eq!(screen.rows[0].cells.len(), 80);
        assert_eq!(screen.cursor_row, 0);
        assert_eq!(screen.cursor_col, 0);

        // All cells should be space with default colors.
        for row in &screen.rows {
            for cell in &row.cells {
                assert_eq!(cell.character, ' ');
                assert_eq!(cell.fg_color, CellColor::Default);
                assert_eq!(cell.bg_color, CellColor::Default);
                assert!(!cell.bold);
                assert!(!cell.italic);
                assert!(!cell.underline);
            }
        }
    }

    #[test]
    fn process_plain_text() {
        let mut emu = TerminalEmulator::new(24, 80);
        emu.process_bytes(b"Hello, world!");

        let screen = emu.screen_content();

        let text: String = screen.rows[0]
            .cells
            .iter()
            .map(|c| c.character)
            .collect::<String>();
        assert!(text.starts_with("Hello, world!"));

        // Cursor should be right after the last character written.
        assert_eq!(screen.cursor_row, 0);
        assert_eq!(screen.cursor_col, 13);
    }

    #[test]
    fn process_ansi_colors() {
        let mut emu = TerminalEmulator::new(24, 80);

        // ESC[31m = set foreground to red (NamedColor::Red = index 1)
        // ESC[42m = set background to green (NamedColor::Green = index 2)
        emu.process_bytes(b"\x1b[31;42mX");

        let screen = emu.screen_content();
        let cell = &screen.rows[0].cells[0];

        assert_eq!(cell.character, 'X');
        assert_eq!(cell.fg_color, CellColor::Indexed(NamedColor::Red as u8));
        assert_eq!(cell.bg_color, CellColor::Indexed(NamedColor::Green as u8));
    }

    #[test]
    fn process_256_color() {
        let mut emu = TerminalEmulator::new(24, 80);

        // ESC[38;5;200m = set foreground to palette index 200
        emu.process_bytes(b"\x1b[38;5;200mA");

        let screen = emu.screen_content();
        let cell = &screen.rows[0].cells[0];
        assert_eq!(cell.character, 'A');
        assert_eq!(cell.fg_color, CellColor::Indexed(200));
    }

    #[test]
    fn process_rgb_color() {
        let mut emu = TerminalEmulator::new(24, 80);

        // ESC[38;2;100;150;200m = set foreground to RGB(100, 150, 200)
        emu.process_bytes(b"\x1b[38;2;100;150;200mR");

        let screen = emu.screen_content();
        let cell = &screen.rows[0].cells[0];
        assert_eq!(cell.character, 'R');
        assert_eq!(cell.fg_color, CellColor::Rgb(100, 150, 200));
    }

    #[test]
    fn process_bold_italic_underline() {
        let mut emu = TerminalEmulator::new(24, 80);

        // ESC[1m = bold, ESC[3m = italic, ESC[4m = underline
        emu.process_bytes(b"\x1b[1;3;4mB");

        let screen = emu.screen_content();
        let cell = &screen.rows[0].cells[0];
        assert_eq!(cell.character, 'B');
        assert!(cell.bold);
        assert!(cell.italic);
        assert!(cell.underline);
    }

    #[test]
    fn cursor_movement() {
        let mut emu = TerminalEmulator::new(24, 80);

        // Move cursor to row 5, col 10 (1-indexed in ANSI: ESC[6;11H)
        emu.process_bytes(b"\x1b[6;11H");

        let screen = emu.screen_content();
        assert_eq!(screen.cursor_row, 5);
        assert_eq!(screen.cursor_col, 10);
    }

    #[test]
    fn cursor_movement_home() {
        let mut emu = TerminalEmulator::new(24, 80);

        // Write some text to move cursor, then move cursor home.
        emu.process_bytes(b"Hello\x1b[H");

        let screen = emu.screen_content();
        assert_eq!(screen.cursor_row, 0);
        assert_eq!(screen.cursor_col, 0);
    }

    #[test]
    fn resize_terminal() {
        let mut emu = TerminalEmulator::new(24, 80);

        // Write something first.
        emu.process_bytes(b"Hello");

        // Resize to smaller dimensions.
        emu.resize(10, 40);

        let screen = emu.screen_content();
        assert_eq!(screen.rows.len(), 10);
        assert_eq!(screen.rows[0].cells.len(), 40);
    }

    #[test]
    fn resize_larger() {
        let mut emu = TerminalEmulator::new(10, 40);

        emu.process_bytes(b"Test");

        emu.resize(24, 80);

        let screen = emu.screen_content();
        assert_eq!(screen.rows.len(), 24);
        assert_eq!(screen.rows[0].cells.len(), 80);

        // The text should still be present.
        let text: String = screen.rows[0]
            .cells
            .iter()
            .take(4)
            .map(|c| c.character)
            .collect();
        assert_eq!(text, "Test");
    }

    #[test]
    fn newline_moves_cursor_down() {
        let mut emu = TerminalEmulator::new(24, 80);

        // \r\n moves to beginning of next line.
        emu.process_bytes(b"Line1\r\nLine2");

        let screen = emu.screen_content();

        let line1: String = screen.rows[0]
            .cells
            .iter()
            .take(5)
            .map(|c| c.character)
            .collect();
        assert_eq!(line1, "Line1");

        let line2: String = screen.rows[1]
            .cells
            .iter()
            .take(5)
            .map(|c| c.character)
            .collect();
        assert_eq!(line2, "Line2");

        assert_eq!(screen.cursor_row, 1);
        assert_eq!(screen.cursor_col, 5);
    }

    #[test]
    fn reset_attributes_after_sgr0() {
        let mut emu = TerminalEmulator::new(24, 80);

        // Set bold+red, write A, reset, write B.
        emu.process_bytes(b"\x1b[1;31mA\x1b[0mB");

        let screen = emu.screen_content();

        let cell_a = &screen.rows[0].cells[0];
        assert_eq!(cell_a.character, 'A');
        assert!(cell_a.bold);
        assert_eq!(cell_a.fg_color, CellColor::Indexed(NamedColor::Red as u8));

        let cell_b = &screen.rows[0].cells[1];
        assert_eq!(cell_b.character, 'B');
        assert!(!cell_b.bold);
        assert_eq!(cell_b.fg_color, CellColor::Default);
    }

    #[test]
    fn erase_display_clears_screen() {
        let mut emu = TerminalEmulator::new(24, 80);

        emu.process_bytes(b"Hello, world!");
        // ESC[2J = erase entire display, ESC[H = cursor home
        emu.process_bytes(b"\x1b[2J\x1b[H");

        let screen = emu.screen_content();

        // All cells on first row should be spaces.
        for cell in &screen.rows[0].cells {
            assert_eq!(cell.character, ' ');
        }

        assert_eq!(screen.cursor_row, 0);
        assert_eq!(screen.cursor_col, 0);
    }
}
