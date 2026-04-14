use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, CursorStyle, Decorations, FocusHandle, Focusable,
    HitboxBehavior, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement,
    Pixels, Render, ResizeEdge, SharedString, Styled, Window, WindowBounds, WindowButton,
    WindowControlArea, WindowControls, WindowDecorations, WindowOptions, canvas, div, point,
    prelude::*, px, rgb, size, svg, transparent_black,
};

use crate::{config_writer, csd, keybindings};

const SETTINGS_SIDEBAR_WIDTH: Pixels = px(180.);
const SETTINGS_RESIZE_BORDER: Pixels = px(10.);

fn settings_sidebar_bg() -> gpui::Rgba {
    let theme = crate::theme::active_theme();
    if theme.background.l > 0.5 {
        rgb(0xe8e8e8)
    } else {
        rgb(0x141414)
    }
}

fn settings_content_bg() -> gpui::Rgba {
    let theme = crate::theme::active_theme();
    if theme.background.l > 0.5 {
        rgb(0xefefef)
    } else {
        rgb(0x1a1a1a)
    }
}

use csd::{default_button_layout, resize_edge};

#[derive(Clone, Copy, PartialEq)]
enum SettingsSection {
    Shortcuts,
    Appearance,
}

pub struct SettingsWindow {
    section: SettingsSection,
    effective_shortcuts: Vec<keybindings::ShortcutEntry>,
    recording_shortcut_idx: Option<usize>,
    settings_focus: FocusHandle,
    mono_font_names: Vec<String>,
    font_dropdown_open: bool,
    font_search: String,
    should_move: bool,
}

