//! US-005 + US-008 (prd-agents-view.md): action handler + lifecycle
//! helpers + render branch entry points for the Agents view.
//!
//! - US-005 built the lightweight shell entity (auth-required card,
//!   missing-agents empty state, embedded login terminal).
//! - US-008 promoted [`paneflow_config::schema::AppMode`] to the source
//!   of truth for which top-level screen renders. The `agents_view`
//!   field on [`PaneFlowApp`] is now just the entity holder so the
//!   discovery results / subscriptions stay alive across renders --
//!   `self.mode` decides whether the view is currently visible.
//!
//! Toggled by the [`crate::OpenAgentsView`] action (Ctrl+Shift+A on
//! Linux/Windows, Cmd+Shift+A on macOS). Both render branches
//! ([`PaneFlowApp::render_agents_main`] and
//! [`PaneFlowApp::render_agents_sidebar`]) are no-ops when
//! `self.mode == AppMode::Cli` -- main `render` only calls them on the
//! Agents arm.

use crate::agents_view::{AgentsView, CloseRequested};
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
    /// Lazily mounts the [`AgentsView`] entity on the first switch
    /// into Agents mode and runs PATH discovery immediately so the
    /// next paint already has the agents-listed / empty / auth state
    /// decided.
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
            AppMode::Cli => {
                self.enter_agents_mode(cx);
            }
        }
    }

    /// Programmatic alias for "the user clicked the close button on
    /// the Agents view header". Kept as a separate entry point so
    /// callers that have no [`Window`] (e.g. event subscribers) can
    /// still toggle back. Focus restore is best-effort here -- the
    /// next mouse / key event in the CLI tree will resolve focus the
    /// regular way.
    pub(crate) fn close_agents_view(&mut self, cx: &mut Context<Self>) {
        if self.mode == AppMode::Agents {
            self.mode = AppMode::Cli;
            self.agents_view = None;
            // US-116: panel is no longer visible -- let the
            // notifications gate know so the next TurnEnded fires the
            // OS toast even though the window is still focused.
            crate::agents::notifications::set_agents_panel_visible(false);
            cx.notify();
        }
    }

    /// Switch the main pane to the AgentsView welcome screen
    /// (PATH-discovered agents + signed-in chips / Sign-in buttons).
    /// Used by the sidebar "Connect" affordance: drops the active
    /// thread selection so `render_agents_main_body` falls through to
    /// the `AgentsView` entity instead of a ThreadView, then re-runs
    /// PATH discovery so freshly installed agents show up without a
    /// restart.
    pub(crate) fn show_agents_welcome(&mut self, cx: &mut Context<Self>) {
        self.active_thread_idx = None;
        self.agents_skills_visible = false;
        let view = match self.agents_view.clone() {
            Some(v) => v,
            None => {
                let fresh = cx.new(|_cx| AgentsView::new());
                cx.subscribe(&fresh, |this, _emitter, _event: &CloseRequested, cx| {
                    this.close_agents_view(cx);
                })
                .detach();
                self.agents_view = Some(fresh.clone());
                fresh
            }
        };
        view.update(cx, |v, cx| v.refresh(cx));
        cx.notify();
    }

    /// Switch the main pane to the Skills browser (~/.claude/skills,
    /// ~/.codex/skills, ~/.agents/skills). Wired to the sidebar's
    /// "Skills" affordance.
    pub(crate) fn show_agents_skills(&mut self, cx: &mut Context<Self>) {
        self.active_thread_idx = None;
        self.agents_skills_visible = true;
        cx.notify();
    }

    /// Mark a skill name as "just copied" so its card label flips to
    /// "Copied" for 2 s. A scheduled task clears the slot iff it
    /// still holds the same name — back-to-back copies of different
    /// skills don't cancel each other's feedback.
    pub(crate) fn mark_skill_copied(&mut self, name: String, cx: &mut Context<Self>) {
        self.agents_skills_copied = Some(name.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            smol::Timer::after(std::time::Duration::from_millis(1500)).await;
            let _ = cx.update(|cx| {
                this.update(cx, |app, cx| {
                    if app.agents_skills_copied.as_deref() == Some(name.as_str()) {
                        app.agents_skills_copied = None;
                        cx.notify();
                    }
                })
            });
        })
        .detach();
    }

    pub(crate) fn enter_agents_mode(&mut self, cx: &mut Context<Self>) {
        self.mode = AppMode::Agents;
        // US-116: panel is now front-and-center; the gate combines
        // this with window-active to decide notification firing.
        crate::agents::notifications::set_agents_panel_visible(true);
        // Lazy mount: a fresh AgentsView on every entry so PATH
        // discovery re-runs (the user may have installed an agent
        // between toggles -- AC of US-004's focus refresh).
        let view = cx.new(|_cx| AgentsView::new());
        view.update(cx, |v, cx| v.refresh(cx));
        cx.subscribe(&view, |this, _emitter, _event: &CloseRequested, cx| {
            this.close_agents_view(cx);
        })
        .detach();
        self.agents_view = Some(view);
        cx.notify();
    }

    fn exit_agents_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = AppMode::Cli;
        self.agents_view = None;
        // US-116: as in `close_agents_view`, flip the gate so the
        // next runtime event surfaces a toast.
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
    /// 1. A selected thread -> render its [`agents::thread_view::ThreadView`]
    ///    (US-013). The entity is lazily mounted and recreated when
    ///    the user picks a different thread.
    /// 2. No selection but the auth/missing-agents shell is mounted ->
    ///    render that (US-005).
    /// 3. Defensive fallback ("Loading agents view...") which should
    ///    not be reachable in normal flow.
    pub(crate) fn render_agents_main(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let body: gpui::AnyElement = self.render_agents_main_body(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h(px(0.)).child(body))
            .into_any_element()
    }

    /// US-104: the inner Agents-view body (ThreadView or AgentsView
    /// fallback). Pulled out so the toolbar wrapping logic stays in
    /// one place. Returns the v1 selection priority unchanged.
    fn render_agents_main_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        // Sidebar "Skills" affordance takes precedence over both the
        // thread view and the welcome view.
        if self.agents_skills_visible {
            return crate::agents_view::render_skills_page(
                self.agents_skills_tab,
                self.agents_skills_copied.clone(),
                cx,
            );
        }
        // US-013 + Terminal Thread surface: per-thread main area.
        if let Some(target) = self.current_thread_view_target() {
            let (p_idx, t_idx) = target;
            let kind = self
                .projects
                .get(p_idx)
                .and_then(|p| p.threads.get(t_idx))
                .map(|t| t.kind)
                .unwrap_or(crate::project::ThreadKind::Agent);
            match kind {
                crate::project::ThreadKind::Agent => {
                    self.ensure_thread_view_mounted(target, cx);
                    if let Some(view) = self.agents_thread_view.clone() {
                        return view.into_any_element();
                    }
                }
                crate::project::ThreadKind::Terminal => {
                    // Drop any stale ThreadView pointer so its
                    // subscriptions don't race against the PTY surface.
                    self.agents_thread_view = None;
                    self.agents_thread_view_for = None;
                    if let Some(view) = self.ensure_terminal_view_mounted(target, cx) {
                        return crate::app::agents_view_actions::render_terminal_thread_surface(
                            view,
                        );
                    }
                }
            }
        } else if self.agents_thread_view.is_some() {
            // The user deselected (e.g. deleted the active thread).
            // Drop the entity so its subscriptions are released.
            self.agents_thread_view = None;
            self.agents_thread_view_for = None;
        }

        if let Some(view) = self.agents_view.clone() {
            return view.into_any_element();
        }
        // Defensive: mode is Agents but the entity is missing. Show a
        // minimal placeholder rather than a blank pane.
        let ui = crate::theme::ui_colors();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(13.))
                    .child("Loading agents view..."),
            )
            .into_any_element()
    }

    /// `(project_idx, thread_idx)` of the currently selected thread,
    /// or `None` when the active project has no thread selected.
    pub(crate) fn current_thread_view_target(&self) -> Option<(usize, usize)> {
        let p = self.active_project_idx;
        let t = self.active_thread_idx?;
        let project = self.projects.get(p)?;
        if t < project.threads.len() {
            Some((p, t))
        } else {
            None
        }
    }

    /// Mount (or remount) the ThreadView entity for `target`. Looks
    /// up the cache by `Thread.id` first — if the user has visited
    /// this thread earlier in the session, the same Entity is
    /// re-bound so the in-memory timeline (reasoning cards, in-flight
    /// streaming pumps, scroll position) survives the round trip.
    /// Otherwise a fresh entity is built and inserted into the cache.
    fn ensure_thread_view_mounted(&mut self, target: (usize, usize), cx: &mut Context<Self>) {
        if self.agents_thread_view_for == Some(target) && self.agents_thread_view.is_some() {
            return;
        }
        let (p_idx, t_idx) = target;
        let Some(thread) = self.projects.get(p_idx).and_then(|p| p.threads.get(t_idx)) else {
            return;
        };
        let thread_id = thread.id;
        // Cache hit: re-bind the existing entity. Subscriptions and
        // streaming tasks owned by the ThreadView stay alive because
        // we never dropped it.
        if let Some(cached) = self.agents_thread_view_cache.get(&thread_id) {
            self.agents_thread_view = Some(cached.clone());
            self.agents_thread_view_for = Some(target);
            return;
        }
        let store_id = thread
            .store_id
            .clone()
            .map(paneflow_threads::ThreadId::from_string);
        let agent_kind = thread.agent;
        let cwd = std::path::PathBuf::from(&thread.cwd);
        let store = self.thread_store.clone();
        // US-016: the Composer hosted inside ThreadView needs its
        // own AgentDiscovery handle to render the agent picker
        // popover (PRD AC #4). AgentDiscovery caches its first
        // probe so allocating one per ThreadView is cheap.
        let discovery = std::sync::Arc::new(paneflow_acp::AgentDiscovery::new());
        let view = cx.new(|cx| {
            crate::agents::thread_view::ThreadView::new(
                store_id, store, agent_kind, cwd, discovery, cx,
            )
        });
        // US-020: subscribe to the ThreadView's ForkRequested events
        // so a Save click in the inline editor lands in
        // `handle_thread_fork`. The subscription is detached -- the
        // ThreadView entity owns its own lifecycle and the
        // subscription drops with the view.
        cx.subscribe(
            &view,
            |this, _src, event: &crate::agents::thread_view::ForkRequested, cx| {
                this.handle_thread_fork(event.message_idx, event.new_text.clone(), cx);
            },
        )
        .detach();
        // Agent-suggested title (Claude Code `/resume` summary, Codex
        // session label) OR client-side auto-derive from first prompt.
        // Renames the matching sidebar row + mirrors into `threads.db`
        // so the new title survives a restart.
        cx.subscribe(
            &view,
            |this, _src, event: &crate::agents::thread_view::TitleSuggested, cx| {
                this.handle_thread_title_suggested(event.title.clone(), event.policy.clone(), cx);
            },
        )
        .detach();
        self.agents_thread_view_cache
            .insert(thread_id, view.clone());
        self.agents_thread_view = Some(view);
        self.agents_thread_view_for = Some(target);
    }

    /// Apply an agent-suggested title to the currently active thread:
    /// updates the in-memory sidebar row, mirrors to `threads.db`
    /// via `ThreadStore::set_summary`, then persists the session so a
    /// restart picks up the new title.
    pub(crate) fn handle_thread_title_suggested(
        &mut self,
        title: String,
        policy: crate::agents::thread_view::TitleReplacePolicy,
        cx: &mut Context<Self>,
    ) {
        let Some((project_idx, thread_idx)) = self.agents_thread_view_for else {
            return;
        };
        // Strip the leading status-glyph decoration (Claude Code's
        // `✻`, Codex's braille spinner, generic `●`/`•`) that some CLI
        // wrappers prepend to the session title -- without this, the
        // sidebar row literally renders the dot in front of the label
        // and reads as a stalled spinner.
        let Some(title) = crate::project::clean_sidebar_title(&title) else {
            return;
        };
        let store_id = match self
            .projects
            .get_mut(project_idx)
            .and_then(|p| p.threads.get_mut(thread_idx))
        {
            Some(thread) => {
                if thread.title == title {
                    return;
                }
                // Replace policy gate. See `TitleReplacePolicy` for the
                // three call sites that produce a suggestion (agent
                // push, client auto-derive, background summarizer) and
                // what each one needs to preserve.
                use crate::agents::thread_view::TitleReplacePolicy;
                let allowed = match &policy {
                    TitleReplacePolicy::Always => true,
                    TitleReplacePolicy::OnlyIfDefault => thread.title == "New thread",
                    TitleReplacePolicy::OnlyIfStillEqualTo(snapshot) => &thread.title == snapshot,
                };
                if !allowed {
                    return;
                }
                thread.title = title.clone();
                thread.store_id.clone()
            }
            None => return,
        };
        if let (Some(id), Some(store)) = (store_id, &self.thread_store) {
            let typed = paneflow_threads::store::ThreadId::from_string(id);
            if let Err(err) = store.set_summary(&typed, &title) {
                log::warn!("agents-view: thread title sync to db failed: {err}");
            }
        }
        self.save_session(cx);
        cx.notify();
    }

    /// Commit an in-place edit on the active thread: truncate every
    /// item from `message_idx` onwards (the edited user message + the
    /// agent's response chain), persist the truncated history, then
    /// dispatch `new_text` through the composer so the agent regenerates
    /// the turn on the SAME thread (no fork, no `(fork)` row).
    pub(crate) fn handle_thread_fork(
        &mut self,
        message_idx: usize,
        new_text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agents_thread_view.clone() else {
            return;
        };
        view.update(cx, |tv, cx| {
            tv.truncate_for_edit(message_idx, cx);
        });
        if let Some(composer) = view.read(cx).composer() {
            composer.update(cx, |c, cx| {
                c.send_prompt(new_text, cx);
            });
        }
    }

    /// Mount (or reuse from cache) the [`TerminalView`] entity that
    /// backs a Terminal Thread at `target`. Returns the entity ready
    /// to be wrapped by [`render_terminal_thread_surface`].
    ///
    /// Mirrors [`Self::ensure_thread_view_mounted`] for the PTY path:
    /// cache hit re-binds the existing entity so the running shell
    /// process survives sidebar navigation; cache miss spawns a fresh
    /// PTY in the thread's cwd via [`TerminalView::with_cwd`].
    ///
    /// `workspace_id` for the new view defaults to the thread's own
    /// `id` so PTY tracking (signal routing, kill-on-quit) keys off a
    /// stable per-thread identifier rather than the CLI-mode workspace
    /// slot (which has no meaning in the Agents view).
    fn ensure_terminal_view_mounted(
        &mut self,
        target: (usize, usize),
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<crate::terminal::view::TerminalView>> {
        let (p_idx, t_idx) = target;
        let thread = self
            .projects
            .get(p_idx)
            .and_then(|p| p.threads.get(t_idx))?;
        let thread_id = thread.id;
        if let Some(cached) = self.agents_terminal_view_cache.get(&thread_id) {
            return Some(cached.clone());
        }
        let cwd = std::path::PathBuf::from(&thread.cwd);
        let view = cx.new(|cx| {
            crate::terminal::view::TerminalView::with_cwd(thread_id, Some(cwd), None, cx)
        });
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
        self.agents_terminal_view_cache
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
