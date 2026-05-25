//! `AgentsView` entity -- the US-005 host for the auth card and the
//! missing-agents empty state.
//!
//! Holds an [`AgentDiscovery`] instance (re-probes on
//! [`AgentsView::refresh`]), the live [`AgentsState`], and an optional
//! embedded [`TerminalView`] for the "Run login" flow.
//!
//! Implements [`gpui::Render`] so [`crate::PaneFlowApp`] can embed it
//! straight into its render tree via `Entity::clone()`.

use crate::agents::continuous_spinner::continuous_spinner;
use crate::agents_view::cards::render_missing_agents_card;
use crate::agents_view::state::{AgentLoadError, AgentsState};
use crate::runtime_paths::data_dir;
use crate::terminal::TerminalView;
use crate::theme::ui_colors;
use crate::widgets::callout::{Callout, CalloutIcon, CalloutSeverity};
use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Entity, EventEmitter, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::*, px, rgb,
};
use paneflow_acp::{AgentDiscovery, AgentKind, AuthRequirement, DiscoveredAgent};
use std::path::PathBuf;
use std::sync::Arc;

/// Event the AgentsView emits up to `PaneFlowApp` when the user
/// clicks the "Close" button in the view's header. The parent reacts
/// by clearing its `agents_view` slot so the next render falls back
/// to CLI mode.
pub(crate) struct CloseRequested;

impl EventEmitter<CloseRequested> for AgentsView {}

pub(crate) struct AgentsView {
    discovery: Arc<AgentDiscovery>,
    state: AgentsState,
    /// Live embedded `TerminalView` for the "Run login" flow.
    /// `Some` iff `state` is `AgentsState::LoginInProgress`.
    login_terminal: Option<Entity<TerminalView>>,
    /// US-024: tracks whether the new-user onboarding banner is
    /// hidden for this session. Mirrored to disk through
    /// [`onboarding_dismissal_marker`] so a restart picks up the
    /// dismissed state (AC #3 -- "persisted in config or session").
    onboarding_dismissed: bool,
}

impl AgentsView {
    pub(crate) fn new() -> Self {
        Self {
            discovery: Arc::new(AgentDiscovery::new()),
            state: AgentsState::Loading,
            login_terminal: None,
            onboarding_dismissed: onboarding_is_dismissed(),
        }
    }

    /// Re-run PATH discovery and recompute the state. Call this on
    /// initial open and whenever the Paneflow window regains focus
    /// (US-004 AC).
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        let agents = self.discovery.refresh();
        self.state = if agents.is_empty() {
            AgentsState::NoAgentsDetected
        } else {
            AgentsState::AgentsListed(agents)
        };
        self.login_terminal = None;
        cx.notify();
    }

    /// Simulate an `AuthRequired` response for `agent`. Used by the
    /// in-app "Try connect" button to demo the auth flow until the
    /// real ACP session-runner lands in US-013/US-016 (where the same
    /// state transition happens in response to a real
    /// `RequestPermission` / `AuthRequired` error from the wire).
    pub(crate) fn trigger_auth_for(&mut self, agent: AgentKind, cx: &mut Context<Self>) {
        let description = match agent {
            AgentKind::ClaudeCode => "Run `claude /login` in the terminal".to_string(),
            AgentKind::Codex => {
                "Sign in with ChatGPT or set CODEX_API_KEY / OPENAI_API_KEY".to_string()
            }
        };
        let requirement = AuthRequirement {
            agent,
            description,
            methods: Vec::new(),
            login_command: paneflow_acp::canonical_login_command(agent),
        };
        self.state = AgentsState::AuthRequired { requirement, agent };
        self.login_terminal = None;
        cx.notify();
    }

    /// Spawn the embedded login terminal for the current auth
    /// requirement. The terminal types the canonical login command
    /// (e.g. `claude /login\n`) into a fresh shell so the user sees
    /// what would otherwise need to be typed by hand.
    pub(crate) fn start_login(&mut self, cx: &mut Context<Self>) {
        let cmd = match &self.state {
            AgentsState::AuthRequired { requirement, .. } => requirement.login_command.clone(),
            _ => None,
        };
        let Some(cmd) = cmd else {
            return;
        };
        // workspace_id=0 is a sentinel for "ad-hoc terminal not bound
        // to any user workspace". The login TerminalView is detached
        // from the multiplexer's workspace tree.
        let terminal = cx.new(|cx| TerminalView::new(0, cx));
        terminal.update(cx, |view, _cx| {
            let mut typed = cmd.into_bytes();
            typed.push(b'\n');
            view.terminal.write_to_pty(typed);
        });
        self.login_terminal = Some(terminal);
        let prev_state = std::mem::replace(&mut self.state, AgentsState::Loading);
        self.state = prev_state.start_login();
        cx.notify();
    }

    /// Cancel the active login terminal and return to the auth card.
    pub(crate) fn cancel_login(&mut self, cx: &mut Context<Self>) {
        self.login_terminal = None;
        let prev_state = std::mem::replace(&mut self.state, AgentsState::Loading);
        self.state = prev_state.cancel_login();
        cx.notify();
    }

    /// US-026: seed the view with a [`AgentsState::LoadFailed`] state
    /// so the load-error Callout renders. Mirrors
    /// [`Self::trigger_auth_for`]: a dev / test entry point until the
    /// real ACP load-error emitter lands.
    #[allow(dead_code)]
    pub(crate) fn trigger_load_error(&mut self, error: AgentLoadError, cx: &mut Context<Self>) {
        self.state = AgentsState::LoadFailed { error };
        self.login_terminal = None;
        cx.notify();
    }

    /// US-024: hide the onboarding banner and persist the dismissal
    /// so a restart does not bring it back. Best-effort persistence:
    /// if the marker file cannot be written (read-only FS, broken
    /// env), the dismissal still applies for the current session and
    /// will reappear on next launch -- matches Zed's
    /// `AtomicBool::store + OnboardingUpsell::set_dismissed`
    /// best-effort contract.
    fn dismiss_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding_dismissed = true;
        write_onboarding_dismissed();
        cx.notify();
    }
}

