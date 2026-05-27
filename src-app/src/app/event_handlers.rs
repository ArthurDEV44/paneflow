//! Event-subscription callbacks and background workers for `PaneFlowApp`.
//!
//! Hosts the GPUI `subscribe` handlers (`handle_title_bar_event`,
//! `handle_pane_event`, `handle_terminal_event`) plus the port-scan /
//! loader-animation / stale-PID-sweep workers and the CWD change handler.
//!
//! Extracted from `main.rs` per US-026 of the src-app refactor PRD — pure
//! code-motion, behaviour unchanged.

use alacritty_terminal::grid::Dimensions;
use gpui::{App, AppContext, Context, Entity};
use notify::Watcher;

use crate::layout::LayoutTree;
use crate::pane::{self, Pane};
use crate::terminal::{self, TerminalView};
use crate::window_chrome::title_bar;
use crate::{PaneFlowApp, ai_types};

/// Cross-platform "is this PID still running?" probe used by the AI agent
/// stale-PID sweep. Unix path preserves the pre-US-034 `kill(pid, 0)` +
/// `ESRCH` semantics (EPERM ⇒ alive). Windows path mirrors the pattern in
/// `terminal::pty_session::Drop`: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`
/// returns NULL for a dead/inaccessible PID. US-034 — keeps `libc::` calls
/// off the Windows compile path.
fn pid_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: `libc::kill` with sig=0 performs error-checking only and
        // does not deliver a signal. The call takes an i32 pid by value and
        // has no memory aliasing requirements.
        let ret = unsafe { libc::kill(pid as i32, 0) };
        if ret == -1 {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            // ESRCH = no such process; EPERM/etc. ⇒ process exists but we
            // can't signal it — keep the entry.
            return errno != libc::ESRCH;
        }
        true
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::OpenProcess;
        // PROCESS_QUERY_LIMITED_INFORMATION (winnt.h: 0x1000) — minimum
        // access right that lets OpenProcess succeed for any visible PID.
        // Declared locally so we don't require an extra windows-sys feature.
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        if pid == 0 {
            return false;
        }
        // SAFETY: `OpenProcess` either returns a valid handle that we close,
        // or NULL. No memory aliasing. `CloseHandle` on a valid handle is
        // always sound.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let _ = CloseHandle(handle);
            true
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Conservative fallback for exotic targets: never sweep. Better to
        // keep a stale entry than to drop a live one and confuse the AI
        // badge state.
        let _ = pid;
        true
    }
}

