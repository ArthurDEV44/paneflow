//! Notification + toast types, helpers and rendering.
//!
//! Extracted from `main.rs` per US-002. Owns:
//! - `Notification`: bell-menu entries pushed by AI hook lifecycle methods.
//! - `Toast`: ephemeral bottom-right confirmation/error pop-ups.
//! - `ToastAction`: optional buttons inside a toast (retry/open-releases).
//! - `show_toast` / `show_update_error_toast` / `push_toast`: convenience
//!   helpers attached to `PaneFlowApp`.
//! - `render_toast`: the deferred rendering block used by `Render for
//!   PaneFlowApp` to paint the active toast.

use gpui::{
    Animation, AnimationExt, AnyElement, AsyncApp, Context, FontWeight, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, WeakEntity, deferred, div, ease_in_out, prelude::*, px,
};

use crate::app::constants::{TOAST_ENTER_MS, TOAST_EXIT_MS, TOAST_HOLD_MS};
use crate::theme::UiColors;
use crate::{PaneFlowApp, StartSelfUpdate, ai_types, update};

/// A notification from Claude Code state changes, displayed in the bell menu.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct Notification {
    pub(crate) workspace_id: u64,
    pub(crate) workspace_title: String,
    pub(crate) message: String,
    pub(crate) kind: ai_types::AiToolState,
    pub(crate) timestamp: std::time::Instant,
    pub(crate) read: bool,
}

pub(crate) struct Toast {
    pub(crate) message: String,
    /// Optional action buttons shown inside the toast. Empty for the
    /// ordinary confirmation toasts ("Path copied", etc); populated for
    /// update-failure toasts (US-013) with Retry / "Open releases" buttons.
    pub(crate) actions: Vec<ToastAction>,
    /// How long the "hold" phase of the toast animation lasts, in ms.
    /// Must match the auto-dismiss timer in [`PaneFlowApp::push_toast`] —
    /// otherwise the exit animation plays early and the element persists
    /// as a ghost at opacity 0 until the dismiss task fires.
    pub(crate) hold_ms: u64,
}

#[derive(Clone)]
pub(crate) enum ToastAction {
    /// "Retry" — re-dispatches the `StartSelfUpdate` action. The action
    /// handler's existing guards (busy check, attempt counter) apply.
    RetryUpdate,
    /// "Open releases" — opens the given URL in the user's browser.
    /// Used for the 4th-attempt fallback (AC: "Download manually from the
    /// releases page").
    OpenReleasesPage(String),
}

impl PaneFlowApp {
    pub(crate) fn show_toast(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.push_toast(message.into(), Vec::new(), TOAST_HOLD_MS, cx);
    }

    /// Surface an update failure as a toast with a "Retry" action button
    /// (US-013). Hold is extended so the user has time to click the button
    /// before auto-dismiss.
    pub(crate) fn show_update_error_toast(
        &mut self,
        err: &update::UpdateError,
        cx: &mut Context<Self>,
    ) {
        self.push_toast(
            err.user_message(),
            vec![ToastAction::RetryUpdate],
            TOAST_HOLD_MS * 4,
            cx,
        );
    }

    pub(crate) fn push_toast(
        &mut self,
        message: String,
        actions: Vec<ToastAction>,
        hold_ms: u64,
        cx: &mut Context<Self>,
    ) {
        self.toast = Some(Toast {
            message,
            actions,
            hold_ms,
        });
        cx.notify();

        // Dropping the previous task cancels its timer automatically.
        let total = TOAST_ENTER_MS + hold_ms + TOAST_EXIT_MS;
        self._toast_task = Some(cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                smol::Timer::after(std::time::Duration::from_millis(total)).await;
                let _ = cx.update(|cx| {
                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                        app.toast = None;
                        app._toast_task = None;
                        cx.notify();
                    })
                });
            },
        ));
    }

    /// Build the deferred element that paints the active toast at the
    /// bottom-right of the window. Caller is responsible for the
    /// `if let Some(toast) = &self.toast` guard.
    pub(crate) fn render_toast(&self, toast: &Toast, ui: UiColors) -> AnyElement {
        let has_actions = !toast.actions.is_empty();
        // Error toasts (those with action buttons) get a warning glyph
        // + wider panel so the message + button have room. Ordinary
        // confirmation toasts keep the tight 320-px "✓ …" layout.
        let (icon, max_w) = if has_actions {
            ("!", px(420.))
        } else {
            ("✓", px(320.))
        };

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .w(px(18.))
                    .h(px(18.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(ui.accent)
                    .child(icon),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ui.text)
                    .child(toast.message.clone()),
            );

        let action_row = if has_actions {
            let mut row = div().flex().flex_row().gap(px(8.)).mt(px(8.)).pl(px(28.));
            for (idx, action) in toast.actions.iter().enumerate() {
                let (label, button_id): (&str, String) = match action {
                    ToastAction::RetryUpdate => ("Retry", format!("toast-retry-{idx}")),
                    ToastAction::OpenReleasesPage(_) => {
                        ("Open releases", format!("toast-releases-{idx}"))
                    }
                };
                let action_clone = action.clone();
                let btn = div()
                    .id(SharedString::from(button_id))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(ui.accent)
                    .text_color(ui.accent)
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.7))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| match &action_clone {
                        ToastAction::RetryUpdate => {
                            window.dispatch_action(Box::new(StartSelfUpdate), cx);
                        }
                        ToastAction::OpenReleasesPage(url) => {
                            let _ = open::that(url);
                        }
                    });
                row = row.child(btn);
            }
            Some(row)
        } else {
            None
        };

        let hold_ms = toast.hold_ms;
        deferred(
            div()
                .id("copy-toast")
                .absolute()
                .right(px(20.))
                .bottom(px(20.))
                .max_w(max_w)
                .px(px(14.))
                .py(px(10.))
                .rounded(px(8.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .shadow_lg()
                .text_sm()
                .text_color(ui.text)
                .flex()
                .flex_col()
                .child(header)
                .children(action_row)
                .with_animations(
                    SharedString::from("copy-toast-anim"),
                    vec![
                        Animation::new(std::time::Duration::from_millis(TOAST_ENTER_MS))
                            .with_easing(ease_in_out),
                        Animation::new(std::time::Duration::from_millis(hold_ms)),
                        Animation::new(std::time::Duration::from_millis(TOAST_EXIT_MS))
                            .with_easing(ease_in_out),
                    ],
                    |toast_el, stage, delta| match stage {
                        0 => {
                            let lift = 8.0 * (1.0 - delta);
                            toast_el.opacity(delta).bottom(px(20.0 + lift))
                        }
                        1 => toast_el.opacity(1.0).bottom(px(20.0)),
                        _ => {
                            let drop = 8.0 * delta;
                            toast_el.opacity(1.0 - delta).bottom(px(20.0 + drop))
                        }
                    },
                ),
        )
        .priority(2)
        .into_any_element()
    }
}
