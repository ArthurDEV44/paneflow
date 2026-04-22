//! GPUI view layer for a single terminal pane.
//!
//! Holds the `TerminalView` struct, its constructor + event batch loop,
//! IME wiring, URL hover detection, the `TerminalEvent` enum emitted to
//! consumers (pane / app), and the `Render` impl that composes
//! `TerminalElement` with the search overlay and copy-mode badge.
//!
//! Extracted from `terminal.rs` per US-016 of the src-app refactor PRD.

use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as AlacEvent, Notify};
use alacritty_terminal::grid::{Dimensions, Scroll as AlacScroll};
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint};
use alacritty_terminal::term::TermMode;
use futures::StreamExt;
use gpui::{
    App, ClipboardItem, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement,
    KeyContext, MouseButton, Render, Styled, Window, div, prelude::*,
};

use crate::pty::PortablePtyBackend;

use super::element::TerminalElement;
use super::pty_session::{ClipboardOp, hsla_to_alac_rgb};
use super::service_detector::ServiceInfo;
use super::types::{CopyModeCursorState, HyperlinkZone, SearchHighlight};
use super::{PtyNotifier, TerminalState};

/// Global flag: when true, terminals skip `cx.notify()` to avoid repaints
/// while a non-terminal page (e.g. settings) is displayed.
pub static SUPPRESS_REPAINTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
// Terminal View — GPUI Render impl
// ---------------------------------------------------------------------------

const CURSOR_BLINK_INTERVAL_MS: u64 = 530;

pub struct TerminalView {
    pub terminal: TerminalState,
    focus_handle: FocusHandle,
    pub(super) cursor_visible: bool,
    /// Track mouse button state for drag selection
    pub(super) selecting: bool,
    /// Last known cell dimensions (from TerminalElement::measure_cell)
    pub(super) cell_width: gpui::Pixels,
    pub(super) line_height: gpui::Pixels,
    /// Element origin in window coordinates — set by TerminalElement::paint(),
    /// read by mouse handlers for pixel→grid conversion.
    pub(super) element_origin: Arc<Mutex<gpui::Point<gpui::Pixels>>>,
    /// Sub-line scroll accumulator for smooth trackpad scrolling
    pub(super) scroll_remainder: f32,
    /// Whether the search overlay is visible
    pub(super) search_active: bool,
    /// Current search query string
    pub(super) search_query: String,
    /// Cached search matches (grid coordinates)
    pub(super) search_matches: Vec<crate::search::SearchMatch>,
    /// Index of the currently focused match (for navigation)
    pub(super) search_current: usize,
    /// Whether regex search mode is active (vs plain text)
    pub(super) search_regex_mode: bool,
    /// Regex compilation error message (None when valid or plain text mode)
    pub(super) search_regex_error: Option<String>,
    /// Current prompt mark navigation index (for jump-to-prompt cycling)
    pub(super) prompt_mark_current: Option<usize>,
    /// Whether Alt key is treated as Meta (ESC prefix). Read from config.
    pub(super) option_as_meta: bool,
    /// Whether copy mode (keyboard-driven selection) is active
    pub(super) copy_mode_active: bool,
    /// Copy mode cursor position in grid coordinates
    pub(super) copy_cursor: AlacPoint,
    /// Display offset frozen at copy mode entry to prevent auto-scroll
    pub(super) copy_mode_frozen_offset: usize,
    /// Previous focus state, used to detect focus transitions for DEC 1004 events.
    was_focused: bool,
    /// Bell flash deadline — background pulse visible until this instant.
    bell_flash_until: Option<std::time::Instant>,
    /// Last hovered cell position for URL regex detection (US-015).
    pub(super) hovered_cell: Option<AlacPoint>,
    /// Active hyperlink under Ctrl+hover — drives underline rendering and Ctrl+click.
    pub(super) ctrl_hovered_link: Option<HyperlinkZone>,
    /// IME preedit text (in-progress composition). Empty when no composition active.
    ime_marked_text: String,
    /// Gate for clearing pre-resize shell startup content on first render.
    /// The PTY is spawned before the first `build_layout()` measures the actual
    /// window dimensions, so shell init bytes land in a 120×40 grid. After the
    /// first resize we clear the grid so those garbled bytes don't appear.
    needs_initial_clear: Arc<std::sync::atomic::AtomicBool>,
}

