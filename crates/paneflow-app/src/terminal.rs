// US-005: alacritty_terminal Grid Integration
//
// Wraps alacritty_terminal::Term<T> to provide cell-level access for the
// GPU renderer. The renderer iterates renderable_content() to extract
// visible cells with their attributes (fg/bg color, flags, character).

use crate::renderer::{self, CellData, TerminalGrid};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor, Rgb};
use iced::Color;
use std::sync::mpsc;

// ─── Event listener ──────────────────────────────────────────────────────────

/// Events from the terminal emulator (bell, title change, etc.)
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used in US-017 (bell/notification system)
pub enum TermEvent {
    Bell,
    Title(String),
    Wakeup,
}

/// Listener that forwards alacritty_terminal events to a channel.
pub struct EventProxy {
    tx: mpsc::Sender<TermEvent>,
}

impl EventProxy {
    pub fn new(tx: mpsc::Sender<TermEvent>) -> Self {
        Self { tx }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let mapped = match event {
            Event::Bell => Some(TermEvent::Bell),
            Event::Title(t) => Some(TermEvent::Title(t)),
            Event::Wakeup => Some(TermEvent::Wakeup),
            _ => None,
        };
        if let Some(e) = mapped {
            let _ = self.tx.send(e);
        }
    }
}

// ─── Terminal state ──────────────────────────────────────────────────────────

/// Owns an alacritty_terminal Term and provides methods to feed bytes
/// and extract renderable cell data for the GPU renderer.
#[allow(dead_code)] // event_rx used in US-017 (bell/notification)
pub struct TerminalState {
    term: Term<EventProxy>,
    processor: Processor,
    event_rx: mpsc::Receiver<TermEvent>,
}

impl TerminalState {
    pub fn new(cols: u16, rows: u16) -> Self {
        let (tx, rx) = mpsc::channel();
        let size = TermSize::new(cols as usize, rows as usize);
        let config = TermConfig {
            scrolling_history: 4000,
            ..Default::default()
        };
        let term = Term::new(config, &size, EventProxy::new(tx));
        let processor = Processor::new();

        Self {
            term,
            processor,
            event_rx: rx,
        }
    }

    /// Feed raw PTY output bytes into the terminal emulator.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    /// Resize the terminal grid.
    #[allow(dead_code)] // Used when pane resize is implemented
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = TermSize::new(cols as usize, rows as usize);
        self.term.resize(size);
    }

    /// Drain pending terminal events (bell, title changes, etc.)
    #[allow(dead_code)] // Used in US-017 (notification system)
    pub fn drain_events(&self) -> Vec<TermEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Extract the current terminal grid state for the renderer.
    /// Iterates alacritty_terminal's renderable_content() and converts
    /// to our CellData format with resolved colors.
    pub fn to_grid(&self) -> TerminalGrid {
        let cols = self.term.columns();
        let rows = self.term.screen_lines();
        let mut grid = TerminalGrid::new(cols, rows);

        let content = self.term.renderable_content();

        // Extract cursor position
        grid.cursor_row = content.cursor.point.line.0 as usize;
        grid.cursor_col = content.cursor.point.column.0;
        grid.cursor_visible = true;

        // Resolve colors from the terminal's color palette
        let colors = content.colors;

        // Extract cells from the grid iterator
        for indexed in content.display_iter {
            let row = indexed.point.line.0 as usize;
            let col = indexed.point.column.0;

            if row >= rows || col >= cols {
                continue;
            }

            let cell = &indexed.cell;
            let fg = resolve_color(cell.fg, colors);
            let bg = resolve_color(cell.bg, colors);

            *grid.cell_mut(row, col) = CellData {
                character: cell.c,
                fg,
                bg,
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.contains(Flags::UNDERLINE),
                strikethrough: cell.flags.contains(Flags::STRIKEOUT),
            };
        }

        grid
    }

    #[allow(dead_code)]
    pub fn cols(&self) -> usize {
        self.term.columns()
    }

    #[allow(dead_code)]
    pub fn rows(&self) -> usize {
        self.term.screen_lines()
    }
}

// ─── Color resolution ────────────────────────────────────────────────────────

/// Resolve an alacritty_terminal Color to an iced Color using the
/// terminal's color palette.
fn resolve_color(color: AnsiColor, colors: &alacritty_terminal::term::color::Colors) -> Color {
    match color {
        AnsiColor::Named(named) => {
            // Look up named color in the terminal's palette
            if let Some(rgb) = colors[named] {
                rgb_to_color(rgb)
            } else {
                named_color_fallback(named)
            }
        }
        AnsiColor::Spec(rgb) => rgb_to_color(rgb),
        AnsiColor::Indexed(idx) => {
            // Look up 256-color index in the palette
            renderer::color_from_256(idx)
        }
    }
}

fn rgb_to_color(rgb: Rgb) -> Color {
    Color::from_rgb(
        rgb.r as f32 / 255.0,
        rgb.g as f32 / 255.0,
        rgb.b as f32 / 255.0,
    )
}

/// Fallback colors for named ANSI colors when the palette doesn't have them.
fn named_color_fallback(named: NamedColor) -> Color {
    renderer::ANSI_COLORS[match named {
        NamedColor::Black => 0,
        NamedColor::Red => 1,
        NamedColor::Green => 2,
        NamedColor::Yellow => 3,
        NamedColor::Blue => 4,
        NamedColor::Magenta => 5,
        NamedColor::Cyan => 6,
        NamedColor::White => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        // Foreground/background/cursor use default text colors
        NamedColor::Foreground | NamedColor::Cursor => 15,
        NamedColor::Background => 0,
        _ => 7,
    }]
}
