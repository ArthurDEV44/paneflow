//! Agent-sessions right sidebar (PRD `prd-agent-sessions-sidebar-2026-Q3`,
//! EP-001).
//!
//! Docked panel that replaces the former anchored popover: it lists the active
//! terminal's cwd-scoped sessions for every enabled agent (Claude Code / Codex
//! / OpenCode) as stacked groups. Toggled by the tab-bar sessions button via
//! `PaneEvent::ToggleAgentSessions`; it stays open while you work because it is
//! a layout child of the root row, not a `deferred()` overlay. Clicking a row
//! issues the agent's `--resume` command into the bound pane and keeps the
//! sidebar open.
//!
//! Reuses the session data layer verbatim (`SessionMeta`,
//! `read_sessions_for_cwd`, `enabled_session_agents`). Per-group cap-5 /
//! "Show more" / collapse caret and the per-group "new session" affordance land
//! in EP-002 — this slice swaps the surface and renders flat groups.

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, Pixels, SharedString, Styled, div, prelude::*, px, rgb, svg,
};

use crate::PaneFlowApp;
use crate::agent_sessions::{SessionAgent, SessionMeta, format_relative_time};
use crate::pane_drag::{SessionDrag, TabDragPreview};

/// Fixed sidebar width — between the CLI (220) and Agents (280) left sidebars,
/// matching VS Code's secondary-bar default. Resizable width is deferred.
const SIDEBAR_WIDTH: Pixels = px(300.);
const ROW_HEIGHT: Pixels = px(30.);

