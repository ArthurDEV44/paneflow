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
    ParentElement, Pixels, SharedString, Styled, Window, div, prelude::*, px, rgb, svg,
};

use crate::PaneFlowApp;
use crate::agent_sessions::{SessionAgent, SessionMeta, format_relative_time};
use crate::pane_drag::{SessionDrag, TabDragPreview};

/// Fixed sidebar width — between the CLI (220) and Agents (280) left sidebars,
/// matching VS Code's secondary-bar default. Resizable width is deferred.
const SIDEBAR_WIDTH: Pixels = px(300.);
const ROW_HEIGHT: Pixels = px(30.);

impl PaneFlowApp {
    /// Open (or re-target) the sessions sidebar for `pane`: resolve the
    /// pane's terminal cwd, bind the resume target, reset per-group state,
    /// and kick the per-agent scans. Shared by the tab-bar toggle
    /// (`PaneEvent::ToggleAgentSessions`) and the workspace switch
    /// (`select_workspace` re-targets an open sidebar to the new active
    /// workspace through this same path).
    pub(crate) fn open_sessions_sidebar_for_pane(
        &mut self,
        pane: &gpui::Entity<crate::pane::Pane>,
        cx: &mut Context<Self>,
    ) {
        // Resolve the active terminal's cwd: prefer the OSC 7 push
        // (`current_cwd`), fall back to the on-demand `cwd_now()` syscall for
        // shells that don't emit OSC 7.
        let cwd_str = pane.read(cx).active_terminal_opt().and_then(|tv| {
            let view = tv.read(cx);
            view.terminal.current_cwd.clone().or_else(|| {
                view.terminal
                    .cwd_now()
                    .map(|p| p.to_string_lossy().into_owned())
            })
        });

        // Mutual exclusion: only one right column. Opening sessions closes
        // the Files sidebar (and vice-versa, in `toggle_files_sidebar`).
        if self.files_sidebar_open {
            self.close_files_sidebar(cx);
        }

        // Close the floating dropdowns so they don't paint over the newly
        // opened sidebar (the sidebar itself is docked, not an overlay, so it
        // does not need mutual exclusion with itself).
        self.workspace_menu_open = None;
        self.profile_menu_open = None;

        self.agent_sessions.sessions_sidebar_open = true;
        self.agent_sessions.claude_sessions_cwd = cwd_str.clone();
        self.agent_sessions.claude_sessions_pane = Some(pane.downgrade());
        self.agent_sessions.claude_sessions.clear();
        self.agent_sessions.codex_sessions.clear();
        self.agent_sessions.opencode_sessions.clear();
        // Fresh per-group state for this open: all expanded, capped at 5,
        // not-yet-scanning (each spawned scan flips its own flag below).
        self.agent_sessions.sessions_group_collapsed = [false; 3];
        self.agent_sessions.sessions_group_show_all = [false; 3];
        self.agent_sessions.sessions_scanning = [false; 3];
        let enabled_agents = crate::agent_sessions::enabled_session_agents();
        // Fresh handle so a previous scroll offset doesn't bleed into the new
        // sidebar.
        self.agent_sessions.claude_sessions_scroll = gpui::ScrollHandle::new();

        if let Some(cwd) = cwd_str {
            // Parallel scans — Claude Code under `~/.claude/projects/<slug>/`,
            // Codex CLI under `~/.codex/sessions/YYYY/MM/DD/`, and OpenCode
            // via a `opencode session list --format json` shell-out (the
            // SQLite schema is unstable; the CLI is the published contract —
            // see US-001 spike notes). Each task writes to its own Vec on the
            // main thread. The sidebar may be closed or re-targeted against a
            // different cwd before any scan finishes, so stale results are
            // dropped by checking `claude_sessions_cwd` matches before
            // applying.
            //
            // Scans for agents the user has hidden in Settings → AI Agent are
            // skipped: with no UI to surface them the disk read would just be
            // wasted I/O.
            let scan_claude = enabled_agents.contains(&SessionAgent::Claude);
            let scan_codex = enabled_agents.contains(&SessionAgent::Codex);
            let scan_opencode = enabled_agents.contains(&SessionAgent::OpenCode);

            if scan_claude {
                let idx = agent_index(SessionAgent::Claude);
                self.agent_sessions.sessions_scanning[idx] = true;
                let claude_cwd_scan = cwd.clone();
                let claude_cwd_match = cwd.clone();
                cx.spawn(async move |this, cx| {
                    let sessions = smol::unblock(move || {
                        crate::claude_sessions::read_sessions_for_cwd(&claude_cwd_scan)
                    })
                    .await;
                    let _ = this.update(cx, |app, cx| {
                        if app.agent_sessions.sessions_sidebar_open
                            && app.agent_sessions.claude_sessions_cwd.as_deref()
                                == Some(claude_cwd_match.as_str())
                        {
                            app.agent_sessions.claude_sessions = sessions;
                            app.agent_sessions.sessions_scanning[idx] = false;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }

            if scan_codex {
                let idx = agent_index(SessionAgent::Codex);
                self.agent_sessions.sessions_scanning[idx] = true;
                let codex_cwd_scan = cwd.clone();
                let codex_cwd_match = cwd.clone();
                cx.spawn(async move |this, cx| {
                    let sessions = smol::unblock(move || {
                        crate::codex_sessions::read_sessions_for_cwd(&codex_cwd_scan)
                    })
                    .await;
                    let _ = this.update(cx, |app, cx| {
                        if app.agent_sessions.sessions_sidebar_open
                            && app.agent_sessions.claude_sessions_cwd.as_deref()
                                == Some(codex_cwd_match.as_str())
                        {
                            app.agent_sessions.codex_sessions = sessions;
                            app.agent_sessions.sessions_scanning[idx] = false;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }

            if scan_opencode {
                let idx = agent_index(SessionAgent::OpenCode);
                self.agent_sessions.sessions_scanning[idx] = true;
                let opencode_cwd_scan = cwd.clone();
                let opencode_cwd_match = cwd;
                cx.spawn(async move |this, cx| {
                    let sessions = smol::unblock(move || {
                        crate::opencode_sessions::read_sessions_for_cwd(&opencode_cwd_scan)
                    })
                    .await;
                    let _ = this.update(cx, |app, cx| {
                        if app.agent_sessions.sessions_sidebar_open
                            && app.agent_sessions.claude_sessions_cwd.as_deref()
                                == Some(opencode_cwd_match.as_str())
                        {
                            app.agent_sessions.opencode_sessions = sessions;
                            app.agent_sessions.sessions_scanning[idx] = false;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// Render the docked sessions sidebar (right edge of the root `flex_row`).
    /// Only called when `sessions_sidebar_open` is true.
    pub(crate) fn render_sessions_sidebar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();
        div()
            .id("sessions-sidebar")
            .flex()
            .flex_col()
            .w(SIDEBAR_WIDTH)
            .flex_shrink_0()
            .h_full()
            // Match the app's other navigation rails: theme-aware native
            // material on Windows/macOS and a light/dark tint on Linux.
            .bg(crate::app::constants::cockpit_chrome_background(
                theme.title_bar_background,
                window.is_window_active(),
            ))
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
            // Quiet header — no divider (Codex: separation by spacing, not
            // borders). 36px matches the unified chrome row height.
            .h(px(36.))
            .flex_none()
            .px(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
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
                        s.bg(crate::app::constants::sidebar_tab_hover_background())
                            .text_color(ui.text)
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
        if self.agent_sessions.claude_sessions_cwd.is_none() {
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
            .track_scroll(&self.agent_sessions.claude_sessions_scroll);

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
        let collapsed = self.agent_sessions.sessions_group_collapsed[idx];
        let show_all = self.agent_sessions.sessions_group_show_all[idx];
        let scanning = self.agent_sessions.sessions_scanning[idx];
        let sessions = self.sessions_for(agent);
        // Distinct chevron per state (US-006): right = collapsed, down =
        // expanded — a static swap, not a tween, so it reads under reduced
        // motion.
        let chevron = if collapsed {
            "icons/chevron-right.svg"
        } else {
            "icons/chevron-down.svg"
        };

        // US-006: the whole header toggles the group's collapse. Styled as a
        // section eyebrow (the Agents-sidebar language): small semibold muted
        // label, brand glyph kept in its native accent — the only color in
        // the rail, carrying real signal (which tool).
        let header = div()
            .id(SharedString::from(format!(
                "sessions-group-{}",
                agent_id_prefix(agent)
            )))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(14.))
            .pt(px(12.))
            .pb(px(4.))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.agent_sessions.sessions_group_collapsed[idx] =
                    !this.agent_sessions.sessions_group_collapsed[idx];
                cx.notify();
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(agent_icon_path(agent))
                    .text_color(agent_brand_color(agent, ui)),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui.muted)
                    .child(agent_label(agent)),
            )
            // Collapse chevron, sitting just after the agent name.
            .child(
                svg()
                    .size(px(12.))
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
                    .mx(px(14.))
                    .px(px(8.))
                    .py(px(6.))
                    .text_size(px(11.))
                    .text_color(ui.muted.opacity(0.8))
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
                            s.bg(crate::app::constants::sidebar_tab_hover_background())
                                .text_color(ui.text)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.agent_sessions.sessions_group_show_all[idx] =
                                !this.agent_sessions.sessions_group_show_all[idx];
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
            SessionAgent::Claude => &self.agent_sessions.claude_sessions,
            SessionAgent::Codex => &self.agent_sessions.codex_sessions,
            SessionAgent::OpenCode => &self.agent_sessions.opencode_sessions,
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
            .mx(px(8.))
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
            .hover(|s| s.bg(crate::app::constants::sidebar_tab_hover_background()))
            // US-007 (partial): resume into the bound pane; the docked sidebar
            // stays open (unlike the old popover).
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                if let Some(cmd) = resume_command(agent, &session_id) {
                    this.send_command_to_sessions_pane(&cmd, cx);
                }
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
        let Some(pane_handle) = self.agent_sessions.claude_sessions_pane.as_ref() else {
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
        self.agent_sessions.sessions_sidebar_open = false;
        self.agent_sessions.claude_sessions.clear();
        self.agent_sessions.codex_sessions.clear();
        self.agent_sessions.opencode_sessions.clear();
        self.agent_sessions.claude_sessions_cwd = None;
        self.agent_sessions.claude_sessions_pane = None;
        // US-006: per-group state is in-memory only — reset so a reopen starts
        // expanded and capped, never stale.
        self.agent_sessions.sessions_group_collapsed = [false; 3];
        self.agent_sessions.sessions_group_show_all = [false; 3];
        self.agent_sessions.sessions_scanning = [false; 3];
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
///
/// Returns `None` when `session_id` fails the strict allow-list — a last gate
/// before interpolation so a tampered record that somehow bypassed the scanner
/// filter (`*_sessions.rs`) can never inject a second shell command. Callers
/// skip the send on `None`.
pub(crate) fn resume_command(agent: SessionAgent, session_id: &str) -> Option<String> {
    if !crate::agent_sessions::is_valid_session_id(session_id) {
        log::warn!("resume_command: refused invalid session id, not sending to PTY");
        return None;
    }
    Some(match agent {
        SessionAgent::Claude => {
            if claude_bypass_enabled() {
                format!("claude --resume {session_id} --permission-mode bypassPermissions")
            } else {
                format!("claude --resume {session_id}")
            }
        }
        SessionAgent::Codex => format!("codex resume {session_id}"),
        SessionAgent::OpenCode => format!("opencode --session {session_id}"),
    })
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
    fn resume_command_neutralizes_flag_shaped_session_id() {
        // US-019: `resume_command` is the single builder that interpolates a
        // persisted/restored `session_id` into a PTY command line. It must
        // re-gate via `is_valid_session_id` so a flag-shaped value (one that
        // could inject e.g. `--dangerously-skip-permissions`) is refused at
        // the builder boundary — the call sites skip the send on `None`.
        // This proves the integration, not just the predicate
        // (`agent_sessions::valid_session_id_rejects_leading_dash_*`).
        for agent in [
            SessionAgent::Claude,
            SessionAgent::Codex,
            SessionAgent::OpenCode,
        ] {
            assert_eq!(
                resume_command(agent, "--dangerously-skip-permissions"),
                None,
                "{agent:?}: a `--`-prefixed id must not build a command"
            );
            assert_eq!(resume_command(agent, "-x"), None);
            assert_eq!(resume_command(agent, "ses_x; rm -rf ~"), None);
            assert_eq!(resume_command(agent, "$(reboot)"), None);
        }
        // A legitimate UUID session id still builds a command for every agent.
        let valid = "019dc9ea-38d7-7372-9cc4-253ce944d41b";
        assert!(resume_command(SessionAgent::Claude, valid).is_some());
        assert!(resume_command(SessionAgent::Codex, valid).is_some());
        assert!(resume_command(SessionAgent::OpenCode, valid).is_some());
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
