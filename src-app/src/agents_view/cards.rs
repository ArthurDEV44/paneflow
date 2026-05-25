//! Stateless GPUI render helpers for the US-005 missing-agents card.
//!
//! Returns a raw element tree with no click handlers attached --
//! the parent ([`super::view::AgentsView`]) wraps it in the right
//! interactive surface via `cx.listener`. The auth-required card
//! migrated to a Paneflow [`crate::widgets::callout::Callout`] in
//! `prd-agent-ui-visual-parity-2026-Q3.md` US-025, and the load-error
//! state landed in US-026 -- both render directly from
//! [`super::view`].
//!
//! Does not accept, store, or log credential material (US-005 AC #6).

use crate::theme::ui_colors;
use gpui::{FontWeight, IntoElement, ParentElement, SharedString, Styled, div, px, rgb};
use paneflow_acp::{MISSING_AGENTS_GUIDANCE, MissingAgentsGuidance};

/// Render the "No AI agents detected" empty state body (US-005 AC #4).
pub(crate) fn render_missing_agents_card() -> impl IntoElement {
    let ui = ui_colors();
    let guidance: &MissingAgentsGuidance = &MISSING_AGENTS_GUIDANCE;

    let commands = guidance
        .install_commands
        .iter()
        .map(|cmd| {
            div()
                .px(px(12.))
                .py(px(8.))
                .my(px(4.))
                .rounded(px(6.))
                .bg(rgb(0x1e1e2e))
                .border_1()
                .border_color(ui.subtle)
                .font_family("monospace")
                .text_size(px(12.))
                .text_color(ui.text)
                .child(SharedString::from(*cmd))
                .into_any_element()
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .max_w(px(560.))
                .px(px(28.))
                .py(px(24.))
                .gap(px(12.))
                .rounded(px(10.))
                .border_1()
                .border_color(ui.subtle)
                .bg(rgb(0x181825))
                .child(
                    div()
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.text)
                        .child(SharedString::from(guidance.title)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(ui.muted)
                        .child(SharedString::from(guidance.message)),
                )
                .children(commands)
                .child(
                    div()
                        .mt(px(4.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child(SharedString::from(format!(
                            "More info: {}",
                            guidance.more_url
                        ))),
                ),
        )
}
