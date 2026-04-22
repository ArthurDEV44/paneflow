//! "Appearance" settings tab — font family dropdown with typeahead search,
//! live preview pane, and reset-to-defaults button.
//!
//! Font list enumeration comes from `crate::fonts::load_mono_fonts()`.
//! Theme selection lives in the title bar menu (`main.rs`).
//!
//! Extracted from `settings_window.rs` per US-021 of the src-app refactor PRD.

use gpui::{
    div, prelude::*, px, svg, ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement,
    ParentElement, Styled,
};

use crate::config_writer;

use super::super::window::SettingsWindow;

impl SettingsWindow {
    pub(crate) fn render_appearance_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current_font =
            crate::terminal::element::resolve_font_family(config.font_family.as_deref());

        let section_header = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.muted)
                    .child("INTERFACE"),
            )
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child("Appearance"),
            );

        let font_label = div()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(ui.muted)
            .pb(px(8.))
            .child("Font Family");

        let font_value_text = if self.font_dropdown_open {
            if self.font_search.is_empty() {
                "Search fonts…".to_string()
            } else {
                format!("{}|", self.font_search)
            }
        } else {
            current_font.clone()
        };

        let font_value_color = if self.font_dropdown_open && self.font_search.is_empty() {
            ui.muted
        } else {
            ui.text
        };

        let font_badge = div()
            .id("font-family-badge")
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .px(px(12.))
            .py(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.surface)
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.border_color(ui.muted))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.font_dropdown_open = !this.font_dropdown_open;
                this.font_search.clear();
                if this.font_dropdown_open && this.mono_font_names.is_empty() {
                    this.mono_font_names = crate::fonts::load_mono_fonts();
                }
                this.settings_focus.focus(window, cx);
                cx.notify();
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .text_color(font_value_color)
                    .font_family(if self.font_dropdown_open {
                        "monospace".to_string()
                    } else {
                        current_font.clone()
                    })
                    .truncate()
                    .child(font_value_text),
            )
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            );

        let mut font_row = div()
            .flex()
            .flex_col()
            .pb(px(20.))
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
                .mt(px(6.))
                .rounded(px(6.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .max_h(px(260.))
                .overflow_y_scroll();

            for (i, name) in filtered.iter().enumerate() {
                let name_owned = (*name).clone();
                let is_current = **name == current_font;
                dropdown = dropdown.child(
                    div()
                        .id(("font", i))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(7.))
                        .cursor(CursorStyle::PointingHand)
                        .text_size(px(13.))
                        .when(is_current, |d| {
                            d.bg(ui.subtle)
                                .text_color(ui.text)
                                .font_weight(gpui::FontWeight::MEDIUM)
                        })
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
                        .child(div().flex_1().min_w_0().truncate().child((*name).clone()))
                        .when(is_current, |d| {
                            d.child(
                                svg()
                                    .size(px(13.))
                                    .flex_none()
                                    .path("icons/checks.svg")
                                    .text_color(ui.text),
                            )
                        }),
                );
            }

            if filtered.is_empty() {
                dropdown = dropdown.child(
                    div()
                        .px(px(12.))
                        .py(px(10.))
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
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(ui.muted)
                    .pb(px(8.))
                    .child("Preview"),
            )
            .child(
                div()
                    .px(px(18.))
                    .py(px(14.))
                    .rounded(px(8.))
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
                    .items_end()
                    .justify_between()
                    .pb(px(20.))
                    .child(section_header)
                    .child(reset_btn),
            )
            .child(font_row)
            .child(preview)
    }
}
