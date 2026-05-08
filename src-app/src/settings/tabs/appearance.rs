//! "Appearance" settings tab — Zed-style setting row for the font family
//! selector with an inline-expanding dropdown, plus a live preview pane
//! and a "Reset to defaults" action button in the section header.
//!
//! Font list enumeration comes from `crate::fonts::load_mono_fonts()`.
//! Theme selection lives in the title bar menu (`main.rs`).

use gpui::{
    deferred, div, prelude::*, px, svg, ClickEvent, Context, CursorStyle, InteractiveElement,
    IntoElement, ParentElement, Styled,
};

use crate::config_writer;
use crate::settings::components::{hairline, secondary_button, setting_text};

use super::super::window::SettingsWindow;

impl SettingsWindow {
    pub(crate) fn render_appearance_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current_font =
            crate::terminal::element::resolve_font_family(config.font_family.as_deref());

        // Section header with inline "Reset to defaults" action on the right.
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
                            .child("APPEARANCE"),
                    )
                    .child(secondary_button(
                        "reset-appearance",
                        "Reset to defaults",
                        ui,
                        cx.listener(|this, _: &ClickEvent, _w, cx| {
                            config_writer::save_config_value(
                                "font_family",
                                serde_json::Value::Null,
                            );
                            config_writer::save_config_value("theme", serde_json::Value::Null);
                            crate::theme::invalidate_theme_cache();
                            this.font_dropdown_open = false;
                            this.font_search.clear();
                            cx.notify();
                        }),
                    )),
            )
            .child(div().h(px(1.)).w_full().bg(ui.border));

        // Setting row: font family label/description on the left,
        // compact dropdown trigger on the right.
        let trigger_label = if self.font_dropdown_open {
            if self.font_search.is_empty() {
                "Search fonts…".to_string()
            } else {
                format!("{}|", self.font_search)
            }
        } else {
            current_font.clone()
        };
        let trigger_label_color = if self.font_dropdown_open && self.font_search.is_empty() {
            ui.muted
        } else {
            ui.text
        };

        let mut font_trigger = div()
            .id("font-family-trigger")
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(10.))
            .py(px(5.))
            .min_w(px(180.))
            .max_w(px(260.))
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
                    .text_size(px(12.))
                    .text_color(trigger_label_color)
                    .font_family(if self.font_dropdown_open {
                        "monospace".to_string()
                    } else {
                        current_font.clone()
                    })
                    .truncate()
                    .child(trigger_label),
            )
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            );

        // True popover: dropdown is rendered as a deferred absolutely-positioned
        // child of the trigger (which is `.relative()` above), so it floats over
        // the preview pane below instead of pushing it down.
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
                .min_w(px(220.))
                .max_w(px(320.))
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
                        .py(px(6.))
                        .cursor(CursorStyle::PointingHand)
                        .text_size(px(12.))
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
                                    .size(px(12.))
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

            font_trigger = font_trigger.child(
                deferred(
                    div()
                        .absolute()
                        .top(px(30.))
                        .right(px(0.))
                        .occlude()
                        .child(dropdown),
                )
                .with_priority(1),
            );
        }

        let font_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .py(px(12.))
            .child(setting_text(
                ui,
                "Font family",
                "Choose the monospace font used by every terminal. \
                 Search the dropdown to filter the list of installed fonts.",
            ))
            .child(div().flex_shrink_0().child(font_trigger));

        let sections = div().flex().flex_col().child(header).child(font_row);

        let preview = div()
            .mt(px(12.))
            .pt(px(12.))
            .child(hairline(ui))
            .child(
                div()
                    .mt(px(12.))
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.muted)
                    .pb(px(8.))
                    .child("PREVIEW"),
            )
            .child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(8.))
                    .bg(ui.preview_bg)
                    .border_1()
                    .border_color(ui.border)
                    .font_family(current_font.clone())
                    .text_size(px(13.))
                    .text_color(ui.text)
                    .child(
                        "The quick brown fox jumps over the lazy dog\n\
                         ABCDEFGHIJKLM 0123456789 {}[]()",
                    ),
            );

        sections.child(preview)
    }
}
