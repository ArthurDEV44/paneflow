//! Terminal state and view — PTY management and GPUI view wrapper.
//!
//! Manages the alacritty_terminal Term, PTY EventLoop, and periodic sync.
//! The TerminalView creates a TerminalElement for cell-by-cell rendering.

use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, Notify, WindowSize};
use alacritty_terminal::event_loop::{EventLoop as AlacEventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Config as TermConfig;
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
    terminal: TerminalState,
    focus_handle: FocusHandle,
    cursor_visible: bool,
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
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        #[cfg(debug_assertions)]
        let _start = if std::env::var("PANEFLOW_LATENCY_PROBE").is_ok() {
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
        if let Some(start) = _start {
            let elapsed = start.elapsed();
            if elapsed.as_micros() > 100 {
                log::warn!("keystroke→PTY latency: {:?}", elapsed);
            }
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
        let terminal_element = TerminalElement::new(
            self.terminal.term.clone(),
            Notifier(self.terminal.notifier.0.clone()),
            self.cursor_visible,
            focused,
            self.terminal.exited,
        );

        div()
            .id("terminal-view")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .size_full()
            .child(terminal_element)
    }
}