/// Path to the onboarding-dismissed marker file in the Paneflow data
/// dir. `None` when `data_dir()` cannot be resolved (broken env, e.g.
/// container with no home).
fn onboarding_dismissal_marker() -> Option<PathBuf> {
    Some(data_dir()?.join("agents_onboarding_dismissed"))
}

fn onboarding_is_dismissed() -> bool {
    onboarding_dismissal_marker()
        .map(|p| p.exists())
        .unwrap_or(false)
}

fn write_onboarding_dismissed() {
    let Some(marker) = onboarding_dismissal_marker() else {
        return;
    };
    if let Err(e) = std::fs::write(&marker, b"1") {
        log::debug!(
            "paneflow: could not persist onboarding dismissal to {} ({e})",
            marker.display()
        );
    }
}

impl Render for AgentsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = ui_colors();
        let theme = crate::theme::active_theme();
        let body: AnyElement = match &self.state {
            AgentsState::Loading => render_loading().into_any_element(),
            AgentsState::NoAgentsDetected => render_missing_agents_card().into_any_element(),
            AgentsState::AgentsListed(agents) => render_agents_list(agents, cx).into_any_element(),
            AgentsState::AuthRequired { requirement, .. } => {
                render_auth_required(requirement, false, cx).into_any_element()
            }
            AgentsState::AuthPending { requirement, .. } => {
                render_auth_required(requirement, true, cx).into_any_element()
            }
            AgentsState::LoadFailed { error } => render_load_error(error, cx).into_any_element(),
            AgentsState::LoginInProgress { requirement, .. } => {
                render_login_in_progress(requirement, self.login_terminal.as_ref(), cx)
                    .into_any_element()
            }
        };

        let banner = self.render_onboarding_banner(cx);

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.title_bar_background)
            .text_color(ui.text)
            .children(banner)
            .child(div().flex_1().child(body))
    }
}

fn render_loading() -> impl IntoElement {
    let ui = ui_colors();
    div()
        .flex()
        .items_center()
        .justify_center()
        .size_full()
        .text_color(ui.muted)
        .child("Scanning PATH for AI agents...")
}

