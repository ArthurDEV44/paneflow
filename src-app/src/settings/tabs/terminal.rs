//! "Terminal" settings tab (US-016) - the small set of terminal preferences
//! worth keeping in the primary Settings UI: cursor shape, bell mode, font
//! family, and font size.
//!
//! Controls map to config like so:
//! - **cursor_shape / bell** -> enum/preset dropdowns (deferred popovers),
//!   persisted into the `terminal` block via
//!   [`config_writer::save_terminal_field`].
//! - **font_family** -> searchable monospace-font dropdown, persisted as a
//!   top-level field via [`config_writer::save_config_value`].
//! - **font_size** -> a `−`/`+` stepper that clamps by construction, persisted
//!   as a top-level field via [`config_writer::save_config_value`].
//!
//! Other advanced terminal knobs remain supported in `paneflow.json`, but are
//! intentionally not mirrored here to keep Settings focused.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, div, prelude::*, px,
};
use serde_json::{Value, json};

use paneflow_config::schema::{CursorShapeConfig, TerminalBellMode};

use crate::settings::components::{
    SETTINGS_CONTROL_CORNER_RADIUS, deferred_select_menu, hairline, section_header, select_chevron,
    select_item, select_menu, select_trigger, setting_card, setting_text,
};

use crate::{PaneFlowApp, TerminalDropdown};

impl PaneFlowApp {
    pub(crate) fn render_terminal_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // US-016: read the cached config (no per-frame `load_config()`).
        let config = &self.cached_config;
        let ui = crate::theme::ui_colors();
        let terminal = config.terminal.clone().unwrap_or_default();

        // ── current values ──────────────────────────────────────────────
        let shape = terminal.cursor_shape.unwrap_or_default();
        let bell = terminal.bell.unwrap_or_default();
        let current_font =
            crate::terminal::element::resolve_font_family(config.font_family.as_deref());
        let font_size = config
            .font_size
            .unwrap_or(crate::terminal::element::DEFAULT_FONT_SIZE) as f64;

        let shape_label = match shape {
            CursorShapeConfig::Block => "Block",
            CursorShapeConfig::Beam => "Beam",
            CursorShapeConfig::Underline => "Underline",
            CursorShapeConfig::Hollow => "Hollow",
        };
        let bell_label = match bell {
            TerminalBellMode::Visual => "Visual flash",
            TerminalBellMode::Audible => "Audible",
            TerminalBellMode::Both => "Both",
            TerminalBellMode::Off => "Off",
        };

        let shape_opts: Vec<(String, Value, bool)> = vec![
            (
                "Block".into(),
                json!("block"),
                shape == CursorShapeConfig::Block,
            ),
            (
                "Beam".into(),
                json!("beam"),
                shape == CursorShapeConfig::Beam,
            ),
            (
                "Underline".into(),
                json!("underline"),
                shape == CursorShapeConfig::Underline,
            ),
            (
                "Hollow".into(),
                json!("hollow"),
                shape == CursorShapeConfig::Hollow,
            ),
        ];
        let bell_opts: Vec<(String, Value, bool)> = vec![
            (
                "Visual flash".into(),
                json!("visual"),
                bell == TerminalBellMode::Visual,
            ),
            (
                "Audible".into(),
                json!("audible"),
                bell == TerminalBellMode::Audible,
            ),
            ("Both".into(), json!("both"), bell == TerminalBellMode::Both),
            ("Off".into(), json!("off"), bell == TerminalBellMode::Off),
        ];

        // ── Cursor ──────────────────────────────────────────────────────
        let cursor_card = setting_card(ui).child(self.terminal_enum_row(
            TerminalDropdown::CursorShape,
            "Cursor shape",
            "Default shape before any app-driven DECSCUSR escape. Takes effect on the next new terminal.",
            shape_label.to_string(),
            shape_opts,
            "cursor_shape",
            true,
            ui,
            cx,
        ));

        // ── Bell ────────────────────────────────────────────────────────
        let bell_card = setting_card(ui).child(self.terminal_enum_row(
            TerminalDropdown::Bell,
            "Bell",
            "How a BEL (\\a) is surfaced: a visual flash, the OS sound, both, or off.",
            bell_label.to_string(),
            bell_opts,
            "bell",
            true,
            ui,
            cx,
        ));

