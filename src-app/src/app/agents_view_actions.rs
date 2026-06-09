//! Action handler + lifecycle helpers + render branch entry points for
//! the Agents view.
//!
//! [`paneflow_config::schema::AppMode`] is the source of truth for which
//! top-level screen renders; `self.mode` decides whether the Agents view
//! is currently visible. The main area is terminal-only: a selected
//! thread renders its PTY, and the no-thread state renders the agent
//! picker for the active project (the home/empty state).
//!
//! Toggled by the [`crate::OpenAgentsView`] action (Ctrl+Shift+A on
//! Linux/Windows, Cmd+Shift+A on macOS). Both render branches
//! ([`PaneFlowApp::render_agents_main`] and
//! [`PaneFlowApp::render_agents_sidebar`]) are no-ops when
//! `self.mode == AppMode::Cli` -- main `render` only calls them on the
//! Agents arm.

use crate::{OpenAgentsView, PaneFlowApp};
use gpui::{AppContext, Context, IntoElement, ParentElement, Styled, Window, div, px};
use paneflow_config::schema::AppMode;

/// Sidebar width when in [`AppMode::Agents`]. Slightly wider than the
/// CLI sidebar (220 px) because thread rows carry more metadata
/// (agent icon, status dot, relative timestamp) than workspace rows.
/// US-009 surfaces this constant to the title bar so the resize edge
/// snaps to the right slot on mode toggle.
pub(crate) const AGENTS_SIDEBAR_WIDTH: f32 = 280.0;

impl PaneFlowApp {
    /// Toggle between [`AppMode::Cli`] and [`AppMode::Agents`].
    ///
    /// Focus contract (US-008 AC): when toggling back to CLI, the
    /// previously active workspace's first pane re-receives focus.
    /// The reverse direction (CLI -> Agents) does not steal focus
    /// proactively; the Agents view rendering takes over the main
    /// surface and any subsequent keystroke targets the new tree.
    pub(crate) fn handle_open_agents_view(
        &mut self,
        _: &OpenAgentsView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.mode {
            AppMode::Agents => {
                self.exit_agents_mode(window, cx);
            }
            // From CLI or the Diff mode, pressing the Agents binding
            // switches into Agents (US-003 of prd-git-diff-mode-2026-Q3.md;
            // `enter_agents_mode` clears any other non-CLI surface).
            AppMode::Cli | AppMode::Diff => {
                self.enter_agents_mode(cx);
            }
        }
    }

    /// Switch the main pane to the Skills browser (~/.claude/skills,
    /// ~/.codex/skills, ~/.agents/skills). Wired to the sidebar's
    /// "Skills" affordance.
    pub(crate) fn show_agents_skills(&mut self, cx: &mut Context<Self>) {
        // US-003: clearing the unified target drops to the picker/home
        // state; the Skills page then takes precedence in the render branch.
        self.agents_target = None;
        self.agents_view.agents_skills_visible = true;
        cx.notify();
    }

