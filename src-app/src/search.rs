//! In-buffer scrollback search for terminal panes.
//!
//! Searches the alacritty_terminal grid (scrollback + visible area) for
//! plain text or regex matches, returning grid-coordinate spans
//! that TerminalElement can highlight.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column as GridCol, Point as AlacPoint};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use regex::Regex;
use std::sync::Arc;

use crate::terminal::ZedListener;

/// Maximum number of matches to collect before stopping search.
const MAX_MATCHES: usize = 10_000;

/// Maximum query length (bytes).
pub const MAX_QUERY_LEN: usize = 512;

/// A single search match: start and end points in the terminal grid.
#[derive(Clone, Debug)]
pub struct SearchMatch {
    pub start: AlacPoint,
    pub end: AlacPoint,
}

/// Result of a search operation.
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    /// If regex mode and the pattern is invalid, contains the error message.
    pub regex_error: Option<String>,
}

/// Search the terminal's full grid (scrollback + visible) for matches.
/// In plain text mode, performs case-insensitive substring matching.
/// In regex mode, compiles the query as a regex pattern.
pub fn search_term(
    term: &Arc<FairMutex<Term<ZedListener>>>,
    query: &str,
    regex_mode: bool,
) -> SearchResult {
    if query.is_empty() {
        return SearchResult {
            matches: Vec::new(),
            regex_error: None,
        };
    }

    // In regex mode, compile the pattern (case-insensitive)
    let compiled_regex = if regex_mode {
        match Regex::new(&format!("(?i)(?:{})", query)) {
            Ok(re) => Some(re),
            Err(e) => {
                return SearchResult {
                    matches: Vec::new(),
                    regex_error: Some(e.to_string()),
                };
            }
        }
    } else {
        None
    };

    let query_lower = query.to_lowercase();
    let query_char_count = query_lower.chars().count();
    let mut matches = Vec::new();

    let term = term.lock();
    let top = term.topmost_line();
    let bottom = term.bottommost_line();
    let cols = term.columns();

    let mut line = top;
    while line <= bottom {
        let mut line_text = String::with_capacity(cols);
        for col in 0..cols {
            let cell = &term.grid()[AlacPoint::new(line, GridCol(col))];
            let c = cell.c;
            if c == '\0' {
                line_text.push(' ');
            } else {
                line_text.push(c);
            }
        }

        if let Some(re) = &compiled_regex {
            // Regex mode: use find_iter for all non-overlapping matches
            for m in re.find_iter(&line_text) {
                let col_start = line_text[..m.start()].chars().count();
                let match_char_count = line_text[m.start()..m.end()].chars().count();
                if match_char_count == 0 {
                    continue;
                }
                let col_end = col_start + match_char_count - 1;

                matches.push(SearchMatch {
                    start: AlacPoint::new(line, GridCol(col_start)),
                    end: AlacPoint::new(line, GridCol(col_end.min(cols.saturating_sub(1)))),
                });

                if matches.len() >= MAX_MATCHES {
                    return SearchResult {
                        matches,
                        regex_error: None,
                    };
                }
            }
        } else {
            // Plain text mode: case-insensitive substring matching
            let line_lower = line_text.to_lowercase();
            let mut search_from = 0;
            while let Some(byte_pos) = line_lower[search_from..].find(&query_lower) {
                let byte_start = search_from + byte_pos;
                let col_start = line_lower[..byte_start].chars().count();
                let col_end = col_start + query_char_count - 1;

                matches.push(SearchMatch {
                    start: AlacPoint::new(line, GridCol(col_start)),
                    end: AlacPoint::new(line, GridCol(col_end.min(cols.saturating_sub(1)))),
                });

                if matches.len() >= MAX_MATCHES {
                    return SearchResult {
                        matches,
                        regex_error: None,
                    };
                }

                search_from = byte_start
                    + line_lower[byte_start..]
                        .chars()
                        .next()
                        .map_or(1, |c| c.len_utf8());
            }
        }

        line += 1;
    }

    SearchResult {
        matches,
        regex_error: None,
    }
}

/// Compute the display offset for scrolling to a match, and apply the scroll
/// in a single lock acquisition. Returns the applied display_offset.
pub fn scroll_to_match(term: &Arc<FairMutex<Term<ZedListener>>>, m: &SearchMatch) -> usize {
    use alacritty_terminal::grid::Scroll as AlacScroll;

    let mut term = term.lock();
    let bottom = term.bottommost_line();
    let screen_lines = term.screen_lines();

    // lines_from_bottom is always >= 0 because matches come from topmost..=bottommost
    let lines_from_bottom = bottom.0.saturating_sub(m.start.line.0);
    let half_screen = screen_lines / 2;

    let target_offset = if lines_from_bottom <= half_screen as i32 {
        0
    } else {
        (lines_from_bottom - half_screen as i32).max(0) as usize
    };

    let current = term.grid().display_offset();
    let delta = target_offset as i32 - current as i32;
    if delta != 0 {
        term.scroll_display(AlacScroll::Delta(delta));
    }

    target_offset
}