impl PaneFlowApp {
    /// Render the docked sessions sidebar (right edge of the root `flex_row`).
    /// Only called when `sessions_sidebar_open` is true.
    pub(crate) fn render_sessions_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        div()
            .id("sessions-sidebar")
            .flex()
            .flex_col()
            .w(SIDEBAR_WIDTH)
            .flex_shrink_0()
            .h_full()
            .bg(ui.surface)
            .border_l_1()
            .border_color(ui.border)
            .child(self.sessions_sidebar_header(ui, cx))
            .child(self.sessions_sidebar_body(ui, cx))
            .into_any_element()
    }

    fn sessions_sidebar_header(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            // Fixed 32px height + bottom border so the header lines up exactly
            // with the pane tab strip on the left (`TAB_BAR_HEIGHT` in pane.rs).
            .h(px(32.))
            .flex_none()
            .px(px(10.))
            .border_b_1()
            .border_color(ui.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ui.text)
                    .child("Agent sessions"),
            )
            .child(
                div()
                    .id("sessions-sidebar-close")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(14.))
                    .text_color(ui.muted)
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle).text_color(ui.text)
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.close_sessions_sidebar(cx);
                        cx.stop_propagation();
                    }))
                    .child("×"),
            )
            .into_any_element()
    }

    fn sessions_sidebar_body(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.claude_sessions_cwd.is_none() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .p(px(14.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("Could not detect the terminal's working directory."),
                )
                .into_any_element();
        }

        // US-008: an agent can be toggled off in Settings while the sidebar is
        // open. The list is driven entirely by `enabled_session_agents()`, so a
        // disabled agent's group simply disappears on the next render; if the
        // user disables them all, show an empty state rather than a blank panel.
        let enabled = crate::agent_sessions::enabled_session_agents();
        if enabled.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .p(px(14.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("No AI agents enabled. Enable one in Settings → AI Agent."),
                )
                .into_any_element();
        }

        let mut body = div()
            .id("sessions-sidebar-body")
            .flex()
            .flex_col()
            .flex_1()
            .py(px(6.))
            // US-009: vertical scroll only — never let a long row title push the
            // panel into horizontal scrolling.
            .overflow_x_hidden()
            .overflow_y_scroll()
            .track_scroll(&self.claude_sessions_scroll);

        for agent in enabled {
            body = body.child(self.sessions_group(agent, ui, cx));
        }
        body.into_any_element()
    }

    fn sessions_group(
        &self,
        agent: SessionAgent,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let idx = agent_index(agent);
        let collapsed = self.sessions_group_collapsed[idx];
        let show_all = self.sessions_group_show_all[idx];
        let scanning = self.sessions_scanning[idx];
        let sessions = self.sessions_for(agent);
        // Distinct chevron per state (US-006): right = collapsed, down =
        // expanded — a static swap, not a tween, so it reads under reduced
        // motion.
        let chevron = if collapsed {
            "icons/chevron-right.svg"
        } else {
            "icons/chevron-down.svg"
        };

        // US-006: the whole header toggles the group's collapse.
        let header = div()
            .id(SharedString::from(format!(
                "sessions-group-{}",
                agent_id_prefix(agent)
            )))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .pt(px(10.))
            .pb(px(4.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.sessions_group_collapsed[idx] = !this.sessions_group_collapsed[idx];
                cx.notify();
            }))
            // Brand glyph in its native accent so each tool reads at a glance.
            .child(
                svg()
                    .size(px(15.))
                    .flex_none()
                    .path(agent_icon_path(agent))
                    .text_color(agent_brand_color(agent, ui)),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ui.muted)
                    .child(agent_label(agent)),
            )
            // Collapse chevron, sitting just after the agent name.
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(chevron)
                    .text_color(ui.muted),
            );

        let mut group = div().flex().flex_col().child(header);

        // Collapsed → header only (US-006).
        if collapsed {
            return group.into_any_element();
        }

        if sessions.is_empty() {
            // US-004: distinguish a pending scan from a genuinely empty group.
            let msg: SharedString = if scanning {
                SharedString::from("Scanning\u{2026}")
            } else {
                empty_message(agent)
            };
            group = group.child(
                div()
                    .mx(px(10.))
                    .px(px(8.))
                    .py(px(8.))
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .child(msg),
            );
        } else {
            // US-005: cap at 5, reveal the rest behind "Show N more".
            let (visible, remaining) = visible_window(sessions.len(), show_all, CAP);
            for session in sessions.iter().take(visible) {
                group = group.child(self.sessions_row(session, ui, cx));
            }
            if sessions.len() > CAP {
                let label: SharedString = if show_all {
                    SharedString::from("Show less")
                } else {
                    format!("Show {remaining} more").into()
                };
                group = group.child(
                    div()
                        .id(SharedString::from(format!(
                            "{}-show-more",
                            agent_id_prefix(agent)
                        )))
                        .mx(px(6.))
                        .px(px(8.))
                        .py(px(5.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ui.muted)
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle).text_color(ui.text)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.sessions_group_show_all[idx] = !this.sessions_group_show_all[idx];
                            cx.notify();
                        }))
                        .child(label),
                );
            }
        }

        group.into_any_element()
    }

    fn sessions_for(&self, agent: SessionAgent) -> &[SessionMeta] {
        match agent {
            SessionAgent::Claude => &self.claude_sessions,
            SessionAgent::Codex => &self.codex_sessions,
            SessionAgent::OpenCode => &self.opencode_sessions,
        }
    }

    fn sessions_row(
        &self,
        session: &SessionMeta,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agent = session.agent;
        let session_id = session.session_id.clone();
        let row_id = SharedString::from(format!("{}-session-{session_id}", agent_id_prefix(agent)));
        let when = SharedString::from(format_relative_time(&session.timestamp));
        let title: SharedString = session
            .summary
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| short_session_id(&session_id))
            .into();

        // Drag payload: dropping this row on a pane spawns a fresh terminal at
        // the session's cwd running its resume command (drop-to-split / append
        // as a tab). The ghost reuses the tab-drag preview.
        let drag_payload = SessionDrag {
            agent,
            session_id: session_id.clone(),
            cwd: session.cwd.clone(),
            title: title.clone(),
            icon: SharedString::from(agent_icon_path(agent)),
        };

        div()
            .id(row_id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .h(ROW_HEIGHT)
            .mx(px(6.))
            .my(px(1.))
            .px(px(8.))
            .rounded(px(6.))
            .cursor_pointer()
            .on_drag(drag_payload, |drag, _offset, _window, cx| {
                cx.new(|_| TabDragPreview {
                    title: drag.title.clone(),
                    icon: drag.icon.clone(),
                })
            })
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            // US-007 (partial): resume into the bound pane; the docked sidebar
            // stays open (unlike the old popover).
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                let cmd = resume_command(agent, &session_id);
                this.send_command_to_sessions_pane(&cmd, cx);
                cx.stop_propagation();
            }))
            // Per-session agent glyph in its brand accent — a touch smaller
            // than the group-header mark so the header still reads as the
            // section anchor.
            .child(
                svg()
                    .size(px(13.))
                    .flex_none()
                    .path(agent_icon_path(agent))
                    .text_color(agent_brand_color(agent, ui)),
            )
            // Title takes the slack and ellipsizes; the relative time is pinned
            // to the trailing edge on the same line (US-009 row stays one line).
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .child(when),
            )
            .into_any_element()
    }

    /// Send a shell command to the pane that opened the sidebar. Silently
    /// no-ops when that pane was dropped (closed/replaced while the sidebar was
    /// open) or no longer has a terminal tab.
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

    /// Tear down all sidebar state in one place — used by the header close
    /// button and the tab-bar toggle (in `event_handlers`).
    pub(crate) fn close_sessions_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sessions_sidebar_open = false;
        self.claude_sessions.clear();
        self.codex_sessions.clear();
        self.opencode_sessions.clear();
        self.claude_sessions_cwd = None;
        self.claude_sessions_pane = None;
        // US-006: per-group state is in-memory only — reset so a reopen starts
        // expanded and capped, never stale.
        self.sessions_group_collapsed = [false; 3];
        self.sessions_group_show_all = [false; 3];
        self.sessions_scanning = [false; 3];
        cx.notify();
    }
}