fn render_agents_list(
    agents: &[DiscoveredAgent],
    cx: &mut Context<AgentsView>,
) -> impl IntoElement {
    let ui = ui_colors();
    let rows = agents
        .iter()
        .map(|agent| {
            let kind = agent.kind;
            let name = kind.display_name();
            let icon_path = match kind {
                paneflow_acp::AgentKind::ClaudeCode => "icons/claude-color.svg",
                paneflow_acp::AgentKind::Codex => "icons/openai.svg",
            };
            // File-system signed-in probe (paneflow-acp/src/auth.rs):
            // `~/.claude/.credentials.json` for Claude Code,
            // `~/.codex/auth.json` OR `CODEX_API_KEY`/`OPENAI_API_KEY`
            // env for Codex. Drives the trailing chip only; the real
            // token validity check happens at session start (the ACP
            // runtime returns `AuthRequired` if the cached token has
            // expired, which still routes the user back to login).
            let signed_in = paneflow_acp::is_signed_in(kind);
            let trailing: AnyElement = if signed_in {
                // "Signed in" chip — Catppuccin Green, same palette as
                // the project git-diff +N badge in the sidebar.
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(0xa6e3a1))
                    .child(
                        gpui::svg()
                            .size(px(11.))
                            .flex_none()
                            .path("icons/check.svg")
                            .text_color(rgb(0xa6e3a1)),
                    )
                    .child("Signed in")
                    .into_any_element()
            } else {
                div()
                    .id(SharedString::from(format!("agents-view-try-{name}")))
                    .flex_none()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .text_color(ui.muted)
                    .text_size(px(12.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| {
                        let ui = ui_colors();
                        s.bg(ui.subtle).text_color(ui.text)
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.trigger_auth_for(kind, cx);
                    }))
                    .child("Sign in")
                    .into_any_element()
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .px(px(12.))
                .py(px(10.))
                .my(px(4.))
                .rounded(px(10.))
                .bg(ui.surface)
                .border_1()
                .border_color(ui.border)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .min_w_0()
                        .child(
                            gpui::svg()
                                .size(px(16.))
                                .flex_none()
                                .path(icon_path)
                                .text_color(ui.text),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(ui.text)
                                .child(SharedString::from(name)),
                        ),
                )
                .child(trailing)
                .into_any_element()
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .size_full()
        .px(px(20.))
        .py(px(16.))
        .child(
            // Centered, content-width column so the cards do not
            // stretch edge-to-edge on a wide window.
            div()
                .w_full()
                .max_w(px(640.))
                .mx_auto()
                .flex()
                .flex_col()
                .children(rows),
        )
}

/// US-025: render the auth-required state as a Paneflow
/// [`Callout`]. `pending` switches the right-side slot from the
/// "Run login -- {cmd}" button to a rotating spinner mirroring Zed's
/// `IconName::ArrowCircle.with_rotate_animation(2)`.
///
/// Mirrors Zed's `render_auth_required_state` (severity Info, title
/// "Authenticate to {agent}" or "Authenticating to {agent}...",
/// description = the per-agent guidance string, actions slot = the
/// primary login action). The configuration_view + per-method buttons
/// from Zed (multi-method picker) are reduced to a single primary
/// action because Paneflow only ships one auth path per agent today.
fn render_auth_required(
    requirement: &AuthRequirement,
    pending: bool,
    cx: &mut Context<AgentsView>,
) -> impl IntoElement {
    let agent_name = requirement.agent.display_name();
    let title = if pending {
        format!("Authenticating to {agent_name}\u{2026}")
    } else {
        format!("Authenticate to {agent_name}")
    };

    let actions_slot: AnyElement = if pending {
        // Zed: Icon::new(IconName::ArrowCircle).with_rotate_animation(2)
        // -- Paneflow uses the existing 1s loader spinner with the
        // accent tint. The `2` constant in Zed is a 2s rotation
        // period; Paneflow's spinner is 1s but the visual intent
        // (continuous rotation while we wait) is preserved.
        continuous_spinner("agents-view-auth-pending", px(16.), ui_colors().accent)
            .into_any_element()
    } else if let Some(cmd) = requirement.login_command.clone() {
        let label = format!("Run login \u{2014} {cmd}");
        render_primary_button(
            "agents-view-run-login",
            label,
            cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.start_login(cx);
            }),
        )
    } else {
        div().into_any_element()
    };

    let description_slot: AnyElement = render_auth_description(requirement);

    let callout = Callout::new(CalloutSeverity::Info, title)
        .icon(CalloutIcon::Info)
        .description_slot(description_slot)
        .actions_slot(actions_slot)
        .render();

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .size_full()
        .child(callout)
}

/// US-025 description slot: the per-agent guidance string + the list
/// of advertised auth methods (when present). Stays close to
/// [`crate::agents_view::cards::render_auth_card_body`] but emits
/// Callout-shaped content (single column, no outer border, no title
/// -- the Callout owns those).
fn render_auth_description(requirement: &AuthRequirement) -> AnyElement {
    let ui = ui_colors();
    let methods: Vec<AnyElement> = requirement
        .methods
        .iter()
        .map(|m| {
            let mut item = div()
                .flex()
                .flex_col()
                .px(px(10.))
                .py(px(8.))
                .rounded(px(6.))
                .bg(rgb(0x1e1e2e))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ui.text)
                        .child(SharedString::from(m.name.clone())),
                );
            if let Some(d) = &m.description {
                item = item.child(
                    div()
                        .mt(px(2.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child(SharedString::from(d.clone())),
                );
            }
            item.into_any_element()
        })
        .collect();

    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(ui.muted)
                .child(SharedString::from(requirement.description.clone())),
        )
        .children(methods)
        .into_any_element()
}

fn render_primary_button(
    id: &'static str,
    label: String,
    handler: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let ui = ui_colors();
    div()
        .id(SharedString::from(id))
        .px(px(14.))
        .py(px(8.))
        .rounded(px(6.))
        .bg(ui.accent)
        .text_color(rgb(0xffffff))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.opacity(0.9))
        .on_mouse_down(MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .on_click(handler)
        .child(SharedString::from(label))
        .into_any_element()
}