    /// Mark a skill name as "just copied" so its card label flips to
    /// "Copied" for 2 s. A scheduled task clears the slot iff it
    /// still holds the same name — back-to-back copies of different
    /// skills don't cancel each other's feedback.
    pub(crate) fn mark_skill_copied(&mut self, name: String, cx: &mut Context<Self>) {
        self.agents_view.agents_skills_copied = Some(name.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            smol::Timer::after(std::time::Duration::from_millis(1500)).await;
            let _ = cx.update(|cx| {
                this.update(cx, |app, cx| {
                    if app.agents_view.agents_skills_copied.as_deref() == Some(name.as_str()) {
                        app.agents_view.agents_skills_copied = None;
                        cx.notify();
                    }
                })
            });
        })
        .detach();
    }

    pub(crate) fn enter_agents_mode(&mut self, cx: &mut Context<Self>) {
        self.mode = AppMode::Agents;
        // US-016 warm-resume: entering Agents from Diff suspends the diff host
        // (releases its watchers + ends its debounce loop) while the cache keeps
        // its computed rows for an instant warm return; no-op from CLI (already
        // parked). Keeps the non-CLI surfaces mutually exclusive (prd-git-diff
        // US-003/US-005) without throwing away the diff.
        self.park_displayed_diff(cx);
        // US-116: panel is now front-and-center; the gate combines
        // this with window-active to decide notification firing.
        crate::agents::notifications::set_agents_panel_visible(true);
        cx.notify();
    }

    fn exit_agents_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = AppMode::Cli;
        // US-116: flip the gate so the next runtime event surfaces a toast.
        crate::agents::notifications::set_agents_panel_visible(false);
        // Focus contract: restore focus to the active workspace's
        // first pane so the keyboard immediately targets the
        // terminal the user left, not a stray top-level handler.
        // Terminal PTYs are detached, so the previously running
        // process is still alive (verified by spawning and switching
        // mid-stream).
        if let Some(ws) = self.workspaces.get_mut(self.active_idx) {
            ws.focus_first(window, cx);
        }
        cx.notify();
    }

    /// Main-content render branch for [`AppMode::Agents`].
    ///
    /// Priority order:
    /// 1. The Skills page, if open.
    /// 2. A selected thread -> its terminal surface (the PTY launching
    ///    the thread's CLI agent).
    /// 3. A project open but no thread selected -> the agent picker for
    ///    that project (the home/empty state).
    /// 4. No project at all -> the "no project" empty state.
    pub(crate) fn render_agents_main(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let body: gpui::AnyElement = self.render_agents_main_body(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h(px(0.)).child(body))
            .into_any_element()
    }

    /// The inner Agents-view body (skills page / terminal surface /
    /// agent picker / empty state). Pulled out so the toolbar wrapping
    /// logic stays in one place.
    fn render_agents_main_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Sidebar "Skills" affordance takes precedence over the thread /
        // picker surfaces.
        if self.agents_view.agents_skills_visible {
            return crate::agents_view::render_skills_page(
                self.agents_view.agents_skills_tab,
                self.agents_view.agents_skills_copied.clone(),
                cx,
            );
        }
        // A selected thread renders its terminal surface. Every thread
        // is a terminal now (the in-app ACP chat was removed); legacy
        // `ThreadKind::Agent` rows relaunch their original CLI agent in a
        // PTY (see `ensure_terminal_view_mounted`).
        if let Some(target) = self.current_thread_view_target()
            && let Some(view) = self.ensure_terminal_view_mounted(target, cx)
        {
            return render_terminal_thread_surface(view);
        }
        // No thread selected: the agent picker for the active project is
        // the home/empty state (start a thread by picking an agent).
        if !self.projects.is_empty() && self.active_project_idx < self.projects.len() {
            let project_idx = self.active_project_idx;
            return self.render_agents_launcher(project_idx, cx);
        }
        // No project at all: a minimal empty state mirroring the
        // sidebar's "No projects yet" copy.
        render_agents_no_project()
    }

    /// Agent picker: a centered card list of the CLI coding agents
    /// enabled in Settings → AI Agent. Clicking one creates a Terminal
    /// Thread in `project_idx` that auto-launches that agent in a PTY
    /// (honoring the bypass-permission flag). This is the Agents view's
    /// home/empty state whenever a project is open but no thread is
    /// selected.
    fn render_agents_launcher(
        &mut self,
        project_idx: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        use crate::agent_launcher::TerminalAgent;
        use gpui::{
            ClickEvent, CursorStyle, FontWeight, InteractiveElement, MouseButton, SharedString,
            StatefulInteractiveElement, rgb,
        };

        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();
        let config = paneflow_config::loader::load_config();
        let agents = TerminalAgent::visible(&config);

        let rows: Vec<gpui::AnyElement> = agents
            .into_iter()
            .map(|agent| {
                let name = agent.display_name();
                let icon_color: gpui::Hsla =
                    agent.accent().map(|c| rgb(c).into()).unwrap_or(ui.text);
                div()
                    .id(SharedString::from(format!(
                        "agents-launcher-{}",
                        agent.tag()
                    )))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .my(px(4.))
                    .rounded(px(10.))
                    .bg(ui.surface)
                    .border_1()
                    .border_color(ui.border)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle).border_color(ui.accent)
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.create_agent_terminal_thread_in(project_idx, agent, cx);
                    }))
                    // Multi-color logos render via `img()` (resvg rasterizes
                    // the SVG, keeping every native fill); monochrome logos
                    // stay a `text_color`-tinted `svg()` mask.
                    .child(if agent.icon_multicolor() {
                        gpui::img(agent.icon_path())
                            .size(px(18.))
                            .flex_none()
                            .into_any_element()
                    } else {
                        gpui::svg()
                            .size(px(18.))
                            .flex_none()
                            .path(agent.icon_path())
                            .text_color(icon_color)
                            .into_any_element()
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ui.text)
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(ui.muted)
                            .child("Open"),
                    )
                    .into_any_element()
            })
            .collect();

        let body: gpui::AnyElement = if rows.is_empty() {
            div()
                .text_size(px(13.))
                .text_color(ui.muted)
                .child(
                    "Every agent is hidden in Settings → AI Agent. Enable one to start a thread.",
                )
                .into_any_element()
        } else {
            div().flex().flex_col().children(rows).into_any_element()
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.title_bar_background)
            .text_color(ui.text)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .px(px(20.))
                    .child(
                        div()
                            .w_full()
                            .max_w(px(640.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .mb(px(4.))
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ui.text)
                                    .child("Start a new thread"),
                            )
                            .child(
                                div()
                                    .mb(px(12.))
                                    .text_size(px(12.))
                                    .text_color(ui.muted)
                                    .child("Pick an agent to launch in a terminal."),
                            )
                            .child(body),
                    ),
            )
            .into_any_element()
    }

    /// The currently selected center target, validated against the live
    /// `projects` / `chats` vectors (US-003). Returns `None` when nothing is
    /// selected (picker/home) OR the stored target points past the end of
    /// its source (e.g. its row was just removed) — both collapse to the
    /// picker rather than rendering a stale row.
    pub(crate) fn current_thread_view_target(&self) -> Option<crate::project::AgentsTarget> {
        use crate::project::AgentsTarget;
        match self.agents_target? {
            AgentsTarget::Thread {
                project_idx,
                thread_idx,
            } => {
                let project = self.projects.get(project_idx)?;
                (thread_idx < project.threads.len()).then_some(AgentsTarget::Thread {
                    project_idx,
                    thread_idx,
                })
            }
            AgentsTarget::Chat { chat_idx } => {
                (chat_idx < self.chats.len()).then_some(AgentsTarget::Chat { chat_idx })
            }
        }
    }

    /// Resolve a center target to its backing [`Thread`], whether it lives
    /// in a project or in the free `chats` list (US-003). `None` when the
    /// target is out of range.
    pub(crate) fn thread_for_target(
        &self,
        target: crate::project::AgentsTarget,
    ) -> Option<&crate::project::Thread> {
        use crate::project::AgentsTarget;
        match target {
            AgentsTarget::Thread {
                project_idx,
                thread_idx,
            } => self.projects.get(project_idx)?.threads.get(thread_idx),
            AgentsTarget::Chat { chat_idx } => self.chats.get(chat_idx),
        }
    }

    /// Mount (or reuse from cache) the [`TerminalView`] entity that
    /// backs a Terminal Thread at `target`. Returns the entity ready
    /// to be wrapped by [`render_terminal_thread_surface`].
    ///
    /// Cache hit re-binds the existing entity so the running shell
    /// process survives sidebar navigation; cache miss spawns a fresh
    /// PTY in the thread's cwd via [`TerminalView::with_cwd`] and (when
    /// the thread is bound to a CLI agent) auto-launches it.
    ///
    /// `workspace_id` for the new view defaults to the thread's own
    /// `id` so PTY tracking (signal routing, kill-on-quit) keys off a
    /// stable per-thread identifier rather than the CLI-mode workspace
    /// slot (which has no meaning in the Agents view).
    fn ensure_terminal_view_mounted(
        &mut self,
        target: crate::project::AgentsTarget,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<crate::terminal::view::TerminalView>> {
        // US-003: resolve the target (project thread OR free chat) to its
        // backing Thread. The cache key below is the stable `Thread::id`,
        // shared across both sources, so a project thread and a chat can
        // never collide and warm-resume survives navigation between them.
        let thread = self.thread_for_target(target)?;
        let thread_id = thread.id;
        if let Some(cached) = self.agents_view.agents_terminal_view_cache.get(&thread_id) {
            return Some(cached.clone());
        }
        let cwd = std::path::PathBuf::from(&thread.cwd);
        // Explicit per-thread agent wins; legacy `Agent`-kind chat rows
        // fall back to their stored ACP agent so reopening them launches
        // the same CLI in a terminal. Plain Terminal Threads stay a bare
        // shell (`None`).
        let terminal_agent = thread.terminal_agent.or_else(|| match thread.kind {
            crate::project::ThreadKind::Agent => Some(
                crate::agent_launcher::TerminalAgent::from_agent_kind(thread.agent),
            ),
            crate::project::ThreadKind::Terminal => None,
        });
        let view = cx.new(|cx| {
            crate::terminal::view::TerminalView::with_cwd(thread_id, Some(cwd), None, cx)
        });
        // Cache miss = first mount of this thread's PTY (fresh creation
        // or first reopen after a restart). When the thread is bound to
        // a CLI agent, auto-run its launch command so opening the thread
        // drops the user straight into the agent. The command honors
        // `claude_code_bypass_permissions` via `launch_command`. Writing
        // immediately is safe even though `with_cwd` opens the PTY on a
        // background thread (US-012): `send_command` → `write_to_pty`
        // buffers into the display-only terminal's `pending_input` queue and
        // `TerminalState::promote` flushes it the moment the PTY goes live,
        // so the command is never dropped on the pre-promotion race.
        // Cache hits (in-session re-selection) skip this, so a running
        // agent is never relaunched on navigation.
        if let Some(agent) = terminal_agent {
            let cmd = agent.launch_command(&paneflow_config::loader::load_config());
            view.read(cx).send_command(&cmd);
        }
        // Mirror Zed's `AgentTerminal::refresh_terminal_metadata`
        // (agent_panel.rs around `TerminalEvent::TitleChanged`): every
        // OSC 0/2 title update from the running process is reflected
        // into the sidebar row label. That's what lets a `claude`
        // session inside a Terminal Thread surface its auto-summary
        // ("Refactor auth middleware") in the sidebar instead of the
        // generic "Terminal" placeholder. The subscription is detached
        // -- the entity owns its lifecycle and the listener drops with
        // it when the cache evicts the entry.
        cx.subscribe(
            &view,
            move |this, src, event: &crate::terminal::view::TerminalEvent, cx| {
                if matches!(event, crate::terminal::view::TerminalEvent::TitleChanged) {
                    let new_title = src.read(cx).terminal.title.clone();
                    this.handle_terminal_thread_title_changed(thread_id, new_title, cx);
                }
            },
        )
        .detach();
        self.agents_view
            .agents_terminal_view_cache
            .insert(thread_id, view.clone());
        Some(view)
    }

    /// React to an OSC-driven title update from the PTY backing a
    /// Terminal Thread. Updates the matching sidebar row's title and
    /// persists the session so the new label survives a restart.
    ///
    /// Skips two cases on purpose:
    /// 1. Empty / whitespace-only titles -- some shells emit a stray
    ///    blank `ESC]0;\x07` on startup before the real prompt loads.
    /// 2. The literal `"Terminal"` fallback alacritty stamps after a
    ///    `ResetTitle` OSC, so a child shell exiting (e.g. `claude`
    ///    completing a session) does not wipe the meaningful
    ///    process-reported title with a generic placeholder.
    pub(crate) fn handle_terminal_thread_title_changed(
        &mut self,
        thread_id: u64,
        new_title: String,
        cx: &mut Context<Self>,
    ) {
        // Strips whitespace + leading spinner/bullet glyphs (Codex
        // braille, Claude Code pinwheel, generic `●`/`•`). Returns
        // `None` if nothing meaningful is left.
        let Some(normalized) = crate::project::clean_sidebar_title(&new_title) else {
            return;
        };
        if normalized == "Terminal" {
            // Don't let alacritty's `ResetTitle` fallback wipe a
            // meaningful process-reported title once a child shell
            // exits and the title resets to the default.
            return;
        }
        for project in self.projects.iter_mut() {
            if let Some(thread) = project.threads.iter_mut().find(|t| t.id == thread_id) {
                if thread.title == normalized {
                    return;
                }
                thread.title = normalized;
                self.save_session(cx);
                cx.notify();
                return;
            }
        }
        // US-003: a chat is a Thread too — its PTY emits OSC titles just
        // like a project thread, so the same label-sync applies to the
        // free `chats` list.
        if let Some(thread) = self.chats.iter_mut().find(|t| t.id == thread_id) {
            if thread.title == normalized {
                return;
            }
            thread.title = normalized;
            self.save_session(cx);
            cx.notify();
        }
    }

    // Sidebar render branch for [`AppMode::Agents`] now lives in
    // [`crate::app::agents_sidebar`] -- US-010 replaced the
    // placeholder shipped here in US-008.
}

