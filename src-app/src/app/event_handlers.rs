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
            title_bar::TitleBarEvent::ToggleMenu(position) => {
                self.workspace_menu_open = None;
                self.notif_menu_open = None;
                self.profile_menu_open = None;
                self.title_bar_menu_open = if self.title_bar_menu_open.is_some() {
                    None
                } else {
                    Some(*position)
                };
                cx.notify();
            }
            title_bar::TitleBarEvent::ToggleProfile(position) => {
                self.workspace_menu_open = None;
                self.notif_menu_open = None;
                self.title_bar_menu_open = None;
                self.profile_menu_open = if self.profile_menu_open.is_some() {
                    None
                } else {
                    Some(*position)
                };
                cx.notify();
            }
            title_bar::TitleBarEvent::CloseRequested => {
                self.save_session(cx);
                // US-013 AC #2 — flush `app_exited` before the process is
                // torn down. Bounded to 2 s by the client; if PostHog is
                // unreachable the worker detaches and quit still proceeds.
                self.emit_app_exited_and_flush();
                cx.quit();
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
                let (source_cwd, initial_size) = {
                    let view = pane.read(cx).active_terminal().read(cx);
                    let cwd = view.terminal.cwd_now();
                    let term = view.terminal.term.lock();
                    let (cols, rows) = (term.columns(), term.screen_lines());
                    let size = match direction {
                        crate::layout::SplitDirection::Horizontal => (cols, (rows / 2).max(1)),
                        crate::layout::SplitDirection::Vertical => ((cols / 2).max(1), rows),
                    };
                    (cwd, size)
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
            // ChildExited + TitleChanged are handled by Pane's subscription
            _ => {}
        }
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
                    .any(|pane| pane.read(cx).tabs.contains(terminal))
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
            if ws.agent_pids.is_empty() {
                continue;
            }
            let before = ws.agent_pids.len();
            ws.agent_pids.retain(|_tool, &mut pid| pid_is_alive(pid));
            if ws.agent_pids.len() < before {
                changed = true;
                // If all agent PIDs were cleared and state is still active, reset to Inactive
                if ws.agent_pids.is_empty() && ws.ai_state != ai_types::AiToolState::Inactive {
                    ws.ai_state = ai_types::AiToolState::Inactive;
                    ws.active_tool_name = None;
                }
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
                                matches!(ws.ai_state, ai_types::AiToolState::Thinking(_))
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
                            if app.settings_section.is_none() {
                                cx.notify();
                            }
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
                            .tabs
                            .iter()
                            .map(|tv| tv.read(cx).terminal.child_pid)
                    })
                    .collect()
            })
            .unwrap_or_default();

        if pids.is_empty() {
            return true;
        }

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let ports = smol::unblock(move || crate::workspace::detect_ports(&pids)).await;
                let _ = cx.update(|cx| {
                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                        if let Some(ws) = app.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                            && ws.port_scan_generation == generation
                            && ws.active_ports != ports
                        {
                            ws.active_ports = ports;
                            // Clean up service labels for ports that are no longer active
                            ws.service_labels
                                .retain(|port, _| ws.active_ports.contains(port));
                            cx.notify();
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
        // Find workspace where this terminal is the active tab in any pane
        let ws_idx = self.workspaces.iter().position(|ws| {
            ws.root.as_ref().is_some_and(|root| {
                root.collect_leaves()
                    .iter()
                    .any(|pane| *pane.read(cx).active_terminal() == *terminal)
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
