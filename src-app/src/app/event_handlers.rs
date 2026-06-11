//! Event-subscription callbacks and background workers for `PaneFlowApp`.
//!
//! Hosts the GPUI `subscribe` handlers (`handle_title_bar_event`,
//! `handle_pane_event`, `handle_terminal_event`) plus the port-scan /
//! loader-animation / stale-PID-sweep workers and the CWD change handler.
//!
//! Extracted from `main.rs` per US-026 of the src-app refactor PRD — pure
//! code-motion, behaviour unchanged.

use gpui::{App, AppContext, Context, Entity};
use notify::Watcher;

use crate::layout::{LayoutTree, MAX_PANES};
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
                let ws_idx = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.root.as_ref().is_some_and(|r| r.contains_leaf(&pane)));
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
            pane::PaneEvent::ToggleAgentSessions => {
                // Toggle: clicking the icon again with the sidebar open closes it.
                if self.agent_sessions.sessions_sidebar_open {
                    self.close_sessions_sidebar(cx);
                    return;
                }
                // Open + bind + scan, extracted to `sessions_sidebar.rs` so a
                // workspace switch can re-target the open sidebar through the
                // exact same path.
                self.open_sessions_sidebar_for_pane(&pane, cx);
            }
            pane::PaneEvent::ToggleFilesSidebar => {
                // Open/close the docked Files tree for the active workspace's
                // folder. Mutual exclusion with the sessions sidebar is handled
                // inside `toggle_files_sidebar`.
                self.toggle_files_sidebar(cx);
            }
            pane::PaneEvent::CopySurfaceRef(surface_id) => {
                // US-010: resolve the globally-disambiguated name (matching the
                // MCP `list_panes` tool) and copy it to the clipboard. Fall back
                // to the raw id if the surface vanished between click and here.
                let sid = *surface_id;
                let reference = self
                    .collect_surface_meta(cx)
                    .into_iter()
                    .find(|m| m.surface_id == sid)
                    .map(|m| m.name)
                    .unwrap_or_else(|| sid.to_string());
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(reference.clone()));
                self.show_toast(format!("Copied surface ref: {reference}"), cx);
            }
            pane::PaneEvent::SurfaceRenamed => {
                // US-013: a tab's custom name changed — persist so it survives
                // restart (the name rides in the layout's SurfaceDefinition).
                self.save_session(cx);
                cx.notify();
            }
            pane::PaneEvent::OpenTabMenu { tab_idx, position } => {
                // EP-002 US-006: open the "Move to pane…" menu for this tab.
                // Mutually exclusive with the other popovers, matching the
                // workspace/profile/sessions menu pattern.
                self.workspace_menu_open = None;
                self.profile_menu_open = None;
                self.tab_menu_open = Some(crate::TabContextMenu {
                    source_pane: pane.clone(),
                    tab_idx: *tab_idx,
                    position: *position,
                });
                cx.notify();
            }
            pane::PaneEvent::DropSplit {
                edge,
                source_pane,
                source_idx,
                duplicate,
            } => {
                let edge = *edge;
                let source_idx = *source_idx;
                let duplicate = *duplicate;
                let source_pane = source_pane.clone();
                let target = &pane; // the emitting pane is the split target

                // Resolve the workspace owning the target pane.
                let Some(ws_idx) = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.root.as_ref().is_some_and(|r| r.contains_leaf(target)))
                else {
                    return;
                };

                // A split adds one pane — refuse at the cap (edge case #5).
                if self.workspaces[ws_idx]
                    .root
                    .as_ref()
                    .map(|r| r.leaf_count())
                    .unwrap_or(0)
                    >= MAX_PANES
                {
                    return;
                }

                // Refuse a meaningless self-split: promoting a pane's *only*
                // tab onto its own edge just replaces it with an identical
                // single-tab pane (edge case #9). A pane with >1 tab can
                // legitimately split one tab out.
                if !duplicate && &source_pane == target && source_pane.read(cx).tabs.len() <= 1 {
                    return;
                }

                let ws_id = self.workspaces[ws_idx].id;

                // Build the new pane: a fresh terminal at the dragged tab's cwd
                // (duplicate, US-010) or the moved tab itself (US-009).
                let new_pane = if duplicate {
                    let cwd = source_pane
                        .read(cx)
                        .tabs
                        .get(source_idx)
                        .and_then(crate::pane::TabContent::as_terminal)
                        .and_then(|t| t.read(cx).terminal.cwd_now());
                    let term = cx.new(|cx| TerminalView::with_cwd(ws_id, cwd, None, cx));
                    self.create_pane(term, ws_id, cx)
                } else {
                    let Some(tab) =
                        source_pane.update(cx, |src, _| src.take_tab_for_move(source_idx))
                    else {
                        return;
                    };
                    let p = cx.new(|cx| crate::pane::Pane::new_with_tab(tab, ws_id, cx));
                    cx.subscribe(&p, Self::handle_pane_event).detach();
                    p
                };

                let (direction, swap) = edge.to_split();
                if let Some(root) = &mut self.workspaces[ws_idx].root {
                    root.split_at_pane(target, direction, new_pane.clone());
                    if swap {
                        root.swap_panes(target, &new_pane);
                    }
                }

                // Move-only: reflow away the source pane if it emptied.
                if !duplicate {
                    source_pane.update(cx, |src, src_cx| {
                        if src.tabs.is_empty() {
                            src_cx.emit(pane::PaneEvent::Remove);
                        } else {
                            src_cx.notify();
                        }
                    });
                }

                self.workspaces[ws_idx].propagate_custom_buttons(cx);
                // Focus the new pane on next render (no Window here).
                self.pending_pane_focus = Some(new_pane);
                self.save_session(cx);
                cx.notify();
            }
            pane::PaneEvent::DuplicateTabInto {
                source_pane,
                source_idx,
                dest_idx,
            } => {
                // EP-003 US-010: a strip/center drop with the duplicate modifier
                // held. Spawn a fresh terminal at the dragged tab's CWD and
                // insert it into the emitting (destination) pane; the original
                // stays put. Spawning here (not in the Pane) is required so the
                // app-level CWD/port subscription gets wired, exactly like the
                // `DropSplit` duplicate path and `create_pane`.
                let source_idx = *source_idx;
                let dest_idx = *dest_idx;
                let source_pane = source_pane.clone();
                let dest = pane.clone(); // the emitting pane is the destination

                // Resolve the workspace owning the destination pane (for ws_id).
                // A bail here also covers the race where `dest` was removed from
                // the tree between the drop emit and this handler.
                let Some(ws_idx) = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.root.as_ref().is_some_and(|r| r.contains_leaf(&dest)))
                else {
                    return;
                };
                let ws_id = self.workspaces[ws_idx].id;

                // CWD of the dragged terminal. `None` (non-terminal tab, or a
                // stale `source_idx`) → fresh terminal at the default cwd,
                // matching `DropSplit`'s duplicate path.
                let cwd = source_pane
                    .read(cx)
                    .tabs
                    .get(source_idx)
                    .and_then(crate::pane::TabContent::as_terminal)
                    .and_then(|t| t.read(cx).terminal.cwd_now());
                let term = cx.new(|cx| TerminalView::with_cwd(ws_id, cwd, None, cx));
                // App-level subscription so CWD/port/service events route
                // (mirrors `create_pane`); the pane-level subscription is wired
                // by `insert_duplicated_tab`.
                cx.subscribe(&term, Self::handle_terminal_event).detach();

                dest.update(cx, |p, cx| {
                    p.insert_duplicated_tab(crate::pane::TabContent::Terminal(term), dest_idx, cx);
                });
                self.workspaces[ws_idx].propagate_custom_buttons(cx);
                // Focus the destination pane (its newly-selected duplicate tab)
                // on next render (no Window here).
                self.pending_pane_focus = Some(dest);
                self.save_session(cx);
                cx.notify();
            }
            pane::PaneEvent::DropSessionSplit {
                edge,
                agent,
                session_id,
                cwd,
            } => {
                // A session row was dropped out of the sidebar onto a pane.
                // Spawn a fresh terminal at the session's cwd running the
                // agent's resume command, then split the target pane toward the
                // previewed edge (or append it here as a tab for center).
                let edge = *edge;
                let agent = *agent;
                let session_id = session_id.clone();
                let cwd = cwd.clone();
                let target = pane.clone(); // the emitting pane is the target

                let Some(ws_idx) = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.root.as_ref().is_some_and(|r| r.contains_leaf(&target)))
                else {
                    return;
                };

                // A split adds one pane — refuse at the cap (edge case #5). A
                // center drop appends a tab to an existing pane, so it doesn't
                // grow the count and isn't capped.
                if edge.is_some()
                    && self.workspaces[ws_idx]
                        .root
                        .as_ref()
                        .map(|r| r.leaf_count())
                        .unwrap_or(0)
                        >= MAX_PANES
                {
                    return;
                }

                let ws_id = self.workspaces[ws_idx].id;
                let cwd_path = (!cwd.is_empty()).then(|| std::path::PathBuf::from(&cwd));
                let term = cx.new(|cx| TerminalView::with_cwd(ws_id, cwd_path, None, cx));
                // Resume the picked session in the new terminal. Honors the
                // Claude bypass flag exactly like a tab-bar launch. Skips the
                // send if the id fails the allow-list (defence-in-depth).
                if let Some(resume) =
                    crate::app::sessions_sidebar::resume_command(agent, &session_id)
                {
                    term.read(cx).send_command(&resume);
                }

                match edge {
                    Some(edge) => {
                        // `create_pane` wires the app-level CWD/port subscription
                        // and the pane-event subscription (mirrors `DropSplit`).
                        let new_pane = self.create_pane(term, ws_id, cx);
                        let (direction, swap) = edge.to_split();
                        if let Some(root) = &mut self.workspaces[ws_idx].root {
                            root.split_at_pane(&target, direction, new_pane.clone());
                            if swap {
                                root.swap_panes(&target, &new_pane);
                            }
                        }
                        self.workspaces[ws_idx].propagate_custom_buttons(cx);
                        self.pending_pane_focus = Some(new_pane);
                    }
                    None => {
                        // Center drop: append the resumed session as a new tab in
                        // the target pane (mirrors `DuplicateTabInto`).
                        cx.subscribe(&term, Self::handle_terminal_event).detach();
                        target.update(cx, |p, cx| {
                            let dest_idx = p.tabs.len();
                            p.insert_duplicated_tab(
                                crate::pane::TabContent::Terminal(term),
                                dest_idx,
                                cx,
                            );
                        });
                        self.workspaces[ws_idx].propagate_custom_buttons(cx);
                        self.pending_pane_focus = Some(target);
                    }
                }
                self.save_session(cx);
                cx.notify();
            }
            pane::PaneEvent::DropMarkdownSplit { edge, path } => {
                // A markdown row was dropped out of the Files sidebar onto a
                // pane (EP-003 US-008). Open it via the existing `MarkdownView`
                // API, then split the target toward the previewed edge or append
                // it here as a tab (center). Mirrors `DropSessionSplit`, minus
                // the terminal spawn.
                let edge = *edge;
                let path = path.clone();
                let target = pane.clone(); // the emitting pane is the target

                let Some(ws_idx) = self
                    .workspaces
                    .iter()
                    .position(|ws| ws.root.as_ref().is_some_and(|r| r.contains_leaf(&target)))
                else {
                    return;
                };

                // A split adds one pane — refuse at the cap (edge case #9). A
                // center drop appends a tab, so it isn't capped.
                if edge.is_some()
                    && self.workspaces[ws_idx]
                        .root
                        .as_ref()
                        .map(|r| r.leaf_count())
                        .unwrap_or(0)
                        >= MAX_PANES
                {
                    return;
                }

                let ws_id = self.workspaces[ws_idx].id;
                let markdown = cx.new(|cx| crate::markdown::MarkdownView::open(path, cx));

                match edge {
                    Some(edge) => {
                        let new_pane = cx.new(|cx| {
                            crate::pane::Pane::new_with_tab(
                                crate::pane::TabContent::Markdown(markdown),
                                ws_id,
                                cx,
                            )
                        });
                        cx.subscribe(&new_pane, Self::handle_pane_event).detach();
                        let (direction, swap) = edge.to_split();
                        if let Some(root) = &mut self.workspaces[ws_idx].root {
                            root.split_at_pane(&target, direction, new_pane.clone());
                            if swap {
                                root.swap_panes(&target, &new_pane);
                            }
                        }
                        self.workspaces[ws_idx].propagate_custom_buttons(cx);
                        self.pending_pane_focus = Some(new_pane);
                    }
                    None => {
                        // Center drop: append the markdown as a new tab in the
                        // target pane (mirrors the click-to-open path).
                        target.update(cx, |p, cx| {
                            p.add_markdown_tab(markdown, cx);
                        });
                        self.pending_pane_focus = Some(target);
                    }
                }
                self.save_session(cx);
                cx.notify();
            }
            pane::PaneEvent::Split(direction) => {
                let direction = *direction;
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
                        let (cols, rows) = crate::terminal::types::grid_size(&term);
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
            terminal::TerminalEvent::OpenCodePath { path, line, col } => {
                // Spawn the editor on the GPUI background executor so a
                // slow editor launch (cold VS Code, remote SSH editor)
                // never blocks the main thread. `open_at_location`
                // already log-swallows failures, so we don't need to
                // surface the result here.
                let path = path.clone();
                let line = *line;
                let col = *col;
                cx.background_executor()
                    .spawn(async move {
                        crate::editor::open_at_location(&path, line, col);
                    })
                    .detach();
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
                // U-013: zero-alloc — `any_leaf` short-circuits without the
                // `collect_leaves()` Vec<Entity<Pane>> clone the old form built.
                root.any_leaf(&mut |pane| pane.read(cx).contains_terminal(terminal))
            })
        })
    }

    /// Probe registered AI agent PIDs and clean up stale entries where the
    /// process no longer exists. See [`pid_is_alive`] for the per-platform
    /// probe (Unix: `kill(pid, 0)` / `ESRCH`; Windows: `OpenProcess` null
    /// handle; other: conservative keep).
    pub(crate) fn sweep_stale_pids(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        // EP-004 US-010: surfaces that still resolve to a live terminal tab.
        // An `Errored` session's PID is dead by definition (the binary
        // exited) — it is spared from the PID reap WHILE its pane lives so
        // the crash signal stays visible, and reaped here once the pane
        // closes. An Errored session that never resolved a surface has no
        // visible anchor beyond the sidebar; it follows the plain PID reap
        // (≤ 30 s) so unresolvable rows can never accumulate.
        let live_surfaces: std::collections::HashSet<u64> = self
            .workspaces
            .iter()
            .filter_map(|ws| ws.root.as_ref())
            .flat_map(|root| root.collect_leaves())
            .flat_map(|pane| {
                pane.read(cx)
                    .terminals()
                    .map(|t| t.entity_id().as_u64())
                    .collect::<Vec<_>>()
            })
            .collect();
        // EP-004 US-011: Stalled detection (default ON, threshold default
        // 300 s — both hot-reload aware via `cached_config`). The sweep runs
        // every 30 s, so the effective granularity is threshold ± 30 s
        // (documented in the PRD AC and the JSON-schema description).
        let stall_enabled = self.cached_config.agent_stall_detection_enabled();
        let stall_threshold = std::time::Duration::from_secs(
            self.cached_config.resolved_agent_stall_threshold_secs(),
        );
        let mut stalled_notifs: Vec<(String, u64)> = Vec::new();
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
            ws.agent_sessions.retain(|&pid, session| {
                pid > i32::MAX as u32
                    || pid_is_alive(pid)
                    || (session.state == ai_types::AgentState::Errored
                        && session
                            .surface_id
                            .is_some_and(|sid| live_surfaces.contains(&sid)))
            });
            if ws.agent_sessions.len() < before {
                changed = true;
            }
            // US-011: a `Thinking` session silent past the threshold flips
            // to `Stalled`. Only `Thinking` flips, so the once-per-episode
            // notification dedup is structural: the session stays Stalled
            // (this branch can't re-trigger) until a hook event revives it,
            // and a NEW episode requires a fresh Thinking phase first.
            if stall_enabled {
                for session in ws.agent_sessions.values_mut() {
                    if session.state == ai_types::AgentState::Thinking
                        && session.last_activity.elapsed() >= stall_threshold
                    {
                        session.state = ai_types::AgentState::Stalled;
                        // This write bypasses `upsert_session_state`, so hold
                        // its invariant by hand: only WaitingForInput carries
                        // a wait stamp. A Thinking row is already None, but
                        // clear defensively rather than rely on that.
                        session.waiting_since = None;
                        stalled_notifs
                            .push((ws.title.clone(), session.last_activity.elapsed().as_secs()));
                        changed = true;
                    }
                }
            }
        }
        // Agents-view threads: a CLI killed mid-turn never sends `ai.stop`,
        // which would leave the row spinner running forever. Same
        // conservative policy as above — a thread whose hook frames carried
        // no PID is kept as-is (cleared by `ai.stop` / `ai.session_end`).
        for t in self
            .projects
            .iter_mut()
            .flat_map(|p| p.threads.iter_mut())
            .chain(self.chats.iter_mut())
        {
            if t.status != crate::project::ThreadStatus::Idle
                && let Some(pid) = t.agent_pid
                && !pid_is_alive(pid)
            {
                t.status = crate::project::ThreadStatus::Idle;
                t.agent_pid = None;
                changed = true;
            }
        }
        if changed {
            // US-018 (orchestration-v2): a swept session may have been
            // driving a pane glow — resync so no orphan attention survives.
            self.sync_attention(cx);
            // EP-001 US-003 (cli-cockpit): a swept `Thinking` session leaves
            // a bare shell — flush (or drop) its queued prompt now, else the
            // buffer and the "1 queued" chip strand forever (no further
            // `ai.*` frame will ever arrive for the dead session).
            self.agent_sessions_changed(cx);
            cx.notify();
        }
        // EP-004 US-011: fire AFTER the state writes so the toast and the UI
        // agree. One entry per Thinking→Stalled transition == one
        // notification per stall episode (PRD dedup AC).
        for (title, silent_secs) in stalled_notifs {
            super::ipc_handler::fire_stalled_notification(
                &title,
                silent_secs,
                cx.background_executor().clone(),
            );
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
                                if ws
                                    .agent_sessions
                                    .values()
                                    .any(|s| s.state == ai_types::AgentState::Thinking)
                                {
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
                root.any_leaf(&mut |pane| {
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

        // US-019: capture the stable workspace id, NOT the positional index.
        // The git probe below awaits (long on big repos / network FS); during
        // that await the main loop can run close/reorder/IPC-close and compact
        // the `Vec`, so a reused `ws_idx` would point at a *different*
        // workspace (silent git-state corruption + watch refcount desync).
        // Re-resolve the index by identity after the await — model:
        // `run_port_scan` / `spawn_initial_git_stats`.
        let ws_id = self.workspaces[ws_idx].id;

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
                        // Re-resolve by identity: the workspace may have been
                        // closed or reordered during the await.
                        let Some(ws_idx) = app.workspaces.iter().position(|ws| ws.id == ws_id)
                        else {
                            return;
                        };
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

    /// US-013: populate a freshly-created workspace's `git diff --shortstat`
    /// stats off the GPUI main thread. The constructors build with
    /// `git_stats: default()` (0/0) so the blocking `git` subprocess never runs
    /// on the render thread; this spawns it via `smol::unblock` and re-injects
    /// the result, keyed by the stable `ws_id` (another workspace may be
    /// created/closed during the await — EP-003 identity model). Mirrors
    /// [`handle_cwd_change`].
    pub(crate) fn spawn_initial_git_stats(ws_id: u64, cwd: String, cx: &mut Context<Self>) {
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let stats =
                    smol::unblock(move || crate::workspace::GitDiffStats::from_cwd(&cwd)).await;
                let _ = cx.update(|cx| {
                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                        if let Some(ws) = app.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                            && ws.git_stats != stats
                        {
                            ws.git_stats = stats;
                            cx.notify();
                        }
                    })
                });
            },
        )
        .detach();
    }
}