/// Wrap a [`TerminalView`] entity into the Agents main area surface.
/// Pulled into a free function so the dispatch branch in
/// [`PaneFlowApp::render_agents_main_body`] stays one line and so the
/// PTY background/padding policy (match the CLI pane shell) lives in a
/// single named spot.
pub(crate) fn render_terminal_thread_surface(
    view: gpui::Entity<crate::terminal::view::TerminalView>,
) -> gpui::AnyElement {
    let ui = crate::theme::ui_colors();
    div()
        .size_full()
        .bg(ui.base)
        .child(view.into_any_element())
        .into_any_element()
}

/// Main-area empty state when the Agents view has no project to pick an
/// agent for. Mirrors the sidebar's "No projects yet" copy; the
/// "New threads" sidebar row opens the folder picker.
fn render_agents_no_project() -> gpui::AnyElement {
    let ui = crate::theme::ui_colors();
    let theme = crate::theme::active_theme();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .bg(theme.title_bar_background)
        .child(
            div()
                .text_size(px(15.))
                .text_color(ui.text)
                .child("No project open"),
        )
        .child(
            div().text_size(px(12.)).text_color(ui.muted).child(
                "Click \"New threads\" in the sidebar to add a project, then pick an agent.",
            ),
        )
        .into_any_element()
}