impl TerminalView {
    pub fn new(workspace_id: u64, cx: &mut Context<Self>) -> Self {
        Self::with_cwd(workspace_id, None, None, cx)
    }

    pub fn with_cwd(
        workspace_id: u64,
        cwd: Option<std::path::PathBuf>,
        initial_size: Option<(usize, usize)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let surface_id = cx.entity_id().as_u64();
        let backend = PortablePtyBackend;
        let mut terminal =
            match TerminalState::new(&backend, cwd, workspace_id, surface_id, initial_size) {
                Ok(t) => t,
                Err(e) => {
                    log::error!("PTY creation failed: {e:#}");
                    eprintln!(
                        "Error: Failed to create terminal PTY.\n\
                     Possible causes:\n\
                     \x20 - /dev/pts exhausted (too many PTY sessions)\n\
                     \x20 - Shell not found (check default_shell in config or $SHELL)\n\
                     \x20 - Permission denied on /dev/ptmx\n\n\
                     Underlying error: {e:#}"
                    );
                    panic!("PTY creation failed: {e:#}");
                }
            };
        let focus_handle = cx.focus_handle();

        // Event batch coalescing (Zed pattern):
        // Phase 1: Block until first event (zero CPU when idle)
        // Phase 2: Batch for 4ms, max 100 events, dedup Wakeup
        // Phase 3: Process batch, yield to other GPUI tasks
        let events_rx = terminal.events_rx.take().expect("events_rx already taken");
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let mut events_rx = events_rx;
                loop {
                    // Phase 1: Block until first event arrives (zero CPU when idle)
                    let Some(first_event) = events_rx.next().await else {
                        break; // Channel closed
                    };

                    // Phase 2: Batch additional events for 4ms, max 100
                    let mut batch: Vec<AlacEvent> = Vec::with_capacity(32);
                    let mut had_wakeup = matches!(first_event, AlacEvent::Wakeup);
                    if !had_wakeup {
                        batch.push(first_event);
                    }

                    {
                        let timer = futures::FutureExt::fuse(smol::Timer::after(
                            std::time::Duration::from_millis(4),
                        ));
                        futures::pin_mut!(timer);
                        loop {
                            futures::select_biased! {
                                event = events_rx.next() => {
                                    match event {
                                        Some(AlacEvent::Wakeup) => had_wakeup = true,
                                        Some(e) => batch.push(e),
                                        None => break,
                                    }
                                    if batch.len() >= 100 { break; }
                                }
                                _ = timer => break,
                            }
                        }
                    }

                    // Phase 3: Process the batch in a single entity update
                    let result = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            let old_title = view.terminal.title.clone();
                            let old_cwd = view.terminal.current_cwd.clone();
                            view.terminal.sync_channels();
                            if had_wakeup {
                                view.terminal.process_event(AlacEvent::Wakeup);
                            }
                            for event in batch {
                                view.terminal.process_event(event);
                            }

                            // Execute deferred clipboard operations (OSC 52)
                            for op in view.terminal.pending_clipboard_ops.drain(..) {
                                match op {
                                    ClipboardOp::Store(text) => {
                                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    }
                                    ClipboardOp::Load(format_fn) => {
                                        let text = cx
                                            .read_from_clipboard()
                                            .and_then(|c| c.text())
                                            .unwrap_or_default();
                                        // Strip ESC and C1 control chars to prevent injection
                                        let sanitized: String = text
                                            .chars()
                                            .filter(|&c| {
                                                c != '\x1b'
                                                    && !(('\u{0080}'..='\u{009f}').contains(&c))
                                            })
                                            .collect();
                                        let response = format_fn(&sanitized);
                                        view.terminal.notifier.notify(response.into_bytes());
                                    }
                                }
                            }

                            // Execute deferred color query responses (OSC 10/11/12)
                            // Cap at 16 to prevent flooding from malicious programs
                            view.terminal.pending_color_ops.truncate(16);
                            if !view.terminal.pending_color_ops.is_empty() {
                                let theme = crate::theme::active_theme();
                                for op in view.terminal.pending_color_ops.drain(..) {
                                    let color = match op.index {
                                        // OSC 10: foreground
                                        10 => Some(theme.foreground),
                                        // OSC 11: background
                                        11 => Some(theme.ansi_background),
                                        // OSC 12: cursor
                                        12 => Some(theme.cursor),
                                        _ => None,
                                    };
                                    if let Some(hsla) = color {
                                        let rgb = hsla_to_alac_rgb(hsla);
                                        let response = (op.format_fn)(rgb);
                                        view.terminal.notifier.notify(response.into_bytes());
                                    }
                                }
                            }

                            // Execute deferred text area size responses
                            view.terminal.pending_size_ops.truncate(8);
                            for format_fn in view.terminal.pending_size_ops.drain(..) {
                                let term = view.terminal.term.lock();
                                let size = alacritty_terminal::event::WindowSize {
                                    num_cols: term.columns() as u16,
                                    num_lines: term.screen_lines() as u16,
                                    cell_width: view.cell_width.as_f32() as u16,
                                    cell_height: view.line_height.as_f32() as u16,
                                };
                                drop(term);
                                let response = format_fn(size);
                                view.terminal.notifier.notify(response.into_bytes());
                            }

                            // Bell: trigger visual flash
                            if view.terminal.bell_active {
                                view.terminal.bell_active = false;
                                view.bell_flash_until = Some(
                                    std::time::Instant::now()
                                        + std::time::Duration::from_millis(200),
                                );
                                cx.emit(TerminalEvent::Bell);
                                // Schedule notify after flash duration to clear it
                                cx.spawn(async |this, cx| {
                                    smol::Timer::after(std::time::Duration::from_millis(200)).await;
                                    let _ = cx.update(|cx| {
                                        this.update(cx, |view, cx| {
                                            // Only clear if no newer bell extended the deadline
                                            if view
                                                .bell_flash_until
                                                .is_some_and(|t| t <= std::time::Instant::now())
                                            {
                                                view.bell_flash_until = None;
                                            }
                                            cx.notify();
                                        })
                                    });
                                })
                                .detach();
                            }

                            if view.terminal.exited.is_some() {
                                cx.emit(TerminalEvent::ChildExited);
                            }
                            if view.terminal.title != old_title {
                                cx.emit(TerminalEvent::TitleChanged);
                            }
                            if view.terminal.current_cwd != old_cwd
                                && let Some(ref cwd) = view.terminal.current_cwd
                            {
                                cx.emit(TerminalEvent::CwdChanged(cwd.clone()));
                            }

                            if view.terminal.dirty {
                                view.terminal.dirty = false;
                                let suppress =
                                    SUPPRESS_REPAINTS.load(std::sync::atomic::Ordering::Relaxed);

                                if !suppress {
                                    view.terminal.output_scan_ticks += 1;
                                    let should_scan = view.terminal.output_scan_ticks <= 10
                                        || view.terminal.output_scan_ticks >= 50;
                                    if should_scan {
                                        view.terminal.output_scan_ticks = 0;
                                        for service in view.terminal.scan_output() {
                                            cx.emit(TerminalEvent::ServiceDetected(service));
                                        }
                                        cx.emit(TerminalEvent::ActivityBurst);
                                    }

                                    // Copy mode: restore frozen display offset
                                    if view.copy_mode_active {
                                        let mut term = view.terminal.term.lock();
                                        let current = term.grid().display_offset();
                                        let frozen = view.copy_mode_frozen_offset;
                                        if current != frozen {
                                            let delta = frozen as i32 - current as i32;
                                            term.scroll_display(AlacScroll::Delta(delta));
                                        }
                                    }

                                    cx.notify();
                                }
                            }
                        })
                    });
                    if result.is_err() {
                        break;
                    }

                    // Yield to other GPUI tasks between batches
                    smol::future::yield_now().await;
                }
            },
        )
        .detach();

        // Cursor blink timer: toggle visibility every 530ms
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(CURSOR_BLINK_INTERVAL_MS))
                        .await;
                    let result = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                            if view.terminal.exited.is_some()
                                || SUPPRESS_REPAINTS.load(std::sync::atomic::Ordering::Relaxed)
                            {
                                return;
                            }
                            if view.terminal.cursor_blinking {
                                view.cursor_visible = !view.cursor_visible;
                            } else {
                                view.cursor_visible = true;
                            }
                            cx.notify();
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        Self {
            terminal,
            focus_handle,
            cursor_visible: true,
            selecting: false,
            cell_width: gpui::px(8.0),
            line_height: gpui::px(16.0),
            element_origin: Arc::new(Mutex::new(gpui::Point::default())),
            scroll_remainder: 0.0,
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current: 0,
            search_regex_mode: false,
            search_regex_error: None,
            prompt_mark_current: None,
            option_as_meta: paneflow_config::loader::load_config()
                .option_as_meta
                .unwrap_or(true),
            copy_mode_active: false,
            copy_cursor: AlacPoint::new(GridLine(0), GridCol(0)),
            copy_mode_frozen_offset: 0,
            was_focused: false,
            bell_flash_until: None,
            hovered_cell: None,
            ctrl_hovered_link: None,
            ime_marked_text: String::new(),
            needs_initial_clear: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }
}

