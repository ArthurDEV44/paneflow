//! AI agent sessions popover — opened by clicking the per-tab-bar
//! sessions icon. A two-tab strip switches between Claude Code and Codex
//! CLI history; clicking a row issues `claude --resume <id>` or
//! `codex resume <id>` back into the originating terminal.
//!
//! The popover is anchored to the click position emitted by
//! `PaneEvent::OpenClaudeSessions`, opens down-and-LEFT from the click
//! (the icon sits on the far right of the tab bar), and is rendered via
//! `deferred()` at the `PaneFlowApp` layer (a popover painted inside
//! `Pane::render` would be clipped to the pane bbox). We deliberately do
//! NOT clamp against `window.window_bounds()`: on Linux/Wayland the
//! bounds API returns logical pixels while `ClickEvent::position()` came
//! back in a different coordinate space, which broke the clamp.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, ParentElement, Pixels, Point, SharedString, Styled, Window, deferred, div,
    prelude::*, px,
};

use crate::PaneFlowApp;
use crate::agent_sessions::{SessionAgent, SessionMeta, format_relative_time};
use crate::widgets::scrollbar::{self, SCROLLBAR_GUTTER};

const MENU_WIDTH: Pixels = px(360.);
const ROW_HEIGHT: Pixels = px(48.);
/// Cap the rendered list to a comfortable height; older sessions stay
/// available via scroll.
const MAX_VISIBLE_ROWS: usize = 8;

