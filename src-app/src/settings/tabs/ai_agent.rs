//! "AI Agent" settings tab — agents-styled compact toggles for the
//! built-in AI command buttons rendered in the tab bar.
//!
//! Two sections, each a lowercase eyebrow ("Tab bar buttons",
//! "Permissions") followed by a `setting_card` containing one row per
//! toggle, separated by `hairline()` dividers. Each row is fully
//! clickable; the switch is purely visual.
//!
//! US-016: persistence goes through `SettingsWindow::persist_setting` — it
//! mutates the cached config for instant feedback and writes `paneflow.json`
//! off the main thread; `pane.rs` picks up the new state via the ConfigWatcher
//! propagation so the tab bar reflects changes without a restart.

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, div, img, prelude::*, px, rgb, svg,
};

use paneflow_mcp_install::{InstallKind, OverallState, StatusKind};

use crate::settings::components::{
    hairline, section_header, setting_card, setting_text, toggle_pill, with_alpha,
};

use super::super::window::SettingsWindow;
use crate::agent_launcher::TerminalAgent;

impl SettingsWindow {
    pub(crate) fn render_ai_agent_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // US-016: read the cached config (no per-frame `load_config()`).
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
             no protection against prompt injection — only enable on \
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

        div()
            .flex()
            .flex_col()
            .child(buttons_section)
            .child(permissions_section)
            .child(self.render_mcp_bridge_section(cx))
    }

    /// US-012 — "MCP bridge" section: one button that registers the
    /// embedded `paneflow-mcp` bridge with every detected CLI agent, plus a
    /// per-agent recap. State (status snapshot + last install result) is
    /// cached on the window and refreshed off-thread, so this render does
    /// zero config I/O.
    fn render_mcp_bridge_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();

        let state = self
            .mcp_status
            .as_deref()
            .map(paneflow_mcp_install::overall_state);

        // Button label + whether the click is live.
        let (label, enabled): (SharedString, bool) = if self.mcp_busy {
            ("Installing…".into(), false)
        } else {
            match state {
                None => ("Checking…".into(), false),
                Some(OverallState::NoAgents) => ("No agents detected".into(), false),
                Some(OverallState::AllInstalled) => ("Reinstall".into(), true),
                Some(OverallState::NeedsRepair) => ("Repair".into(), true),
                Some(OverallState::NeedsInstall) => ("Install MCP bridge".into(), true),
            }
        };

        let mut button = div()
            .id("mcp-install-btn")
            .flex_shrink_0()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(6.))
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::MEDIUM);
        button = if enabled {
            button
                .cursor(CursorStyle::PointingHand)
                .bg(ui.accent)
                .text_color(gpui::white())
                .hover(|s| s.bg(with_alpha(ui.accent, 0.85)))
                .child(label)
                .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                    this.start_mcp_install(cx);
                }))
        } else {
            button.bg(ui.subtle).text_color(ui.muted).child(label)
        };

        let header_row = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(16.))
            .px(px(12.))
            .py(px(10.))
            .child(setting_text(
                ui,
                "Read your panes from your agents",
                "Registers the bundled paneflow-mcp bridge with every detected CLI \
                 agent (Claude Code, Codex, Gemini, opencode) so they can read other \
                 panes' output. Idempotent, backed up, and only touches the paneflow \
                 entry. Re-run after an update if a path goes stale.",
            ))
            .child(button);

        let mut card = setting_card(ui).child(header_row);

        // Per-agent recap: prefer the last install result; otherwise show the
        // cached status snapshot. Each row is one agent + a short state.
        let recap_lines = self.mcp_recap_lines();
        if let Some(error) = self.mcp_install_error() {
            card = card.child(hairline(ui)).child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .text_size(px(12.))
                    .text_color(danger_color())
                    .child(error),
            );
        }
        for (line, is_error) in recap_lines {
            card = card.child(hairline(ui)).child(
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(if is_error { danger_color() } else { ui.muted })
                    .child(line),
            );
        }

        div()
            .mt(px(24.))
            .flex()
            .flex_col()
            .child(section_header(ui, "MCP bridge"))
            .child(card)
    }

    /// The refusal message from the last install, if it failed wholesale
    /// (bridge missing / data dir unresolved).
    fn mcp_install_error(&self) -> Option<SharedString> {
        match &self.mcp_install {
            Some(Err(msg)) => Some(SharedString::from(msg.clone())),
            _ => None,
        }
    }

    /// Per-agent recap lines `(text, is_error)`. Uses the last install
    /// result when present, else the cached status snapshot.
    fn mcp_recap_lines(&self) -> Vec<(SharedString, bool)> {
        if let Some(Ok(results)) = &self.mcp_install {
            return results
                .iter()
                .map(|r| {
                    let (state, err) = match &r.kind {
                        InstallKind::Installed => ("installed", false),
                        InstallKind::Updated => ("updated", false),
                        InstallKind::AlreadyCurrent => ("already up to date", false),
                        InstallKind::SkippedAbsent => ("not detected", false),
                        InstallKind::Error(e) => {
                            return (format!("{}: error — {e}", r.label).into(), true);
                        }
                    };
                    (format!("{}: {state}", r.label).into(), err)
                })
                .collect();
        }
        match &self.mcp_status {
            Some(statuses) => statuses
                .iter()
                .map(|r| {
                    let (state, err) = match &r.kind {
                        StatusKind::NotDetected => ("not detected", false),
                        StatusKind::Installed { .. } => ("installed", false),
                        StatusKind::Stale { .. } => ("stale path — click Repair", false),
                        StatusKind::NotInstalled => ("not installed", false),
                        StatusKind::Error(e) => {
                            return (format!("{}: error — {e}", r.label).into(), true);
                        }
                    };
                    (format!("{}: {state}", r.label).into(), err)
                })
                .collect(),
            None => Vec::new(),
        }
    }
}