// ---------------------------------------------------------------------------
// IME composition methods (US-017)
// ---------------------------------------------------------------------------

impl TerminalView {
    /// Set preedit text during IME composition.
    pub fn set_marked_text(&mut self, text: String, cx: &mut Context<Self>) {
        self.ime_marked_text = text;
        cx.notify();
    }

    /// Clear preedit text (cancel composition).
    pub fn clear_marked_text(&mut self, cx: &mut Context<Self>) {
        self.ime_marked_text.clear();
        cx.notify();
    }

    /// Commit composed text to the PTY.
    pub fn commit_text(&mut self, text: &str, _cx: &mut Context<Self>) {
        self.terminal.write_to_pty(text.as_bytes().to_vec());
    }

    /// Send arbitrary text to the PTY (no bracketed paste wrapping).
    /// Used by AI agents and automation tools via IPC.
    pub fn send_text(&self, text: &str) {
        self.terminal.write_to_pty(text.as_bytes().to_vec());
    }

    /// Send a shell command to the PTY and execute it (appends `\r`).
    /// Used by tab-bar command buttons.
    pub fn send_command(&self, command: &str) {
        let mut bytes = command.as_bytes().to_vec();
        bytes.push(b'\r');
        self.terminal.write_to_pty(bytes);
    }

