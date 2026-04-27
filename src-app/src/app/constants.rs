//! Layout & timing constants shared across the app shell.
//!
//! Extracted from `main.rs` per US-002 (anti edit-thrashing). All items
//! are `pub(crate)` and re-exported at the crate root via `main.rs` so
//! existing `crate::SIDEBAR_WIDTH` / `crate::TOAST_HOLD_MS` references in
//! sibling modules keep compiling without import churn.

use gpui::{Pixels, px};

/// Sidebar width in pixels — shared between sidebar and title bar for alignment.
pub(crate) const SIDEBAR_WIDTH: f32 = 240.;

/// Claude Code spinner glyphs — same characters Claude renders in the terminal.
pub(crate) const CLAUDE_SPINNER_FRAMES: [char; 6] = ['·', '✻', '✽', '✶', '✳', '✢'];

/// Codex spinner glyphs — pulsing dot from the dots animation variant.
pub(crate) const CODEX_SPINNER_FRAMES: [char; 4] = ['●', '○', '◉', '○'];

/// Toast animation durations (ms). The `hold_ms` carried on each `Toast`
/// must match the dismiss timer in `push_toast` — otherwise the exit
/// animation plays early and the element persists as a ghost.
pub(crate) const TOAST_ENTER_MS: u64 = 180;
pub(crate) const TOAST_HOLD_MS: u64 = 1440;
pub(crate) const TOAST_EXIT_MS: u64 = 180;

/// Maximum number of closed-pane records kept for undo-close-pane (US-014).
pub(crate) const MAX_CLOSED_PANES: usize = 5;

/// Width of the invisible border zone used for CSD edge/corner resize handles.
pub(crate) const RESIZE_BORDER: Pixels = px(10.0);
