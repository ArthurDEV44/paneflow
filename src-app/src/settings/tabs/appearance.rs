//! "Appearance" settings tab — font family selector with an inline-
//! expanding dropdown. "Reset to defaults" sits inline in the section header.
//!
//! Font list enumeration comes from `crate::fonts::load_mono_fonts()`.
//! Theme selection lives in the title bar menu (`main.rs`).

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, Styled,
    deferred, div, prelude::*, px, svg,
};

use crate::PaneFlowApp;
use crate::settings::components::{
    secondary_button, section_header_with_action, setting_card, setting_text,
};

impl PaneFlowApp {
    pub(crate) fn render_appearance_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // US-016: read the cached config (no per-frame `load_config()`).
        let config = &self.cached_config;
        let ui = crate::theme::ui_colors();
        let current_font =
            crate::terminal::element::resolve_font_family(config.font_family.as_deref());

        let reset_btn = secondary_button(
            "reset-appearance",
            "Reset to defaults",
            ui,
            cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.font_dropdown_open = false;
                this.font_search.clear();
                // US-016: clear both fields in the cache + persist off-thread.
                this.persist_setting(false, "font_family", serde_json::Value::Null, cx);
                this.persist_setting(false, "theme", serde_json::Value::Null, cx);
                crate::theme::invalidate_theme_cache();
            }),
        );

        let header = section_header_with_action(ui, "Font", reset_btn);

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
            .bg(ui.base)
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.border_color(ui.muted))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.font_dropdown_open = !this.font_dropdown_open;
                this.font_search.clear();
                if this.font_dropdown_open && this.mono_font_names.is_empty() {
                    // US-016: enumerate fonts (the `fc-list` subprocess on
                    // Linux/macOS) off the main thread; the dropdown opens
                    // empty and fills in when the scan lands.
                    cx.spawn(async move |this, cx| {
                        let fonts = smol::unblock(crate::fonts::load_mono_fonts).await;
                        let _ = this.update(cx, |this, cx| {
                            this.mono_font_names = fonts;
                            cx.notify();
                        });
                    })
                    .detach();
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
        // the content below instead of pushing it down.
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
                            this.font_dropdown_open = false;
                            this.font_search.clear();
                            // US-016: cache-mutate + notify + off-thread persist.
                            this.persist_setting(
                                false,
                                "font_family",
                                serde_json::Value::String(name_owned.clone()),
                                cx,
                            );
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
            .px(px(12.))
            .py(px(10.))
            .child(setting_text(
                ui,
                "Font family",
                "Choose the monospace font used by every terminal. \
                 Search the dropdown to filter the list of installed fonts.",
            ))
            .child(div().flex_shrink_0().child(font_trigger));

        let font_card = setting_card(ui).child(font_row);

        div().flex().flex_col().child(header).child(font_card)
    }
}
