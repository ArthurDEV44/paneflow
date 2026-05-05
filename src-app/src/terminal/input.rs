//! Keyboard, mouse, clipboard, and scroll handlers for `TerminalView`.
//!
//! Every method here is an `impl TerminalView` entry reached from the GPUI
//! event dispatch wired in `mod.rs::Render`. Field access from these methods
//! is what forces the `pub(super)` visibility on `TerminalView`'s fields.
//!
//! Extracted from `terminal.rs` per US-013 of the src-app refactor PRD.

use std::borrow::Cow;

use alacritty_terminal::grid::{Dimensions, Scroll as AlacScroll};
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use gpui::{
    ClipboardEntry, ClipboardItem, Context, ExternalPaths, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ScrollWheelEvent, Window,
};

use crate::mouse;
use crate::terminal::types::{HyperlinkSource, HyperlinkZone};

#[cfg(debug_assertions)]
use super::probe_enabled;
use super::{TerminalEvent, TerminalView};

/// Returns true when the platform-appropriate "open link" modifier is held:
/// Cmd on macOS, Ctrl on Linux/Windows (US-019 AC).
#[inline]
fn open_link_modifier_held(modifiers: &gpui::Modifiers) -> bool {
    #[cfg(target_os = "macos")]
    {
        modifiers.platform
    }
    #[cfg(not(target_os = "macos"))]
    {
        modifiers.control
    }
}

impl TerminalView {
    pub(super) fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Cancel swap mode on Escape — checked before any other mode handling
        if crate::SWAP_MODE.load(std::sync::atomic::Ordering::Relaxed)
            && event.keystroke.key == "escape"
        {
            cx.emit(TerminalEvent::CancelSwapMode);
            return;
        }

        // When search overlay is active, redirect typed characters to search query
        if self.search_active {
            let keystroke = &event.keystroke;
            if keystroke.key == "backspace" && !keystroke.modifiers.control {
                self.search_query.pop();
                self.run_search();
                cx.notify();
                return;
            }
            if let Some(key_char) = &keystroke.key_char
                && !keystroke.modifiers.control
                && !keystroke.modifiers.alt
                && self.search_query.len() < crate::search::MAX_QUERY_LEN
            {
                self.search_query.push_str(key_char);
                self.run_search();
                cx.notify();
                return;
            }
            // Consume all other keys while search is active — action bindings
            // (ToggleSearch, DismissSearch, SearchNext, SearchPrev) are dispatched
            // by GPUI before on_key_down, so they still fire. Everything else
            // (F-keys, Alt combos, etc.) is intentionally suppressed.
            return;
        }

        // When copy mode is active, intercept navigation and exit keys
        if self.copy_mode_active {
            let keystroke = &event.keystroke;
            let key = keystroke.key.as_str();
            let shift = keystroke.modifiers.shift;

            match key {
                "left" | "right" | "up" | "down" => {
                    let (dx, dy): (i32, i32) = match key {
                        "left" => (-1, 0),
                        "right" => (1, 0),
                        "up" => (0, -1),
                        "down" => (0, 1),
                        _ => unreachable!(),
                    };
                    if shift {
                        self.extend_copy_selection(dx, dy, cx);
                    } else {
                        self.move_copy_cursor(dx, dy, cx);
                    }
                }
                "enter" => {
                    self.exit_copy_mode(true, cx);
                }
                "escape" => {
                    self.exit_copy_mode(false, cx);
                }
                _ => {
                    // 'q' exits copy mode (vi-style)
                    if keystroke.key_char.as_deref() == Some("q")
                        && !keystroke.modifiers.control
                        && !keystroke.modifiers.alt
                    {
                        self.exit_copy_mode(false, cx);
                    }
                    // All other keys consumed — not sent to PTY
                }
            }
            return;
        }