    /// Send a keystroke to the PTY by converting it to an escape sequence.
    /// `keystroke_str` is a dash-separated description like "ctrl-c", "enter", "alt-f".
    /// Returns Ok(()) on success, Err(message) if the keystroke string is invalid.
    pub fn send_keystroke(&self, keystroke_str: &str) -> Result<(), String> {
        let keystroke = gpui::Keystroke::parse(keystroke_str).map_err(|e| format!("{e}"))?;
        let mode = *self.terminal.term.lock().mode();
        if let Some(seq) = crate::keys::to_esc_str(&keystroke, &mode, self.option_as_meta) {
            self.terminal.write_to_pty(seq.as_bytes().to_vec());
        } else if let Some(ref key_char) = keystroke.key_char {
            self.terminal.write_to_pty(key_char.as_bytes().to_vec());
        }
        Ok(())
    }

    /// Return the UTF-16 range of the current preedit text, if any.
    pub fn marked_text_range(&self) -> Option<std::ops::Range<usize>> {
        if self.ime_marked_text.is_empty() {
            None
        } else {
            let utf16_len: usize = self.ime_marked_text.encode_utf16().count();
            Some(0..utf16_len)
        }
    }
}

// ---------------------------------------------------------------------------
// URL detection on hover (US-015)
// ---------------------------------------------------------------------------

impl TerminalView {
    /// Detect regex URLs on the line at the given grid point.
    /// Extracts line text from the locked term grid, runs the URL regex,
    /// and returns zones that cover the given column (for hover hit-testing).
    #[allow(dead_code)]
    pub fn detect_url_at_hover(&self) -> Vec<HyperlinkZone> {
        let point = match self.hovered_cell {
            Some(p) => p,
            None => return Vec::new(),
        };
        let term = self.terminal.term.lock();
        let grid = term.grid();
        let line = point.line;

        // Extract line text from grid cells, skipping wide-char spacer placeholders.
        // Track a char-to-column mapping so regex byte offsets map to grid columns.
        let cols = term.columns();
        let mut line_text = String::with_capacity(cols);
        let mut char_to_col: Vec<usize> = Vec::with_capacity(cols);
        for col in 0..cols {
            let cell = &grid[line][alacritty_terminal::index::Column(col)];
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
            {
                continue; // Skip trailing spacer of wide chars
            }
            char_to_col.push(col);
            line_text.push(cell.c);
        }

        // Trim trailing whitespace for cleaner regex matching
        let trimmed = line_text.trim_end();
        crate::terminal::element::detect_urls_on_line_mapped(trimmed, line, &char_to_col)
    }
}

