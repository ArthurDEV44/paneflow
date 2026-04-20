//! "Appearance" settings tab — theme picker, font family dropdown with
//! typeahead search, live preview pane, and reset-to-defaults button.
//!
//! Font list enumeration comes from `crate::fonts::load_mono_fonts()`.
//! Theme changes invalidate the shared theme cache so the main window
//! repaints immediately.
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::config_writer;

use super::super::window::SettingsWindow;

impl SettingsWindow {
    pub(crate) fn render_appearance_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current_font =
            crate::terminal::element::resolve_font_family(config.font_family.as_deref());
        let current_theme = config
            .theme
            .clone()
            .unwrap_or_else(|| "Catppuccin Mocha".to_string());

        let section_header = div()
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(ui.text)
            .child("APPEARANCE");

        let theme_label = div()
            .text_size(px(13.))
            .text_color(ui.muted)
            .pb(px(6.))
            .child("Theme");

        let themes = [
            ("Catppuccin Mocha", "Default (Dark)"),
            ("PaneFlow Light", "Light"),
        ];
        let mut theme_row_inner = div().flex().flex_row().gap(px(8.));

        for (theme_id, label) in themes {
            let is_active = current_theme == theme_id;
            let theme_id_owned = theme_id.to_string();
            theme_row_inner = theme_row_inner.child(
                div()
                    .id(SharedString::from(format!("theme-{theme_id}")))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(13.))
                    .when(is_active, |d| d.bg(ui.accent).text_color(ui.base))
                    .when(!is_active, |d| {
                        d.bg(ui.subtle)
                            .text_color(ui.muted)
                            .hover(|s| s.bg(ui.overlay))
                    })
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _w, cx| {
                        config_writer::save_config_value(
                            "theme",
                            serde_json::Value::String(theme_id_owned.clone()),
                        );
                        crate::theme::invalidate_theme_cache();
                        cx.notify();
                    }))
                    .child(label),
            );
        }

        let theme_row = div()
            .flex()
            .flex_col()
            .pb(px(12.))
            .child(theme_label)
            .child(theme_row_inner);

        let font_label = div()
            .text_size(px(13.))
            .text_color(ui.muted)
            .pb(px(6.))
            .child("Font Family");

        let font_value_text = if self.font_dropdown_open {
            if self.font_search.is_empty() {
                "Search fonts...".to_string()
            } else {
                format!("{}|", self.font_search)
            }
        } else {
            current_font.clone()
        };

        let font_value_color = if self.font_dropdown_open {
            ui.accent
        } else {
            ui.text
        };

        let font_badge = div()
            .id("font-family-badge")
            .px(px(8.))
            .py(px(3.))
            .rounded(px(4.))
            .bg(ui.overlay)
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(ui.subtle))
            .text_size(px(13.))
            .text_color(font_value_color)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.font_dropdown_open = !this.font_dropdown_open;
                this.font_search.clear();
                if this.font_dropdown_open && this.mono_font_names.is_empty() {
                    this.mono_font_names = crate::fonts::load_mono_fonts();
                }
                this.settings_focus.focus(window, cx);
                cx.notify();
            }))
            .child(font_value_text);

        let mut font_row = div()
            .flex()
            .flex_col()
            .pb(px(12.))
            .child(font_label)
            .child(font_badge);

        if self.font_dropdown_open {
            let search = self.font_search.to_lowercase();
            let filtered: Vec<&String> = self
                .mono_font_names
                .iter()
                .filter(|name| search.is_empty() || name.to_lowercase().contains(&search))
                .collect();

            let mut dropdown = div()
                .id("font-dropdown")
                .flex()
                .flex_col()
                .mt(px(4.))
                .rounded(px(6.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .max_h(px(250.))
                .overflow_y_scroll();

            for (i, name) in filtered.iter().enumerate() {
                let name_owned = (*name).clone();
                let is_current = **name == current_font;
                dropdown = dropdown.child(
                    div()
                        .id(("font", i))
                        .px(px(12.))
                        .py(px(6.))
                        .cursor(CursorStyle::PointingHand)
                        .text_size(px(13.))
                        .when(is_current, |d| d.text_color(ui.accent).bg(ui.subtle))
                        .when(!is_current, |d| {
                            d.text_color(ui.text).hover(|s| s.bg(ui.subtle))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            config_writer::save_config_value(
                                "font_family",
                                serde_json::Value::String(name_owned.clone()),
                            );
                            this.font_dropdown_open = false;
                            this.font_search.clear();
                            cx.notify();
                        }))
                        .child((*name).clone()),
                );
            }

            if filtered.is_empty() {
                dropdown = dropdown.child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("No matching fonts"),
                );
            }

            font_row = font_row.child(dropdown);
        }

        let preview = div()
            .pb(px(12.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(ui.muted)
                    .pb(px(6.))
                    .child("Preview"),
            )
            .child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(6.))
                    .bg(ui.preview_bg)
                    .border_1()
                    .border_color(ui.border)
                    .font_family(current_font.clone())
                    .text_size(px(14.))
                    .text_color(ui.text)
                    .child("The quick brown fox jumps over the lazy dog\nABCDEFGHIJKLM 0123456789 {}[]()"),
            );

        let reset_btn = div()
            .id("reset-appearance")
            .px(px(10.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor(CursorStyle::PointingHand)
            .bg(ui.subtle)
            .hover(|s| s.bg(ui.overlay))
            .text_size(px(12.))
            .text_color(ui.text)
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                config_writer::save_config_value("font_family", serde_json::Value::Null);
                config_writer::save_config_value("theme", serde_json::Value::Null);
                crate::theme::invalidate_theme_cache();
                this.font_dropdown_open = false;
                this.font_search.clear();
                cx.notify();
            }))
            .child("Reset to defaults");

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .pb(px(10.))
                    .child(section_header)
                    .child(reset_btn),
            )
            .child(theme_row)
            .child(font_row)
            .child(preview)
    }
}
