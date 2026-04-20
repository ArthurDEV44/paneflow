//! Settings page rendering + shortcut-recording handlers for `PaneFlowApp`.
//!
//! Owns the "Shortcuts" and "Appearance" panels, the sidebar that switches
//! between them, and the key-capture flow used when the user rebinds an
//! action.
//!
//! Extracted from `main.rs` per US-018 of the src-app refactor PRD.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    SharedString, Styled, Window, div, prelude::*, px,
};

use crate::{PaneFlowApp, SettingsSection, config_writer, keybindings};

impl PaneFlowApp {
    // ── Settings page: sidebar + content ──────────────────────────────

    pub(crate) fn render_settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.settings_section.unwrap_or(SettingsSection::Shortcuts);
        let ui = crate::theme::ui_colors();

        // Header bar: title + close button
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(24.))
            .pt(px(20.))
            .pb(px(16.))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(ui.text)
                    .child("Settings"),
            )
            .child(
                div()
                    .id("settings-close")
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(ui.subtle))
                    .text_size(px(14.))
                    .text_color(ui.muted)
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.close_settings(cx);
                        cx.notify();
                    }))
                    .child("Close"),
            );

        // Content area based on active section
        let content = match section {
            SettingsSection::Shortcuts => self.render_shortcuts_content(cx).into_any_element(),
            SettingsSection::Appearance => self.render_appearance_content(cx).into_any_element(),
        };

        div()
            .id("settings-page")
            .track_focus(&self.settings_focus)
            .on_key_down(cx.listener(Self::handle_settings_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(ui.base)
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    // Left sidebar
                    .child(self.render_settings_sidebar(section, ui, cx))
                    // Right content
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(24.))
                            .py(px(12.))
                            .child(content),
                    ),
            )
    }

    pub(crate) fn render_settings_sidebar(
        &self,
        active: SettingsSection,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sections = [
            ("Shortcuts", SettingsSection::Shortcuts),
            ("Appearance", SettingsSection::Appearance),
        ];

        let mut nav = div()
            .flex()
            .flex_col()
            .w(px(180.))
            .h_full()
            .border_r_1()
            .border_color(ui.border)
            .bg(ui.base)
            .pt(px(4.));

        for (label, section) in sections {
            let is_active = section == active;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{label}")))
                    .mx(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .cursor(CursorStyle::PointingHand)
                    .when(is_active, |d| d.bg(ui.overlay).text_color(ui.text))
                    .when(!is_active, |d| {
                        d.text_color(ui.muted).hover(|s| s.bg(ui.subtle))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.settings_section = Some(section);
                        // Reset editing state when switching tabs
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

    pub(crate) fn render_shortcuts_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let recording_idx = self.recording_shortcut_idx;

        // Section header + reset button
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

            let row_bg = if is_recording { ui.overlay } else { ui.base };

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
                    .bg(row_bg)
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

        // ── Theme ──
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
                    .px(px(14.))
                    .py(px(8.))
                    .rounded(px(6.))
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
            .pb(px(20.))
            .child(theme_label)
            .child(theme_row_inner);

        // ── Font Family ──
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
            .px(px(12.))
            .py(px(6.))
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
            .pb(px(20.))
            .child(font_label)
            .child(font_badge);

        // Dropdown list
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

        // ── Font Preview ──
        let preview = div()
            .pb(px(20.))
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

        // ── Reset to defaults ──
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
                    .pb(px(16.))
                    .child(section_header)
                    .child(reset_btn),
            )
            .child(theme_row)
            .child(font_row)
            .child(preview)
    }

    pub(crate) fn handle_settings_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Font dropdown search
        if self.font_dropdown_open {
            let key = event.keystroke.key.as_str();
            match key {
                "escape" => {
                    self.font_dropdown_open = false;
                    self.font_search.clear();
                    cx.notify();
                }
                "backspace" => {
                    self.font_search.pop();
                    cx.notify();
                }
                _ => {
                    if let Some(ch) = &event.keystroke.key_char
                        && !ch.is_empty()
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.platform
                    {
                        self.font_search.push_str(ch);
                        cx.notify();
                    }
                }
            }
            return;
        }

        // Shortcut recording (only on Shortcuts tab)
        if self.settings_section == Some(SettingsSection::Shortcuts) {
            self.handle_shortcut_recording(event, _window, cx);
        }
    }

    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = None;
        self.title_bar_menu_open = None;
        self.font_dropdown_open = false;
        self.font_search.clear();
        if self.recording_shortcut_idx.is_some() {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
        }
        crate::terminal::SUPPRESS_REPAINTS.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn open_settings_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notif_menu_open = false;
        self.workspace_menu_open = None;
        self.title_bar_menu_open = None;
        crate::settings::open_or_focus(window, cx);
        cx.notify();
    }

    pub(crate) fn handle_shortcut_recording(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.recording_shortcut_idx else {
            return;
        };

        // Ignore bare modifier presses (Shift alone, Ctrl alone, etc.)
        if keybindings::is_bare_modifier(&event.keystroke) {
            return;
        }

        // Escape cancels recording
        if event.keystroke.key == "escape" {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        }

        // Get the action name for this shortcut index
        let Some(action_name) = keybindings::action_name_at(idx) else {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        };

        // Format keystroke to GPUI string (e.g. "ctrl-shift-d")
        let new_key = event.keystroke.to_string();

        // Save to config file
        config_writer::save_shortcut(&new_key, action_name);

        // Re-apply keybindings from updated config
        let config = paneflow_config::loader::load_config();
        keybindings::apply_keybindings(cx, &config.shortcuts);
        self.effective_shortcuts = keybindings::effective_shortcuts(&config.shortcuts);
        self.recording_shortcut_idx = None;
        cx.notify();
    }
}
