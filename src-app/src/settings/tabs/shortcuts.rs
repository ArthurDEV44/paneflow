//! "Shortcuts" settings tab — Zed-style list of every rebindable action
//! with click-to-record key capture.
//!
//! Section header is rendered inline with a "Reset to defaults" action
//! button on the right. Each shortcut row is a Zed-style flex row
//! (description left, key badge right) separated by 1px hairlines.
//! Click capture is driven by `settings::keyboard::handle_shortcut_recording`.

use gpui::{
    div, prelude::*, px, ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement,
    ParentElement, Styled,
};

use crate::settings::components::{hairline, secondary_button};
use crate::{config_writer, keybindings};

use super::super::window::SettingsWindow;

impl SettingsWindow {
    pub(crate) fn render_shortcuts_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let recording_idx = self.recording_shortcut_idx;

        let header = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mb(px(4.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(ui.muted)
                            .child("KEYBOARD"),
                    )
                    .child(secondary_button(
                        "reset-shortcuts",
                        "Reset to defaults",
                        ui,
                        cx.listener(|this, _: &ClickEvent, _w, cx| {
                            config_writer::reset_shortcuts();
                            let config = paneflow_config::loader::load_config();
                            keybindings::apply_keybindings(cx, &config.shortcuts);
                            this.effective_shortcuts =
                                keybindings::effective_shortcuts(&config.shortcuts);
                            this.recording_shortcut_idx = None;
                            cx.notify();
                        }),
                    )),
            )
            .child(div().h(px(1.)).w_full().bg(ui.border));

        let mut list = div().flex().flex_col();

        let total = self.effective_shortcuts.len();
        for (i, entry) in self.effective_shortcuts.iter().enumerate() {
            let is_recording = recording_idx == Some(i);
            let is_last = i + 1 == total;

            let key_badge = if is_recording {
                div()
                    .px(px(10.))
                    .py(px(3.))
                    .rounded(px(5.))
                    .bg(ui.text)
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.base)
                    .child("Press a key…")
            } else {
                div()
                    .px(px(10.))
                    .py(px(3.))
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

            let row = div()
                .id(("shortcut", i))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.))
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

            list = list.child(row);
            if !is_last {
                list = list.child(hairline(ui));
            }
        }

        let hint = div()
            .pt(px(14.))
            .text_size(px(11.))
            .text_color(ui.muted)
            .child("Click a row to record a new shortcut. Escape to cancel.");

        div()
            .flex()
            .flex_col()
            .child(header)
            .child(list)
            .child(hint)
    }
}
