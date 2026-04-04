//! Terminal state and view — PTY management and GPUI view wrapper.
//!
//! Manages the alacritty_terminal Term, PTY EventLoop, and periodic sync.
//! The TerminalView creates a TerminalElement for cell-by-cell rendering.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as AlacEvent, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop as AlacEventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::tty;
use alacritty_terminal::Term;

use gpui::{
    div, prelude::*, App, ClipboardItem, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Render, Styled,
    Window,
};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

use crate::terminal_element::TerminalElement;

// ---------------------------------------------------------------------------
// Debug latency probes — zero overhead in release builds
// ---------------------------------------------------------------------------

/// Check once whether PANEFLOW_LATENCY_PROBE=1 is set.
/// Cached in a OnceLock so the env var is read only on first call.
#[cfg(debug_assertions)]
pub(crate) fn probe_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("PANEFLOW_LATENCY_PROBE").as_deref() == Ok("1"))
}

// ---------------------------------------------------------------------------
// Dimensions adapter
// ---------------------------------------------------------------------------

pub struct SpikeTermSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl Dimensions for SpikeTermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

// ---------------------------------------------------------------------------
// Event listener — receives events from alacritty's event loop
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ZedListener(UnboundedSender<AlacEvent>);

impl EventListener for ZedListener {
    fn send_event(&self, event: AlacEvent) {
        let _ = self.0.unbounded_send(event);
    }
}

// ---------------------------------------------------------------------------
// Terminal state
// ---------------------------------------------------------------------------

pub struct TerminalState {
    pub term: Arc<FairMutex<Term<ZedListener>>>,
    pub notifier: Notifier,
    events_rx: UnboundedReceiver<AlacEvent>,
    pub exited: Option<i32>,
    /// Set when PTY output has been processed (Wakeup event received).
    /// Cleared after cx.notify() triggers a repaint.
    pub dirty: bool,
    /// Timestamp of the most recent keystroke, used by latency probes
    /// to measure total keystroke-to-pixel time. Debug builds only.
    /// Note: on rapid keystrokes before a render frame, earlier timestamps are overwritten.
    #[cfg(debug_assertions)]
    pub(crate) last_keystroke_at: Option<std::time::Instant>,
}

impl TerminalState {
    pub fn new() -> anyhow::Result<Self> {
        let (events_tx, events_rx) = unbounded();
        let listener = ZedListener(events_tx.clone());

        let cols: usize = 80;
        let rows: usize = 24;

        let config = TermConfig::default();
        let dimensions = SpikeTermSize {
            columns: cols,
            screen_lines: rows,
        };

        let term = Term::new(config, &dimensions, listener.clone());
        let term = Arc::new(FairMutex::new(term));

        // Create PTY
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(shell, vec![])),
            working_directory: Some(std::env::current_dir().unwrap_or_else(|_| "/".into())),
            drain_on_exit: false,
            env: std::collections::HashMap::new(),
        };

        let window_size = WindowSize {
            num_cols: cols as u16,
            num_lines: rows as u16,
            cell_width: 8,
            cell_height: 16,
        };

        let pty = tty::new(&pty_config, window_size, 0)?;

        let event_loop = AlacEventLoop::new(
            term.clone(),
            ZedListener(events_tx),
            pty,
            false,
            false,
        )?;

        let pty_tx = event_loop.channel();
        let _io_thread = event_loop.spawn();

        Ok(Self {
            term,
            notifier: Notifier(pty_tx),
            events_rx,
            exited: None,
            dirty: true, // Force initial render
            #[cfg(debug_assertions)]
            last_keystroke_at: None,
        })
    }

    /// Drain alacritty events. Sets `dirty = true` when PTY output was processed.
    pub fn sync(&mut self) {
        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                AlacEvent::Wakeup => {
                    self.dirty = true;
                }
                AlacEvent::ChildExit(status) => {
                    self.exited = Some(status);
                    self.dirty = true;
                }
                AlacEvent::Exit => {
                    self.exited = Some(-1);
                    self.dirty = true;
                }
                _ => {} // Bell, Title, ClipboardStore, etc.
            }
        }
    }

    pub fn write_to_pty(&self, input: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(input);
    }


}

