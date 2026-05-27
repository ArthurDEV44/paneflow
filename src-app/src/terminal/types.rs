//! Neutral type definitions shared between `terminal` (logic) and
//! `terminal_element` (rendering).
//!
//! Pulling these types out of `terminal_element.rs` breaks the circular
//! coupling where `terminal.rs` referenced `crate::terminal_element::…`
//! for hyperlink / search / copy-mode state. Both modules now depend on
//! this neutral leaf, allowing further decomposition (US-005 onward).

/// A search match highlight to be painted by TerminalElement.
pub struct SearchHighlight {
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
    pub is_active: bool,
}

/// Where a hyperlink was detected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HyperlinkSource {
    /// Explicit OSC 8 escape sequence from the program.
    Osc8,
    /// Regex pattern match on terminal output.
    Regex,
    /// Markdown file path (`.md` / `.markdown`) — opens in the in-pane
    /// markdown viewer via `TerminalEvent::OpenMarkdownPath`.
    FilePath,
    /// Source-code file path (`.rs`, `.ts`, `.py`, ...) optionally followed
    /// by `:line[:col]`. Opens in the user's `$VISUAL`/`$EDITOR` (or a probed
    /// fallback) via `TerminalEvent::OpenCodePath`. `uri` holds the resolved
    /// absolute path; `line` / `col` carry the optional location captured
    /// from `path:42` or `path:42:7` style references that compilers, test
    /// runners, and linters emit.
    CodePath,
}

/// A detected OSC 8 hyperlink zone spanning one or more cells.
/// Fields are populated here (US-014) and consumed by hover/click (US-015/US-016).
#[allow(dead_code)]
pub struct HyperlinkZone {
    pub uri: String,
    pub id: String,
    pub start: alacritty_terminal::index::Point,
    pub end: alacritty_terminal::index::Point,
    /// Whether this URL's scheme is in the openable allowlist.
    pub is_openable: bool,
    /// How this hyperlink was detected (OSC 8 takes priority over regex).
    pub source: HyperlinkSource,
    /// 1-based line number for `CodePath` matches (`file.rs:42` → `Some(42)`).
    /// `None` for `Osc8`, `Regex`, `FilePath`, and `CodePath` with no `:line`
    /// suffix in the matched text.
    pub line: Option<u32>,
    /// 1-based column number for `CodePath` matches (`file.rs:42:7` →
    /// `Some(7)`). Always `None` when `line` is `None`.
    pub col: Option<u32>,
}

/// Copy mode cursor state for rendering.
pub struct CopyModeCursorState {
    /// Grid-coordinate line of the copy cursor (current/end of selection)
    pub grid_line: i32,
    /// Column of the copy cursor
    pub col: usize,
    /// Grid-coordinate line of the selection anchor (start), when a selection is active.
    /// Rendered as a distinct tmux-style marker so the user can see where the selection began.
    pub anchor_grid_line: Option<i32>,
    /// Column of the selection anchor.
    pub anchor_col: usize,
}
