//! "Appearance" settings tab — font family selector with an inline-
//! expanding dropdown. "Reset to defaults" sits inline in the section header.
//!
//! Font list enumeration comes from `crate::fonts::load_mono_fonts()`.
//! Theme selection lives in the title bar menu (`main.rs`).

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Styled, div, prelude::*, px, svg,
};

use crate::PaneFlowApp;
use crate::settings::components::{
    deferred_select_menu, secondary_button, section_header_with_action, select_chevron,
    select_item, select_menu, select_trigger, setting_card, setting_text,
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

        // Snapshot `font_dropdown_open` so the trigger toggle is decided from
        // render time, not live state (the menu's `on_mouse_down_out` fires on
        // the same press — see general.rs).
        let font_open = self.font_dropdown_open;
        let mut font_trigger = select_trigger("font-family-trigger", ui)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.font_dropdown_open = !font_open;
                    this.font_search.clear();
                    if this.font_dropdown_open && this.mono_font_names.is_empty() {
                        // Enumerate fonts (the `fc-list` subprocess on Linux/macOS)
                        // off the main thread; the menu opens empty and fills in.
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
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(trigger_label_color)
                    .truncate()
                    .child(trigger_label),
            )
            .child(select_chevron(ui));

        if self.font_dropdown_open {
            let search = self.font_search.to_lowercase();
            let filtered: Vec<&String> = self
                .mono_font_names
                .iter()
                .filter(|name| search.is_empty() || name.to_lowercase().contains(&search))
                .collect();

            let mut menu = select_menu("font-dropdown", ui).on_mouse_down_out(cx.listener(
                |this, _, _w, cx| {
                    if this.font_dropdown_open {
                        this.font_dropdown_open = false;
                        this.font_search.clear();
                        cx.notify();
                    }
                },
            ));

            for (i, name) in filtered.iter().enumerate() {
                let name_owned = (*name).clone();
                let is_current = **name == current_font;
                let item = select_item(("font", i), is_current, ui)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.font_dropdown_open = false;
                        this.font_search.clear();
                        this.persist_setting(
                            false,
                            "font_family",
                            serde_json::Value::String(name_owned.clone()),
                            cx,
                        );
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(ui.text)
                            .child((*name).clone()),
                    );
                menu = menu.child(item);
            }

            if filtered.is_empty() {
                menu = menu.child(
                    div()
                        .px(px(8.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("No matching fonts"),
                );
            }

            font_trigger = font_trigger.child(deferred_select_menu(menu));
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

        // Theme mode selector (Light / Dark / System) — the Codex-style card at
        // the top of the page. `theme_mode` is UI state for now; selecting a
        // segment highlights it, ready to drive theme resolution once the light
        // theme lands.
        let modes: [(crate::ThemeMode, &str, &str, &str); 3] = [
            (
                crate::ThemeMode::Light,
                "Light",
                "icons/sun.svg",
                "theme-mode-light",
            ),
            (
                crate::ThemeMode::Dark,
                "Dark",
                "icons/moon.svg",
                "theme-mode-dark",
            ),
            (
                crate::ThemeMode::System,
                "System",
                "icons/device-desktop.svg",
                "theme-mode-system",
            ),
        ];
        let mut mode_switch = div().flex().flex_row().items_center().gap(px(2.));
        for (mode, label, icon, id) in modes {
            let is_active = self.theme_mode == mode;
            let fg = if is_active { ui.text } else { ui.muted };
            let mut seg = div()
                .id(id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(5.))
                // Generous radius approximating Codex's Apple-style corner
                // smoothing (GPUI draws circular-arc corners — no true squircle).
                .rounded(crate::app::constants::SIDEBAR_TAB_CORNER_RADIUS)
                .child(svg().size(px(14.)).flex_none().path(icon).text_color(fg))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_color(fg)
                        .child(label),
                );
            if is_active {
                seg = seg.bg(crate::app::constants::sidebar_tab_active_background());
            } else {
                seg = seg
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(crate::app::constants::sidebar_tab_hover_background()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.apply_theme_mode(mode, window, cx);
                    }));
            }
            mode_switch = mode_switch.child(seg);
        }

        let theme_row = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .child(setting_text(
                ui,
                "Theme",
                "Use the light, dark, or system mode",
            ))
            .child(div().flex_shrink_0().child(mode_switch));
        // Shared Codex-style card look (white/#f2f2f2 in light, #232323/#303030
        // in dark, generous Apple-approximating radius) — see `setting_card`.
        let theme_card = setting_card(ui).child(theme_row);

        let font_section = div().flex().flex_col().child(header).child(font_card);

        div()
            .flex()
            .flex_col()
            .gap(px(24.))
            .child(theme_card)
            .child(font_section)
    }

    /// Apply a Light/Dark/System selection from the Themes page. Records the
    /// chosen segment, resolves it to a concrete bundled theme name (System
    /// follows the OS appearance reported by GPUI), then switches the active
    /// theme via the shared `apply_theme_by_name` path (synchronous config
    /// write + cache invalidation) and repaints.
    pub(crate) fn apply_theme_mode(
        &mut self,
        mode: crate::ThemeMode,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) {
        self.theme_mode = mode;
        let name = match mode {
            crate::ThemeMode::Light => "PaneFlow Light",
            crate::ThemeMode::Dark => "One Dark",
            crate::ThemeMode::System => {
                if matches!(
                    window.appearance(),
                    gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight
                ) {
                    "PaneFlow Light"
                } else {
                    "One Dark"
                }
            }
        };
        Self::apply_theme_by_name(name);
        cx.notify();
    }
}
