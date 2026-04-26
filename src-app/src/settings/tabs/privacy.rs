//! "Privacy" settings tab — tri-state telemetry consent toggle
//! (US-014) plus a link to the public privacy page.
//!
//! State mapping:
//! - `None`        → "Not set" (consent modal will ask at next launch).
//! - `Some(true)`  → "Enabled" (events flow to PostHog EU).
//! - `Some(false)` → "Disabled" (Null client, no HTTP).
//!
//! The actual client hot-swap on flip lives in
//! `PaneFlowApp::process_config_changes` — the watcher picks up the
//! config file write, reconciles the old/new consent value, rebuilds
//! the `TelemetryClient` on transition, and surfaces a confirmation
//! toast. This module only writes the config; the main app handles
//! the effect.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

use crate::config_writer;

use super::super::window::SettingsWindow;

const PRIVACY_URL: &str = "https://paneflow.dev/legal/privacy";

impl SettingsWindow {
    pub(crate) fn render_privacy_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current = config.telemetry.as_ref().and_then(|t| t.enabled);

        let section_header = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.muted)
                    .child("PRIVACY"),
            )
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child("Anonymous telemetry"),
            );

        let description = div()
            .mt(px(10.))
            .text_size(px(13.))
            .text_color(ui.muted)
            .child(
                "PaneFlow can send anonymous events (startup, shutdown, updates) to help us \
                 understand how it is used. No paths, no terminal content, and no personal \
                 information are transmitted.",
            );

        let status_label = match current {
            None => "Current state: Not set (you will be asked at next launch)",
            Some(true) => "Current state: Enabled",
            Some(false) => "Current state: Disabled",
        };
        let status_color = match current {
            Some(true) => ui.accent,
            _ => ui.muted,
        };

        let status = div()
            .mt(px(14.))
            .text_size(px(12.))
            .text_color(status_color)
            .child(status_label);

        // Segmented two-button toggle. Neither button is "active" when
        // current == None (tri-state rendering); the user picks either
        // Enable or Disable to resolve the state.
        let segmented = div()
            .mt(px(12.))
            .flex()
            .flex_row()
            .gap(px(8.))
            .child(render_toggle_button(
                "telemetry-enable",
                "Enable",
                current == Some(true),
                ui,
                cx,
                true,
            ))
            .child(render_toggle_button(
                "telemetry-disable",
                "Disable",
                current == Some(false),
                ui,
                cx,
                false,
            ));

        let privacy_link = div()
            .id("privacy-link")
            .mt(px(14.))
            .text_size(px(12.))
            .text_color(ui.muted)
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.text_color(ui.accent))
            .child("Learn more about our privacy policy →")
            .on_click(|_: &ClickEvent, _window, _cx| {
                // `open::that` dispatches through xdg-open / open / start
                // depending on platform; failure to open a browser is
                // non-fatal and logged.
                if let Err(e) = open::that(PRIVACY_URL) {
                    log::warn!("settings/privacy: failed to open {PRIVACY_URL}: {e}");
                }
            });

        div()
            .flex()
            .flex_col()
            .child(section_header)
            .child(description)
            .child(status)
            .child(segmented)
            .child(privacy_link)
    }
}

/// Render one segmented toggle button. `active` highlights the button
/// that reflects the currently-persisted consent state; clicking writes
/// `config.telemetry.enabled = Some(target_value)` and lets the config
/// watcher pick up the change on the main-app side (hot-swap + toast).
fn render_toggle_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<SettingsWindow>,
    target_value: bool,
) -> impl IntoElement {
    let base = div()
        .id(id)
        .px(px(16.))
        .py(px(8.))
        .rounded(px(6.))
        .border_1()
        .text_size(px(13.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .cursor(CursorStyle::PointingHand)
        .child(label);

    let styled = if active {
        base.bg(ui.overlay)
            .border_color(ui.text)
            .text_color(ui.text)
    } else {
        base.bg(ui.surface)
            .border_color(ui.border)
            .text_color(ui.muted)
            .hover(|s| s.border_color(ui.muted).text_color(ui.text))
    };

    styled.on_click(cx.listener(move |_this, _: &ClickEvent, _window, cx| {
        let ok = config_writer::save_config_value_checked(
            "telemetry",
            serde_json::json!({ "enabled": target_value }),
        );
        if !ok {
            log::warn!(
                "settings/privacy: failed to persist telemetry.enabled = {target_value}; user choice is in-memory only for this session"
            );
        }
        cx.notify();
    }))
}