        #[cfg(debug_assertions)]
        let _probe_start = if probe_enabled() {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Reset cursor blink on keystroke
        self.cursor_visible = true;

        let keystroke = &event.keystroke;

        // End key (no modifiers) while scrolled back — snap to bottom instead of
        // sending "end of line" to the shell.
        if keystroke.key == "end"
            && !keystroke.modifiers.shift
            && !keystroke.modifiers.control
            && !keystroke.modifiers.alt
            && !keystroke.modifiers.platform
        {
            let mut term = self.terminal.term.lock();
            if term.grid().display_offset() > 0 {
                term.scroll_display(AlacScroll::Bottom);
                self.terminal.dirty = true;
                drop(term);
                cx.notify();
                return;
            }
        }

        // Get current TermMode for key mapping (APP_CURSOR, etc.)
        let term_guard = self.terminal.term.lock();
        let mode = *term_guard.mode();
        drop(term_guard);

        // Special keys / modifiers → write the escape sequence directly.
        // Printable characters are NOT handled here: GPUI's InputHandler
        // (replace_text_in_range) is the single source of truth for them on
        // both normal and alt screens. Writing them here as well caused
        // character doubling in ALT_SCREEN mode (e.g. Claude Code fullscreen TUI).
        if let Some(seq) = crate::keys::to_esc_str(keystroke, &mode, self.option_as_meta) {
            match seq {
                Cow::Borrowed(s) => {
                    self.terminal.write_to_pty(Cow::Borrowed(s.as_bytes()));
                }
                Cow::Owned(s) => {
                    self.terminal.write_to_pty(s.into_bytes());
                }
            }
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

    pub(super) fn pixel_to_grid(&self, pos: gpui::Point<gpui::Pixels>) -> (AlacPoint, Side) {
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

    /// Convert pixel position to viewport grid coordinates (for mouse reporting).
    /// Unlike `pixel_to_grid`, this returns 0-based viewport coordinates without
    /// the scrollback display_offset subtraction.
    pub(super) fn pixel_to_viewport(&self, pos: gpui::Point<gpui::Pixels>) -> AlacPoint {
        let origin = *self.element_origin.lock().unwrap();
        let relative_x = (pos.x - origin.x).max(gpui::px(0.0));
        let relative_y = (pos.y - origin.y).max(gpui::px(0.0));
        let col_f = relative_x / self.cell_width;
        let term = self.terminal.term.lock();
        let max_col = term.columns().saturating_sub(1);
        let max_line = term.screen_lines().saturating_sub(1) as i32;
        drop(term);
        let col = (col_f as usize).min(max_col);
        let line = ((relative_y / self.line_height) as i32).min(max_line);
        AlacPoint::new(GridLine(line), GridCol(col))
    }

    /// Write a mouse report to the PTY using the appropriate encoding format.
    pub(super) fn write_mouse_report(
        &self,
        point: AlacPoint,
        button: u8,
        pressed: bool,
        mode: TermMode,
    ) {
        let format = mouse::MouseFormat::from_mode(mode);
        let bytes = match format {
            mouse::MouseFormat::Sgr => mouse::sgr_mouse_report(point, button, pressed).into_bytes(),
            mouse::MouseFormat::Normal { utf8 } => {
                // Normal/UTF-8 encoding: release always uses button code 3 (no per-button release)
                let btn = if pressed { button } else { 3 };
                match mouse::normal_mouse_report(point, btn, utf8) {
                    Some(b) => b,
                    None => return, // position exceeds encoding limits
                }
            }
        };
        self.terminal.write_to_pty(bytes);
    }

    // --- Mouse selection handlers ---

    pub(super) fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Cmd/Ctrl+Left-click: open hyperlink (US-016 + US-019 + US-020).
        // Modifier is platform-aware: Cmd on macOS, Ctrl elsewhere.
        if event.button == MouseButton::Left
            && open_link_modifier_held(&event.modifiers)
            && event.click_count == 1
        {
            if let Some(ref link) = self.ctrl_hovered_link
                && link.is_openable
            {
                match link.source {
                    HyperlinkSource::FilePath => {
                        // US-020: route to the markdown viewer pipeline.
                        // PaneFlowApp subscribes and splits the pane; we
                        // never call `open::that` for `.md` files because
                        // that would launch the OS default app instead of
                        // the in-pane viewer.
                        let path = std::path::PathBuf::from(&link.uri);
                        cx.emit(TerminalEvent::OpenMarkdownPath(path));
                    }
                    HyperlinkSource::Osc8 | HyperlinkSource::Regex => {
                        let _ = open::that(&link.uri);
                    }
                }
            }
            return;
        }

        let mode = { *self.terminal.term.lock().mode() };

        // Forward to PTY when mouse reporting is active.
        // Shift overrides mouse mode for text selection (standard terminal convention).
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let point = self.pixel_to_viewport(event.position);
            let button = mouse::mouse_button_code(event.button, event.modifiers);
            self.write_mouse_report(point, button, true, mode);
            return;
        }

        // Text selection (mouse mode inactive or Shift held)
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

    pub(super) fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = { *self.terminal.term.lock().mode() };

        // Forward motion to PTY when mouse tracking is active.
        // Shift overrides mouse mode for text selection.
        if !event.modifiers.shift
            && (mode.contains(TermMode::MOUSE_MOTION)
                || (mode.contains(TermMode::MOUSE_DRAG) && event.pressed_button.is_some()))
        {
            let point = self.pixel_to_viewport(event.position);
            let button_base = match event.pressed_button {
                Some(btn) => mouse::mouse_button_code(btn, event.modifiers),
                None => 3, // no button held = release code in motion reports
            };
            // Motion events add +32 to the button code per protocol spec
            let button = button_base + 32;
            self.write_mouse_report(point, button, true, mode);
            return;
        }

        // Track hovered cell for URL regex detection (US-015)
        let (hover_point, _) = self.pixel_to_grid(event.position);
        self.hovered_cell = Some(hover_point);

        // Cmd/Ctrl+hover: detect link under cursor for hyperlink rendering
        // (US-016 + US-019). OSC 8 takes priority over regex URL detection,
        // which takes priority over file-path detection.
        if open_link_modifier_held(&event.modifiers) {
            // Check OSC 8 hyperlink on the hovered cell first
            let osc8_link = {
                let term = self.terminal.term.lock();
                let cell = &term.grid()[hover_point.line][hover_point.column];
                cell.hyperlink().map(|hl| {
                    use crate::terminal::element::is_url_scheme_openable;
                    HyperlinkZone {
                        uri: hl.uri().to_string(),
                        id: hl.id().to_string(),
                        start: hover_point,
                        end: hover_point, // Single cell — hover underline will cover it
                        is_openable: is_url_scheme_openable(hl.uri()),
                        source: HyperlinkSource::Osc8,
                    }
                })
            };
            self.ctrl_hovered_link = osc8_link
                .or_else(|| {
                    let zones = self.detect_url_at_hover();
                    zones.into_iter().find(|z| {
                        hover_point.line == z.start.line
                            && hover_point.column >= z.start.column
                            && hover_point.column <= z.end.column
                    })
                })
                .or_else(|| {
                    // US-019: fall back to .md/.markdown file-path detection.
                    let zones = self.detect_file_path_at_hover();
                    zones.into_iter().find(|z| {
                        hover_point.line == z.start.line
                            && hover_point.column >= z.start.column
                            && hover_point.column <= z.end.column
                    })
                });
            cx.notify();
        } else if self.ctrl_hovered_link.is_some() {
            self.ctrl_hovered_link = None;
            cx.notify();
        }

        // Text selection (mouse mode inactive)
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

    pub(super) fn handle_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = { *self.terminal.term.lock().mode() };

        // Forward release to PTY when mouse reporting is active.
        // Shift overrides mouse mode for text selection.
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let point = self.pixel_to_viewport(event.position);
            let button = mouse::mouse_button_code(event.button, event.modifiers);
            self.write_mouse_report(point, button, false, mode);
            return;
        }

        // Middle-click: paste from primary selection (X11/Wayland only;
        // `read_from_primary` is gated to linux+freebsd in GPUI — mirror
        // the same gate here. On macOS/Windows middle-click has no primary
        // paste convention, so the block is a no-op and we just return.
        if event.button == MouseButton::Middle {
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            {
                if let Some(item) = cx.read_from_primary()
                    && let Some(text) = item.text()
                {
                    self.write_paste_text(&text, mode);
                }
            }
            return;
        }

        // Text selection cleanup (mouse mode inactive or Shift held)
        if event.button != MouseButton::Left {
            return;
        }
        self.selecting = false;

        // Clear empty selections, or auto-copy non-empty selections (tmux-style):
        // write to both PRIMARY (middle-click paste) and CLIPBOARD (Ctrl+V),
        // then clear the selection so the disappearing highlight signals the copy.
        let mut term = self.terminal.term.lock();
        let copied = if let Some(ref sel) = term.selection
            && sel.is_empty()
        {
            term.selection = None;
            None
        } else {
            term.selection_to_string().inspect(|_| {
                term.selection = None;
            })
        };
        drop(term);

        if let Some(text) = copied {
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            cx.write_to_primary(ClipboardItem::new_string(text.clone()));
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(TerminalEvent::SelectionCopied);
        }

        cx.notify();
    }

