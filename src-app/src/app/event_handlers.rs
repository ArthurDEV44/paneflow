//! Event-subscription callbacks and background workers for `PaneFlowApp`.
//!
//! Hosts the GPUI `subscribe` handlers (`handle_title_bar_event`,
//! `handle_pane_event`, `handle_terminal_event`) plus the port-scan /
//! loader-animation / stale-PID-sweep workers and the CWD change handler.
//!
//! Extracted from `main.rs` per US-026 of the src-app refactor PRD — pure
//! code-motion, behaviour unchanged.

use alacritty_terminal::grid::Dimensions;
use gpui::{App, AppContext, Context, Entity, Window};
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
            pane::PaneEvent::ToggleAgentSessions => {
                // Toggle: clicking the icon again with the sidebar open closes it.
                if self.sessions_sidebar_open {
                    self.close_sessions_sidebar(cx);
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

                // Mutual exclusion: only one right column. Opening sessions
                // closes the Files sidebar (and vice-versa, in
                // `toggle_files_sidebar`).
                if self.files_sidebar_open {
                    self.close_files_sidebar(cx);
                }

                // Close the floating dropdowns so they don't paint over the
                // newly opened sidebar (the sidebar itself is docked, not an
                // overlay, so it does not need mutual exclusion with itself).
                self.workspace_menu_open = None;
                self.profile_menu_open = None;

                self.sessions_sidebar_open = true;
                self.claude_sessions_cwd = cwd_str.clone();
                self.claude_sessions_pane = Some(pane.downgrade());
                self.claude_sessions.clear();
                self.codex_sessions.clear();
                self.opencode_sessions.clear();
                // Fresh per-group state for this open: all expanded, capped at 5,
                // not-yet-scanning (each spawned scan flips its own flag below).
                self.sessions_group_collapsed = [false; 3];
                self.sessions_group_show_all = [false; 3];
                self.sessions_scanning = [false; 3];
                let enabled_agents = crate::agent_sessions::enabled_session_agents();
                // Fresh handle so a previous scroll offset doesn't bleed into
                // the new sidebar.
                self.claude_sessions_scroll = gpui::ScrollHandle::new();

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
                        let idx = crate::app::sessions_sidebar::agent_index(
                            crate::agent_sessions::SessionAgent::Claude,
                        );
                        self.sessions_scanning[idx] = true;
                        let claude_cwd_scan = cwd.clone();
                        let claude_cwd_match = cwd.clone();
                        cx.spawn(async move |this, cx| {
                            let sessions = smol::unblock(move || {
                                crate::claude_sessions::read_sessions_for_cwd(&claude_cwd_scan)
                            })
                            .await;
                            let _ = this.update(cx, |app, cx| {
                                if app.sessions_sidebar_open
                                    && app.claude_sessions_cwd.as_deref()
                                        == Some(claude_cwd_match.as_str())
                                {
                                    app.claude_sessions = sessions;
                                    app.sessions_scanning[idx] = false;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }

                    if scan_codex {
                        let idx = crate::app::sessions_sidebar::agent_index(
                            crate::agent_sessions::SessionAgent::Codex,
                        );
                        self.sessions_scanning[idx] = true;
                        let codex_cwd_scan = cwd.clone();
                        let codex_cwd_match = cwd.clone();
                        cx.spawn(async move |this, cx| {
                            let sessions = smol::unblock(move || {
                                crate::codex_sessions::read_sessions_for_cwd(&codex_cwd_scan)
                            })
                            .await;
                            let _ = this.update(cx, |app, cx| {
                                if app.sessions_sidebar_open
                                    && app.claude_sessions_cwd.as_deref()
                                        == Some(codex_cwd_match.as_str())
                                {
                                    app.codex_sessions = sessions;
                                    app.sessions_scanning[idx] = false;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }

                    if scan_opencode {
                        let idx = crate::app::sessions_sidebar::agent_index(
                            crate::agent_sessions::SessionAgent::OpenCode,
                        );
                        self.sessions_scanning[idx] = true;
                        let opencode_cwd_scan = cwd.clone();
                        let opencode_cwd_match = cwd;
                        cx.spawn(async move |this, cx| {
                            let sessions = smol::unblock(move || {
                                crate::opencode_sessions::read_sessions_for_cwd(&opencode_cwd_scan)
                            })
                            .await;
                            let _ = this.update(cx, |app, cx| {
                                if app.sessions_sidebar_open
                                    && app.claude_sessions_cwd.as_deref()
                                        == Some(opencode_cwd_match.as_str())
                                {
                                    app.opencode_sessions = sessions;
                                    app.sessions_scanning[idx] = false;
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }
                }
                cx.notify();
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
                use crate::pane_drag::DropEdge;
                let edge = *edge;
                let source_idx = *source_idx;
                let duplicate = *duplicate;
                let source_pane = source_pane.clone();
                let target = &pane; // the emitting pane is the split target

                const MAX_PANES: usize = 32;

                // Resolve the workspace owning the target pane.
                let Some(ws_idx) = self.workspaces.iter().position(|ws| {
                    ws.root
                        .as_ref()
                        .is_some_and(|r| r.collect_leaves().contains(target))
                }) else {
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

                // `split_at_pane` always inserts the new pane *after* the
                // target, so the leading edges (Up/Left) additionally swap so
                // the moved/duplicated pane ends up on the correct side.
                let (direction, swap) = match edge {
                    DropEdge::Up => (crate::layout::SplitDirection::Horizontal, true),
                    DropEdge::Down => (crate::layout::SplitDirection::Horizontal, false),
                    DropEdge::Left => (crate::layout::SplitDirection::Vertical, true),
                    DropEdge::Right => (crate::layout::SplitDirection::Vertical, false),
                };
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
                let Some(ws_idx) = self.workspaces.iter().position(|ws| {
                    ws.root
                        .as_ref()
                        .is_some_and(|r| r.collect_leaves().contains(&dest))
                }) else {
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
                use crate::pane_drag::DropEdge;
                let edge = *edge;
                let agent = *agent;
                let session_id = session_id.clone();
                let cwd = cwd.clone();
                let target = pane.clone(); // the emitting pane is the target

                const MAX_PANES: usize = 32;

                let Some(ws_idx) = self.workspaces.iter().position(|ws| {
                    ws.root
                        .as_ref()
                        .is_some_and(|r| r.collect_leaves().contains(&target))
                }) else {
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
                // Claude bypass flag exactly like a tab-bar launch.
                let resume = crate::app::sessions_sidebar::resume_command(agent, &session_id);
                term.read(cx).send_command(&resume);

                match edge {
                    Some(edge) => {
                        // `create_pane` wires the app-level CWD/port subscription
                        // and the pane-event subscription (mirrors `DropSplit`).
                        let new_pane = self.create_pane(term, ws_id, cx);
                        let (direction, swap) = match edge {
                            DropEdge::Up => (crate::layout::SplitDirection::Horizontal, true),
                            DropEdge::Down => (crate::layout::SplitDirection::Horizontal, false),
                            DropEdge::Left => (crate::layout::SplitDirection::Vertical, true),
                            DropEdge::Right => (crate::layout::SplitDirection::Vertical, false),
                        };
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
                use crate::pane_drag::DropEdge;
                let edge = *edge;
                let path = path.clone();
                let target = pane.clone(); // the emitting pane is the target

                const MAX_PANES: usize = 32;

                let Some(ws_idx) = self.workspaces.iter().position(|ws| {
                    ws.root
                        .as_ref()
                        .is_some_and(|r| r.collect_leaves().contains(&target))
                }) else {
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
                        let (direction, swap) = match edge {
                            DropEdge::Up => (crate::layout::SplitDirection::Horizontal, true),
                            DropEdge::Down => (crate::layout::SplitDirection::Horizontal, false),
                            DropEdge::Left => (crate::layout::SplitDirection::Vertical, true),
                            DropEdge::Right => (crate::layout::SplitDirection::Vertical, false),
                        };
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

    /// US-003 (prd-multi-worktree-diff) — action handler: open the
    /// multi-worktree diff view for the *active* workspace's repo. A no-op
    /// when the active workspace has no resolved `repo_root` (not a git repo).
    pub(crate) fn handle_open_multi_diff(
        &mut self,
        _: &crate::app::actions::OpenMultiDiff,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo_root) = self
            .workspaces
            .get(self.active_idx)
            .and_then(|ws| ws.repo_root.clone())
        else {
            return;
        };
        self.open_multi_diff_for_repo(repo_root, window, cx);
    }

    /// Open a `DiffView` tab seeded with every sibling worktree sharing
    /// `repo_root`. The tab is hosted in the active workspace's focused pane
    /// (falling back to its first leaf); the diff content itself is
    /// repo-scoped, independent of which pane hosts it. Ephemeral — not
    /// persisted to the session. EP-002+ fills the seeded worktrees with
    /// real diff columns.
    /// Gather the sibling-worktree seed for a repo: one [`crate::diff::DiffWorktree`]
    /// per open workspace whose `repo_root` matches. US-005 of
    /// prd-git-diff-mode-2026-Q3.md extracted this from `open_multi_diff_for_repo`
    /// so the dedicated Diff mode (`rebuild_diff_view`) and the legacy tab path
    /// share one source of truth. Pure in-memory read — no git subprocess, safe
    /// to call on the main thread.
    pub(crate) fn collect_diff_worktrees(
        &self,
        repo_root: &std::path::Path,
    ) -> Vec<crate::diff::DiffWorktree> {
        self.workspaces
            .iter()
            .filter(|ws| ws.repo_root.as_deref() == Some(repo_root))
            .map(|ws| crate::diff::DiffWorktree {
                path: std::path::PathBuf::from(&ws.cwd),
                branch: ws.git_branch.clone(),
                workspace_id: Some(ws.id),
            })
            .collect()
    }

    /// US-011: the active workspace as a single-element worktree seed (Project
    /// scope). Empty when there is no active workspace. Pure in-memory read.
    pub(crate) fn collect_project_worktrees(&self) -> Vec<crate::diff::DiffWorktree> {
        self.workspaces
            .get(self.active_idx)
            .map(|ws| {
                vec![crate::diff::DiffWorktree {
                    path: std::path::PathBuf::from(&ws.cwd),
                    branch: ws.git_branch.clone(),
                    workspace_id: Some(ws.id),
                }]
            })
            .unwrap_or_default()
    }

    /// US-014: every open workspace grouped by canonicalized `repo_root`
    /// (Multi-project scope). `BTreeMap` keying gives stable repo ordering;
    /// workspaces with no resolved repo are skipped. Pure in-memory read.
    pub(crate) fn collect_multiproject_groups(&self) -> Vec<crate::diff::RepoGroup> {
        use std::collections::BTreeMap;
        let mut map: BTreeMap<std::path::PathBuf, crate::diff::RepoGroup> = BTreeMap::new();
        for ws in &self.workspaces {
            let Some(root) = ws.repo_root.clone() else {
                continue;
            };
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            map.entry(root.clone())
                .or_insert_with(|| crate::diff::RepoGroup {
                    repo_root: root.clone(),
                    repo_name: name,
                    worktrees: Vec::new(),
                })
                .worktrees
                .push(crate::diff::DiffWorktree {
                    path: std::path::PathBuf::from(&ws.cwd),
                    branch: ws.git_branch.clone(),
                    workspace_id: Some(ws.id),
                });
        }
        map.into_values().collect()
    }

    pub(crate) fn open_multi_diff_for_repo(
        &mut self,
        repo_root: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Gather sibling worktrees across all workspaces sharing this repo.
        let worktrees = self.collect_diff_worktrees(&repo_root);

        // Resolve a host pane from the active workspace (focused, else first leaf).
        let target_pane = {
            let Some(ws) = self.workspaces.get(self.active_idx) else {
                return;
            };
            let Some(root) = ws.root.as_ref() else {
                return;
            };
            root.focused_pane(window, cx)
                .or_else(|| root.collect_leaves().into_iter().next())
        };
        let Some(target_pane) = target_pane else {
            return;
        };

        let diff = cx.new(|cx| crate::diff::DiffView::new(repo_root, worktrees, cx));
        target_pane.update(cx, |pane, cx| {
            pane.add_diff_tab(diff, cx);
            cx.notify();
        });
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
