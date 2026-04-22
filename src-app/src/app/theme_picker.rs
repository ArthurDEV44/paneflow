//! Theme picker modal — command-palette style selector opened from the
//! title-bar burger menu. Lists bundled themes from `crate::theme::THEMES`
//! with a typeahead filter and keyboard navigation.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, SharedString, Styled, Window, deferred, div, prelude::*, px,
};

use crate::{PaneFlowApp, config_writer};

impl PaneFlowApp {
    /// Resolve the theme currently persisted in config (or the built-in default).
    fn current_theme_name() -> String {
        paneflow_config::loader::load_config()
            .theme
            .unwrap_or_else(|| "Catppuccin Mocha".to_string())
    }

    fn current_theme_index() -> usize {
        let name = Self::current_theme_name();
        crate::theme::THEMES
            .iter()
            .position(|(n, _)| *n == name)
            .unwrap_or(0)
    }

    /// Returns theme names matching the current query (case-insensitive
    /// substring). Matches the `THEMES` table order so defaults appear first.
    fn theme_picker_matches(&self) -> Vec<&'static str> {
        let q = self.theme_picker_query.to_lowercase();
        crate::theme::THEMES
            .iter()
            .filter(|(name, _)| q.is_empty() || name.to_lowercase().contains(&q))
            .map(|(name, _)| *name)
            .collect()
    }

    pub(crate) fn open_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_theme_picker = true;
        self.theme_picker_query.clear();
        // Pre-select the currently applied theme so the list opens on it.
        self.theme_picker_selected_idx = Self::current_theme_index();
        self.theme_picker_focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_theme_picker(&mut self, cx: &mut Context<Self>) {
        self.show_theme_picker = false;
        self.theme_picker_query.clear();
        self.theme_picker_selected_idx = 0;
        cx.notify();
    }

    fn apply_theme_by_name(name: &str) {
        config_writer::save_config_value("theme", serde_json::Value::String(name.to_string()));
        crate::theme::invalidate_theme_cache();
    }

    fn commit_theme_picker_selection(&mut self, cx: &mut Context<Self>) {
        let matches = self.theme_picker_matches();
        if let Some(name) = matches.get(self.theme_picker_selected_idx) {
            Self::apply_theme_by_name(name);
        }
        self.close_theme_picker(cx);
    }

    pub(crate) fn handle_theme_picker_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let len = self.theme_picker_matches().len();

        match key {
            "escape" => self.close_theme_picker(cx),
            "enter" => {
                if len > 0 {
                    self.commit_theme_picker_selection(cx);
                }
            }
            "up" => {
                if len > 0 && self.theme_picker_selected_idx > 0 {
                    self.theme_picker_selected_idx -= 1;
                    cx.notify();
                }
            }
            "down" => {
                if len > 0 && self.theme_picker_selected_idx + 1 < len {
                    self.theme_picker_selected_idx += 1;
                    cx.notify();
                }
            }
            "backspace" => {
                if self.theme_picker_query.pop().is_some() {
                    self.theme_picker_selected_idx = 0;
                    cx.notify();
                }
            }
            _ => {
                if let Some(ch) = &event.keystroke.key_char
                    && !ch.is_empty()
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.alt
                {
                    self.theme_picker_query.push_str(ch);
                    self.theme_picker_selected_idx = 0;
                    cx.notify();
                }
            }
        }
    }

    pub(crate) fn render_theme_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let matches = self.theme_picker_matches();
        let current_name = Self::current_theme_name();

        let query_text: SharedString = if self.theme_picker_query.is_empty() {
            "Select Theme…".into()
        } else {
            format!("{}|", self.theme_picker_query).into()
        };
        let query_color = if self.theme_picker_query.is_empty() {
            ui.muted
        } else {
            ui.text
        };

        let search_input = div()
            .px(px(14.))
            .py(px(10.))
            .text_size(px(13.))
            .text_color(query_color)
            .border_b_1()
            .border_color(ui.border)
            .child(query_text);

        let mut list = div()
            .id("theme-picker-list")
            .flex()
            .flex_col()
            .max_h(px(360.))
            .overflow_y_scroll();

        if matches.is_empty() {
            list = list.child(
                div()
                    .px(px(14.))
                    .py(px(12.))
                    .text_size(px(12.))
                    .text_color(ui.muted)
                    .child("No matching themes"),
            );
        } else {
            for (idx, name) in matches.iter().enumerate() {
                let is_selected = idx == self.theme_picker_selected_idx;
                let is_current = *name == current_name.as_str();
                let label = if *name == "Catppuccin Mocha" {
                    format!("{} (Default)", name)
                } else {
                    (*name).to_string()
                };
                let name_owned = name.to_string();

                list = list.child(
                    div()
                        .id(SharedString::from(format!("theme-picker-row-{idx}")))
                        .px(px(14.))
                        .py(px(6.))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .when(is_selected, |d| d.bg(ui.subtle))
                        .when(!is_selected, |d| d.hover(|s| s.bg(ui.subtle)))
                        .when(is_current, |d| d.text_color(ui.accent))
                        .when(!is_current, |d| d.text_color(ui.text))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            Self::apply_theme_by_name(&name_owned);
                            this.close_theme_picker(cx);
                            cx.stop_propagation();
                        }))
                        .child(label),
                );
            }
        }

        deferred(
            div()
                .id("theme-picker-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_start()
                .justify_center()
                .pt(px(96.))
                .bg(gpui::hsla(0., 0., 0., 0.4))
                .child(
                    div()
                        .id("theme-picker")
                        .occlude()
                        .track_focus(&self.theme_picker_focus)
                        .on_key_down(cx.listener(Self::handle_theme_picker_key_down))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.close_theme_picker(cx);
                        }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .w(px(520.))
                        .flex()
                        .flex_col()
                        .bg(ui.overlay)
                        .border_1()
                        .border_color(ui.border)
                        .rounded(px(8.))
                        .shadow_lg()
                        .overflow_hidden()
                        .child(search_input)
                        .child(list),
                ),
        )
        .with_priority(6)
        .into_any_element()
    }
}
