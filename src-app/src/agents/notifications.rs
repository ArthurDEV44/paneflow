//! Window/panel visibility flags for the Agents view.
//!
//! These two `AtomicBool`s tracked whether the Paneflow window was the
//! user-focused surface so the in-app ACP chat could fire an OS toast
//! on turn-end while the user was looking elsewhere. The chat (and its
//! `RuntimeEvent::TurnEnded` signal source) was removed, so the
//! firing path no longer exists - but the cheap visibility setters are
//! kept (and still called from the window-activation observer and the
//! mode switcher) so a future terminal-driven notification can re-read
//! them without re-threading the call sites.

use std::sync::atomic::{AtomicBool, Ordering};

/// Window-active gate updated by `cx.observe_window_activation`.
/// `true` while the OS reports the Paneflow window as the focused one.
static WINDOW_ACTIVE: AtomicBool = AtomicBool::new(true);

/// Agents-panel gate updated by `app::agents_view_actions`. `true`
/// while the user is in `AppMode::Agents`.
static AGENTS_PANEL_VISIBLE: AtomicBool = AtomicBool::new(false);

/// Update the window-active flag. Called from
/// `cx.observe_window_activation` and from the initial activation
/// tick that GPUI fires when the observer registers.
pub fn set_window_active(active: bool) {
    WINDOW_ACTIVE.store(active, Ordering::Relaxed);
}

/// Is the Paneflow window currently the focused surface? EP-004 US-020 reads
/// this on `ai.stop` to suppress the turn-end desktop notification while the
/// user is already looking at Paneflow.
pub fn window_active() -> bool {
    WINDOW_ACTIVE.load(Ordering::Relaxed)
}

/// Update the agents-panel-visible flag. Called from the mode toggle
/// and from the bootstrap when the persisted session restores into
/// agents mode.
pub fn set_agents_panel_visible(visible: bool) {
    AGENTS_PANEL_VISIBLE.store(visible, Ordering::Relaxed);
}
