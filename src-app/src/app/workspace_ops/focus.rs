//! Focus-movement handlers for `PaneFlowApp`.
//!
//! Part of the US-023 workspace_ops decomposition — behaviour identical to
//! the pre-refactor `main.rs` implementation.

use gpui::{Context, Focusable, Window};

use crate::PaneFlowApp;
use crate::layout::FocusDirection;
use crate::{FocusDown, FocusLeft, FocusRight, FocusUp, SWAP_MODE};

impl PaneFlowApp {
    pub(crate) fn handle_focus(
        &mut self,
        dir: FocusDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // When swap mode is active, perform the swap instead of just moving focus
        if let Some(source) = self.swap_source.take() {
            SWAP_MODE.store(false, std::sync::atomic::Ordering::Relaxed);

            if let Some(ws) = self.active_workspace()
                && let Some(root) = &ws.root
            {
                // Move focus to find the target pane
                root.focus_in_direction(dir, window, cx);
                if let Some(target) = root.focused_pane(window, cx)
                    && target != source
                {
                    // Swap the panes in the tree
                    if let Some(ws) = self.active_workspace_mut()
                        && let Some(ref mut root) = ws.root
                    {
                        root.swap_panes(&source, &target);
                    }
                    // Focus the original source pane (now at the target's position)
                    source.read(cx).focus_handle(cx).focus(window, cx);
                }
            }
            self.save_session(cx);
            cx.notify();
            return;
        }

        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
        {
            root.focus_in_direction(dir, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn handle_focus_left(
        &mut self,
        _: &FocusLeft,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_focus(FocusDirection::Left, w, cx);
    }
    pub(crate) fn handle_focus_right(
        &mut self,
        _: &FocusRight,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_focus(FocusDirection::Right, w, cx);
    }
    pub(crate) fn handle_focus_up(&mut self, _: &FocusUp, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Up, w, cx);
    }
    pub(crate) fn handle_focus_down(
        &mut self,
        _: &FocusDown,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_focus(FocusDirection::Down, w, cx);
    }
}
