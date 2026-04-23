//! Left-side navigation for the settings window — one row per tab
//! (`Shortcuts`, `Appearance`), highlighting the active section and
//! resetting per-tab editing state on switch.
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::keybindings;

use super::window::{SETTINGS_SIDEBAR_WIDTH, SettingsSection, SettingsWindow, settings_sidebar_bg};

impl SettingsWindow {
    pub(crate) fn render_settings_sidebar(
        &self,
        active: SettingsSection,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sections = [
            ("Shortcuts", SettingsSection::Shortcuts),
            ("Appearance", SettingsSection::Appearance),
            ("Privacy", SettingsSection::Privacy),
        ];

        let mut nav = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .w(SETTINGS_SIDEBAR_WIDTH)
            .h_full()
            .border_r_1()
            .border_color(ui.border)
            .bg(settings_sidebar_bg())
            .pt(px(10.))
            .px(px(8.));

        for (label, section) in sections {
            let is_active = section == active;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{label}")))
                    .relative()
                    .pl(px(12.))
                    .pr(px(10.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .font_weight(if is_active {
                        gpui::FontWeight::MEDIUM
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .cursor(CursorStyle::PointingHand)
                    .when(is_active, |d| {
                        d.bg(ui.overlay).text_color(ui.text).child(
                            div()
                                .absolute()
                                .left(px(3.))
                                .top(px(7.))
                                .bottom(px(7.))
                                .w(px(2.))
                                .rounded(px(1.))
                                .bg(ui.text),
                        )
                    })
                    .when(!is_active, |d| {
                        d.text_color(ui.muted)
                            .hover(|s| s.bg(ui.subtle).text_color(ui.text))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.section = section;
                        this.font_dropdown_open = false;
                        this.font_search.clear();
                        if this.recording_shortcut_idx.is_some() {
                            this.recording_shortcut_idx = None;
                            let config = paneflow_config::loader::load_config();
                            keybindings::apply_keybindings(cx, &config.shortcuts);
                        }
                        cx.notify();
                    }))
                    .child(label),
            );
        }

        nav
    }
}
