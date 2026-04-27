//! Shared cursor-blink phase (US-006, terminal-rendering-parity PRD).
//!
//! Replaces N independent `smol::Timer::after(530ms)` loops (one per
//! `TerminalView`) with a single app-scoped tick that every visible pane
//! observes. All cursors blink in phase as a result — the polish goal of
//! the story.
//!
//! ## Architecture
//!
//! - One [`BlinkPhase`] entity is constructed in `PaneFlowApp::new` and
//!   installed as a [`BlinkPhaseGlobal`] via `cx.set_global(...)`.
//! - One bootstrap-spawned `cx.spawn` loop toggles `phase.visible` every
//!   [`CURSOR_BLINK_INTERVAL`] and calls `cx.notify()`.
//! - Each `TerminalView::with_cwd` calls `cx.global::<BlinkPhaseGlobal>()`
//!   to grab the entity, then `cx.observe(&phase, …)` so its
//!   `cursor_visible` mirrors `phase.visible` (subject to the per-pane
//!   `cursor_blinking` / `exited` / `SUPPRESS_REPAINTS` short-circuits).
//!
//! Using a GPUI global keeps the constructor signatures of `TerminalView`
//! unchanged across all 18 call sites — adding a parameter would have
//! cascaded into `Pane::new`, `restore_workspaces`, every `workspace_ops`
//! method, etc., for a feature that is conceptually a single app-wide
//! singleton.

use std::time::Duration;

use gpui::{Entity, Global};

/// Interval between blink-phase toggles. Matches the previous per-terminal
/// `CURSOR_BLINK_INTERVAL_MS = 530` constant from `view.rs`.
pub const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(530);

/// The shared blink phase. A single instance is owned by the
/// [`BlinkPhaseGlobal`]; every `TerminalView` observes the same entity.
pub struct BlinkPhase {
    /// `true` when cursors should render visible, `false` when they
    /// should render hidden. Toggles every [`CURSOR_BLINK_INTERVAL`].
    pub visible: bool,
}

impl Default for BlinkPhase {
    fn default() -> Self {
        // Start visible so a freshly-launched app shows the cursor on the
        // first frame, before the first tick fires.
        Self { visible: true }
    }
}

/// GPUI global wrapping the shared [`BlinkPhase`] entity. Installed in
/// `PaneFlowApp::new` via `cx.set_global(BlinkPhaseGlobal(...))`; read by
/// `TerminalView::with_cwd` via `cx.global::<BlinkPhaseGlobal>()`.
pub struct BlinkPhaseGlobal(pub Entity<BlinkPhase>);

impl Global for BlinkPhaseGlobal {}

// `CURSOR_BLINK_INTERVAL = 530 ms` is a deliberate UX value matching the
// pre-US-006 per-terminal `CURSOR_BLINK_INTERVAL_MS`. Any change should be
// reviewed as a UX decision, not an accidental refactor. (No unit test
// here — asserting `CONST == literal` against the same module's literal
// would be a tautology.)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_visible() {
        // First-frame invariant: cursor must be visible at startup so the
        // user sees a caret immediately rather than a hidden one for the
        // first 530 ms.
        assert!(BlinkPhase::default().visible);
    }
}
