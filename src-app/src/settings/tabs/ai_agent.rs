//! "AI Agent" settings tab — agents-styled compact toggles for the
//! built-in AI command buttons rendered in the tab bar.
//!
//! Two sections, each a lowercase eyebrow ("Tab bar buttons",
//! "Permissions") followed by a `setting_card` containing one row per
//! toggle, separated by `hairline()` dividers. Each row is fully
//! clickable; the switch is purely visual.
//!
//! Persistence mirrors `tabs::privacy`: clicks call
//! `config_writer::save_config_value_checked`, and `pane.rs` re-reads
//! the config on the next render so the tab bar reflects changes
//! without a restart.

use gpui::{
    ClickEvent, Context, CursorStyle, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, prelude::*, px,
};

use crate::config_writer;
use crate::settings::components::{
    hairline, section_header, setting_card, setting_text, toggle_pill,
};

use super::super::window::SettingsWindow;

impl SettingsWindow {
    pub(crate) fn render_ai_agent_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();

        let claude_visible = config.claude_code_button_visible.unwrap_or(true);
        let codex_visible = config.codex_button_visible.unwrap_or(true);
        let opencode_visible = config.opencode_button_visible.unwrap_or(true);
        let bypass = config.claude_code_bypass_permissions.unwrap_or(false);

        let buttons_card = setting_card(ui)
            .child(setting_row(
                "row-claude-visible",
                "Claude Code",
                "Show the Claude Code launcher button in every tab bar.",
                claude_visible,
                "claude_code_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-codex-visible",
                "Codex",
                "Show the Codex launcher button in every tab bar.",
                codex_visible,
                "codex_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-opencode-visible",
                "Opencode",
                "Show the Opencode launcher button in every tab bar.",
                opencode_visible,
                "opencode_button_visible",
                ui,
                cx,
            ));

        let buttons_section = div()
            .flex()
            .flex_col()
            .child(section_header(ui, "Tab bar buttons"))
            .child(buttons_card);

        let permissions_card = setting_card(ui).child(setting_row(
            "row-claude-bypass",
            "Bypass permissions",
            "Disables every confirmation prompt across Paneflow. The \
             terminal Claude Code launcher adds --permission-mode \
             bypassPermissions, and the Agents view auto-approves every \
             tool call for both Claude Code and Codex sessions. \
             Anthropic warns this mode offers no protection against \
             prompt injection — only enable on machines you trust.",
            bypass,
            "claude_code_bypass_permissions",
            ui,
            cx,
        ));

        let permissions_section = div()
            .mt(px(24.))
            .flex()
            .flex_col()
            .child(section_header(ui, "Permissions"))
            .child(permissions_card);

        div()
            .flex()
            .flex_col()
            .child(buttons_section)
            .child(permissions_section)
    }
}

fn setting_row(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    current: bool,
    config_key: &'static str,
    ui: crate::theme::UiColors,
    cx: &mut Context<SettingsWindow>,
) -> impl IntoElement {
    let target_value = !current;

    div()
        .id(SharedString::from(id))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .px(px(12.))
        .py(px(10.))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(ui.subtle))
        .child(setting_text(ui, title, description))
        .child(toggle_pill(current, ui))
        .on_click(cx.listener(move |_this, _: &ClickEvent, _window, cx| {
            let ok = config_writer::save_config_value_checked(
                config_key,
                serde_json::Value::Bool(target_value),
            );
            if !ok {
                log::warn!(
                    "settings/ai_agent: failed to persist {config_key} = {target_value}; choice is in-memory only for this session"
                );
            }
            cx.notify();
        }))
}