// ---------------------------------------------------------------------------
// Terminal events
// ---------------------------------------------------------------------------

/// Events emitted by TerminalView via GPUI's EventEmitter.
/// Pane subscribes for ChildExited/TitleChanged; PaneFlowApp subscribes
/// for CwdChanged/ActivityBurst/ServiceDetected to drive sidebar updates.
pub enum TerminalEvent {
    /// The shell process exited (e.g. user typed `exit`).
    ChildExited,
    /// The terminal title changed (via OSC 0/2 escape sequence).
    TitleChanged,
    /// The shell's working directory changed (detected via OSC 7 escape sequence).
    CwdChanged(String),
    /// Terminal output activity detected — triggers port scanning via `/proc/net/tcp`.
    /// Emitted alongside `ServiceDetected` during output scan ticks.
    ActivityBurst,
    /// A server/service was detected in PTY output (e.g. "Listening on :3000").
    /// Enriches the bare port from `/proc/net/tcp` with label and URL.
    ServiceDetected(ServiceInfo),
    /// Terminal bell (\a) was triggered — visual flash notification.
    Bell,
    /// Escape pressed while swap mode is active — requests cancellation.
    CancelSwapMode,
    /// A mouse selection was auto-copied to the clipboard on mouse release.
    /// Consumed by `PaneFlowApp` to surface a "Copied" toast.
    SelectionCopied,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalView {
    /// Build a rich key context from the terminal's current mode flags.
    /// Enables keybindings scoped to terminal state (e.g. `"Terminal && screen == alt"`).
    fn dispatch_context(&self) -> KeyContext {
        let mode = *self.terminal.term.lock().mode();
        let mut ctx = KeyContext::default();
        ctx.add("Terminal");

        // Screen mode
        if mode.contains(TermMode::ALT_SCREEN) {
            ctx.set("screen", "alt");
        } else {
            ctx.set("screen", "normal");
        }

        // DEC private modes
        if mode.contains(TermMode::APP_CURSOR) {
            ctx.add("DECCKM");
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            ctx.add("DECPAM");
        }
        if mode.contains(TermMode::BRACKETED_PASTE) {
            ctx.add("bracketed_paste");
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            ctx.add("report_focus");
        }
        if mode.contains(TermMode::ALTERNATE_SCROLL) {
            ctx.add("alternate_scroll");
        }

        // Mouse reporting mode
        if mode.intersects(TermMode::MOUSE_MODE) {
            ctx.add("any_mouse_reporting");
            if mode.contains(TermMode::MOUSE_MOTION) {
                ctx.set("mouse_reporting", "motion");
            } else if mode.contains(TermMode::MOUSE_DRAG) {
                ctx.set("mouse_reporting", "drag");
            } else {
                ctx.set("mouse_reporting", "click");
            }
        } else {
            ctx.set("mouse_reporting", "off");
        }

        // Mouse encoding format
        if mode.contains(TermMode::SGR_MOUSE) {
            ctx.set("mouse_format", "sgr");
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            ctx.set("mouse_format", "utf8");
        } else {
            ctx.set("mouse_format", "normal");
        }

        ctx
    }

    /// Build the top-right search overlay bar. Caller is responsible for
    /// adding it to the main element tree (and for gating on `search_active`).
    fn render_search_overlay(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let search_query = self.search_query.clone();
        let search_regex_mode = self.search_regex_mode;
        let search_has_regex_error = self.search_regex_error.is_some();
        let match_count = self.search_matches.len();
        let current_match = if match_count > 0 {
            self.search_current + 1
        } else {
            0
        };

        let status_text = if search_has_regex_error {
            "Invalid regex".to_string()
        } else if search_query.is_empty() {
            String::new()
        } else if match_count == 0 {
            "0 results".to_string()
        } else {
            format!("{current_match}/{match_count}")
        };

        // Regex toggle button: highlighted when active
        let regex_toggle = div()
            .id("search-regex-toggle")
            .px(gpui::px(4.0))
            .py(gpui::px(2.0))
            .rounded_sm()
            .cursor_pointer()
            .text_size(gpui::px(12.0))
            .when(search_regex_mode, |el| {
                el.bg(gpui::rgb(0x89b4fa)).text_color(gpui::rgb(0x1e1e2e))
            })
            .when(!search_regex_mode, |el| {
                el.bg(gpui::rgb(0x45475a)).text_color(gpui::rgb(0x6c7086))
            })
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _window, cx| {
                    this.toggle_search_regex(cx);
                }),
            )
            .child(".*");