impl Drop for TerminalState {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

// ---------------------------------------------------------------------------
// Terminal View — GPUI Render impl
// ---------------------------------------------------------------------------

const CURSOR_BLINK_INTERVAL_MS: u64 = 530;

pub struct TerminalView {
    pub terminal: TerminalState,
    focus_handle: FocusHandle,
    cursor_visible: bool,
    /// Track mouse button state for drag selection
    selecting: bool,
    /// Last known cell dimensions (from TerminalElement::measure_cell)
    cell_width: gpui::Pixels,
    line_height: gpui::Pixels,
    /// Element origin in window coordinates — set by TerminalElement::paint(),
    /// read by mouse handlers for pixel→grid conversion.
    element_origin: Arc<Mutex<gpui::Point<gpui::Pixels>>>,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let terminal = TerminalState::new().expect("Failed to create terminal");
        let focus_handle = cx.focus_handle();

        // Demand-driven sync: poll for new PTY events every 4ms,
        // but only trigger repaint when dirty (new output received).
        // Idle terminal = zero repaints, zero CPU.
        cx.spawn(async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            loop {
                smol::Timer::after(std::time::Duration::from_millis(4)).await;
                let result = cx.update(|cx| {
                    this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                        view.terminal.sync();
                        if view.terminal.dirty {
                            view.terminal.dirty = false;
                            cx.notify();
                        }
                    })
                });
                if result.is_err() {
                    break;
                }
            }
        })
        .detach();

        // Cursor blink timer: toggle visibility every 530ms
        cx.spawn(async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            loop {
                smol::Timer::after(std::time::Duration::from_millis(CURSOR_BLINK_INTERVAL_MS))
                    .await;
                let result = cx.update(|cx| {
                    this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                        // Skip blink repaints when process has exited
                        if view.terminal.exited.is_some() {
                            return;
                        }
                        view.cursor_visible = !view.cursor_visible;
                        cx.notify();
                    })
                });
                if result.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            terminal,
            focus_handle,
            cursor_visible: true,
            selecting: false,
            cell_width: gpui::px(8.0),
            line_height: gpui::px(16.0),
            element_origin: Arc::new(Mutex::new(gpui::Point::default())),
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        let _probe_start = if probe_enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Reset cursor blink on keystroke
        self.cursor_visible = true;

        let keystroke = &event.keystroke;

        // Get current TermMode for key mapping (APP_CURSOR, etc.)
        let term_guard = self.terminal.term.lock();
        let mode = *term_guard.mode();
        drop(term_guard);

        // Try the key mapping module first (handles ctrl, special keys, modifiers)
        if let Some(seq) = crate::keys::to_esc_str(keystroke, &mode) {
            match seq {
                Cow::Borrowed(s) => {
                    // Zero allocation — static byte slice
                    self.terminal.write_to_pty(Cow::Borrowed(s.as_bytes()));
                }
                Cow::Owned(s) => {
                    // One allocation for modifier combos
                    self.terminal.write_to_pty(s.into_bytes());
                }
            }
        } else if let Some(key_char) = &keystroke.key_char {
            // Printable character input — single allocation (String → Vec<u8>)
            self.terminal
                .write_to_pty(key_char.as_bytes().to_vec());
        }

        #[cfg(debug_assertions)]
        if let Some(start) = _probe_start {
            let elapsed = start.elapsed();
            // Store timestamp for total keystroke→pixel measurement in paint()
            self.terminal.last_keystroke_at = Some(start);
            if elapsed.as_millis() > 1 {
                log::warn!(
                    "[latency] keystroke→PTY: {:.2}ms",
                    elapsed.as_secs_f64() * 1000.0
                );
            }
        }
    }

    // --- Pixel → grid coordinate conversion ---

    fn pixel_to_grid(&self, pos: gpui::Point<gpui::Pixels>) -> (AlacPoint, Side) {
        let origin = *self.element_origin.lock().unwrap();
        let relative_x = (pos.x - origin.x).max(gpui::px(0.0));
        let relative_y = (pos.y - origin.y).max(gpui::px(0.0));

        let col_f = relative_x / self.cell_width;
        let half_cell = self.cell_width / 2.0;
        let cell_x = relative_x % self.cell_width;
        let side = if cell_x > half_cell {
            Side::Right
        } else {
            Side::Left
        };

        let term = self.terminal.term.lock();
        let max_col = term.columns().saturating_sub(1);
        let max_line = term.screen_lines().saturating_sub(1) as i32;
        let display_offset = term.grid().display_offset();
        drop(term);

        let col = (col_f as usize).min(max_col);
        let line = ((relative_y / self.line_height) as i32).min(max_line);

        (
            AlacPoint::new(GridLine(line - display_offset as i32), GridCol(col)),
            side,
        )
    }

    // --- Mouse selection handlers ---

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }

        let (point, side) = self.pixel_to_grid(event.position);

        let selection_type = match event.click_count {
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => return,
        };

        let selection = Selection::new(selection_type, point, side);
        let mut term = self.terminal.term.lock();
        term.selection = Some(selection);
        drop(term);

        self.selecting = true;
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selecting {
            return;
        }

        let (point, side) = self.pixel_to_grid(event.position);

        let mut term = self.terminal.term.lock();
        if let Some(ref mut selection) = term.selection {
            selection.update(point, side);
        }
        drop(term);

        cx.notify();
    }

    fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        self.selecting = false;

        // Clear empty selections (single click without drag)
        let mut term = self.terminal.term.lock();
        if let Some(ref sel) = term.selection
            && sel.is_empty()
        {
            term.selection = None;
        }
        drop(term);

        cx.notify();
    }

    // --- Clipboard handlers ---

    fn handle_copy(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let term = self.terminal.term.lock();
        if let Some(text) = term.selection_to_string() {
            drop(term);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn handle_paste(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard()
            && let Some(text) = item.text()
        {
            let mode = {
                let term = self.terminal.term.lock();
                *term.mode()
            };
            let paste_text = if mode.contains(TermMode::BRACKETED_PASTE) {
                // Strip ESC and C1 control chars (U+0080..U+009F) to prevent
                // bracketed paste escape and CSI injection
                let sanitized: String = text
                    .chars()
                    .filter(|&c| c != '\x1b' && !(('\u{0080}'..='\u{009f}').contains(&c)))
                    .collect();
                format!("\x1b[200~{sanitized}\x1b[201~")
            } else {
                text.replace("\r\n", "\r").replace('\n', "\r")
            };
            self.terminal.write_to_pty(paste_text.into_bytes());
        }
    }
}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);

        // Update cell dimensions for mouse → grid mapping
        let dims = TerminalElement::measure_cell(window, cx);
        self.cell_width = dims.cell_width;
        self.line_height = dims.line_height;

        #[cfg(debug_assertions)]
        let keystroke_at = self.terminal.last_keystroke_at.take();

        let terminal_element = TerminalElement::new(
            self.terminal.term.clone(),
            Notifier(self.terminal.notifier.0.clone()),
            self.cursor_visible,
            focused,
            self.terminal.exited,
            self.element_origin.clone(),
            #[cfg(debug_assertions)]
            keystroke_at,
        );

        div()
            .id("terminal-view")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_action(cx.listener(|this, _: &crate::TerminalCopy, window, cx| {
                this.handle_copy(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::TerminalPaste, window, cx| {
                this.handle_paste(window, cx);
            }))
            .size_full()
            .child(terminal_element)
    }
}
