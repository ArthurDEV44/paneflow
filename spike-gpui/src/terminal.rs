//! Terminal state and view — PTY management and GPUI view wrapper.
//!
//! Manages the alacritty_terminal Term, PTY EventLoop, and periodic sync.
//! The TerminalView creates a TerminalElement for cell-by-cell rendering.

use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop as AlacEventLoop, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::Term;

use gpui::{
    div, prelude::*, App, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    Render, Styled, Window,
};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

use crate::terminal_element::TerminalElement;

// ---------------------------------------------------------------------------
// Dimensions adapter
// ---------------------------------------------------------------------------

struct SpikeTermSize {
    columns: usize,
    screen_lines: usize,
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
    notifier: Notifier,
    _events_rx: UnboundedReceiver<AlacEvent>,
    _mode: TermMode,
    _cols: usize,
    _rows: usize,
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
            _events_rx: events_rx,
            _mode: TermMode::empty(),
            _cols: cols,
            _rows: rows,
        })
    }

    /// Triggers a repaint. The actual grid read happens in TerminalElement::build_layout
    /// to avoid double-locking the FairMutex.
    pub fn sync(&mut self) {
        // No-op — repaint is requested by cx.notify() in the timer callback.
        // The term lock is acquired only once per frame, in TerminalElement::build_layout.
    }

    pub fn write_to_pty(&self, input: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(input);
    }


}

// ---------------------------------------------------------------------------
// Terminal View — GPUI Render impl
// ---------------------------------------------------------------------------

pub struct TerminalView {
    terminal: TerminalState,
    focus_handle: FocusHandle,
}

impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let terminal = TerminalState::new().expect("Failed to create terminal");
        let focus_handle = cx.focus_handle();

        // Periodic sync: pull PTY output into terminal grid, request repaint
        cx.spawn(async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
            loop {
                smol::Timer::after(std::time::Duration::from_millis(4)).await;
                let result = cx.update(|cx| {
                    this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                        view.terminal.sync();
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
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;

        // Regular character input via key_char
        if let Some(key_char) = &keystroke.key_char {
            self.terminal.write_to_pty(key_char.clone().into_bytes());
            return;
        }

        let key = keystroke.key.as_str();
        let ctrl = keystroke.modifiers.control;
        let shift = keystroke.modifiers.shift;

        // Control characters
        if ctrl {
            let byte = match key {
                "a" => Some(0x01u8),
                "b" => Some(0x02),
                "c" => Some(0x03),
                "d" => Some(0x04),
                "e" => Some(0x05),
                "f" => Some(0x06),
                "h" => Some(0x08),
                "k" => Some(0x0b),
                "l" => Some(0x0c),
                "n" => Some(0x0e),
                "p" => Some(0x10),
                "r" => Some(0x12),
                "u" => Some(0x15),
                "w" => Some(0x17),
                "z" => Some(0x1a),
                _ => None,
            };
            if let Some(b) = byte {
                self.terminal.write_to_pty(vec![b]);
                return;
            }
        }

        // Special keys → ANSI escape sequences
        let esc: Option<&[u8]> = match key {
            "enter" => Some(b"\r"),
            "tab" => Some(b"\t"),
            "escape" => Some(b"\x1b"),
            "backspace" => Some(b"\x7f"),
            "delete" => Some(b"\x1b[3~"),
            "up" => Some(b"\x1b[A"),
            "down" => Some(b"\x1b[B"),
            "right" => Some(b"\x1b[C"),
            "left" => Some(b"\x1b[D"),
            "home" => Some(b"\x1b[H"),
            "end" => Some(b"\x1b[F"),
            "pageup" => Some(b"\x1b[5~"),
            "pagedown" => Some(b"\x1b[6~"),
            "space" => Some(b" "),
            _ => None,
        };

        if let Some(seq) = esc {
            self.terminal.write_to_pty(seq.to_vec());
            return;
        }

        // Regular single character
        if key.len() == 1 && !ctrl {
            let ch = if shift {
                key.to_uppercase()
            } else {
                key.to_string()
            };
            self.terminal.write_to_pty(ch.into_bytes());
        }
    }
}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let terminal_element = TerminalElement::new(self.terminal.term.clone());

        div()
            .id("terminal-view")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .child(terminal_element)
    }
}