/// Default per-group row cap before "Show more" (US-005).
const CAP: usize = 5;

/// Stable group index for the per-group `[bool; 3]` state arrays
/// (Claude=0, Codex=1, OpenCode=2). Shared with `event_handlers` so the
/// scan-in-flight flag and the render read the same slot.
pub(crate) fn agent_index(agent: SessionAgent) -> usize {
    match agent {
        SessionAgent::Claude => 0,
        SessionAgent::Codex => 1,
        SessionAgent::OpenCode => 2,
    }
}

/// Given a group of `len` rows, the cap, and whether the group is expanded,
/// return `(visible, remaining)`: how many rows to render and how many are
/// hidden behind "Show more". Pure — unit-tested (US-005).
fn visible_window(len: usize, show_all: bool, cap: usize) -> (usize, usize) {
    if show_all || len <= cap {
        (len, 0)
    } else {
        (cap, len - cap)
    }
}

fn empty_message(agent: SessionAgent) -> SharedString {
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

/// Display name for a group header.
fn agent_label(agent: SessionAgent) -> &'static str {
    match agent {
        SessionAgent::Claude => "Claude Code",
        SessionAgent::Codex => "Codex",
        SessionAgent::OpenCode => "OpenCode",
    }
}

/// Brand glyph for a group/session — the same monochrome (`currentColor`) SVGs
/// the tab-bar launcher buttons use, tinted at the call site.
fn agent_icon_path(agent: SessionAgent) -> &'static str {
    match agent {
        SessionAgent::Claude => "icons/claude-color.svg",
        SessionAgent::Codex => "icons/codex-color.svg",
        SessionAgent::OpenCode => "icons/opencode-color.svg",
    }
}

/// Accent for a group's brand glyph — matches the launcher buttons in
/// `pane.rs` (Claude orange, Codex blue). OpenCode's mark is monochrome, so it
/// rides the theme text color to stay legible on dark and light surfaces.
fn agent_brand_color(agent: SessionAgent, ui: crate::theme::UiColors) -> Hsla {
    match agent {
        SessionAgent::Claude => rgb(0xd97757).into(),
        SessionAgent::Codex => rgb(0x7a9dff).into(),
        SessionAgent::OpenCode => ui.text,
    }
}

/// True when Settings → AI Agent has `claude_code_bypass_permissions` toggled
/// on. Read at click time so a Settings flip takes effect without a restart.
fn claude_bypass_enabled() -> bool {
    paneflow_config::loader::load_config()
        .claude_code_bypass_permissions
        .unwrap_or(false)
}

/// Build the command sent to the bound terminal when a session row is clicked.
/// For Claude, honor `claude_code_bypass_permissions` so resumed sessions match
/// a fresh launch from the tab-bar button.
pub(crate) fn resume_command(agent: SessionAgent, session_id: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_index_is_stable() {
        assert_eq!(agent_index(SessionAgent::Claude), 0);
        assert_eq!(agent_index(SessionAgent::Codex), 1);
        assert_eq!(agent_index(SessionAgent::OpenCode), 2);
    }

    #[test]
    fn visible_window_empty() {
        assert_eq!(visible_window(0, false, CAP), (0, 0));
    }

    #[test]
    fn visible_window_at_cap_has_no_remainder() {
        assert_eq!(visible_window(5, false, CAP), (5, 0));
    }

    #[test]
    fn visible_window_over_cap_caps_and_reports_remainder() {
        assert_eq!(visible_window(6, false, CAP), (5, 1));
        assert_eq!(visible_window(100, false, CAP), (5, 95));
    }

    #[test]
    fn visible_window_show_all_reveals_everything() {
        assert_eq!(visible_window(6, true, CAP), (6, 0));
        assert_eq!(visible_window(100, true, CAP), (100, 0));
    }
}