        // ── Display ─────────────────────────────────────────────────────
        let display_card = setting_card(ui)
            .child(self.terminal_font_family_row(current_font, ui, cx))
            .child(hairline(ui))
            .child(self.terminal_stepper_row(
                "term-font-size",
                "Font size",
                "Terminal font size in pixels (8-32). Hot-reloads.",
                font_size,
                8.0,
                32.0,
                1.0,
                0,
                "font_size",
                ui,
                cx,
            ));

        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(section_header(ui, "Cursor"))
            .child(cursor_card)
            .child(section_header(ui, "Bell"))
            .child(bell_card)
            .child(section_header(ui, "Display"))
            .child(display_card)
    }

    fn terminal_font_family_row(
        &self,
        current_font: String,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let default_font = crate::terminal::element::resolve_font_family(None);
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

        let font_open = self.font_dropdown_open;
        let mut trigger = select_trigger("terminal-font-family-trigger", ui)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.terminal_dropdown = None;
                    this.font_dropdown_open = !font_open;
                    this.font_search.clear();
                    if this.font_dropdown_open && this.mono_font_names.is_empty() {
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
            let default_label = format!("PaneFlow default - {default_font}");
            let default_matches =
                search.is_empty() || default_label.to_lowercase().contains(&search);
            let filtered: Vec<&String> = self
                .mono_font_names
                .iter()
                .filter(|name| {
                    name.as_str() != default_font.as_str()
                        && (search.is_empty() || name.to_lowercase().contains(&search))
                })
                .collect();

            let mut menu = select_menu("terminal-font-dropdown", ui).on_mouse_down_out(
                cx.listener(|this, _, _w, cx| {
                    if this.font_dropdown_open {
                        this.font_dropdown_open = false;
                        this.font_search.clear();
                        cx.notify();
                    }
                }),
            );

            if default_matches {
                menu = menu.child(
                    select_item(
                        ("terminal-font-default", 0usize),
                        current_font == default_font,
                        ui,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.font_dropdown_open = false;
                        this.font_search.clear();
                        this.persist_setting(false, "font_family", Value::Null, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(ui.text)
                            .child(default_label),
                    ),
                );
            }

            for (i, name) in filtered.iter().enumerate() {
                let name_owned = (*name).clone();
                let is_current = **name == current_font;
                menu = menu.child(
                    select_item(("terminal-font", i), is_current, ui)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.font_dropdown_open = false;
                            this.font_search.clear();
                            this.persist_setting(
                                false,
                                "font_family",
                                Value::String(name_owned.clone()),
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
                        ),
                );
            }

            if !default_matches && filtered.is_empty() {
                menu = menu.child(
                    div()
                        .px(px(8.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("No matching fonts"),
                );
            }

            trigger = trigger.child(deferred_select_menu(menu));
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .child(setting_text(
                ui,
                "Font family",
                "Choose the monospace font used by every terminal. Hot-reloads.",
            ))
            .child(div().flex_shrink_0().child(trigger))
            .into_any_element()
    }

    /// One settings row: label/description on the left, a Codex-style select on
    /// the right (shared `components::select_*` primitives). `options` are
    /// `(label, json_value_to_write, is_selected)`. `nested` routes the write to
    /// the `terminal` block vs. a top-level key.
    #[allow(clippy::too_many_arguments)]
    fn terminal_enum_row(
        &self,
        which: TerminalDropdown,
        title: &'static str,
        description: &'static str,
        current_label: String,
        options: Vec<(String, Value, bool)>,
        config_key: &'static str,
        nested: bool,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let is_open = self.terminal_dropdown == Some(which);

        // Decide open/close from the render-time `is_open` snapshot, not the live
        // state: the menu's `on_mouse_down_out` fires on the same press and may
        // have already cleared it, so a live toggle would re-open (see general.rs).
        let mut trigger = select_trigger(SharedString::from(format!("term-dd-{config_key}")), ui)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.font_dropdown_open = false;
                    this.font_search.clear();
                    this.terminal_dropdown = if is_open { None } else { Some(which) };
                    this.settings_focus.focus(window, cx);
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .truncate()
                    .child(current_label),
            )
            .child(select_chevron(ui));

        if is_open {
            let mut menu =
                select_menu(SharedString::from(format!("term-dd-list-{config_key}")), ui)
                    .on_mouse_down_out(cx.listener(move |this, _, _w, cx| {
                        if this.terminal_dropdown == Some(which) {
                            this.terminal_dropdown = None;
                            cx.notify();
                        }
                    }));
            for (i, (label, value, selected)) in options.into_iter().enumerate() {
                let value_for_click = value;
                let item = select_item((config_key, i), selected, ui)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.terminal_dropdown = None;
                        this.persist_setting(nested, config_key, value_for_click.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(ui.text)
                            .child(label),
                    );
                menu = menu.child(item);
            }
            trigger = trigger.child(deferred_select_menu(menu));
        }

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .child(setting_text(ui, title, description))
            .child(div().flex_shrink_0().child(trigger))
            .into_any_element()
    }

    /// A `−`/`+` numeric stepper row for a top-level float field. The value is
    /// clamped to `[min, max]` on every step and rounded to `decimals` places
    /// before being written, so it can never go out of range and never writes
    /// a float-precision-noisy value.
    #[allow(clippy::too_many_arguments)]
    fn terminal_stepper_row(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        decimals: usize,
        config_key: &'static str,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let factor = 10f64.powi(decimals as i32);
        let round = move |v: f64| (v.clamp(min, max) * factor).round() / factor;
        let at_min = value <= min + f64::EPSILON;
        let at_max = value >= max - f64::EPSILON;

        let dec = cx.listener(move |this, _: &ClickEvent, _w, cx| {
            // US-016: cache-mutate + notify + off-thread persist.
            this.persist_setting(false, config_key, json!(round(value - step)), cx);
        });
        let inc = cx.listener(move |this, _: &ClickEvent, _w, cx| {
            this.persist_setting(false, config_key, json!(round(value + step)), cx);
        });

        let button = |btn_id: String, glyph: &'static str, disabled: bool| {
            div()
                .id(SharedString::from(btn_id))
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.))
                .h(px(24.))
                .rounded(SETTINGS_CONTROL_CORNER_RADIUS)
                .border_1()
                .border_color(ui.border)
                .bg(ui.base)
                .text_size(px(15.))
                .text_color(if disabled { ui.muted } else { ui.text })
                .when(!disabled, |d| {
                    d.cursor(CursorStyle::PointingHand)
                        .hover(|s| s.border_color(ui.muted))
                })
                .child(glyph)
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .child(setting_text(ui, title, description))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        button(format!("{id}-dec"), "−", at_min)
                            .when(!at_min, move |b| b.on_click(dec)),
                    )
                    .child(
                        div()
                            .w(px(48.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(ui.text)
                            .child(format!("{value:.decimals$}")),
                    )
                    .child(
                        button(format!("{id}-inc"), "+", at_max)
                            .when(!at_max, move |b| b.on_click(inc)),
                    ),
            )
    }
}
