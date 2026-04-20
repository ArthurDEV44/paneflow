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
            .items_center()
            .justify_between()
            .pb(px(16.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child("KEYBOARD SHORTCUTS"),
            )
            .child(
                div()
                    .id("reset-shortcuts")
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor(CursorStyle::PointingHand)
                    .bg(ui.subtle)
                    .hover(|s| s.bg(ui.overlay))
                    .text_size(px(12.))
                    .text_color(ui.text)
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

        let mut list = div().flex().flex_col();

        for (i, entry) in self.effective_shortcuts.iter().enumerate() {
            let is_recording = recording_idx == Some(i);

            let key_badge = if is_recording {
                div()
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(ui.overlay)
                    .text_size(px(12.))
                    .text_color(ui.accent)
                    .child("Press a key...")
            } else {
                div()
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(ui.subtle)
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .child(entry.key.clone())
            };

            list = list.child(
                div()
                    .id(("shortcut", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(4.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(ui.overlay))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.recording_shortcut_idx = Some(i);
                        cx.clear_key_bindings();
                        this.settings_focus.focus(window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(ui.text)
                            .child(entry.description.clone()),
                    )
                    .child(key_badge),
            );
        }

        div().flex().flex_col().child(section_header).child(list)
    }
}
