//! "AI Agent" settings page - compact toggles for the built-in AI launcher
//! buttons rendered in every tab bar, plus the Claude bypass-permissions guard.
//!
//! Sections use lowercase eyebrows followed by `setting_card` groups of toggles,
//! separated by `hairline()` dividers. Only the switch is interactive - the row
//! itself does not hover or click.
//!
//! Persistence goes through [`PaneFlowApp::persist_setting`] - it mutates the
//! cached config for instant feedback and writes `paneflow.json` off the main
//! thread; `pane.rs` picks up the new state via the ConfigWatcher propagation so
//! the tab bar reflects changes without a restart. The MCP bridge installer
//! lives on its own page (`settings::tabs::mcp`).

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, div, img, prelude::*, px, rgb, svg,
};

use crate::PaneFlowApp;
use crate::agent_launcher::TerminalAgent;
use crate::settings::components::{
    hairline, section_header, setting_card, setting_text, toggle_pill,
};

impl PaneFlowApp {
    pub(crate) fn render_ai_agent_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Read the cached config (no per-frame `load_config()`).
        let config = &self.cached_config;
        let ui = crate::theme::ui_colors();

        // Effective state, not the raw key: an absent key defaults to
        // "shown only if the agent's CLI is installed" (see
        // `TerminalAgent::is_visible`). Toggling writes an explicit
        // `Some(..)` that pins the choice regardless of install state.
        let claude_visible = TerminalAgent::ClaudeCode.is_visible(config);
        let codex_visible = TerminalAgent::Codex.is_visible(config);
        let opencode_visible = TerminalAgent::OpenCode.is_visible(config);
        let pi_visible = TerminalAgent::Pi.is_visible(config);
        let hermes_agent_visible = TerminalAgent::Hermes.is_visible(config);
        let grok_visible = TerminalAgent::Grok.is_visible(config);
        let amp_visible = TerminalAgent::Amp.is_visible(config);
        let cursor_visible = TerminalAgent::Cursor.is_visible(config);
        let gemini_visible = TerminalAgent::Gemini.is_visible(config);
        let kiro_visible = TerminalAgent::Kiro.is_visible(config);
        let antigravity_visible = TerminalAgent::Antigravity.is_visible(config);
        let copilot_visible = TerminalAgent::Copilot.is_visible(config);
        let codebuddy_visible = TerminalAgent::CodeBuddy.is_visible(config);
        let factory_visible = TerminalAgent::Factory.is_visible(config);
        let qoder_visible = TerminalAgent::Qoder.is_visible(config);
        let openclaw_visible = TerminalAgent::Openclaw.is_visible(config);
        let bypass = config.claude_code_bypass_permissions.unwrap_or(false);
        // EP-003 US-009 (agent-control-plane): AI free-access mode + the
        // independent injection fence. Defaults: unrestricted OFF, fence ON.
        let unrestricted = config.ai_unrestricted_enabled();
        let fence = config.ai_injection_fence_enabled();
        let rosetta_enabled = config.rosetta_enabled();
        let rosetta_show_passive = config.rosetta_show_passive_enabled();

