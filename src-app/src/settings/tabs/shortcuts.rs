//! "Shortcuts" settings tab — lists every rebindable action and lets the
//! user capture a new key combination for any row.
//!
//! Key capture is driven by `settings::keyboard::handle_shortcut_recording`;
//! this file owns only the list rendering + per-row click handlers.
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::{config_writer, keybindings};

use super::super::window::SettingsWindow;

impl SettingsWindow {
    pub(crate) fn render_shortcuts_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let recording_idx = self.recording_shortcut_idx;

        let section_header = div()
            .flex()
            .flex_row()
            .items_end()
            .justify_between()
            .pb(px(20.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(ui.muted)
                            .child("KEYBOARD"),
                    )
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(ui.text)
                            .child("Shortcuts"),
                    ),
            )
            .child(
                div()
                    .id("reset-shortcuts")
                    .px(px(12.))
                    .py(px(5.))
                    .rounded(px(6.))
                    .cursor(CursorStyle::PointingHand)
                    .border_1()
                    .border_color(ui.border)
                    .hover(|s| s.bg(ui.subtle).text_color(ui.text))
                    .text_size(px(12.))
                    .text_color(ui.muted)
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        config_writer::reset_shortcuts();
                        let config = paneflow_config::loader::load_config();
                        keybindings::apply_keybindings(cx, &config.shortcuts);
                        this.effective_shortcuts =
                            keybindings::effective_shortcuts(&config.shortcuts);
                        this.recording_shortcut_idx = None;
                        cx.notify();
                    }))
                    .child("Reset to defaults"),
            );

        let mut list = div()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.surface)
            .overflow_hidden();

        let total = self.effective_shortcuts.len();
        for (i, entry) in self.effective_shortcuts.iter().enumerate() {
            let is_recording = recording_idx == Some(i);
            let is_last = i + 1 == total;

            let key_badge = if is_recording {
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .bg(ui.text)
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.base)
                    .child("Press a key…")
            } else {
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(ui.border)
                    .bg(ui.base)
                    .text_size(px(11.))
                    .font_family("monospace")
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(ui.text)
                    .child(entry.key.clone())
            };

            let mut row = div()
                .id(("shortcut", i))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .cursor(CursorStyle::PointingHand)
                .hover(|s| s.bg(ui.subtle))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.recording_shortcut_idx = Some(i);
                    cx.clear_key_bindings();
                    this.settings_focus.focus(window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(ui.text)
                        .truncate()
                        .child(entry.description.clone()),
                )
                .child(key_badge);

            if !is_last {
                row = row.border_b_1().border_color(ui.border);
            }

            list = list.child(row);
        }

        let hint = div()
            .pt(px(10.))
            .text_size(px(11.))
            .text_color(ui.muted)
            .child("Click a row to record a new shortcut. Escape to cancel.");

        div()
            .flex()
            .flex_col()
            .child(section_header)
            .child(list)
            .child(hint)
    }
}
