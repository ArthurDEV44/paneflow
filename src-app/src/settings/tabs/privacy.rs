//! "Privacy" settings tab — telemetry consent toggle plus a link to the
//! public privacy page (US-014).
//!
//! The persisted state is tri-state at the schema level
//! (`Some(true) | Some(false) | None`); in this UI it is rendered as a
//! single boolean toggle:
//! - `Some(true)`  → toggle ON.
//! - `Some(false)` → toggle OFF.
//! - `None`        → toggle OFF (the first-launch consent modal is what
//!   resolves the undecided state — once the user has interacted with
//!   the toggle, `enabled` is always `Some(_)`).
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
use crate::settings::components::{
    hairline, section_header, setting_card, setting_text, toggle_pill,
};

use super::super::window::SettingsWindow;

const PRIVACY_URL: &str = "https://paneflow.dev/legal/privacy";

impl SettingsWindow {
    pub(crate) fn render_privacy_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current = config.telemetry.as_ref().and_then(|t| t.enabled);
        let on = current == Some(true);
        let target_value = !on;

        let telemetry_row = div()
            .id("row-telemetry")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(ui.subtle))
            .child(setting_text(
                ui,
                "Anonymous telemetry",
                "Send anonymous events (startup, shutdown, updates) to help us \
                 understand how Paneflow is used. No paths, no terminal content, \
                 and no personal information are transmitted.",
            ))
            .child(toggle_pill(on, ui))
            .on_click(cx.listener(move |_this, _: &ClickEvent, _window, cx| {
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
            }));

        let privacy_link_row = div()
            .id("row-privacy-link")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(ui.subtle))
            .child(setting_text(
                ui,
                "Privacy policy",
                "Read what data we collect, how it's stored, and how to request deletion.",
            ))
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .text_color(ui.accent)
                    .child("Open in browser →"),
            )
            .on_click(|_: &ClickEvent, _window, _cx| {
                if let Err(e) = open::that(PRIVACY_URL) {
                    log::warn!("settings/privacy: failed to open {PRIVACY_URL}: {e}");
                }
            });

        let card = setting_card(ui)
            .child(telemetry_row)
            .child(hairline(ui))
            .child(privacy_link_row);

        div()
            .flex()
            .flex_col()
            .child(section_header(ui, "Telemetry"))
            .child(card)
    }
}