        let buttons_card = setting_card(ui)
            .child(setting_row(
                "row-claude-visible",
                "Claude Code",
                "Show the Claude Code launcher button in every tab bar.",
                Some(TerminalAgent::ClaudeCode),
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
                Some(TerminalAgent::Codex),
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
                Some(TerminalAgent::OpenCode),
                opencode_visible,
                "opencode_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-pi-visible",
                "Pi",
                "Show the Pi launcher button in every tab bar.",
                Some(TerminalAgent::Pi),
                pi_visible,
                "pi_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-hermes-agent-visible",
                "Hermes Agent",
                "Show the Hermes Agent launcher button in every tab bar.",
                Some(TerminalAgent::Hermes),
                hermes_agent_visible,
                "hermes_agent_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-grok-visible",
                "Grok",
                "Show the Grok launcher button in every tab bar.",
                Some(TerminalAgent::Grok),
                grok_visible,
                "grok_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-amp-visible",
                "Amp",
                "Show the Amp launcher button in every tab bar.",
                Some(TerminalAgent::Amp),
                amp_visible,
                "amp_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-cursor-visible",
                "Cursor",
                "Show the Cursor launcher button in every tab bar.",
                Some(TerminalAgent::Cursor),
                cursor_visible,
                "cursor_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-gemini-visible",
                "Gemini",
                "Show the Gemini launcher button in every tab bar.",
                Some(TerminalAgent::Gemini),
                gemini_visible,
                "gemini_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-kiro-visible",
                "Kiro",
                "Show the Kiro launcher button in every tab bar.",
                Some(TerminalAgent::Kiro),
                kiro_visible,
                "kiro_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-antigravity-visible",
                "Antigravity",
                "Show the Antigravity launcher button in every tab bar.",
                Some(TerminalAgent::Antigravity),
                antigravity_visible,
                "antigravity_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-copilot-visible",
                "Copilot",
                "Show the Copilot launcher button in every tab bar.",
                Some(TerminalAgent::Copilot),
                copilot_visible,
                "copilot_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-codebuddy-visible",
                "CodeBuddy",
                "Show the CodeBuddy launcher button in every tab bar.",
                Some(TerminalAgent::CodeBuddy),
                codebuddy_visible,
                "codebuddy_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-factory-visible",
                "Factory",
                "Show the Factory launcher button in every tab bar.",
                Some(TerminalAgent::Factory),
                factory_visible,
                "factory_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-qoder-visible",
                "Qoder",
                "Show the Qoder launcher button in every tab bar.",
                Some(TerminalAgent::Qoder),
                qoder_visible,
                "qoder_button_visible",
                ui,
                cx,
            ))
            .child(hairline(ui))
            .child(setting_row(
                "row-openclaw-visible",
                "Openclaw",
                "Show the Openclaw launcher button in every tab bar.",
                Some(TerminalAgent::Openclaw),
                openclaw_visible,
                "openclaw_button_visible",
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
            "Adds --permission-mode bypassPermissions whenever Paneflow \
             launches Claude Code in a terminal (tab-bar button and the \
             Agents-view thread picker). Anthropic warns this mode offers \
             no protection against prompt injection - only enable on \
             machines you trust.",
            None,
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

        let mut rosetta_card = setting_card(ui).child(setting_row(
            "row-rosetta-enabled",
            "Rosetta",
            "Show the in-app agent status card for supported Paneflow modes.",
            None,
            rosetta_enabled,
            "rosetta_enabled",
            ui,
            cx,
        ));
        if rosetta_enabled {
            rosetta_card = rosetta_card.child(hairline(ui)).child(setting_row(
                "row-rosetta-passive",
                "Show running agents",
                "Allow passive running-only agent rows to appear as a compact Rosetta card. Turn off for urgent-only status.",
                None,
                rosetta_show_passive,
                "rosetta_show_passive",
                ui,
                cx,
            ));
        }

        let rosetta_section = div()
            .mt(px(24.))
            .flex()
            .flex_col()
            .child(section_header(ui, "Rosetta"))
            .child(rosetta_card);

        // EP-003 US-009: AI access (free-access mode + injection fence). The
        // fence sub-toggle only appears once free-access is on: with the mode
        // off, surface.read is always fenced and there is nothing to relax.
        let mut access_card = setting_card(ui).child(setting_row(
            "row-ai-unrestricted",
            "AI free access",
            "Lets a conductor (a CLI agent or external orchestrator) auto-submit \
             prompts to your other panes without the PANEFLOW_IPC_SCRIPTING env \
             gate. Off by default. Best on isolated worktrees or throwaway \
             branches: an agent driving its peers has a wide blast radius. Every \
             write it makes is logged.",
            None,
            unrestricted,
            "ai_unrestricted",
            ui,
            cx,
        ));
        if unrestricted {
            access_card = access_card.child(hairline(ui)).child(setting_row(
                "row-ai-injection-fence",
                "Injection fence",
                "Keeps a peer pane's output wrapped as untrusted when a conductor \
                 reads it (surface.read / paneflow read), so a malicious repo \
                 cannot hijack the conductor. On by default even here: it \
                 protects the AI, it does not restrict it. Turning it off opens a \
                 hijack vector that resuming control by hand will not catch in time.",
                None,
                fence,
                "ai_injection_fence",
                ui,
                cx,
            ));
            // AC #3: once the fence is OFF, surface the active risk in red so
            // the trade-off is explicit and impossible to miss.
            if !fence {
                access_card = access_card.child(hairline(ui)).child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(rgb(0xE0_6C_75))
                        .child(
                            "Fence disabled: a malicious pane can redirect your \
                             conductor, and resuming control by hand will not undo \
                             a fast, silent injection. Re-enable it unless you fully \
                             trust every repo your agents read.",
                        ),
                );
            }
        }
        let access_section = div()
            .mt(px(24.))
            .flex()
            .flex_col()
            .child(section_header(ui, "AI access"))
            .child(access_card);

        div()
            .flex()
            .flex_col()
            .child(buttons_section)
            .child(permissions_section)
            .child(rosetta_section)
            .child(access_section)
            .child(div().h(px(180.)).flex_none())
    }
}

/// The agent's logo for its settings row, rendered identically to the tab
/// bar: multi-color logos via `img()` (native palette preserved), monochrome
/// logos via a `text_color`-tinted `svg()` mask (brand accent if any, else
/// the theme's primary text color).
fn agent_icon_el(agent: TerminalAgent, ui: crate::theme::UiColors) -> AnyElement {
    let path = SharedString::from(agent.icon_path());
    if agent.icon_multicolor() {
        img(path).size(px(18.)).flex_none().into_any_element()
    } else {
        let tint: Hsla = agent.accent().map(|c| rgb(c).into()).unwrap_or(ui.text);
        svg()
            .size(px(18.))
            .flex_none()
            .path(path)
            .text_color(tint)
            .into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn setting_row(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    icon: Option<TerminalAgent>,
    current: bool,
    config_key: &'static str,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> impl IntoElement {
    let target_value = !current;

    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .px(px(12.))
        .py(px(10.))
        .when_some(icon, |d, agent| d.child(agent_icon_el(agent, ui)))
        .child(setting_text(ui, title, description))
        .child(
            // Only the switch is interactive - the row no longer hovers/toggles.
            div()
                .id(SharedString::from(id))
                .flex_shrink_0()
                .cursor(CursorStyle::PointingHand)
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    // cache-mutate + notify + off-thread persist.
                    this.persist_setting(
                        false,
                        config_key,
                        serde_json::Value::Bool(target_value),
                        cx,
                    );
                }))
                .child(toggle_pill(current, ui)),
        )
}