impl PaneFlowApp {
    pub(crate) fn render_claude_sessions_menu(
        &self,
        anchor: Point<Pixels>,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui_colors();

        let anchor_x = f32::from(anchor.x);
        let menu_w = f32::from(MENU_WIDTH);
        let desired_left = (anchor_x - menu_w + 16.0).max(4.0);
        let left = px(desired_left);
        let top = px(f32::from(anchor.y) + 8.0);

        let header = self.sessions_menu_header(ui);
        let tab_strip = self.sessions_menu_tabs(ui, cx);
        let body = self.sessions_menu_body(cx);

        deferred(
            div()
                .id("claude-sessions-menu")
                .occlude()
                .absolute()
                .left(left)
                .top(top)
                .w(MENU_WIDTH)
                .max_h(px(460.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .rounded(px(8.))
                .shadow_lg()
                .flex()
                .flex_col()
                .overflow_hidden()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.close_claude_sessions_menu(cx);
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .on_mouse_move(cx.listener(Self::on_sessions_drag_move))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        if this.claude_sessions_drag.take().is_some() {
                            cx.notify();
                        }
                    }),
                )
                .child(header)
                .child(tab_strip)
                .child(body),
        )
        .priority(3)
        .into_any_element()
    }

    fn sessions_menu_header(&self, ui: crate::theme::UiColors) -> AnyElement {
        let cwd_label: SharedString = self
            .claude_sessions_cwd
            .as_deref()
            .map(prettify_cwd)
            .unwrap_or_else(|| "Unknown directory".to_string())
            .into();
        div()
            .flex()
            .flex_col()
            .px(px(14.))
            .pt(px(10.))
            .pb(px(8.))
            .gap(px(3.))
            .border_b_1()
            .border_color(ui.border)
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(ui.text)
                    .child("AI agent sessions"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(cwd_label),
            )
            .into_any_element()
    }

    /// Tab strip toggling between the *visible* AI agents. Pills for
    /// agents whose tab-bar button has been toggled off in
    /// Settings → AI Agent are suppressed — the popover only surfaces
    /// what the user has opted into.
    fn sessions_menu_tabs(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> AnyElement {
        let active = self.sessions_active_agent;
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .py(px(8.))
            .border_b_1()
            .border_color(ui.border);

        for agent in crate::agent_sessions::enabled_session_agents() {
            let (id, label, count) = match agent {
                SessionAgent::Claude => (
                    "claude-sessions-tab-claude",
                    "Claude",
                    self.claude_sessions.len(),
                ),
                SessionAgent::Codex => (
                    "claude-sessions-tab-codex",
                    "Codex",
                    self.codex_sessions.len(),
                ),
                SessionAgent::OpenCode => (
                    "claude-sessions-tab-opencode",
                    "OpenCode",
                    self.opencode_sessions.len(),
                ),
            };
            let is_active = active == agent;
            row = row.child(self.sessions_tab_pill(
                id,
                label,
                count,
                is_active,
                ui,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_sessions_active_agent(agent, cx);
                    cx.stop_propagation();
                }),
            ));
        }
        row.into_any_element()
    }

    fn sessions_tab_pill(
        &self,
        id: &'static str,
        label: &'static str,
        count: usize,
        is_active: bool,
        ui: crate::theme::UiColors,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> AnyElement {
        let count_label: SharedString = format!("{count}").into();
        let pill = div()
            .id(SharedString::from(id))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::NORMAL)
            .when(is_active, |d| d.bg(ui.surface).text_color(ui.text))
            .when(!is_active, |d| {
                d.text_color(ui.muted).hover(|s| s.bg(ui.subtle))
            })
            .on_click(on_click);

        let badge_bg = if is_active { ui.subtle } else { ui.surface };
        pill.child(div().child(label))
            .child(
                div()
                    .px(px(5.))
                    .py(px(0.))
                    .rounded(px(4.))
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .bg(badge_bg)
                    .child(count_label),
            )
            .into_any_element()
    }

    /// Switch the active tab. Resets the scroll handle so the new list
    /// always opens at the top, and clears any in-flight drag (which
    /// would dangle if the user dragged on one tab and then switched).
    fn set_sessions_active_agent(&mut self, agent: SessionAgent, cx: &mut Context<Self>) {
        if self.sessions_active_agent == agent {
            return;
        }
        self.sessions_active_agent = agent;
        self.claude_sessions_scroll = gpui::ScrollHandle::new();
        self.claude_sessions_drag = None;
        cx.notify();
    }

    fn sessions_menu_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        // Settings can flip an agent off while the popover is open. Fall
        // back to the first still-enabled agent so the body never points
        // at a hidden tab — the open path also seeds `sessions_active_agent`
        // to an enabled one, so the mismatch only happens on a live config
        // change. The render layer absorbs it instead of pushing a mutation
        // through `&self`.
        let enabled = crate::agent_sessions::enabled_session_agents();
        let agent = if enabled.contains(&self.sessions_active_agent) {
            self.sessions_active_agent
        } else {
            enabled
                .first()
                .copied()
                .unwrap_or(self.sessions_active_agent)
        };
        let sessions: &[SessionMeta] = match agent {
            SessionAgent::Claude => &self.claude_sessions,
            SessionAgent::Codex => &self.codex_sessions,
            SessionAgent::OpenCode => &self.opencode_sessions,
        };

        if sessions.is_empty() {
            return div()
                .flex()
                .flex_col()
                .p(px(8.))
                .gap(px(6.))
                .child(
                    div()
                        .px(px(8.))
                        .py(px(12.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .text_align(gpui::TextAlign::Center)
                        .child(empty_message(self.claude_sessions_cwd.is_some(), agent)),
                )
                .child(self.sessions_menu_start_row(agent, ui, cx))
                .into_any_element();
        }

        let mut list = div()
            .id("claude-sessions-list")
            .flex()
            .flex_col()
            .pr(SCROLLBAR_GUTTER + px(2.))
            .py(px(6.))
            .gap(px(2.))
            .max_h(ROW_HEIGHT * (MAX_VISIBLE_ROWS as f32) + px(12.))
            .overflow_y_scroll()
            .track_scroll(&self.claude_sessions_scroll);

        for session in sessions {
            list = list.child(self.sessions_menu_row(session, ui, cx));
        }

        const PER_ROW: f32 = 50.0;
        const PADDING_V: f32 = 8.0;
        let est_content = (sessions.len() as f32 * PER_ROW - 2.0).max(0.0) + PADDING_V;
        let max_viewport = f32::from(ROW_HEIGHT) * (MAX_VISIBLE_ROWS as f32) + PADDING_V;

        let bar = scrollbar::render(
            &self.claude_sessions_scroll,
            ui,
            Some((est_content, max_viewport)),
            "claude-sessions-track",
            "claude-sessions-thumb",
            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                if let Some(off) =
                    scrollbar::track_click_offset(&this.claude_sessions_scroll, ev.position.y)
                {
                    this.claude_sessions_scroll
                        .set_offset(Point::new(px(0.), px(off)));
                    cx.notify();
                }
            }),
            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                this.claude_sessions_drag = Some(scrollbar::begin_drag(
                    &this.claude_sessions_scroll,
                    ev.position.y,
                ));
                cx.stop_propagation();
            }),
        );

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("claude-sessions-scroll-wrapper")
                    .relative()
                    .flex()
                    .flex_col()
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .child(list)
                    .when_some(bar, |d, sb| d.child(sb)),
            )
            .child(div().h_px().bg(ui.border))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .p(px(6.))
                    .child(self.sessions_menu_start_row(agent, ui, cx)),
            )
            .into_any_element()
    }

    fn on_sessions_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(drag) = self.claude_sessions_drag
            && let Some(off) =
                scrollbar::drag_offset(&self.claude_sessions_scroll, &drag, event.position.y)
        {
            self.claude_sessions_scroll
                .set_offset(Point::new(px(0.), px(off)));
            cx.notify();
        }
    }

    fn sessions_menu_row(
        &self,
        session: &SessionMeta,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agent = session.agent;
        let session_id = session.session_id.clone();
        let row_id = SharedString::from(format!("{}-session-{session_id}", agent_id_prefix(agent)));
        let when = SharedString::from(format_relative_time(&session.timestamp));
        let branch_label = if session.git_branch.is_empty() {
            None
        } else {
            Some(SharedString::from(session.git_branch.clone()))
        };
        let title: SharedString = session
            .summary
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| short_session_id(&session_id))
            .into();

        let meta_row = {
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .text_size(px(10.))
                .text_color(ui.muted)
                .child(div().child(when));
            if let Some(branch) = branch_label {
                row = row
                    .child(div().w_px().h(px(10.)).bg(ui.border))
                    .child(div().overflow_x_hidden().text_ellipsis().child(branch));
            }
            row
        };

        div()
            .id(row_id)
            .flex()
            .flex_col()
            .h(ROW_HEIGHT)
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .gap(px(2.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                let cmd = resume_command(agent, &session_id);
                this.send_command_to_sessions_pane(&cmd, cx);
                this.close_claude_sessions_menu(cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(ui.text)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
            .child(meta_row)
            .into_any_element()
    }

    fn sessions_menu_start_row(
        &self,
        agent: SessionAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label = match agent {
            SessionAgent::Claude => "Start a new Claude session",
            SessionAgent::Codex => "Start a new Codex session",
            SessionAgent::OpenCode => "Start a new OpenCode session",
        };
        div()
            .id(SharedString::from(format!(
                "{}-sessions-start",
                agent_id_prefix(agent)
            )))
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .h(px(30.))
            .px(px(10.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(ui.text)
            .bg(ui.subtle)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.border)
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                let cmd = fresh_session_command(agent);
                this.send_command_to_sessions_pane(&cmd, cx);
                this.close_claude_sessions_menu(cx);
                cx.stop_propagation();
            }))
            .child(label)
            .into_any_element()
    }

    /// Send a shell command to the pane that opened the popover. Silently
    /// no-ops when the originating pane has been dropped (closed/replaced
    /// while the menu was open) or the pane no longer has a terminal tab.
    fn send_command_to_sessions_pane(&self, command: &str, cx: &mut Context<Self>) {
        let Some(pane_handle) = self.claude_sessions_pane.as_ref() else {
            return;
        };
        let Some(pane) = pane_handle.upgrade() else {
            return;
        };
        let pane_ref = pane.read(cx);
        if let Some(terminal) = pane_ref.active_terminal_opt() {
            terminal.read(cx).send_command(command);
        }
    }

    /// Tear down all popover state in one place — used by the
    /// `on_mouse_down_out` handler and the per-row click paths.
    fn close_claude_sessions_menu(&mut self, cx: &mut Context<Self>) {
        self.claude_sessions_menu_open = None;
        self.claude_sessions.clear();
        self.codex_sessions.clear();
        self.opencode_sessions.clear();
        self.claude_sessions_cwd = None;
        self.claude_sessions_pane = None;
        self.claude_sessions_drag = None;
        self.sessions_active_agent = SessionAgent::Claude;
        cx.notify();
    }
}