        // Query display — red border on regex error
        let query_border_color = if search_has_regex_error {
            gpui::rgb(0xf38ba8) // Catppuccin red
        } else {
            gpui::rgb(0x45475a) // Subtle border
        };

        let query_display = div()
            .id("search-query-display")
            .min_w(gpui::px(120.0))
            .px_2()
            .py(gpui::px(2.0))
            .rounded_sm()
            .bg(gpui::rgb(0x1e1e2e))
            .border_1()
            .border_color(query_border_color)
            .text_color(gpui::rgb(0xcdd6f4))
            .text_size(gpui::px(13.0))
            .child(if search_query.is_empty() {
                "...".to_string()
            } else {
                search_query
            });

        div()
            .id("search-overlay")
            .absolute()
            .top_1()
            .right_1()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(gpui::rgb(0x313244))
            .border_1()
            .border_color(gpui::rgb(0x585b70))
            .child(
                div()
                    .id("search-icon")
                    .text_color(gpui::rgb(0x6c7086))
                    .text_size(gpui::px(13.0))
                    .child("Find:"),
            )
            .child(regex_toggle)
            .child(query_display)
            .child(
                div()
                    .id("search-status")
                    .text_color(if search_has_regex_error {
                        gpui::rgb(0xf38ba8)
                    } else {
                        gpui::rgb(0xa6adc8)
                    })
                    .text_size(gpui::px(12.0))
                    .child(status_text),
            )
            .into_any_element()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);

        // DEC 1004: send focus in/out events on focus transitions
        if focused != self.was_focused {
            let mode = { *self.terminal.term.lock().mode() };
            if mode.contains(TermMode::FOCUS_IN_OUT) {
                if focused {
                    self.terminal.write_to_pty(b"\x1b[I".to_vec());
                } else {
                    self.terminal.write_to_pty(b"\x1b[O".to_vec());
                }
            }
            self.was_focused = focused;
        }

        // Update cell dimensions for mouse → grid mapping
        let dims = crate::terminal::element::measure_cell(window, cx);
        self.cell_width = dims.cell_width;
        self.line_height = dims.line_height;

        #[cfg(debug_assertions)]
        let keystroke_at = self.terminal.last_keystroke_at.take();

        // Collect search match rects for the element to paint
        let search_match_rects = if self.search_active && !self.search_matches.is_empty() {
            self.search_matches
                .iter()
                .enumerate()
                .map(|(i, m)| SearchHighlight {
                    start: m.start,
                    end: m.end,
                    is_active: i == self.search_current,
                })
                .collect()
        } else {
            Vec::new()
        };

        // Build copy mode cursor state for the element. When a selection is active,
        // also expose the anchor (selection start) so the element can render it as
        // a distinct tmux-style marker.
        let copy_cursor_state = if self.copy_mode_active {
            let (anchor_grid_line, anchor_col) = {
                let term = self.terminal.term.lock();
                term.selection
                    .as_ref()
                    .and_then(|sel| sel.to_range(&term))
                    .map(|range| (Some(range.start.line.0), range.start.column.0))
                    .unwrap_or((None, 0))
            };
            Some(CopyModeCursorState {
                grid_line: self.copy_cursor.line.0,
                col: self.copy_cursor.column.0,
                anchor_grid_line,
                anchor_col,
            })
        } else {
            None
        };

        // ALT_SCREEN: cursor always visible (no blink-off) for TUI apps
        let cursor_visible = self.cursor_visible
            || self
                .terminal
                .term
                .lock()
                .mode()
                .contains(TermMode::ALT_SCREEN);

        let terminal_element = TerminalElement::new(
            self.terminal.term.clone(),
            PtyNotifier(self.terminal.notifier.0.clone()),
            cursor_visible,
            focused,
            self.terminal.exited,
            self.element_origin.clone(),
            search_match_rects,
            copy_cursor_state,
            self.bell_flash_until
                .is_some_and(|t| std::time::Instant::now() < t),
            self.ctrl_hovered_link
                .as_ref()
                .map(|link| (link.start.line, link.start.column.0, link.end.column.0)),
            self.ctrl_hovered_link.as_ref().map(|link| link.uri.clone()),
            self.ime_marked_text.clone(),
            self.focus_handle.clone(),
            cx.entity().clone(),
            self.needs_initial_clear.clone(),
            #[cfg(debug_assertions)]
            keystroke_at,
        );

        // Search overlay bar
        let search_active = self.search_active;

        let mut el = div()
            .id("terminal-view")
            .key_context(self.dispatch_context())
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_any_mouse_down(cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::handle_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::handle_mouse_up))
            .on_action(cx.listener(|this, _: &crate::TerminalCopy, window, cx| {
                this.handle_copy(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::TerminalPaste, window, cx| {
                this.handle_paste(window, cx);
            }))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .on_action(cx.listener(|this, _: &crate::ScrollPageUp, window, cx| {
                this.handle_scroll_page_up(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ScrollPageDown, window, cx| {
                this.handle_scroll_page_down(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleSearch, _window, cx| {
                this.toggle_search(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::DismissSearch, _window, cx| {
                this.dismiss_search(cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::ToggleSearchRegex, _window, cx| {
                    this.toggle_search_regex(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &crate::SearchNext, _window, cx| {
                this.search_next(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::SearchPrev, _window, cx| {
                this.search_prev(cx);
            }))
            .on_action(cx.listener(|this, _: &crate::ToggleCopyMode, _window, cx| {
                this.toggle_copy_mode(cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::JumpToPromptPrev, _window, cx| {
                    this.jump_to_prompt_prev(cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::JumpToPromptNext, _window, cx| {
                    this.jump_to_prompt_next(cx);
                }),
            )
            .on_drop(cx.listener(Self::handle_file_drop))
            .on_action(
                cx.listener(|this, _: &crate::ClearScrollHistory, _window, cx| {
                    this.clear_scroll_history(cx);
                }),
            )
            .on_action(cx.listener(|this, _: &crate::ResetTerminal, _window, cx| {
                this.reset_terminal(cx);
            }))
            .size_full()
            .child(terminal_element);

        if search_active {
            // Add "Search" key context for search-scoped bindings
            el = el.key_context("Search");
            el = el.child(self.render_search_overlay(cx));
        }

        if self.copy_mode_active {
            let copy_badge = div()
                .id("copy-mode-badge")
                .absolute()
                .top_1()
                .right_1()
                .px_2()
                .py(gpui::px(2.0))
                .rounded_md()
                .bg(gpui::rgba(0x89b4facc))
                .text_color(gpui::rgb(0x1e1e2e))
                .text_size(gpui::px(11.0))
                .font_weight(gpui::FontWeight::BOLD)
                .child("COPY");
            el = el.child(copy_badge);
        }

        el
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::{PtyBackend, PtyProcess};
    use crate::terminal::pty_session::strip_partial_ansi_tail;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Mock PTY backend for testing — creates in-memory reader/writer pairs
    /// without spawning a real shell process.
    struct MockPtyBackend;

    /// Minimal mock child process that reports exit code 0.
    #[derive(Debug)]
    struct MockChild;

    impl portable_pty::ChildKiller for MockChild {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }

        fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
            Box::new(MockChild)
        }
    }

    impl portable_pty::Child for MockChild {
        fn process_id(&self) -> Option<u32> {
            Some(9999)
        }

        fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
            Ok(portable_pty::ExitStatus::with_exit_code(0))
        }

        fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
            Ok(Some(portable_pty::ExitStatus::with_exit_code(0)))
        }

        // portable_pty::Child adds an `as_raw_handle` method on Windows
        // that mirrors `as_raw_fd` on Unix. The trait method is gated on
        // the portable_pty side; we stub it here so MockChild satisfies
        // the Windows trait surface during CI `cargo test` on the
        // x86_64-pc-windows-msvc target.
        #[cfg(windows)]
        fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
            None
        }
    }

    /// Minimal mock MasterPty — resize is a no-op.
    struct MockMasterPty;

    impl portable_pty::MasterPty for MockMasterPty {
        fn resize(&self, _size: portable_pty::PtySize) -> Result<(), anyhow::Error> {
            Ok(())
        }

        fn get_size(&self) -> Result<portable_pty::PtySize, anyhow::Error> {
            Ok(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
        }

        fn try_clone_reader(&self) -> Result<Box<dyn std::io::Read + Send>, anyhow::Error> {
            Ok(Box::new(Cursor::new(Vec::new())))
        }

        fn take_writer(&self) -> Result<Box<dyn std::io::Write + Send>, anyhow::Error> {
            Ok(Box::new(Vec::new()))
        }

        // `process_group_leader` is a Unix-only trait method in
        // portable_pty 0.8 (the concept doesn't map to Windows process
        // groups); gating the impl to cfg(unix) keeps the Windows build
        // from erroring with E0407.
        #[cfg(unix)]
        fn process_group_leader(&self) -> Option<i32> {
            None
        }

        #[cfg(unix)]
        fn as_raw_fd(&self) -> Option<i32> {
            None
        }
    }

    impl PtyBackend for MockPtyBackend {
        fn spawn(
            &self,
            _command: &str,
            _args: &[String],
            _cwd: &Path,
            _env: &HashMap<String, String>,
            _rows: u16,
            _cols: u16,
        ) -> anyhow::Result<PtyProcess> {
            // Return an empty reader that immediately reaches EOF,
            // causing the reader thread to exit cleanly.
            let reader: Box<dyn std::io::Read + Send> = Box::new(Cursor::new(Vec::new()));
            let writer: Box<dyn std::io::Write + Send> = Box::new(Vec::new());
            let child: Box<dyn portable_pty::Child + Send + Sync> = Box::new(MockChild);
            let master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>> =
                Arc::new(Mutex::new(Box::new(MockMasterPty)));

            Ok(PtyProcess {
                reader,
                writer,
                child,
                master,
                child_pid: 9999,
            })
        }
    }

    #[test]
    fn mock_backend_creates_terminal_state() {
        let backend = MockPtyBackend;
        let state = TerminalState::new(&backend, None, 1, 1, None);
        assert!(state.is_ok(), "TerminalState::new with mock backend failed");

        let state = state.unwrap();
        assert_eq!(state.child_pid, 9999);
        assert!(state.exited.is_none());
        assert_eq!(state.title, "Terminal");
    }

    // --- strip_partial_ansi_tail tests (US-012 / US-015) ---

    #[test]
    fn strip_ansi_plain_text_unchanged() {
        let mut s = "hello world\nline two".to_string();
        strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "hello world\nline two");
    }

    #[test]
    fn strip_ansi_lone_esc_removed() {
        let mut s = "hello\x1b".to_string();
        strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "hello");
    }

    #[test]
    fn strip_ansi_incomplete_csi_removed() {
        // Incomplete CSI: \x1b[38;2; (no terminating byte in 0x40..0x7E)
        let mut s = "text\x1b[38;2;".to_string();
        strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "text");
    }

    #[test]
    fn strip_ansi_complete_csi_kept() {
        // Complete CSI: \x1b[0m (terminated by 'm')
        let mut s = "text\x1b[0m".to_string();
        strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "text\x1b[0m");
    }

    #[test]
    fn strip_ansi_incomplete_osc_removed() {
        // Incomplete OSC: \x1b]7;file:// (no BEL or ST)
        let mut s = "prompt\x1b]7;file://host/dir".to_string();
        strip_partial_ansi_tail(&mut s);
        assert_eq!(s, "prompt");
    }

    // --- extract_scrollback / restore_scrollback tests (US-011 / US-015) ---

    #[test]
    fn scrollback_round_trip_via_mock() {
        let backend = MockPtyBackend;
        let state = TerminalState::new(&backend, None, 1, 1, None).unwrap();

        // Feed some text into the terminal grid
        state.restore_scrollback("line one\nline two\nline three");

        // Extract it back
        let scrollback = state.extract_scrollback();
        assert!(scrollback.is_some(), "Expected scrollback content");
        let text = scrollback.unwrap();
        assert!(text.contains("line one"), "Missing 'line one' in: {text}");
        assert!(text.contains("line two"), "Missing 'line two' in: {text}");
        assert!(
            text.contains("line three"),
            "Missing 'line three' in: {text}"
        );
    }

    #[test]
    fn extract_scrollback_empty_terminal_returns_none() {
        let backend = MockPtyBackend;
        let state = TerminalState::new(&backend, None, 1, 1, None).unwrap();
        // Fresh terminal with no content beyond the initial blank grid
        // May return None or Some with only whitespace — both are acceptable
        let scrollback = state.extract_scrollback();
        if let Some(ref text) = scrollback {
            assert!(
                text.trim().is_empty(),
                "Expected empty or whitespace-only scrollback, got: {text}"
            );
        }
    }
}
