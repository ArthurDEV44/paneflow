//! "Terminal" settings tab (US-016) — cursor shape/blink, bell mode,
//! scrollback, font size, line height, ligatures, and option-as-meta.
//!
//! Controls map to config like so:
//! - **cursor_shape / cursor_blink / bell / scrollback_lines** → enum/preset
//!   dropdowns (deferred popover, same recipe as the font picker in
//!   `appearance.rs`), persisted into the `terminal` block via
//!   [`config_writer::save_terminal_field`].
//! - **font_size / line_height** → `−`/`+` steppers that clamp by construction
//!   (so an out-of-range value can never be entered), persisted as top-level
//!   fields via [`config_writer::save_config_value`].
//! - **ligatures** (terminal block) / **option_as_meta** (top-level) → toggle
//!   pills.
//!
//! Cursor/bell/blink/scrollback/ligatures are read once at terminal spawn, so
//! their rows note "next new terminal"; font size and line height hot-reload.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, div, prelude::*, px,
};
use serde_json::{Value, json};

use paneflow_config::schema::{CursorBlinkConfig, CursorShapeConfig, TerminalBellMode};

use crate::settings::components::{
    deferred_select_menu, hairline, section_header, select_chevron, select_item, select_menu,
    select_trigger, setting_card, setting_text, toggle_pill,
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
        let blink = terminal.cursor_blink.unwrap_or_default();
        let bell = terminal.bell.unwrap_or_default();
        let scrollback = terminal.resolved_scrollback_lines();
        let ligatures_on = terminal.ligatures.unwrap_or(false);
        let meta_on = config
            .option_as_meta
            .unwrap_or_else(crate::keys::default_option_as_meta);
        let font_size = config.font_size.unwrap_or(14.0) as f64;
        let line_height = config.line_height.unwrap_or(1.3) as f64;

        let shape_label = match shape {
            CursorShapeConfig::Block => "Block",
            CursorShapeConfig::Beam => "Beam",
            CursorShapeConfig::Underline => "Underline",
            CursorShapeConfig::Hollow => "Hollow",
        };
        let blink_label = match blink {
            CursorBlinkConfig::On => "Always blink",
            CursorBlinkConfig::Off => "Never blink",
            CursorBlinkConfig::TerminalControlled => "Terminal controlled",
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
        let blink_opts: Vec<(String, Value, bool)> = vec![
            (
                "Terminal controlled".into(),
                json!("terminal_controlled"),
                blink == CursorBlinkConfig::TerminalControlled,
            ),
            (
                "Always blink".into(),
                json!("on"),
                blink == CursorBlinkConfig::On,
            ),
            (
                "Never blink".into(),
                json!("off"),
                blink == CursorBlinkConfig::Off,
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
        let scrollback_opts: Vec<(String, Value, bool)> =
            [1_000usize, 5_000, 10_000, 25_000, 50_000, 100_000]
                .into_iter()
                .map(|n| (n.to_string(), json!(n), n == scrollback))
                .collect();

        // ── Cursor ──────────────────────────────────────────────────────
        let cursor_card = setting_card(ui)
            .child(self.terminal_enum_row(
                TerminalDropdown::CursorShape,
                "Cursor shape",
                "Default shape before any app-driven DECSCUSR escape. Takes effect on the next new terminal.",
                shape_label.to_string(),
                shape_opts,
                "cursor_shape",
                true,
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(self.terminal_enum_row(
                TerminalDropdown::CursorBlink,
                "Cursor blink",
                "Override the program's DECSCUSR blink preference. Takes effect on the next new terminal.",
                blink_label.to_string(),
                blink_opts,
                "cursor_blink",
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
            .child(self.terminal_enum_row(
                TerminalDropdown::Scrollback,
                "Scrollback lines",
                "Max history kept per terminal. Takes effect on the next new terminal.",
                scrollback.to_string(),
                scrollback_opts,
                "scrollback_lines",
                true,
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(self.terminal_stepper_row(
                "term-font-size",
                "Font size",
                "Terminal font size in pixels (8–32). Hot-reloads.",
                font_size,
                8.0,
                32.0,
                1.0,
                0,
                "font_size",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(self.terminal_stepper_row(
                "term-line-height",
                "Line height",
                "Line height multiplier (1.0–2.5). Hot-reloads.",
                line_height,
                1.0,
                2.5,
                0.1,
                1,
                "line_height",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(self.terminal_toggle_row(
                "term-ligatures",
                "Ligatures",
                "Render programming-font ligatures (=> != ===). Takes effect on the next new terminal.",
                ligatures_on,
                "ligatures",
                true,
                ui,
                cx,
            ));

        // ── Input ───────────────────────────────────────────────────────
        let input_card = setting_card(ui).child(self.terminal_toggle_row(
            "term-option-as-meta",
            "Option as Meta",
            "Send an ESC prefix for Alt/Option chords. Disable on macOS to type Unicode via Option.",
            meta_on,
            "option_as_meta",
            false,
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
            .child(section_header(ui, "Input"))
            .child(input_card)
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
                .rounded(px(6.))
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

    /// A toggle row — only the switch is interactive (the row itself does not
    /// hover or click). `nested` routes the write to the `terminal` block (e.g.
    /// `ligatures`) vs. a top-level key (`option_as_meta`).
    #[allow(clippy::too_many_arguments)]
    fn terminal_toggle_row(
        &self,
        id: &'static str,
        title: &'static str,
        description: &'static str,
        current: bool,
        config_key: &'static str,
        nested: bool,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                    .id(id)
                    .flex_shrink_0()
                    .cursor(CursorStyle::PointingHand)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        // US-016: cache-mutate + notify + off-thread persist.
                        this.persist_setting(nested, config_key, Value::Bool(!current), cx);
                    }))
                    .child(toggle_pill(current, ui)),
            )
    }
}
