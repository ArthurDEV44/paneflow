//! Semantic markdown palette derived from the active `TerminalTheme`.
//!
//! No hardcoded colors. The terminal theme owns the user-visible colors;
//! markdown borrows from it so a theme switch repaints everything.

use gpui::Hsla;

use crate::theme::{TerminalTheme, active_theme};

/// Resolved palette for a single render pass. Built once per `Render` call
/// from the current `active_theme()` snapshot — `Hsla` is `Copy`, so the
/// whole struct can be passed by value to per-block helpers without lifetime
/// pressure.
#[derive(Clone, Copy)]
pub(crate) struct MarkdownPalette {
    pub background: Hsla,
    /// Body text — same as terminal foreground for parity with shell output.
    pub body: Hsla,
    /// Heading color — full-strength terminal foreground for hierarchy.
    pub heading: Hsla,
    /// Code-block / inline-code background, derived from `ansi_background`
    /// for visual continuity with terminal panes.
    pub code_bg: Hsla,
    pub code_fg: Hsla,
    /// Border / accent for blockquote left rail.
    pub blockquote_border: Hsla,
    pub blockquote_text: Hsla,
    /// Link color (matches the terminal's `link_text` so URL highlighting is
    /// consistent across surfaces).
    pub link: Hsla,
    /// Subtle hairline color for table borders + horizontal rules.
    pub rule: Hsla,
}

impl MarkdownPalette {
    pub(crate) fn from_active() -> Self {
        Self::from_terminal(&active_theme())
    }

    pub(crate) fn from_terminal(t: &TerminalTheme) -> Self {
        Self {
            background: t.background,
            body: t.foreground,
            heading: t.bright_foreground,
            code_bg: t.ansi_background,
            code_fg: t.foreground,
            blockquote_border: t.link_text,
            blockquote_text: t.dim_foreground,
            link: t.link_text,
            rule: t.dim_foreground,
        }
    }
}