    // --- Clipboard handlers ---

    pub(super) fn handle_copy(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let term = self.terminal.term.lock();
        if let Some(text) = term.selection_to_string() {
            drop(term);
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(super) fn handle_paste(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(clipboard) = cx.read_from_clipboard() else {
            return;
        };

        // Image in clipboard: forward raw Ctrl+V (0x16) so TUI agents can read it
        if let Some(ClipboardEntry::Image(image)) = clipboard.entries().first()
            && !image.bytes.is_empty()
        {
            self.terminal.write_to_pty(vec![0x16]);
            return;
        }

        // Text paste
        if let Some(text) = clipboard.text() {
            let mode = { *self.terminal.term.lock().mode() };
            self.write_paste_text(&text, mode);
        }
    }

    /// Prepare and write paste text to PTY, respecting bracketed paste mode.
    /// Strips ESC and C1 control chars when bracketed paste is active.
    pub(super) fn handle_file_drop(
        &mut self,
        paths: &ExternalPaths,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        let quoted: Vec<String> = paths
            .paths()
            .iter()
            .filter_map(|p| {
                let s = p.to_string_lossy();
                // Reject paths with newlines or null bytes — they break shell quoting
                if s.contains('\n') || s.contains('\0') {
                    return None;
                }
                if s.contains(' ') || s.contains('\'') || s.contains('"') || s.contains('\\') {
                    Some(format!("'{}'", s.replace('\'', "'\\''")))
                } else {
                    Some(s.into_owned())
                }
            })
            .collect();
        if quoted.is_empty() {
            return;
        }
        let text = quoted.join(" ");
        let mode = *self.terminal.term.lock().mode();
        self.write_paste_text(&text, mode);
    }

    pub(super) fn write_paste_text(&self, text: &str, mode: TermMode) {
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

    // --- Scroll handlers ---

    pub(super) fn handle_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = { *self.terminal.term.lock().mode() };

        // Forward scroll to PTY when mouse reporting is active.
        // Shift overrides mouse mode for scrollback.
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let delta_y = event.delta.pixel_delta(self.line_height).y;
            self.scroll_remainder += delta_y / self.line_height;
            self.scroll_remainder = self.scroll_remainder.clamp(-500.0, 500.0);
            let lines = self.scroll_remainder as i32;
            if lines == 0 {
                return;
            }
            self.scroll_remainder -= lines as f32;

            let point = self.pixel_to_viewport(event.position);
            let direction = if lines > 0 {
                mouse::ScrollDirection::Up
            } else {
                mouse::ScrollDirection::Down
            };
            let button = mouse::scroll_button_code(direction, event.modifiers);
            // Send one report per scroll line
            for _ in 0..lines.unsigned_abs() {
                self.write_mouse_report(point, button, true, mode);
            }
            return;
        }

        // Alternate scroll: ALT_SCREEN + ALTERNATE_SCROLL without MOUSE_MODE
        // Synthesize arrow key sequences so scroll works in less, vim, htop, etc.
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
            && !event.modifiers.shift
        {
            let delta_y = event.delta.pixel_delta(self.line_height).y;
            self.scroll_remainder += delta_y / self.line_height;
            self.scroll_remainder = self.scroll_remainder.clamp(-500.0, 500.0);
            let lines = self.scroll_remainder as i32;
            if lines == 0 {
                return;
            }
            self.scroll_remainder -= lines as f32;

            let app_cursor = mode.contains(TermMode::APP_CURSOR);
            let arrow: &[u8] = match (lines > 0, app_cursor) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let count = lines.unsigned_abs() as usize;
            let mut buf = Vec::with_capacity(arrow.len() * count);
            for _ in 0..count {
                buf.extend_from_slice(arrow);
            }
            self.terminal.write_to_pty(buf);
            return;
        }

        // Scrollback (mouse mode inactive, not alt screen alternate scroll)
        // Convert pixel delta to fractional lines, accumulate sub-line remainders
        let delta_y = event.delta.pixel_delta(self.line_height).y;
        self.scroll_remainder += delta_y / self.line_height;

        // Clamp to prevent extreme values from synthesised events
        self.scroll_remainder = self.scroll_remainder.clamp(-500.0, 500.0);

        let lines = self.scroll_remainder as i32;
        if lines == 0 {
            return;
        }
        self.scroll_remainder -= lines as f32;

        // Positive wheel delta means scrolling up (toward history in natural-scroll
        // convention), which matches AlacScroll::Delta positive = scroll toward history.
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::Delta(lines));
        self.terminal.dirty = true;
        drop(term);

        cx.notify();
    }

    pub(super) fn handle_scroll_page_up(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::PageUp);
        self.terminal.dirty = true;
        drop(term);
        cx.notify();
    }

    pub(super) fn handle_scroll_page_down(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::PageDown);
        self.terminal.dirty = true;
        drop(term);
        cx.notify();
    }
}