impl PaneFlowApp {
    pub(crate) fn handle_title_bar_event(
        &mut self,
        _title_bar: Entity<title_bar::TitleBar>,
        event: &title_bar::TitleBarEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            title_bar::TitleBarEvent::CloseRequested => {
                self.save_session(cx);
                // US-013 AC #2 — flush `app_exited` before the process is
                // torn down. Bounded to 2 s by the client; if PostHog is
                // unreachable the worker detaches and quit still proceeds.
                self.emit_app_exited_and_flush();
                cx.quit();
            }
            title_bar::TitleBarEvent::ToggleAgentsView => {
                // Direct toggle: dispatching `OpenAgentsView` via the
                // focus chain is unreliable when a child entity (e.g.
                // the composer's focused TextArea) intercepts the
                // action before it reaches the root `on_action`
                // listener. The two underlying helpers both take only
                // `&mut Context<Self>` so we can call them straight
                // from here; the Window-only branch of `handle_open_agents_view`
                // (`exit_agents_mode` focus restore) is replaced by
                // `close_agents_view`, which does the same teardown
                // minus the explicit focus call -- the next mouse /
                // key event in the CLI tree resolves focus naturally.
                match self.mode {
                    paneflow_config::schema::AppMode::Agents => self.close_agents_view(cx),
                    paneflow_config::schema::AppMode::Cli => self.enter_agents_mode(cx),
                }
            }
        }
    }

    pub(crate) fn handle_pane_event(
        &mut self,
        pane: Entity<Pane>,
        event: &pane::PaneEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            pane::PaneEvent::Remove => {
                // Find the workspace that owns this pane (not necessarily the
                // active one — shells can exit in background workspaces).
                let ws_idx = self.workspaces.iter().position(|ws| {
                    ws.root
                        .as_ref()
                        .is_some_and(|r| r.collect_leaves().contains(&pane))
                });
                let Some(ws_idx) = ws_idx else {
                    return;
                };

                // Remove this pane from the split tree
                if let Some(root) = self.workspaces[ws_idx].root.take() {
                    self.workspaces[ws_idx].root = root.remove_pane(&pane);
                }

                // Never leave a workspace without a pane — respawn at the
                // workspace's root cwd so the user returns to the right folder.
                if self.workspaces[ws_idx].root.is_none() {
                    let ws_id = self.workspaces[ws_idx].id;
                    let cwd = std::path::PathBuf::from(&self.workspaces[ws_idx].cwd);
                    let terminal = cx.new(|cx| TerminalView::with_cwd(ws_id, Some(cwd), None, cx));
                    cx.subscribe(&terminal, Self::handle_terminal_event)
                        .detach();
                    let new_pane = self.create_pane(terminal, ws_id, cx);
                    self.workspaces[ws_idx].root = Some(LayoutTree::Leaf(new_pane));
                    // The freshly-spawned replacement pane starts with an
                    // empty `custom_buttons` list — push the workspace's
                    // persisted set so the tab bar renders them again.
                    self.workspaces[ws_idx].propagate_custom_buttons(cx);
                }
                self.save_session(cx);
                cx.notify();
            }
            pane::PaneEvent::OpenClaudeSessions(position) => {
                // Toggle: clicking the icon again with the menu open closes it.
                if self.claude_sessions_menu_open.is_some() {
                    self.claude_sessions_menu_open = None;
                    self.claude_sessions.clear();
                    self.codex_sessions.clear();
                    self.opencode_sessions.clear();
                    self.claude_sessions_cwd = None;
                    self.claude_sessions_pane = None;
                    self.sessions_active_agent = crate::agent_sessions::SessionAgent::Claude;
                    cx.notify();
                    return;
                }

                // Resolve the active terminal's cwd: prefer the OSC 7 push
                // (`current_cwd`), fall back to the on-demand `cwd_now()`
                // syscall for shells that don't emit OSC 7.
                let cwd_str = pane.read(cx).active_terminal_opt().and_then(|tv| {
                    let view = tv.read(cx);
                    view.terminal.current_cwd.clone().or_else(|| {
                        view.terminal
                            .cwd_now()
                            .map(|p| p.to_string_lossy().into_owned())
                    })
                });

                // Mutually exclusive with the other dropdowns; matches the
                // title-bar / profile / notif menu pattern.
                self.workspace_menu_open = None;
                self.profile_menu_open = None;

                self.claude_sessions_menu_open = Some(*position);
                self.claude_sessions_cwd = cwd_str.clone();
                self.claude_sessions_pane = Some(pane.downgrade());
                self.claude_sessions.clear();
                self.codex_sessions.clear();
                self.opencode_sessions.clear();
                // Seed the active tab to the first agent the user has kept
                // visible in Settings → AI Agent. Falling back to Claude
                // when the list is empty is harmless: with no agent enabled
                // the sessions icon itself is hidden in `pane.rs`, so this
                // open path can't fire — the value just needs to satisfy
                // the field's type.
                let enabled_agents = crate::agent_sessions::enabled_session_agents();
                self.sessions_active_agent = enabled_agents
                    .first()
                    .copied()
                    .unwrap_or(crate::agent_sessions::SessionAgent::Claude);
                // Fresh handle so a previous scroll offset doesn't bleed
                // into the new popover.
                self.claude_sessions_scroll = gpui::ScrollHandle::new();
                self.claude_sessions_drag = None;

                if let Some(cwd) = cwd_str {
                    // Parallel scans — Claude Code under
                    // `~/.claude/projects/<slug>/`, Codex CLI under
                    // `~/.codex/sessions/YYYY/MM/DD/`, and OpenCode via
                    // a `opencode session list --format json` shell-out
                    // (the SQLite schema is unstable; the CLI is the
                    // published contract — see US-001 spike notes).
                    // Each task writes to its own Vec on the main
                    // thread. The popover may be closed or re-opened
                    // against a different cwd before any scan finishes,
                    // so we drop stale results by checking
                    // `claude_sessions_cwd` matches before applying.
                    //
                    // Scans for agents the user has hidden in Settings →
                    // AI Agent are skipped: with no UI to surface them
                    // the disk read would just be wasted I/O.
                    let scan_claude =
                        enabled_agents.contains(&crate::agent_sessions::SessionAgent::Claude);
                    let scan_codex =
                        enabled_agents.contains(&crate::agent_sessions::SessionAgent::Codex);
                    let scan_opencode =
                        enabled_agents.contains(&crate::agent_sessions::SessionAgent::OpenCode);

                    if scan_claude {
                        let claude_cwd_scan = cwd.clone();
                        let claude_cwd_match = cwd.clone();
                        cx.spawn(async move |this, cx| {
                            let sessions = smol::unblock(move || {
                                crate::claude_sessions::read_sessions_for_cwd(&claude_cwd_scan)
                            })
                            .await;
                            let _ = this.update(cx, |app, cx| {
                                if app.claude_sessions_menu_open.is_some()
                                    && app.claude_sessions_cwd.as_deref()
                                        == Some(claude_cwd_match.as_str())
                                {
                                    app.claude_sessions = sessions;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }

                    if scan_codex {
                        let codex_cwd_scan = cwd.clone();
                        let codex_cwd_match = cwd.clone();
                        cx.spawn(async move |this, cx| {
                            let sessions = smol::unblock(move || {
                                crate::codex_sessions::read_sessions_for_cwd(&codex_cwd_scan)
                            })
                            .await;
                            let _ = this.update(cx, |app, cx| {
                                if app.claude_sessions_menu_open.is_some()
                                    && app.claude_sessions_cwd.as_deref()
                                        == Some(codex_cwd_match.as_str())
                                {
                                    app.codex_sessions = sessions;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }

                    if scan_opencode {
                        let opencode_cwd_scan = cwd.clone();
                        let opencode_cwd_match = cwd;
                        cx.spawn(async move |this, cx| {
                            let sessions = smol::unblock(move || {
                                crate::opencode_sessions::read_sessions_for_cwd(&opencode_cwd_scan)
                            })
                            .await;
                            let _ = this.update(cx, |app, cx| {
                                if app.claude_sessions_menu_open.is_some()
                                    && app.claude_sessions_cwd.as_deref()
                                        == Some(opencode_cwd_match.as_str())
                                {
                                    app.opencode_sessions = sessions;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }
                }
                cx.notify();
            }
            pane::PaneEvent::Split(direction) => {
                let direction = *direction;
                const MAX_PANES: usize = 32;
                if let Some(ws) = self.active_workspace()
                    && let Some(root) = &ws.root
                    && root.leaf_count() >= MAX_PANES
                {
                    return;
                }
                // Inherit CWD and estimate initial grid size from the source terminal.
                // Grid is halved in the split direction; refined to exact size on first prepaint.
                // US-020: markdown panes have no terminal — fall back to the
                // workspace's root cwd and a default 80×24 grid so a split
                // request from a markdown pane still yields a usable terminal.
                let (source_cwd, initial_size) = match pane.read(cx).active_terminal_opt() {
                    Some(active) => {
                        let view = active.read(cx);
                        let cwd = view.terminal.cwd_now();
                        let term = view.terminal.term.lock();
                        let (cols, rows) = (term.columns(), term.screen_lines());
                        let size = match direction {
                            crate::layout::SplitDirection::Horizontal => (cols, (rows / 2).max(1)),
                            crate::layout::SplitDirection::Vertical => ((cols / 2).max(1), rows),
                        };
                        (cwd, size)
                    }
                    None => {
                        let cwd = self
                            .active_workspace()
                            .map(|ws| std::path::PathBuf::from(&ws.cwd));
                        (cwd, (80, 24))
                    }
                };
                let ws_id = self.active_workspace().map(|ws| ws.id).unwrap_or(0);
                let new_terminal =
                    cx.new(|cx| TerminalView::with_cwd(ws_id, source_cwd, Some(initial_size), cx));
                let new_pane = self.create_pane(new_terminal, ws_id, cx);
                if let Some(ws) = self.active_workspace_mut()
                    && let Some(root) = &mut ws.root
                {
                    root.split_at_pane(&pane, direction, new_pane);
                }
                // The freshly-spawned pane starts with an empty
                // `custom_buttons` list — push the workspace's current set
                // so the new pane's tab bar matches its siblings.
                if let Some(ws) = self.workspaces.get(self.active_idx) {
                    ws.propagate_custom_buttons(cx);
                }
                self.save_session(cx);
                cx.notify();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Terminal event handling — push-based port detection and CWD tracking
    // -----------------------------------------------------------------------

    pub(crate) fn handle_terminal_event(
        &mut self,
        terminal: Entity<TerminalView>,
        event: &terminal::TerminalEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            terminal::TerminalEvent::ActivityBurst => {
                if let Some(ws_idx) = self.workspace_idx_for_terminal(&terminal, cx) {
                    self.schedule_port_scan(ws_idx, cx);
                }
            }
            terminal::TerminalEvent::CwdChanged(new_cwd) => {
                self.handle_cwd_change(&terminal, new_cwd, cx);
            }
            terminal::TerminalEvent::ServiceDetected(info) => {
                if let Some(ws_idx) = self.workspace_idx_for_terminal(&terminal, cx) {
                    let ws = &mut self.workspaces[ws_idx];
                    // Don't overwrite a frontend label with a non-frontend one.
                    // A backend terminal might reference "localhost:3000" in CORS
                    // config, but the frontend terminal already claimed that port.
                    if let Some(existing) = ws.service_labels.get(&info.port)
                        && existing.is_frontend
                        && !info.is_frontend
                    {
                        return;
                    }
                    ws.service_labels.insert(info.port, info.clone());
                    if self.settings_section.is_none() {
                        cx.notify();
                    }
                }
            }
            terminal::TerminalEvent::CancelSwapMode => {
                self.cancel_swap_mode(cx);
            }
            terminal::TerminalEvent::SelectionCopied => {
                self.show_toast("Copied", cx);
            }
            terminal::TerminalEvent::OpenMarkdownPath(path) => {
                self.open_markdown_in_pane(&terminal, path.clone(), cx);
            }
            // ChildExited + TitleChanged are handled by Pane's subscription
            _ => {}
        }
    }

    /// US-020 — append a markdown tab to the pane that owns `source_terminal`.
    ///
    /// The historical implementation split the layout vertically and created
    /// a dedicated markdown pane; the user feedback was that opening a doc
    /// shouldn't shrink the terminal real-estate. The current behaviour is to
    /// make markdown a peer tab inside the same pane — the user keeps the
    /// terminal+markdown pair via Ctrl+Tab / mouse-click, and the layout tree
    /// is untouched.
    fn open_markdown_in_pane(
        &mut self,
        source_terminal: &Entity<TerminalView>,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(ws_idx) = self.workspace_idx_for_terminal(source_terminal, cx) else {
            return;
        };
        let source_pane = self.workspaces[ws_idx].root.as_ref().and_then(|root| {
            root.collect_leaves()
                .into_iter()
                .find(|pane| pane.read(cx).contains_terminal(source_terminal))
        });
        let Some(source_pane) = source_pane else {
            return;
        };

        let path_for_pane = path.clone();
        let markdown = cx.new(|cx: &mut Context<crate::markdown::MarkdownView>| {
            crate::markdown::MarkdownView::open(path_for_pane, cx)
        });

        source_pane.update(cx, |pane, cx| {
            pane.add_markdown_tab(markdown, cx);
            cx.notify();
        });
        self.save_session(cx);
        cx.notify();
    }

    /// Find which workspace contains the given terminal entity.
    fn workspace_idx_for_terminal(
        &self,
        terminal: &Entity<TerminalView>,
        cx: &App,
    ) -> Option<usize> {
        self.workspaces.iter().position(|ws| {
            ws.root.as_ref().is_some_and(|root| {
                root.collect_leaves()
                    .iter()
                    .any(|pane| pane.read(cx).contains_terminal(terminal))
            })
        })
    }

    /// Probe registered AI agent PIDs and clean up stale entries where the
    /// process no longer exists. See [`pid_is_alive`] for the per-platform
    /// probe (Unix: `kill(pid, 0)` / `ESRCH`; Windows: `OpenProcess` null
    /// handle; other: conservative keep).
    pub(crate) fn sweep_stale_pids(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for ws in &mut self.workspaces {
            if ws.agent_sessions.is_empty() {
                continue;
            }
            let before = ws.agent_sessions.len();
            // Synthetic PIDs (from the upsert fallback for legacy shims
            // without `pid` on every frame) are stored in the high half
            // of u32 — outside the OS-assignable range on all supported
            // platforms — so probing them with `kill(pid, 0)` would
            // always say "dead" and immediately drop a live legacy
            // session. Keep them around: they'll be cleared by
            // `ai.session_end` or by the next state transition.
            ws.agent_sessions
                .retain(|&pid, _session| pid > i32::MAX as u32 || pid_is_alive(pid));
            if ws.agent_sessions.len() < before {
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Start the spinner animation loop. Runs at ~60fps, advancing
    /// `loader_angle` on all Thinking workspaces. Self-stops when no
    /// workspace is in Thinking state.
    pub(crate) fn start_loader_animation(&mut self, cx: &mut Context<Self>) {
        self.loader_anim_running = true;
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(16)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let any_thinking = app.workspaces.iter().any(|ws| {
                                ws.agent_sessions
                                    .values()
                                    .any(|s| s.state == ai_types::AgentState::Thinking)
                            });
                            if !any_thinking {
                                app.loader_anim_running = false;
                                return false;
                            }
                            // 0.9s per revolution ≈ 0.1164 rad/frame at 60fps
                            let delta = std::f32::consts::TAU / (0.9 * 60.0);
                            for ws in &app.workspaces {
                                if matches!(ws.ai_state, ai_types::AiToolState::Thinking(_)) {
                                    let angle = ws.loader_angle.get() + delta;
                                    ws.loader_angle.set(angle % std::f32::consts::TAU);
                                }
                            }
                            // US-010 — notify unconditionally. The prior
                            // `settings_section.is_none()` guard was a
                            // premature optimization introduced alongside
                            // the AI-detection plumbing (commit b99d58b,
                            // no accompanying rationale) and carried
                            // across the src-app refactor verbatim. With
                            // the guard in place, the sidebar-loader
                            // angle advanced silently while Settings was
                            // open; closing Settings would redraw the
                            // spinner at a stale position until the next
                            // real event. `cx.notify()` only sets a
                            // dirty flag (no re-entrancy — the render
                            // pass is deferred past this closure's
                            // update scope); GPUI's diffing then skips
                            // GPU work when no tracked reads changed,
                            // so the effective cost while Settings is
                            // open is one flag set plus one short
                            // layout pass per 16 ms frame.
                            cx.notify();
                            true
                        })
                    });
                    match result {
                        Ok(true) => {}
                        _ => break,
                    }
                }
            },
        )
        .detach();
    }

    /// Schedule a debounced port scan for the given workspace.
    /// Uses a generation counter to cancel superseded scans.
    fn schedule_port_scan(&mut self, ws_idx: usize, cx: &mut Context<Self>) {
        let ws = &mut self.workspaces[ws_idx];
        ws.port_scan_generation += 1;
        let generation = ws.port_scan_generation;
        let ws_id = ws.id;

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                // Debounce: wait 500ms for activity to settle
                smol::Timer::after(std::time::Duration::from_millis(500)).await;

                // Burst scan at 0s, +2s, +6s after debounce
                for delay_ms in [0u64, 2000, 6000] {
                    if delay_ms > 0 {
                        smol::Timer::after(std::time::Duration::from_millis(delay_ms)).await;
                    }
                    let should_continue = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            app.run_port_scan(ws_id, generation, cx)
                        })
                    });
                    match should_continue {
                        Ok(true) => {}
                        _ => break,
                    }
                }
            },
        )
        .detach();
    }

    /// Execute a single port scan for a workspace. Returns `false` if the scan
    /// should be aborted (generation superseded or workspace removed).
    fn run_port_scan(&mut self, ws_id: u64, generation: u64, cx: &mut Context<Self>) -> bool {
        let ws = match self.workspaces.iter().find(|ws| ws.id == ws_id) {
            Some(ws) if ws.port_scan_generation == generation => ws,
            _ => return false,
        };

        let pids: Vec<u32> = ws
            .root
            .as_ref()
            .map(|root| {
                root.collect_leaves()
                    .iter()
                    .flat_map(|pane| {
                        pane.read(cx)
                            .terminals()
                            .map(|tv| tv.read(cx).terminal.child_pid)
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
            .unwrap_or_default();

        if pids.is_empty() {
            return true;
        }

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                // One descendant walk feeds two consumers: the port
                // table and the AI-process detector. Doing both in the
                // same `smol::unblock` keeps the walk single-shot per
                // scan even though the two scans are logically
                // independent.
                let pids_for_scan = pids.clone();
                let (ports, detected_agents) = smol::unblock(move || {
                    (
                        crate::workspace::detect_ports(&pids_for_scan),
                        crate::workspace::detect_ai_processes(&pids_for_scan),
                    )
                })
                .await;
                let _ = cx.update(|cx| {
                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                        if let Some(ws) = app.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                            && ws.port_scan_generation == generation
                        {
                            let mut changed = false;
                            if ws.active_ports != ports {
                                ws.active_ports = ports;
                                ws.service_labels
                                    .retain(|port, _| ws.active_ports.contains(port));
                                changed = true;
                            }
                            if ws.detected_agents != detected_agents {
                                ws.detected_agents = detected_agents;
                                changed = true;
                            }
                            if changed {
                                cx.notify();
                            }
                        }
                    })
                });
            },
        )
        .detach();
        true
    }

    /// Handle a CWD change from a terminal. Only processes if the terminal is
    /// the active tab of a pane in its workspace (background terminals ignored).
    fn handle_cwd_change(
        &mut self,
        terminal: &Entity<TerminalView>,
        new_cwd: &str,
        cx: &mut Context<Self>,
    ) {
        // Find workspace where this terminal is the active tab in any pane.
        // US-020: skip markdown panes — they have no active terminal, so the
        // identity check via `active_terminal_opt` returns None for them.
        let ws_idx = self.workspaces.iter().position(|ws| {
            ws.root.as_ref().is_some_and(|root| {
                root.collect_leaves().iter().any(|pane| {
                    pane.read(cx)
                        .active_terminal_opt()
                        .is_some_and(|t| *t == *terminal)
                })
            })
        });
        let Some(ws_idx) = ws_idx else { return };

        if self.workspaces[ws_idx].cwd == new_cwd {
            return;
        }

        let new_cwd_owned = new_cwd.to_string();

        // Run git probe off main thread
        cx.spawn({
            let new_cwd = new_cwd_owned.clone();
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let (git_dir, branch, is_repo, stats) = smol::unblock({
                    let cwd = new_cwd.clone();
                    move || {
                        let git_dir = crate::workspace::find_git_dir(&cwd);
                        let (branch, is_repo) = crate::workspace::detect_branch(&cwd);
                        let stats = crate::workspace::GitDiffStats::from_cwd(&cwd);
                        (git_dir, branch, is_repo, stats)
                    }
                })
                .await;

                let _ = cx.update(|cx| {
                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                        if ws_idx >= app.workspaces.len() {
                            return;
                        }
                        // Unwatch old git dir
                        let old_git_dir = app.workspaces[ws_idx].git_dir.clone();
                        if let Some(ref dir) = old_git_dir {
                            app.unwatch_git_dir(dir);
                        }
                        // Update workspace git tracking (cwd stays fixed at creation —
                        // it represents the workspace's root folder and must not drift
                        // when the user `cd`s inside the shell).
                        let ws = &mut app.workspaces[ws_idx];
                        ws.git_dir = git_dir.clone();
                        ws.git_branch = branch.clone();
                        ws.is_git_repo = is_repo;
                        ws.git_stats = stats.clone();
                        // Watch new git dir
                        if let Some(ref dir) = git_dir {
                            let count = app.git_watch_counts.entry(dir.clone()).or_insert(0);
                            *count += 1;
                            if *count == 1
                                && let Some(ref mut watcher) = app.git_watcher
                                && let Err(e) =
                                    watcher.watch(dir, notify::RecursiveMode::NonRecursive)
                            {
                                log::warn!("git watcher: failed to watch {}: {e}", dir.display());
                            }
                        }
                        log::debug!("workspace CWD changed to: {new_cwd}");
                        cx.notify();
                    })
                });
            }
        })
        .detach();
    }
}