impl SettingsWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|this, cx| {
            this.cleanup(cx);
        })
        .detach();

        let config = paneflow_config::loader::load_config();

        Self {
            section: SettingsSection::Shortcuts,
            effective_shortcuts: keybindings::effective_shortcuts(&config.shortcuts),
            recording_shortcut_idx: None,
            settings_focus: cx.focus_handle(),
            mono_font_names: Vec::new(),
            font_dropdown_open: false,
            font_search: String::new(),
            should_move: false,
        }
    }

    fn cleanup(&mut self, cx: &mut App) {
        if self.recording_shortcut_idx.is_some() {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
        }
    }

    fn render_settings_sidebar(
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
            .gap(px(2.))
            .w(SETTINGS_SIDEBAR_WIDTH)
            .h_full()
            .border_r_1()
            .border_color(ui.border)
            .bg(settings_sidebar_bg())
            .pt(px(4.));

        for (label, section) in sections {
            let is_active = section == active;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{label}")))
                    .mx(px(4.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .cursor(CursorStyle::PointingHand)
                    .when(is_active, |d| d.bg(ui.overlay).text_color(ui.text))
                    .when(!is_active, |d| {
                        d.text_color(ui.muted).hover(|s| s.bg(ui.subtle))
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

    fn render_window_button_group(
        &self,
        side: &'static str,
        buttons: &[Option<WindowButton>; 3],
        is_maximized: bool,
        supported: &WindowControls,
    ) -> Option<AnyElement> {
        let children: Vec<AnyElement> = buttons
            .iter()
            .filter_map(|slot| *slot)
            .filter(|button| match button {
                WindowButton::Minimize => supported.minimize,
                WindowButton::Maximize => supported.maximize,
                WindowButton::Close => true,
            })
            .map(|button| self.render_window_button(side, button, is_maximized))
            .collect();

        if children.is_empty() {
            return None;
        }

        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .children(children)
                .into_any_element(),
        )
    }

    fn render_window_button(
        &self,
        side: &'static str,
        button: WindowButton,
        is_maximized: bool,
    ) -> AnyElement {
        let id = match button {
            WindowButton::Minimize => "wc-minimize",
            WindowButton::Maximize => "wc-maximize",
            WindowButton::Close => "wc-close",
        };

        let icon_path = match button {
            WindowButton::Minimize => "icons/generic_minimize.svg",
            WindowButton::Maximize if is_maximized => "icons/generic_restore.svg",
            WindowButton::Maximize => "icons/generic_maximize.svg",
            WindowButton::Close => "icons/generic_close.svg",
        };

        let control_area = match button {
            WindowButton::Minimize => WindowControlArea::Min,
            WindowButton::Maximize => WindowControlArea::Max,
            WindowButton::Close => WindowControlArea::Close,
        };

        div()
            .id(SharedString::from(format!("{id}-{side}")))
            .window_control_area(control_area)
            .flex()
            .items_center()
            .justify_center()
            .w(px(28.))
            .h(px(22.))
            .rounded_sm()
            .cursor_pointer()
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .active(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match button {
                    WindowButton::Minimize => window.minimize_window(),
                    WindowButton::Maximize => window.zoom_window(),
                    WindowButton::Close => window.remove_window(),
                }
            })
            .child({
                let ui = crate::theme::ui_colors();
                svg()
                    .size(px(16.))
                    .flex_none()
                    .path(icon_path)
                    .text_color(ui.text)
            })
            .into_any_element()
    }

    fn render_title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let height = (1.75 * window.rem_size()).max(px(34.));
        let decorations = window.window_decorations();
        let is_csd = matches!(decorations, Decorations::Client { .. });
        let theme = crate::theme::active_theme();
        let bg_color = if window.is_window_active() {
            theme.title_bar_background
        } else {
            theme.title_bar_inactive_background
        };
        let layout = cx.button_layout().unwrap_or_else(default_button_layout);
        let is_maximized = window.is_maximized();
        let supported = window.window_controls();

        let left_controls = if is_csd {
            self.render_window_button_group("l", &layout.left, is_maximized, &supported)
        } else {
            None
        };

        let right_controls = if is_csd {
            self.render_window_button_group("r", &layout.right, is_maximized, &supported)
        } else {
            None
        };

        let ui = crate::theme::ui_colors();
        let brand = div()
            .w(SETTINGS_SIDEBAR_WIDTH)
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .pl_3()
            .overflow_x_hidden()
            .child(
                div()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("PaneFlow"),
            );

        let title = div()
            .flex_1()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .px(px(12.))
            .child(
                div()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Settings"),
            );

        let csd_rounding = px(10.);

        let mut bar = div()
            .id("settings-title-bar")
            .window_control_area(WindowControlArea::Drag)
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(height)
            .bg(bg_color)
            .pr(px(12.));

        if let Decorations::Client { tiling } = decorations {
            if !(tiling.top || tiling.left) {
                bar = bar.rounded_tl(csd_rounding);
            }
            if !(tiling.top || tiling.right) {
                bar = bar.rounded_tr(csd_rounding);
            }
            bar = bar
                .mt(px(-1.))
                .mb(px(-1.))
                .border(px(1.))
                .border_color(bg_color);
        }

        bar.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, _| {
                this.should_move = true;
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| {
                this.should_move = false;
            }),
        )
        .on_mouse_down_out(cx.listener(|this, _, _, _| {
            this.should_move = false;
        }))
        .on_mouse_move(cx.listener(|this, _, window, _| {
            if this.should_move {
                this.should_move = false;
                window.start_window_move();
            }
        }))
        .on_click(|event, window, _| {
            if event.click_count() == 2 {
                window.zoom_window();
            }
        })
        .when(supported.window_menu, |bar| {
            bar.on_mouse_down(MouseButton::Right, |ev, window, _| {
                window.show_window_menu(ev.position);
            })
        })
        .children(left_controls)
        .child(brand)
        .child(title)
        .children(right_controls)
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(1.))
                .bg(ui.border),
        )
        .into_any_element()
    }

    fn render_shortcuts_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    fn render_appearance_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current_font =
            crate::terminal_element::resolve_font_family(config.font_family.as_deref());
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
                    this.mono_font_names = config_writer::load_mono_fonts();
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
                    .pb(px(16.))
                    .child(section_header)
                    .child(reset_btn),
            )
            .child(theme_row)
            .child(font_row)
            .child(preview)
    }

    fn handle_settings_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

        if self.section == SettingsSection::Shortcuts {
            self.handle_shortcut_recording(event, cx);
        }
    }

    fn handle_shortcut_recording(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(idx) = self.recording_shortcut_idx else {
            return;
        };

        if keybindings::is_bare_modifier(&event.keystroke) {
            return;
        }

        if event.keystroke.key == "escape" {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        }

        let Some(action_name) = keybindings::action_name_at(idx) else {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        };

        let new_key = event.keystroke.to_string();
        config_writer::save_shortcut(&new_key, action_name);

        let config = paneflow_config::loader::load_config();
        keybindings::apply_keybindings(cx, &config.shortcuts);
        self.effective_shortcuts = keybindings::effective_shortcuts(&config.shortcuts);
        self.recording_shortcut_idx = None;
        cx.notify();
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.settings_focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let ui_font = crate::terminal_element::resolve_font_family(
            paneflow_config::loader::load_config()
                .font_family
                .as_deref(),
        );
        let decorations = window.window_decorations();

        let content = match self.section {
            SettingsSection::Shortcuts => self.render_shortcuts_content(cx).into_any_element(),
            SettingsSection::Appearance => self.render_appearance_content(cx).into_any_element(),
        };

        match decorations {
            Decorations::Client { .. } => window.set_client_inset(SETTINGS_RESIZE_BORDER),
            Decorations::Server => window.set_client_inset(px(0.0)),
        }

        let app_content = div()
            .id("settings-window")
            .font_family(ui_font)
            .track_focus(&self.settings_focus)
            .on_key_down(cx.listener(Self::handle_settings_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(settings_content_bg())
            .child(self.render_title_bar(window, cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_settings_sidebar(self.section, ui, cx))
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .bg(settings_content_bg())
                            .overflow_y_scroll()
                            .px(px(24.))
                            .pt(px(20.))
                            .pb(px(12.))
                            .child(content),
                    ),
            );

        div()
            .id("settings-window-backdrop")
            .bg(transparent_black())
            .size_full()
            .map(|d| match decorations {
                Decorations::Server => d,
                Decorations::Client { tiling } => d
                    .child(
                        canvas(
                            |_bounds, window, _cx| {
                                window.insert_hitbox(
                                    Bounds::new(
                                        point(px(0.0), px(0.0)),
                                        window.window_bounds().get_bounds().size,
                                    ),
                                    HitboxBehavior::Normal,
                                )
                            },
                            move |_bounds, hitbox, window, _cx| {
                                let mouse = window.mouse_position();
                                let win_size = window.window_bounds().get_bounds().size;
                                let Some(edge) =
                                    resize_edge(mouse, SETTINGS_RESIZE_BORDER, win_size, tiling)
                                else {
                                    return;
                                };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
                                        ResizeEdge::Left | ResizeEdge::Right => {
                                            CursorStyle::ResizeLeftRight
                                        }
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                            CursorStyle::ResizeUpLeftDownRight
                                        }
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                            CursorStyle::ResizeUpRightDownLeft
                                        }
                                    },
                                    &hitbox,
                                );
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    .when(!tiling.top, |d| d.pt(SETTINGS_RESIZE_BORDER))
                    .when(!tiling.bottom, |d| d.pb(SETTINGS_RESIZE_BORDER))
                    .when(!tiling.left, |d| d.pl(SETTINGS_RESIZE_BORDER))
                    .when(!tiling.right, |d| d.pr(SETTINGS_RESIZE_BORDER))
                    .on_mouse_move(|_e, window, _cx| window.refresh())
                    .on_mouse_down(MouseButton::Left, move |e, window, _cx| {
                        let win_size = window.window_bounds().get_bounds().size;
                        if let Some(edge) =
                            resize_edge(e.position, SETTINGS_RESIZE_BORDER, win_size, tiling)
                        {
                            window.start_window_resize(edge);
                        }
                    }),
            })
            .child(app_content)
    }
}

