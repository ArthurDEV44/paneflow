//! Context-menu row helpers shared between the sidebar workspace menu and the
//! title-bar burger menu. Includes the `shortcut_for_description` lookup used
//! to render the keyboard-shortcut label next to each action.
//!
//! Part of the US-025 sidebar decomposition.

use gpui::{
    App, ClickEvent, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, Window,
    div, prelude::*, px,
};

use crate::PaneFlowApp;

impl PaneFlowApp {
    pub(crate) fn shortcut_for_description(&self, description: &str) -> Option<&str> {
        self.effective_shortcuts
            .iter()
            .find(|entry| entry.description == description)
            .map(|entry| entry.key.as_str())
    }

    pub(crate) fn render_context_menu_item(
        &self,
        id: SharedString,
        label: &str,
        shortcut: Option<SharedString>,
        ui: crate::theme::UiColors,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(ui.text)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(on_click)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label.to_string()),
            )
            .when_some(shortcut, |d, shortcut| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(ui.muted)
                        .child(shortcut),
                )
            })
    }
}
