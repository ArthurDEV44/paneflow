//! Search, prompt-mark jumping, copy-mode navigation, and terminal reset
//! actions on `TerminalView`.
//!
//! Text matching, scroll-to-match, and the `SearchMatch` type live in the
//! crate-level `crate::search` module — this file only owns the `TerminalView`
//! plumbing that wires those utilities to keyboard actions and updates copy
//! mode state.
//!
//! Extracted from `terminal.rs` per US-014 of the src-app refactor PRD.

use alacritty_terminal::grid::{Dimensions, Scroll as AlacScroll};
use alacritty_terminal::index::{Column as GridCol, Line as GridLine, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use gpui::{ClipboardItem, Context};

use super::{PromptMarkKind, TerminalView};

impl TerminalView {
    // --- Terminal control actions ---

    pub(super) fn clear_scroll_history(&mut self, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        term.grid_mut().clear_history();
        drop(term);
        cx.notify();
    }

    pub(super) fn reset_terminal(&mut self, cx: &mut Context<Self>) {
        self.terminal.write_to_pty(b"\x1bc".as_ref());
        cx.notify();
    }

    // --- Search ---

    pub(super) fn toggle_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = !self.search_active;
        if !self.search_active {
            self.search_query.clear();
            self.search_matches.clear();
            self.search_current = 0;
            self.search_regex_error = None;
            // Reset scroll position
            let mut term = self.terminal.term.lock();
            term.scroll_display(AlacScroll::Bottom);
        }
        cx.notify();
    }

    pub(super) fn dismiss_search(&mut self, cx: &mut Context<Self>) {
        self.search_active = false;
        self.search_query.clear();
        self.search_matches.clear();
        self.search_current = 0;
        self.search_regex_error = None;
        let mut term = self.terminal.term.lock();
        term.scroll_display(AlacScroll::Bottom);
        cx.notify();
    }

    pub(super) fn toggle_search_regex(&mut self, cx: &mut Context<Self>) {
        self.search_regex_mode = !self.search_regex_mode;
        if !self.search_query.is_empty() {
            self.run_search();
        }
        cx.notify();
    }

    pub(super) fn search_next(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        self.search_current = (self.search_current + 1) % self.search_matches.len();
        self.scroll_to_current_match();
        cx.notify();
    }

    pub(super) fn search_prev(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        if self.search_current == 0 {
            self.search_current = self.search_matches.len() - 1;
        } else {
            self.search_current -= 1;
        }
        self.scroll_to_current_match();
        cx.notify();
    }

    pub(super) fn run_search(&mut self) {
        let result = crate::search::search_term(
            &self.terminal.term,
            &self.search_query,
            self.search_regex_mode,
        );
        self.search_matches = result.matches;
        self.search_regex_error = result.regex_error;
        self.search_current = 0;
        if !self.search_matches.is_empty() {
            self.scroll_to_current_match();
        }
    }

    fn scroll_to_current_match(&mut self) {
        if let Some(m) = self.search_matches.get(self.search_current) {
            crate::search::scroll_to_match(&self.terminal.term, m);
        }
    }

    // --- Prompt-mark navigation ---

    pub(super) fn jump_to_prompt_prev(&mut self, cx: &mut Context<Self>) {
        let marks = &self.terminal.prompt_marks;
        if marks.is_empty() {
            return;
        }
        // Find prompt-start marks only (kind A) — these are the actual prompt boundaries
        let prompt_indices: Vec<usize> = marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.kind == PromptMarkKind::PromptStart)
            .map(|(i, _)| i)
            .collect();
        if prompt_indices.is_empty() {
            return;
        }
        let current = self.prompt_mark_current.unwrap_or(prompt_indices.len());
        let next = if current == 0 {
            0 // Stay at first prompt
        } else {
            current.saturating_sub(1)
        };
        if let Some(&mark_idx) = prompt_indices.get(next) {
            self.prompt_mark_current = Some(next);
            let mark = &marks[mark_idx];
            let search_match = crate::search::SearchMatch {
                start: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
                end: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
            };
            crate::search::scroll_to_match(&self.terminal.term, &search_match);
            cx.notify();
        }
    }

    pub(super) fn jump_to_prompt_next(&mut self, cx: &mut Context<Self>) {
        let marks = &self.terminal.prompt_marks;
        if marks.is_empty() {
            return;
        }
        let prompt_indices: Vec<usize> = marks
            .iter()
            .enumerate()
            .filter(|(_, m)| m.kind == PromptMarkKind::PromptStart)
            .map(|(i, _)| i)
            .collect();
        if prompt_indices.is_empty() {
            return;
        }
        let next = self
            .prompt_mark_current
            .map_or(0, |c| (c + 1).min(prompt_indices.len() - 1));
        if let Some(&mark_idx) = prompt_indices.get(next) {
            self.prompt_mark_current = Some(next);
            let mark = &marks[mark_idx];
            let search_match = crate::search::SearchMatch {
                start: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
                end: AlacPoint::new(alacritty_terminal::index::Line(mark.line), GridCol(0)),
            };
            crate::search::scroll_to_match(&self.terminal.term, &search_match);
            cx.notify();
        }
    }

    // --- Copy mode ---

    pub(super) fn toggle_copy_mode(&mut self, cx: &mut Context<Self>) {
        if self.copy_mode_active {
            self.exit_copy_mode(false, cx);
        } else {
            self.enter_copy_mode(cx);
        }
    }

    pub(super) fn enter_copy_mode(&mut self, cx: &mut Context<Self>) {
        // Dismiss search if active
        if self.search_active {
            self.dismiss_search(cx);
        }

        let mut term = self.terminal.term.lock();
        let cursor_point = term.renderable_content().cursor.point;
        let display_offset = term.grid().display_offset();
        let screen_lines = term.screen_lines();
        term.selection = None;

        // Convert display-relative cursor to grid coordinates.
        // If the cursor is off-screen (scrolled away), place at viewport center.
        let cursor_display_line = cursor_point.line.0;
        let copy_cursor = if cursor_display_line >= 0 && cursor_display_line < screen_lines as i32 {
            AlacPoint::new(
                GridLine(cursor_display_line - display_offset as i32),
                cursor_point.column,
            )
        } else {
            // Cursor off-screen — place at center of viewport
            let center_display = screen_lines as i32 / 2;
            AlacPoint::new(GridLine(center_display - display_offset as i32), GridCol(0))
        };
        drop(term);

        self.copy_cursor = copy_cursor;
        self.copy_mode_frozen_offset = display_offset;
        self.copy_mode_active = true;

        cx.notify();
    }

    pub(super) fn exit_copy_mode(&mut self, copy_to_clipboard: bool, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();

        if copy_to_clipboard {
            if let Some(text) = term.selection_to_string() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            // After copying, scroll to bottom
            term.scroll_display(AlacScroll::Bottom);
        } else {
            // On cancel, restore the scroll position from before copy mode entry
            let current = term.grid().display_offset();
            let frozen = self.copy_mode_frozen_offset;
            if current != frozen {
                let delta = frozen as i32 - current as i32;
                term.scroll_display(AlacScroll::Delta(delta));
            }
        }

        term.selection = None;
        drop(term);

        self.copy_mode_active = false;
        cx.notify();
    }

    pub(super) fn move_copy_cursor(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        let (cols, top, bottom) = {
            let mut term = self.terminal.term.lock();
            term.selection = None;
            (term.columns(), term.topmost_line(), term.bottommost_line())
        };

        let new_col = (self.copy_cursor.column.0 as i32 + dx)
            .max(0)
            .min(cols as i32 - 1) as usize;
        let new_line = (self.copy_cursor.line.0 + dy).max(top.0).min(bottom.0);
        self.copy_cursor = AlacPoint::new(GridLine(new_line), GridCol(new_col));

        self.ensure_copy_cursor_visible();
        cx.notify();
    }

    pub(super) fn extend_copy_selection(&mut self, dx: i32, dy: i32, cx: &mut Context<Self>) {
        let mut term = self.terminal.term.lock();
        let cols = term.columns();
        let top = term.topmost_line();
        let bottom = term.bottommost_line();

        // Start a new selection if none exists
        if term.selection.is_none() {
            let sel = Selection::new(SelectionType::Simple, self.copy_cursor, Side::Left);
            term.selection = Some(sel);
        }

        // Move cursor and update selection endpoint — all under the same lock
        let new_col = (self.copy_cursor.column.0 as i32 + dx)
            .max(0)
            .min(cols as i32 - 1) as usize;
        let new_line = (self.copy_cursor.line.0 + dy).max(top.0).min(bottom.0);
        self.copy_cursor = AlacPoint::new(GridLine(new_line), GridCol(new_col));

        if let Some(ref mut sel) = term.selection {
            sel.update(self.copy_cursor, Side::Right);
        }
        drop(term);

        self.ensure_copy_cursor_visible();
        cx.notify();
    }

    /// Scroll the view to keep the copy cursor visible, updating the frozen offset.
    fn ensure_copy_cursor_visible(&mut self) {
        let offset = self.copy_mode_frozen_offset as i32;
        let cursor_display_line = self.copy_cursor.line.0 + offset;

        let mut term = self.terminal.term.lock();
        let screen_lines = term.screen_lines() as i32;

        let new_offset = if cursor_display_line < 0 {
            // Cursor is above visible area — scroll up
            Some((offset - cursor_display_line) as usize)
        } else if cursor_display_line >= screen_lines {
            // Cursor is below visible area — scroll down
            let excess = cursor_display_line - screen_lines + 1;
            Some((offset - excess).max(0) as usize)
        } else {
            None
        };

        if let Some(new_offset) = new_offset {
            self.copy_mode_frozen_offset = new_offset;
            let current = term.grid().display_offset();
            let delta = new_offset as i32 - current as i32;
            if delta != 0 {
                term.scroll_display(AlacScroll::Delta(delta));
            }
        }
    }
}