pub fn open_or_focus<T>(_window: &mut Window, cx: &mut Context<T>) {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<SettingsWindow>())
    {
        existing
            .update(cx, |settings_window, window, cx| {
                window.activate_window();
                settings_window.settings_focus.focus(window, cx);
            })
            .ok();
        return;
    }

    let config = paneflow_config::loader::load_config();
    let decorations = match config.window_decorations.as_deref() {
        Some("server") => WindowDecorations::Server,
        Some("client") | None => WindowDecorations::Client,
        Some(_) => WindowDecorations::Client,
    };

    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(980.), px(720.)), cx)),
        window_min_size: Some(size(px(900.), px(520.))),
        window_decorations: Some(decorations),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("PaneFlow - Settings".into()),
            appears_transparent: true,
            ..Default::default()
        }),
        app_id: Some("paneflow".into()),
        focus: true,
        show: true,
        ..Default::default()
    };

    if let Ok(settings_window) = cx.open_window(options, |window, cx| {
        let settings_window = cx.new(SettingsWindow::new);
        let focus = settings_window.read(cx).settings_focus.clone();
        focus.focus(window, cx);
        settings_window
    }) {
        let _ = settings_window.update(cx, |settings_window, window, cx| {
            window.activate_window();
            settings_window.settings_focus.focus(window, cx);
        });
    }
}