/// US-026: render an `AgentLoadError` as an Error-severity Callout
/// (XCircle icon, title from
/// [`AgentLoadError::title`], multi-line body, Copy button when the
/// body is non-trivial). Body capped at `max_h(px(220.))` +
/// scrollable so a 10KB stderr does not blow up the layout (AC #3).
fn render_load_error(error: &AgentLoadError, cx: &mut Context<AgentsView>) -> impl IntoElement {
    let ui = ui_colors();
    let body_text = error.body();
    let body_for_copy = body_text.clone();

    let body_block = div()
        .id("agents-view-load-error-body")
        .flex()
        .flex_col()
        .max_h(px(220.))
        .overflow_y_scroll()
        .text_size(px(13.))
        .text_color(ui.muted)
        .font_family("monospace")
        .child(SharedString::from(body_text))
        .into_any_element();

    let copy_button: AnyElement = render_primary_button(
        "agents-view-copy-load-error",
        "Copy".to_string(),
        cx.listener(move |_this, _: &ClickEvent, _w, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(body_for_copy.clone()));
        }),
    );

    let callout = Callout::new(CalloutSeverity::Error, error.title())
        .icon(CalloutIcon::XCircle)
        .description_slot(body_block)
        .actions_slot(copy_button)
        .render();

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .size_full()
        .px(px(20.))
        .child(callout)
}

impl AgentsView {
    /// US-024: optional onboarding banner that sits at the top of the
    /// panel for new users. Returns `None` once dismissed (volatile +
    /// persisted via [`write_onboarding_dismissed`]) or when the
    /// current state already conveys "no agents to onboard with"
    /// (the missing-agents card carries its own install guidance, so
    /// stacking two banners on top of it adds noise without value).
    fn render_onboarding_banner(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.onboarding_dismissed {
            return None;
        }
        // Hide while the agent is actively authenticating / loading
        // so the banner does not distract from the modal-ish flow.
        if matches!(
            self.state,
            AgentsState::AuthRequired { .. }
                | AgentsState::AuthPending { .. }
                | AgentsState::LoginInProgress { .. }
                | AgentsState::LoadFailed { .. }
                | AgentsState::NoAgentsDetected
        ) {
            return None;
        }
        let ui = ui_colors();
        let banner = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(12.))
            .w_full()
            .px(px(20.))
            .pt(px(16.))
            .pb(px(12.))
            .child(
                // Centered max-width column so the banner copy lines
                // up with the agent cards beneath.
                div()
                    .flex_1()
                    .w_full()
                    .max_w(px(640.))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(ui.text)
                            .child("Welcome to Paneflow Agents"),
                    )
                    .child(div().text_size(px(12.)).text_color(ui.muted).child(
                        "Pick an AI agent below to start a thread. Sessions persist \
                                 across restarts and run in their own working directory.",
                    )),
            )
            .child(
                div()
                    .id("agents-view-onboarding-dismiss")
                    .flex_none()
                    .size(px(22.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| {
                        let ui = ui_colors();
                        s.bg(ui.subtle)
                    })
                    .text_color(ui.muted)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.dismiss_onboarding(cx);
                    }))
                    .child(
                        gpui::svg()
                            .size(px(12.))
                            .flex_none()
                            .path("icons/close.svg")
                            .text_color(ui.muted),
                    ),
            );

        Some(banner.into_any_element())
    }
}

fn render_login_in_progress(
    requirement: &AuthRequirement,
    terminal: Option<&Entity<TerminalView>>,
    cx: &mut Context<AgentsView>,
) -> impl IntoElement {
    let ui = ui_colors();
    let cmd = requirement
        .login_command
        .clone()
        .unwrap_or_else(|| "login".to_string());
    let mut body = div()
        .flex()
        .flex_col()
        .size_full()
        .px(px(28.))
        .py(px(20.))
        .gap(px(8.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.text)
                        .child(SharedString::from(format!("Running: {cmd}"))),
                )
                .child(
                    div()
                        .id("agents-view-cancel-login")
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(6.))
                        .bg(ui.subtle)
                        .text_color(ui.text)
                        .text_size(px(12.))
                        .cursor(CursorStyle::PointingHand)
                        .hover(|s| s.opacity(0.85))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.cancel_login(cx);
                        }))
                        .child("Close"),
                ),
        );
    if let Some(term) = terminal {
        body = body.child(
            div()
                .flex_1()
                .border_1()
                .border_color(ui.subtle)
                .rounded(px(8.))
                .bg(rgb(0x0e0e16))
                .child(term.clone()),
        );
    }
    body
}