fn prettify_cwd(cwd: &str) -> String {
    if let Some(home) = dirs::home_dir().and_then(|p| p.to_str().map(str::to_owned))
        && let Some(rest) = cwd.strip_prefix(home.as_str())
    {
        return format!("~{rest}");
    }
    cwd.to_string()
}

fn empty_message(cwd_known: bool, agent: SessionAgent) -> SharedString {
    if !cwd_known {
        return SharedString::from("Could not detect the terminal's working directory.");
    }
    match agent {
        SessionAgent::Claude => {
            SharedString::from("No Claude Code sessions for this directory yet.")
        }
        SessionAgent::Codex => SharedString::from("No Codex CLI sessions for this directory yet."),
        SessionAgent::OpenCode => {
            SharedString::from("No OpenCode sessions for this directory yet.")
        }
    }
}

fn short_session_id(id: &str) -> String {
    id.split('-').next().unwrap_or(id).to_string()
}

fn agent_id_prefix(agent: SessionAgent) -> &'static str {
    match agent {
        SessionAgent::Claude => "claude",
        SessionAgent::Codex => "codex",
        SessionAgent::OpenCode => "opencode",
    }
}

/// True when Settings -> AI Agent has `claude_code_bypass_permissions`
/// toggled on. Read at click time so a flip in Settings while the
/// popover is open takes effect on the next click without a restart.
fn claude_bypass_enabled() -> bool {
    paneflow_config::loader::load_config()
        .claude_code_bypass_permissions
        .unwrap_or(true)
}

/// Build the command sent to the originating terminal when a session row
/// is clicked. For Claude, honor `claude_code_bypass_permissions` so
/// resumed sessions get the same `--permission-mode bypassPermissions`
/// treatment as a fresh launch from the tab-bar button (`pane.rs:685`).
fn resume_command(agent: SessionAgent, session_id: &str) -> String {
    match agent {
        SessionAgent::Claude => {
            if claude_bypass_enabled() {
                format!("claude --resume {session_id} --permission-mode bypassPermissions")
            } else {
                format!("claude --resume {session_id}")
            }
        }
        SessionAgent::Codex => format!("codex resume {session_id}"),
        SessionAgent::OpenCode => format!("opencode --session {session_id}"),
    }
}

/// Build the command sent when the user clicks "Start a new <agent>
/// session" in the popover footer. Mirrors `resume_command` for the
/// Claude bypass flag.
fn fresh_session_command(agent: SessionAgent) -> String {
    match agent {
        SessionAgent::Claude => {
            if claude_bypass_enabled() {
                "claude --permission-mode bypassPermissions".to_string()
            } else {
                "claude".to_string()
            }
        }
        SessionAgent::Codex => "codex".to_string(),
        SessionAgent::OpenCode => "opencode".to_string(),
    }
}