impl SettingsWindow {
    /// Refresh the cached MCP bridge status off the main thread. Reads each
    /// agent's config (no writes), then stores the snapshot + repaints.
    /// Called once at window construction and after each install.
    pub(crate) fn refresh_mcp_status(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let status = smol::unblock(|| {
                let bridge = crate::runtime_paths::bridge_binary_path();
                paneflow_mcp_install::status_all(bridge.as_deref())
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_status = Some(status);
                cx.notify();
            });
        })
        .detach();
    }

    /// Install the bridge into every detected agent, off the main thread.
    /// Extracts the bridge binary first (so the registered path exists),
    /// then runs the install + a fresh status probe, and stores both.
    fn start_mcp_install(&mut self, cx: &mut Context<Self>) {
        if self.mcp_busy {
            return;
        }
        self.mcp_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (install, status) = smol::unblock(|| {
                // Guarantee the bridge binary is on disk at the stable path
                // before we write configs that point at it.
                let bridge = match crate::ai_hooks::extract::ensure_bridge_extracted() {
                    Ok(p) => Some(p),
                    Err(e) => {
                        log::warn!(
                            "settings: MCP bridge extraction failed ({e:#}); install may refuse"
                        );
                        crate::runtime_paths::bridge_binary_path()
                    }
                };
                let install = paneflow_mcp_install::install_all(bridge.as_deref());
                let status = paneflow_mcp_install::status_all(bridge.as_deref());
                (install, status)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.mcp_busy = false;
                this.mcp_install = Some(install);
                this.mcp_status = Some(status);
                cx.notify();
            });
        })
        .detach();
    }
}

/// Error-text color for the MCP recap. `UiColors` has no danger slot and
/// the sidebar/settings chrome is theme-independent, so a fixed One Dark
/// red keeps the cue readable on every bundled theme.
fn danger_color() -> gpui::Hsla {
    gpui::rgb(0xE0_6C_75).into()
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
        .when_some(icon, |d, agent| d.child(agent_icon_el(agent, ui)))
        .child(setting_text(ui, title, description))
        .child(toggle_pill(current, ui))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            // US-016: cache-mutate + notify + off-thread persist.
            this.persist_setting(false, config_key, serde_json::Value::Bool(target_value), cx);
        }))
}
