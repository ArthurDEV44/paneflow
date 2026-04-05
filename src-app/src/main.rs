//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar workspace list + main content area.

mod assets;
mod ipc;
mod keys;
mod pane;
mod split;
mod terminal;
mod terminal_element;
pub mod theme;
mod title_bar;
mod workspace;

use gpui::{
    App, Bounds, ClickEvent, Context, CursorStyle, Decorations, Entity, Focusable, HitboxBehavior,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, MouseButton, PathPromptOptions,
    Pixels, Point, Render, ResizeEdge, SharedString, Size, Styled, Window, WindowBounds,
    WindowDecorations, WindowOptions, actions, canvas, div, point, prelude::*, px, rgb, size, svg,
    transparent_black,
};
use gpui_platform::application;
use notify::Watcher;

use std::collections::VecDeque;

use paneflow_config::schema::LayoutNode;

use crate::pane::Pane;
use crate::split::{FocusDirection, LayoutTree, SplitDirection};
use crate::terminal::TerminalView;
use crate::workspace::Workspace;

/// Write text to the first leaf pane's active terminal PTY in a layout tree.
fn send_text_to_first_leaf(node: &LayoutTree, text: &str, cx: &App) {
    match node {
        LayoutTree::Leaf(pane) => {
            pane.read(cx)
                .active_terminal()
                .read(cx)
                .terminal
                .write_to_pty(text.as_bytes().to_vec());
        }
        LayoutTree::Container { children, .. } => {
            if let Some(first) = children.first() {
                send_text_to_first_leaf(&first.node, text, cx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

actions!(
    paneflow,
    [
        SplitHorizontally,
        SplitVertically,
        ClosePane,
        NewTab,
        CloseTab,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        NewWorkspace,
        CloseWorkspace,
        NextWorkspace,
        SelectWorkspace1,
        SelectWorkspace2,
        SelectWorkspace3,
        SelectWorkspace4,
        SelectWorkspace5,
        SelectWorkspace6,
        SelectWorkspace7,
        SelectWorkspace8,
        SelectWorkspace9,
        TerminalCopy,
        TerminalPaste,
        ScrollPageUp,
        ScrollPageDown,
        CloseWindow,
        ToggleZoom,
        LayoutEvenHorizontal,
        LayoutEvenVertical,
        LayoutMainVertical,
        LayoutTiled
    ]
);

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

/// Sidebar width in pixels — shared between sidebar and title bar for alignment.
const SIDEBAR_WIDTH: f32 = 240.;

struct PaneFlowApp {
    workspaces: Vec<Workspace>,
    active_idx: usize,
    renaming_idx: Option<usize>,
    rename_text: String,
    last_config_mtime: Option<std::time::SystemTime>,
    ipc_rx: std::sync::mpsc::Receiver<ipc::IpcRequest>,
    title_bar: Entity<title_bar::TitleBar>,
    /// File watcher for `.git/HEAD` and `.git/index` across all workspaces.
    /// `None` if the OS watcher could not be created (graceful degradation).
    git_watcher: Option<notify::RecommendedWatcher>,
    /// Receiver for raw notify events from the git file watcher.
    git_event_rx: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    /// Refcount for watched `.git` directories (multiple workspaces may share a repo).
    git_watch_counts: std::collections::HashMap<std::path::PathBuf, usize>,
}

impl PaneFlowApp {
    /// Add a workspace's `.git` directory to the file watcher.
    /// Uses refcounting so multiple workspaces sharing a repo don't conflict.
    /// Silently skipped if the workspace is not in a git repo or watcher is unavailable.
    fn watch_git_dir(&mut self, ws: &Workspace) {
        if let Some(ref git_dir) = ws.git_dir {
            let count = self.git_watch_counts.entry(git_dir.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                // First workspace watching this git dir — register with OS
                if let Some(ref mut watcher) = self.git_watcher {
                    if let Err(e) = watcher.watch(git_dir, notify::RecursiveMode::NonRecursive) {
                        log::warn!("git watcher: failed to watch {}: {e}", git_dir.display());
                    }
                }
            }
        }
    }

    /// Remove a workspace's `.git` directory from the file watcher.
    /// Only unwatches when the last workspace using this git dir is removed.
    fn unwatch_git_dir(&mut self, git_dir: &std::path::Path) {
        if let Some(count) = self.git_watch_counts.get_mut(git_dir) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.git_watch_counts.remove(git_dir);
                if let Some(ref mut watcher) = self.git_watcher {
                    let _ = watcher.unwatch(git_dir);
                }
            }
        }
    }

    /// Create a new pane wrapping a terminal, and subscribe to its events.
    /// When the pane emits `PaneEvent::Remove` (last tab closed), the pane
    /// is removed from the split tree — following Zed's EventEmitter pattern.
    fn create_pane(
        &mut self,
        terminal: Entity<TerminalView>,
        cx: &mut Context<Self>,
    ) -> Entity<Pane> {
        let pane = cx.new(|cx| Pane::new(terminal, cx));
        cx.subscribe(&pane, Self::handle_pane_event).detach();
        pane
    }

    fn handle_pane_event(
        &mut self,
        pane: Entity<Pane>,
        event: &pane::PaneEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            pane::PaneEvent::Remove => {
                // Remove this pane from the split tree of the active workspace
                if let Some(ws) = self.active_workspace_mut()
                    && let Some(root) = ws.root.take()
                {
                    ws.root = root.remove_pane(&pane);
                }
                // Safety: never leave a workspace without a pane
                if let Some(ws) = self.active_workspace()
                    && ws.root.is_none()
                {
                    let terminal = cx.new(TerminalView::new);
                    let new_pane = self.create_pane(terminal, cx);
                    if let Some(ws) = self.active_workspace_mut() {
                        ws.root = Some(LayoutTree::Leaf(new_pane));
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
                let new_terminal = cx.new(TerminalView::new);
                let new_pane = self.create_pane(new_terminal, cx);
                if let Some(ws) = self.active_workspace_mut()
                    && let Some(root) = &mut ws.root
                {
                    root.split_at_pane(&pane, direction, new_pane);
                }
                cx.notify();
            }
        }
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(TerminalView::new);
        let pane = cx.new(|cx| Pane::new(terminal, cx));
        cx.subscribe(&pane, Self::handle_pane_event).detach();
        let dir_name = std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "Terminal 1".into());
        let ws = Workspace::new(dir_name, pane);
        let title_bar = cx.new(title_bar::TitleBar::new);
        let last_config_mtime = crate::theme::config_mtime();
        let ipc_rx = ipc::start_server();

        // Setup notify file watcher for .git directories
        let (git_event_tx, git_event_rx) = std::sync::mpsc::channel();
        let mut git_watcher = match notify::recommended_watcher(git_event_tx) {
            Ok(w) => Some(w),
            Err(e) => {
                log::warn!("git file watcher unavailable: {e}. Falling back to polling.");
                None
            }
        };
        let mut git_watch_counts = std::collections::HashMap::new();
        // Watch the initial workspace's .git directory
        if let Some(ref mut watcher) = git_watcher {
            if let Some(ref git_dir) = ws.git_dir {
                if let Err(e) = watcher.watch(git_dir, notify::RecursiveMode::NonRecursive) {
                    log::warn!("git watcher: failed to watch {}: {e}", git_dir.display());
                } else {
                    git_watch_counts.insert(git_dir.clone(), 1);
                }
            }
        }

        // Poll git watcher events with 300ms debounce.
        // Filter: only HEAD and index matter. NonRecursive mode limits events to
        // top-level entries of .git/ so no subdirectory false positives.
        // On debounce fire, run git probes off main thread and apply results.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let debounce = std::time::Duration::from_millis(300);
                let mut last_event = std::time::Instant::now() - debounce;
                let mut pending = false;
                let mut pending_git_dirs = std::collections::HashSet::<std::path::PathBuf>::new();

                loop {
                    smol::Timer::after(std::time::Duration::from_millis(50)).await;

                    // Drain events from the watcher channel, collect affected .git dirs
                    let new_dirs = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, _cx: &mut Context<Self>| {
                            let mut dirs = Vec::new();
                            while let Ok(event) = app.git_event_rx.try_recv() {
                                if let Ok(ref ev) = event {
                                    for p in &ev.paths {
                                        if matches!(
                                            p.file_name().and_then(|n| n.to_str()),
                                            Some("HEAD" | "index")
                                        ) && let Some(parent) = p.parent()
                                        {
                                            dirs.push(parent.to_path_buf());
                                        }
                                    }
                                }
                            }
                            dirs
                        })
                    });

                    match new_dirs {
                        Ok(dirs) if !dirs.is_empty() => {
                            pending_git_dirs.extend(dirs);
                            last_event = std::time::Instant::now();
                            pending = true;
                        }
                        Ok(_) => {}
                        Err(_) => break, // app shutting down
                    }

                    // Debounce: fire after 300ms of quiet
                    if pending && last_event.elapsed() >= debounce {
                        pending = false;
                        let affected_dirs = std::mem::take(&mut pending_git_dirs);
                        log::debug!(
                            "git watcher: debounced event fired for {} dir(s)",
                            affected_dirs.len()
                        );

                        // Collect CWDs of affected workspaces (main thread)
                        let cwds = cx.update(|cx| {
                            this.update(cx, |app: &mut Self, _cx: &mut Context<Self>| {
                                app.workspaces
                                    .iter()
                                    .filter(|ws| {
                                        ws.git_dir
                                            .as_ref()
                                            .is_some_and(|gd| affected_dirs.contains(gd))
                                    })
                                    .map(|ws| ws.cwd.clone())
                                    .collect::<Vec<String>>()
                            })
                        });

                        let cwds = match cwds {
                            Ok(c) => c,
                            Err(_) => break,
                        };

                        if cwds.is_empty() {
                            continue;
                        }

                        // Run git probes off main thread
                        let results = smol::unblock(move || {
                            cwds.into_iter()
                                .map(|cwd| {
                                    let (branch, is_repo) = crate::workspace::detect_branch(&cwd);
                                    let stats = crate::workspace::GitDiffStats::from_cwd(&cwd);
                                    (cwd, branch, is_repo, stats)
                                })
                                .collect::<Vec<_>>()
                        })
                        .await;

                        // Apply results to matching workspaces (main thread)
                        let apply = cx.update(|cx| {
                            this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                                let mut changed = false;
                                for (cwd, branch, is_repo, stats) in &results {
                                    for ws in &mut app.workspaces {
                                        if ws.cwd != *cwd {
                                            continue;
                                        }
                                        if ws.git_branch != *branch || ws.is_git_repo != *is_repo {
                                            ws.git_branch = branch.clone();
                                            ws.is_git_repo = *is_repo;
                                            changed = true;
                                        }
                                        if ws.git_stats != *stats {
                                            ws.git_stats = stats.clone();
                                            changed = true;
                                        }
                                    }
                                }
                                if changed {
                                    cx.notify();
                                }
                            })
                        });
                        if apply.is_err() {
                            break;
                        }
                    }
                }
            },
        )
        .detach();

        // Poll IPC requests every 10ms
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(10)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            app.process_ipc_requests(cx);
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // Poll config file for theme changes every 500ms
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(500)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let current_mtime = crate::theme::config_mtime();
                            if current_mtime != app.last_config_mtime {
                                app.last_config_mtime = current_mtime;
                                cx.notify(); // Trigger repaint with new theme
                            }
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // Fallback: poll git metadata for all workspaces every 30s.
        // Primary detection is event-driven (US-003 notify watcher above).
        // This timer catches edge cases where file system events are missed.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(30)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let mut changed = false;
                            for ws in &mut app.workspaces {
                                let new_stats = crate::workspace::GitDiffStats::from_cwd(&ws.cwd);
                                if new_stats != ws.git_stats {
                                    ws.git_stats = new_stats;
                                    changed = true;
                                }
                                let (new_branch, new_is_repo) =
                                    crate::workspace::detect_branch(&ws.cwd);
                                if new_branch != ws.git_branch || new_is_repo != ws.is_git_repo {
                                    ws.git_branch = new_branch;
                                    ws.is_git_repo = new_is_repo;
                                    changed = true;
                                }
                            }
                            if changed {
                                cx.notify();
                            }
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // Port scan loop: debounce terminal Wakeup events (500ms), then burst
        // scan at [1s, 3s, 7s] offsets. Detects servers binding ports.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);
                const BURST_OFFSETS: [std::time::Duration; 3] = [
                    std::time::Duration::from_secs(1),
                    std::time::Duration::from_secs(3),
                    std::time::Duration::from_secs(7),
                ];

                loop {
                    smol::Timer::after(std::time::Duration::from_millis(100)).await;

                    // Phase 1: detect terminal output, manage debounce/burst, collect PIDs
                    let to_scan = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let now = std::time::Instant::now();
                            let mut scans = Vec::new();

                            for ws in &mut app.workspaces {
                                // Sum wakeup counts for all terminals in workspace
                                let mut current_sum = 0u64;
                                if let Some(root) = &ws.root {
                                    for pane in root.collect_leaves() {
                                        for tv in &pane.read(cx).tabs {
                                            current_sum += tv.read(cx).terminal.wakeup_count;
                                        }
                                    }
                                }

                                // Detect new terminal output (only schedule if
                                // no burst is already in progress)
                                if current_sum != ws.last_wakeup_sum {
                                    ws.last_wakeup_sum = current_sum;
                                    if ws.port_scan_burst_start.is_none() {
                                        ws.port_scan_last_output = Some(now);
                                        ws.port_scan_pending = true;
                                    }
                                }

                                // Debounce: fire after 500ms of quiet
                                if ws.port_scan_pending
                                    && let Some(last) = ws.port_scan_last_output
                                    && now.duration_since(last) >= DEBOUNCE
                                {
                                    ws.port_scan_pending = false;
                                    ws.port_scan_burst_start = Some(now);
                                    ws.port_scan_burst_idx = 0;
                                    log::debug!("port scan: debounce fired for '{}'", ws.title);
                                }

                                // Burst scan: fire all overdue scans (catch up
                                // if the poll loop was delayed)
                                if let Some(burst_start) = ws.port_scan_burst_start {
                                    let mut fired = false;
                                    while ws.port_scan_burst_idx < BURST_OFFSETS.len() {
                                        let offset = BURST_OFFSETS[ws.port_scan_burst_idx];
                                        if now.duration_since(burst_start) < offset {
                                            break;
                                        }
                                        ws.port_scan_burst_idx += 1;
                                        fired = true;
                                    }
                                    if fired {
                                        // Collect PIDs from all terminals
                                        let mut pids = Vec::new();
                                        if let Some(root) = &ws.root {
                                            for pane in root.collect_leaves() {
                                                for tv in &pane.read(cx).tabs {
                                                    pids.push(tv.read(cx).terminal.child_pid);
                                                }
                                            }
                                        }
                                        scans.push((ws.id, pids));
                                    }
                                    if ws.port_scan_burst_idx >= BURST_OFFSETS.len() {
                                        ws.port_scan_burst_start = None;
                                    }
                                }
                            }

                            scans
                        })
                    });

                    let to_scan = match to_scan {
                        Ok(s) if !s.is_empty() => s,
                        Ok(_) => continue,
                        Err(_) => break,
                    };

                    // Phase 2: run port detection off main thread
                    let results = smol::unblock(move || {
                        to_scan
                            .into_iter()
                            .map(|(ws_id, pids)| {
                                let ports = crate::workspace::detect_ports(&pids);
                                (ws_id, ports)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;

                    // Phase 3: apply results if ports changed (match by workspace ID)
                    let apply = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let mut changed = false;
                            for (ws_id, ports) in &results {
                                for ws in &mut app.workspaces {
                                    if ws.id == *ws_id && ws.active_ports != *ports {
                                        ws.active_ports = ports.clone();
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                cx.notify();
                            }
                        })
                    });
                    if apply.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        Self {
            workspaces: vec![ws],
            active_idx: 0,
            renaming_idx: None,
            rename_text: String::new(),
            last_config_mtime,
            ipc_rx,
            title_bar,
            git_watcher,
            git_event_rx,
            git_watch_counts,
        }
    }

    fn process_ipc_requests(&mut self, cx: &mut Context<Self>) {
        while let Ok(req) = self.ipc_rx.try_recv() {
            let result = self.handle_ipc(&req.method, &req.params, cx);
            let _ = req.response_tx.send(result);
        }
    }

    fn handle_ipc(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> serde_json::Value {
        match method {
            "workspace.list" => {
                let list: Vec<_> = self
                    .workspaces
                    .iter()
                    .enumerate()
                    .map(|(i, ws)| {
                        serde_json::json!({
                            "index": i,
                            "title": ws.title,
                            "cwd": ws.cwd,
                            "panes": ws.pane_count(),
                            "active": i == self.active_idx,
                        })
                    })
                    .collect();
                serde_json::json!({"workspaces": list})
            }
            "workspace.current" => {
                if let Some(ws) = self.active_workspace() {
                    let layout = ws.serialize_layout(cx);
                    serde_json::json!({
                        "index": self.active_idx,
                        "title": ws.title,
                        "cwd": ws.cwd,
                        "panes": ws.pane_count(),
                        "layout": layout.map(|l| serde_json::to_value(l).ok()).flatten(),
                    })
                } else {
                    serde_json::json!(null)
                }
            }
            "workspace.create" => {
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("Terminal");
                let terminal = cx.new(TerminalView::new);
                let pane = self.create_pane(terminal, cx);
                let ws = Workspace::new(name, pane);
                self.watch_git_dir(&ws);
                self.workspaces.push(ws);
                let idx = self.workspaces.len() - 1;
                cx.notify();
                serde_json::json!({"index": idx, "title": name})
            }
            "workspace.select" => {
                let idx = params.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                if idx < self.workspaces.len() {
                    self.active_idx = idx;
                    cx.notify();
                    serde_json::json!({"selected": idx})
                } else {
                    serde_json::json!({"error": "Index out of bounds"})
                }
            }
            "workspace.close" => {
                if self.workspaces.len() <= 1 {
                    serde_json::json!({"error": "Cannot close last workspace"})
                } else {
                    let idx = params
                        .get("index")
                        .and_then(|i| i.as_u64())
                        .map(|i| i as usize)
                        .unwrap_or(self.active_idx);
                    if idx < self.workspaces.len() {
                        if let Some(dir) = self.workspaces[idx].git_dir.clone() {
                            self.unwatch_git_dir(&dir);
                        }
                        self.workspaces.remove(idx);
                        if self.active_idx >= self.workspaces.len() {
                            self.active_idx = self.workspaces.len() - 1;
                        }
                        cx.notify();
                        serde_json::json!({"closed": idx})
                    } else {
                        serde_json::json!({"error": "Index out of bounds"})
                    }
                }
            }
            "surface.list" => {
                let count = self.active_workspace().map_or(0, |ws| ws.pane_count());
                serde_json::json!({"pane_count": count, "workspace": self.active_idx})
            }
            "surface.send_text" => {
                let text = params.get("text").and_then(|t| t.as_str()).unwrap_or("");
                if text.is_empty() {
                    return serde_json::json!({"error": "Missing 'text' parameter"});
                }
                const MAX_TEXT_LEN: usize = 64 * 1024; // 64 KiB
                if text.len() > MAX_TEXT_LEN {
                    return serde_json::json!({"error": "Text exceeds 64 KiB limit"});
                }
                // Write to the focused terminal's PTY in the active workspace
                if let Some(ws) = self.active_workspace()
                    && let Some(root) = &ws.root
                {
                    send_text_to_first_leaf(root, text, cx);
                    return serde_json::json!({"sent": true, "length": text.len()});
                }
                serde_json::json!({"error": "No active terminal"})
            }
            "surface.split" => {
                let dir_str = params
                    .get("direction")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                let direction = match dir_str {
                    "horizontal" => SplitDirection::Horizontal,
                    "vertical" => SplitDirection::Vertical,
                    _ => {
                        return serde_json::json!({"error": "Missing or invalid 'direction' parameter (use \"horizontal\" or \"vertical\")"});
                    }
                };
                const MAX_PANES: usize = 32;
                let Some(ws) = self.active_workspace() else {
                    return serde_json::json!({"error": "No active workspace"});
                };
                let Some(root) = &ws.root else {
                    return serde_json::json!({"error": "No active workspace"});
                };
                if root.leaf_count() >= MAX_PANES {
                    return serde_json::json!({"error": "Maximum pane count reached"});
                }
                let new_terminal = cx.new(TerminalView::new);
                let new_pane = self.create_pane(new_terminal, cx);
                let Some(ws) = self.active_workspace_mut() else {
                    return serde_json::json!({"error": "No active workspace"});
                };
                let Some(root) = ws.root.as_mut() else {
                    return serde_json::json!({"error": "Workspace has no root"});
                };
                root.split_first_leaf(direction, new_pane);
                let panes = ws.pane_count();
                cx.notify();
                serde_json::json!({"split": true, "direction": dir_str, "panes": panes})
            }
            "workspace.restore_layout" => {
                let Some(layout_value) = params.get("layout") else {
                    return serde_json::json!({"error": "Missing 'layout' parameter"});
                };
                let mut layout: LayoutNode = match serde_json::from_value(layout_value.clone()) {
                    Ok(l) => l,
                    Err(e) => {
                        return serde_json::json!({"error": format!("Invalid layout JSON: {e}")});
                    }
                };
                match self.apply_layout_from_json(&mut layout, cx) {
                    Ok(()) => {
                        let panes = self.active_workspace().map_or(0, |ws| ws.pane_count());
                        serde_json::json!({"restored": true, "panes": panes})
                    }
                    Err(e) => serde_json::json!({"error": e}),
                }
            }
            _ => {
                serde_json::json!({"error": format!("Unknown method: {method}")})
            }
        }
    }

    fn active_workspace(&self) -> Option<&Workspace> {
        debug_assert!(
            self.workspaces.is_empty() || self.active_idx < self.workspaces.len(),
            "active_idx out of bounds"
        );
        self.workspaces.get(self.active_idx)
    }

    fn active_workspace_mut(&mut self) -> Option<&mut Workspace> {
        self.workspaces.get_mut(self.active_idx)
    }

    fn select_workspace(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx < self.workspaces.len() && idx != self.active_idx {
            self.active_idx = idx;
            self.workspaces[idx].focus_first(window, cx);
            cx.notify();
        }
    }

    #[allow(dead_code)]
    fn create_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const MAX_WORKSPACES: usize = 20;
        if self.workspaces.len() >= MAX_WORKSPACES {
            return;
        }
        let n = self.workspaces.len() + 1;
        let terminal = cx.new(TerminalView::new);
        let pane = self.create_pane(terminal, cx);
        let ws = Workspace::new(format!("Terminal {n}"), pane);
        self.watch_git_dir(&ws);
        self.workspaces.push(ws);
        self.active_idx = self.workspaces.len() - 1;
        self.workspaces[self.active_idx].focus_first(window, cx);
        cx.notify();
    }

    fn create_workspace_with_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        const MAX_WORKSPACES: usize = 20;
        if self.workspaces.len() >= MAX_WORKSPACES {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                if let Ok(Ok(Some(paths))) = receiver.await {
                    if let Some(path) = paths.into_iter().next() {
                        let _ = cx.update(|cx| {
                            this.update(cx, |app, cx| {
                                let n = app.workspaces.len() + 1;
                                let dir = path.clone();
                                let title = dir
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| format!("Terminal {n}"));
                                let terminal = cx.new(|cx| TerminalView::with_cwd(Some(path), cx));
                                let pane = app.create_pane(terminal, cx);
                                let ws = Workspace::with_cwd(title, dir, pane);
                                app.watch_git_dir(&ws);
                                app.workspaces.push(ws);
                                app.active_idx = app.workspaces.len() - 1;
                                // Cannot call focus_first here (no Window ref in async),
                                // but cx.notify() triggers repaint which selects the workspace.
                                cx.notify();
                            })
                        });
                    }
                }
            },
        )
        .detach();
    }

    // --- Split/close/focus handlers (operate on active workspace) ---

    fn split(&mut self, direction: SplitDirection, window: &mut Window, cx: &mut Context<Self>) {
        // No split while zoomed — the zoomed view is a temporary single-leaf root
        if let Some(ws) = self.active_workspace()
            && ws.is_zoomed()
        {
            return;
        }
        const MAX_PANES: usize = 32;
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && root.leaf_count() >= MAX_PANES
        {
            return;
        }
        let new_terminal = cx.new(TerminalView::new);
        let new_pane = self.create_pane(new_terminal, cx);
        if let Some(ws) = self.active_workspace_mut()
            && let Some(root) = &mut ws.root
            && root.split_at_focused(direction, new_pane.clone(), window, cx)
        {
            new_pane.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn handle_split_h(&mut self, _: &SplitHorizontally, w: &mut Window, cx: &mut Context<Self>) {
        self.split(SplitDirection::Horizontal, w, cx);
    }
    fn handle_split_v(&mut self, _: &SplitVertically, w: &mut Window, cx: &mut Context<Self>) {
        self.split(SplitDirection::Vertical, w, cx);
    }

    fn handle_close_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ws) = self.active_workspace_mut()
            && ws.is_zoomed()
        {
            // Close-while-zoomed: exit zoom first, then remove the pane from the
            // restored layout. This prevents orphan pane references in saved_layout.
            let zoomed_pane = ws.root.as_ref().and_then(|r| r.first_leaf());
            if let Some(saved) = ws.saved_layout.take() {
                ws.root = Some(saved);
                if let Some(pane) = zoomed_pane
                    && let Some(root) = ws.root.take()
                {
                    ws.root = root.remove_pane(&pane);
                }
                // Focus the next available pane
                if let Some(ref root) = ws.root {
                    root.focus_first(window, cx);
                }
            }
        } else if let Some(ws) = self.active_workspace_mut()
            && let Some(root) = ws.root.take()
        {
            let (new_root, _closed, focus_target) = root.close_focused(window, cx);
            ws.root = new_root;

            if ws.root.is_some() {
                if let Some(target) = focus_target {
                    target.read(cx).focus_handle(cx).focus(window, cx);
                } else if let Some(ref root) = ws.root {
                    root.focus_first(window, cx);
                }
            }
        }

        // Destroy workspace if its root is now empty (last pane was closed)
        if let Some(ws) = self.active_workspace()
            && ws.root.is_none()
        {
            if self.workspaces.len() > 1 {
                self.close_workspace_at(self.active_idx, window, cx);
            } else {
                // Last workspace: spawn a fresh pane instead of destroying
                let terminal = cx.new(TerminalView::new);
                let new_pane = self.create_pane(terminal, cx);
                if let Some(ws) = self.active_workspace_mut() {
                    ws.root = Some(LayoutTree::Leaf(new_pane));
                }
                self.workspaces[self.active_idx].focus_first(window, cx);
            }
        }

        cx.notify();
    }

    fn handle_toggle_zoom(&mut self, _: &ToggleZoom, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws) = self.active_workspace_mut() else {
            return;
        };

        if ws.is_zoomed() {
            // Un-zoom: restore the saved layout
            let zoomed_pane = ws.root.as_ref().and_then(|r| r.first_leaf());
            if let Some(saved) = ws.saved_layout.take() {
                ws.root = Some(saved);
                if let Some(pane) = zoomed_pane {
                    pane.update(cx, |p, _| p.zoomed = false);
                    pane.read(cx).focus_handle(cx).focus(window, cx);
                }
            }
        } else {
            // Zoom: save the full tree, replace root with the focused pane
            let Some(root) = &ws.root else { return };

            if root.leaf_count() <= 1 {
                return;
            }

            let Some(focused) = root.focused_pane(window, cx) else {
                return;
            };

            focused.update(cx, |p, _| p.zoomed = true);
            let full_tree = ws.root.take().unwrap();
            ws.saved_layout = Some(full_tree);
            ws.root = Some(LayoutTree::Leaf(focused.clone()));
            focused.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Apply a layout preset: collect all panes, rebuild tree with the given factory.
    fn apply_layout_preset(
        &mut self,
        build: impl FnOnce(Vec<Entity<Pane>>) -> Option<LayoutTree>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Exit zoom if active
        if let Some(ws) = self.active_workspace_mut()
            && ws.is_zoomed()
        {
            if let Some(saved) = ws.saved_layout.take() {
                ws.root = Some(saved);
            }
        }

        let Some(ws) = self.active_workspace_mut() else {
            return;
        };
        let Some(root) = ws.root.take() else { return };
        let panes = root.collect_leaves();

        // No-op for single pane
        if panes.len() <= 1 {
            ws.root = Some(root);
            return;
        }

        // Rebuild tree and focus first pane
        // (root is consumed by collect_leaves moving entities out — but collect_leaves
        //  clones Entity refs, so root is still valid. We drop it explicitly.)
        drop(root);
        ws.root = build(panes);
        if let Some(ref r) = ws.root {
            r.focus_first(window, cx);
        }
        cx.notify();
    }

    /// Apply a layout from a `LayoutNode` (deserialized JSON) to the active workspace.
    ///
    /// Handles pane count mismatch: spawns new panes when the layout has more
    /// leaves than available, drops extras when fewer. Exits zoom first.
    fn apply_layout_from_json(
        &mut self,
        layout: &mut LayoutNode,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        const MAX_PANES: usize = 32;

        // Validate the layout (clamps ratios, pads children, etc.)
        paneflow_config::loader::validate_layout(layout);

        let needed = layout.leaf_count();
        if needed == 0 {
            return Err("Layout has no panes".into());
        }
        if needed > MAX_PANES {
            return Err(format!("Layout exceeds maximum pane count ({MAX_PANES})"));
        }

        // Exit zoom if active, clearing the zoomed flag on the pane
        if let Some(ws) = self.active_workspace_mut()
            && ws.is_zoomed()
        {
            let zoomed_pane = ws.root.as_ref().and_then(|r| r.first_leaf());
            if let Some(saved) = ws.saved_layout.take() {
                ws.root = Some(saved);
            }
            if let Some(pane) = zoomed_pane {
                pane.update(cx, |p, _| p.zoomed = false);
            }
        }

        let Some(ws) = self.active_workspace_mut() else {
            return Err("No active workspace".into());
        };

        // Collect existing panes and drop the old tree
        let existing: Vec<Entity<Pane>> = ws
            .root
            .take()
            .map(|r| r.collect_leaves())
            .unwrap_or_default();

        // Keep only the panes we need; extras are dropped with the old tree
        let mut pane_deque: VecDeque<Entity<Pane>> = existing.into_iter().take(needed).collect();

        let app_ref = &mut *self;
        let tree = LayoutTree::from_layout_node(layout, &mut pane_deque, &mut || {
            let terminal = cx.new(TerminalView::new);
            app_ref.create_pane(terminal, cx)
        });

        let Some(ws) = self.active_workspace_mut() else {
            return Err("No active workspace".into());
        };
        ws.root = Some(tree);
        cx.notify();
        Ok(())
    }

    fn handle_layout_even_h(
        &mut self,
        _: &LayoutEvenHorizontal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_preset(
            |panes| LayoutTree::from_panes_equal(SplitDirection::Vertical, panes),
            window,
            cx,
        );
    }

    fn handle_layout_even_v(
        &mut self,
        _: &LayoutEvenVertical,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_preset(
            |panes| LayoutTree::from_panes_equal(SplitDirection::Horizontal, panes),
            window,
            cx,
        );
    }

    fn handle_layout_main_v(
        &mut self,
        _: &LayoutMainVertical,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Exit zoom if active
        if let Some(ws) = self.active_workspace_mut()
            && ws.is_zoomed()
        {
            if let Some(saved) = ws.saved_layout.take() {
                ws.root = Some(saved);
            }
        }

        let Some(ws) = self.active_workspace() else {
            return;
        };
        let Some(root) = &ws.root else { return };

        if root.leaf_count() <= 1 {
            return;
        }

        // The main pane is the focused one, or the first leaf
        let main_pane = root
            .focused_pane(window, cx)
            .or_else(|| root.first_leaf())
            .unwrap();

        let panes = root.collect_leaves();
        let others: Vec<_> = panes.into_iter().filter(|p| *p != main_pane).collect();

        let ws = self.active_workspace_mut().unwrap();
        drop(ws.root.take());
        ws.root = LayoutTree::main_vertical(main_pane.clone(), others);
        main_pane.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn handle_layout_tiled(
        &mut self,
        _: &LayoutTiled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_preset(|panes| LayoutTree::tiled(panes), window, cx);
    }

    fn handle_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && let Some(pane) = root.focused_pane(window, cx)
        {
            let terminal = cx.new(TerminalView::new);
            pane.update(cx, |p, cx| {
                p.add_tab(terminal, cx);
            });
            pane.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }

    fn handle_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && let Some(pane) = root.focused_pane(window, cx)
        {
            // close_selected_tab emits PaneEvent::Remove if last tab,
            // which is handled by handle_pane_event via cx.subscribe.
            pane.update(cx, |p, cx| {
                p.close_selected_tab(cx);
            });
            // If pane still has tabs, refocus
            if !pane.read(cx).tabs.is_empty() {
                pane.read(cx).focus_handle(cx).focus(window, cx);
            } else if let Some(ws) = self.active_workspace()
                && let Some(root) = &ws.root
            {
                root.focus_first(window, cx);
            }
            cx.notify();
        }
    }

    fn handle_focus(&mut self, dir: FocusDirection, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
        {
            root.focus_in_direction(dir, window, cx);
        }
        cx.notify();
    }

    fn handle_focus_left(&mut self, _: &FocusLeft, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Left, w, cx);
    }
    fn handle_focus_right(&mut self, _: &FocusRight, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Right, w, cx);
    }
    fn handle_focus_up(&mut self, _: &FocusUp, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Up, w, cx);
    }
    fn handle_focus_down(&mut self, _: &FocusDown, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_focus(FocusDirection::Down, w, cx);
    }

    fn handle_new_workspace(&mut self, _: &NewWorkspace, w: &mut Window, cx: &mut Context<Self>) {
        self.create_workspace_with_picker(w, cx);
    }

    fn handle_close_workspace(
        &mut self,
        _: &CloseWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_workspace_at(self.active_idx, window, cx);
    }

    fn close_workspace_at(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Guard: don't close the last workspace
        if self.workspaces.len() <= 1 || idx >= self.workspaces.len() {
            return;
        }
        if let Some(dir) = self.workspaces[idx].git_dir.clone() {
            self.unwatch_git_dir(&dir);
        }
        self.workspaces.remove(idx);
        // Clamp active_idx
        if self.active_idx >= self.workspaces.len() {
            self.active_idx = self.workspaces.len() - 1;
        } else if self.active_idx > idx {
            self.active_idx -= 1;
        }
        self.workspaces[self.active_idx].focus_first(window, cx);
        cx.notify();
    }

    fn commit_rename(&mut self) {
        if let Some(idx) = self.renaming_idx.take() {
            let text = std::mem::take(&mut self.rename_text);
            if !text.is_empty()
                && let Some(ws) = self.workspaces.get_mut(idx)
            {
                ws.title = text;
            }
        }
    }

    fn handle_next_workspace(
        &mut self,
        _: &NextWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspaces.is_empty() {
            let next = (self.active_idx + 1) % self.workspaces.len();
            self.select_workspace(next, window, cx);
        }
    }

    fn handle_select_ws(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_workspace(idx, window, cx);
    }

    // Macro-like handlers for Ctrl+1-9
    fn handle_ws1(&mut self, _: &SelectWorkspace1, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(0, w, cx);
    }
    fn handle_ws2(&mut self, _: &SelectWorkspace2, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(1, w, cx);
    }
    fn handle_ws3(&mut self, _: &SelectWorkspace3, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(2, w, cx);
    }
    fn handle_ws4(&mut self, _: &SelectWorkspace4, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(3, w, cx);
    }
    fn handle_ws5(&mut self, _: &SelectWorkspace5, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(4, w, cx);
    }
    fn handle_ws6(&mut self, _: &SelectWorkspace6, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(5, w, cx);
    }
    fn handle_ws7(&mut self, _: &SelectWorkspace7, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(6, w, cx);
    }
    fn handle_ws8(&mut self, _: &SelectWorkspace8, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(7, w, cx);
    }
    fn handle_ws9(&mut self, _: &SelectWorkspace9, w: &mut Window, cx: &mut Context<Self>) {
        self.handle_select_ws(8, w, cx);
    }

    // --- Sidebar rendering ---

    fn sidebar_action_btn(
        &self,
        id: &'static str,
        label: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_color(rgb(0x6c7086))
            .text_xs()
            .hover(|s| s.bg(rgb(0x313244)).text_color(rgb(0xcdd6f4)))
            .on_click(move |e, w, cx| on_click(e, w, cx))
            .child(label)
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = div()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(0x181825))
            .border_r_1()
            .border_color(rgb(0x313244))
            .flex()
            .flex_col();

        // Top spacing for traffic-light / title-bar area (matches cmux trafficLightPadding)
        sidebar = sidebar.child(div().h(px(28.)));

        // ── Action buttons row ──
        sidebar = sidebar.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(10.))
                .py(px(6.))
                .child(
                    // Left side — section label
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x6c7086))
                        .child("WORKSPACES"),
                )
                .child(
                    // Right side — action buttons
                    div().flex().flex_row().items_center().gap(px(2.)).child(
                        self.sidebar_action_btn(
                            "sidebar-new-ws",
                            "+",
                            cx.listener(|this, _: &ClickEvent, w, cx| {
                                this.create_workspace_with_picker(w, cx);
                            }),
                        ),
                    ),
                ),
        );

        // Workspace list — scrollable area
        let mut list = div()
            .flex_1()
            .overflow_hidden()
            .flex()
            .flex_col()
            .py_2()
            .gap(px(2.)); // ~2px spacing between cards

        for (i, ws) in self.workspaces.iter().enumerate() {
            let is_active = i == self.active_idx;

            let title = ws.title.clone();
            // Format cwd as ~/... (collapse home dir)
            let cwd_display = {
                let home = std::env::var("HOME").unwrap_or_default();
                if !home.is_empty() && ws.cwd.starts_with(&home) {
                    format!("~{}", &ws.cwd[home.len()..])
                } else {
                    ws.cwd.clone()
                }
            };
            let pane_count = ws.pane_count();
            let pane_label = format!(
                "{pane_count} pane{}",
                if pane_count != 1 { "s" } else { "" }
            );

            let idx = i;
            let card_bg = if is_active {
                rgb(0x313244) // Surface0 — active card fill
            } else {
                rgb(0x181825) // Mantle — same as sidebar bg (invisible card)
            };

            let mut card = div()
                .id(SharedString::from(format!("ws-{i}")))
                .mx(px(6.))
                .px(px(10.))
                .py(px(8.))
                .bg(card_bg)
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0x45475a)))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let is_double = matches!(e, ClickEvent::Mouse(m) if m.down.click_count == 2);
                    if is_double {
                        this.commit_rename(); // commit any previous rename
                        this.rename_text = this.workspaces[idx].title.clone();
                        this.renaming_idx = Some(idx);
                    } else {
                        this.commit_rename();
                        this.select_workspace(idx, window, cx);
                    }
                    cx.notify();
                }))
                .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _window, cx| {
                    if this.renaming_idx != Some(idx) {
                        return;
                    }
                    let key = e.keystroke.key.as_str();
                    match key {
                        "enter" => {
                            this.commit_rename();
                            cx.notify();
                        }
                        "escape" => {
                            this.renaming_idx = None;
                            this.rename_text.clear();
                            cx.notify();
                        }
                        "backspace" => {
                            this.rename_text.pop();
                            cx.notify();
                        }
                        _ => {
                            if let Some(ch) = &e.keystroke.key_char {
                                if !ch.is_empty()
                                    && !e.keystroke.modifiers.control
                                    && !e.keystroke.modifiers.platform
                                {
                                    this.rename_text.push_str(ch);
                                    cx.notify();
                                }
                            }
                        }
                    }
                }))
                .flex()
                .flex_col()
                .gap_1();

            // ── Row 1: Title + close button ──
            let can_close = self.workspaces.len() > 1;
            let title_el = if self.renaming_idx == Some(i) {
                div()
                    .text_color(rgb(0xcdd6f4))
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .bg(rgb(0x45475a))
                    .px_1()
                    .rounded_sm()
                    .child(format!("{}|", self.rename_text))
            } else {
                div()
                    .text_color(if is_active {
                        rgb(0xcdd6f4)
                    } else {
                        rgb(0xbac2de) // Subtext1 for inactive
                    })
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(title)
            };
            let mut title_row = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(title_el);

            if can_close {
                title_row = title_row.child(
                    div()
                        .id(SharedString::from(format!("ws-close-{i}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(18.))
                        .h(px(18.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .text_color(rgb(0x585b70))
                        .text_xs()
                        .hover(|s| s.bg(rgb(0x45475a)).text_color(rgb(0xf38ba8)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.close_workspace_at(idx, window, cx);
                        }))
                        .child(
                            svg()
                                .size(px(12.))
                                .flex_none()
                                .path("icons/trash.svg")
                                .text_color(rgb(0x585b70)),
                        ),
                );
            }

            card = card.child(title_row);

            // ── Row 2: Subtitle — pane count as status ──
            card = card.child(
                div()
                    .text_color(if is_active {
                        rgb(0xa6adc8) // Subtext0 — slightly brighter when active
                    } else {
                        rgb(0x6c7086) // Overlay0
                    })
                    .text_xs()
                    .child(pane_label),
            );

            // ── Row 3: Git diff stats ──
            if !ws.git_stats.is_empty() {
                let ins = ws.git_stats.insertions;
                let del = ws.git_stats.deletions;
                card = card.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .text_xs()
                        .child(
                            div()
                                .text_color(rgb(0xa6e3a1)) // Catppuccin Green
                                .child(format!("+{ins}")),
                        )
                        .child(
                            div()
                                .text_color(rgb(0xf38ba8)) // Catppuccin Red
                                .child(format!("-{del}")),
                        ),
                );
            } else if ws.is_git_repo {
                card = card.child(
                    div()
                        .text_color(rgb(0x6c7086)) // Catppuccin Overlay0
                        .text_xs()
                        .child("No changes detected"),
                );
            }

            // ── Row 4: Working directory (monospace-style) ──
            card = card.child(
                div()
                    .text_color(if is_active {
                        rgb(0x9399b2) // Overlay2 when active
                    } else {
                        rgb(0x585b70) // Surface2 when inactive
                    })
                    .text_xs()
                    .child(cwd_display),
            );

            list = list.child(card);
        }

        sidebar = sidebar.child(list);

        sidebar
    }
}

// ---------------------------------------------------------------------------
// CSD window resize helpers
// ---------------------------------------------------------------------------

/// Width of the invisible border zone used for edge/corner resize handles.
const RESIZE_BORDER: Pixels = px(10.0);

/// Determine which resize edge/corner the mouse is hovering, if any.
fn resize_edge(
    pos: Point<Pixels>,
    border: Pixels,
    window_size: Size<Pixels>,
    tiling: gpui::Tiling,
) -> Option<ResizeEdge> {
    // If the cursor is well inside the content area, no edge.
    let inner = Bounds::new(Point::default(), window_size).inset(border * 1.5);
    if inner.contains(&pos) {
        return None;
    }

    let corner = size(border * 1.5, border * 1.5);

    // Corners first (larger hit zone = 1.5× border)
    if !tiling.top && !tiling.left && Bounds::new(point(px(0.), px(0.)), corner).contains(&pos) {
        return Some(ResizeEdge::TopLeft);
    }
    if !tiling.top
        && !tiling.right
        && Bounds::new(point(window_size.width - corner.width, px(0.)), corner).contains(&pos)
    {
        return Some(ResizeEdge::TopRight);
    }
    if !tiling.bottom
        && !tiling.left
        && Bounds::new(point(px(0.), window_size.height - corner.height), corner).contains(&pos)
    {
        return Some(ResizeEdge::BottomLeft);
    }
    if !tiling.bottom
        && !tiling.right
        && Bounds::new(
            point(
                window_size.width - corner.width,
                window_size.height - corner.height,
            ),
            corner,
        )
        .contains(&pos)
    {
        return Some(ResizeEdge::BottomRight);
    }

    // Edges
    if !tiling.top && pos.y < border {
        Some(ResizeEdge::Top)
    } else if !tiling.bottom && pos.y > window_size.height - border {
        Some(ResizeEdge::Bottom)
    } else if !tiling.left && pos.x < border {
        Some(ResizeEdge::Left)
    } else if !tiling.right && pos.x > window_size.width - border {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

impl Render for PaneFlowApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main_content = if let Some(ws) = self.active_workspace() {
            if let Some(root) = &ws.root {
                root.render(window, cx)
            } else {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_full()
                    .child(
                        div()
                            .text_color(rgb(0x6c7086))
                            .child("No terminal panes open"),
                    )
                    .into_any_element()
            }
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .child(div().text_color(rgb(0x6c7086)).child("No workspaces"))
                .into_any_element()
        };

        // Update title bar with current workspace name
        let ws_name = self.active_workspace().map(|ws| ws.title.clone());
        self.title_bar.update(cx, |tb, _| {
            tb.workspace_name = ws_name;
            tb.sidebar_width = px(SIDEBAR_WIDTH);
        });

        // --- CSD resize backdrop ---
        let decorations = window.window_decorations();

        match decorations {
            Decorations::Client { .. } => window.set_client_inset(RESIZE_BORDER),
            Decorations::Server => window.set_client_inset(px(0.0)),
        }

        // The inner app content (title bar + sidebar + main)
        let app_content = div()
            .flex()
            .flex_col()
            .size_full()
            .cursor(CursorStyle::Arrow)
            .on_action(cx.listener(Self::handle_split_h))
            .on_action(cx.listener(Self::handle_split_v))
            .on_action(cx.listener(Self::handle_close_pane))
            .on_action(cx.listener(Self::handle_new_tab))
            .on_action(cx.listener(Self::handle_close_tab))
            .on_action(cx.listener(Self::handle_focus_left))
            .on_action(cx.listener(Self::handle_focus_right))
            .on_action(cx.listener(Self::handle_focus_up))
            .on_action(cx.listener(Self::handle_focus_down))
            .on_action(cx.listener(Self::handle_new_workspace))
            .on_action(cx.listener(Self::handle_close_workspace))
            .on_action(cx.listener(Self::handle_next_workspace))
            .on_action(cx.listener(Self::handle_toggle_zoom))
            .on_action(cx.listener(Self::handle_layout_even_h))
            .on_action(cx.listener(Self::handle_layout_even_v))
            .on_action(cx.listener(Self::handle_layout_main_v))
            .on_action(cx.listener(Self::handle_layout_tiled))
            .on_action(cx.listener(Self::handle_ws1))
            .on_action(cx.listener(Self::handle_ws2))
            .on_action(cx.listener(Self::handle_ws3))
            .on_action(cx.listener(Self::handle_ws4))
            .on_action(cx.listener(Self::handle_ws5))
            .on_action(cx.listener(Self::handle_ws6))
            .on_action(cx.listener(Self::handle_ws7))
            .on_action(cx.listener(Self::handle_ws8))
            .on_action(cx.listener(Self::handle_ws9))
            .on_action(
                cx.listener(|_this: &mut Self, _: &CloseWindow, _window, cx| {
                    cx.quit();
                }),
            )
            .on_mouse_move(|_e, _, cx| cx.stop_propagation())
            // Title bar (Entity with drag-to-move support)
            .child(self.title_bar.clone())
            // Sidebar + main content area
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_sidebar(cx))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .bg(rgb(0x1e1e2e))
                            .overflow_hidden()
                            .child(main_content),
                    ),
            );

        // Outer backdrop div — provides the invisible resize border zone for CSD
        div()
            .id("window-backdrop")
            .bg(transparent_black())
            .size_full()
            .map(|d| match decorations {
                Decorations::Server => d,
                Decorations::Client { tiling } => d
                    // Resize cursor canvas (absolute overlay for the full window)
                    .child(
                        canvas(
                            |_bounds, window, _cx| {
                                window.insert_hitbox(
                                    Bounds::new(
                                        point(px(0.0), px(0.0)),
                                        window.window_bounds().get_bounds().size,
                                    ),
                                    HitboxBehavior::Normal,
                                )
                            },
                            move |_bounds, hitbox, window, _cx| {
                                let mouse = window.mouse_position();
                                let win_size = window.window_bounds().get_bounds().size;
                                let Some(edge) =
                                    resize_edge(mouse, RESIZE_BORDER, win_size, tiling)
                                else {
                                    return;
                                };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
                                        ResizeEdge::Left | ResizeEdge::Right => {
                                            CursorStyle::ResizeLeftRight
                                        }
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                            CursorStyle::ResizeUpLeftDownRight
                                        }
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                            CursorStyle::ResizeUpRightDownLeft
                                        }
                                    },
                                    &hitbox,
                                );
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    // Padding on non-tiled edges creates the invisible resize border
                    .when(!tiling.top, |d| d.pt(RESIZE_BORDER))
                    .when(!tiling.bottom, |d| d.pb(RESIZE_BORDER))
                    .when(!tiling.left, |d| d.pl(RESIZE_BORDER))
                    .when(!tiling.right, |d| d.pr(RESIZE_BORDER))
                    // Refresh on mouse move so cursor style updates every frame
                    .on_mouse_move(|_e, window, _cx| window.refresh())
                    // Initiate resize on mouse-down in the border zone
                    .on_mouse_down(MouseButton::Left, move |e, window, _cx| {
                        let win_size = window.window_bounds().get_bounds().size;
                        if let Some(edge) = resize_edge(e.position, RESIZE_BORDER, win_size, tiling)
                        {
                            window.start_window_resize(edge);
                        }
                    }),
            })
            .child(app_content)
    }
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "info,wgpu_hal=off,wgpu_core=warn,naga=warn,zbus=warn,tracing::span=warn",
    ))
    .init();

    application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            cx.bind_keys([
                KeyBinding::new("ctrl-shift-d", SplitHorizontally, None),
                KeyBinding::new("ctrl-shift-e", SplitVertically, None),
                KeyBinding::new("ctrl-shift-w", ClosePane, None),
                KeyBinding::new("ctrl-shift-n", NewWorkspace, None),
                KeyBinding::new("ctrl-shift-q", CloseWorkspace, None),
                KeyBinding::new("ctrl-tab", NextWorkspace, None),
                KeyBinding::new("alt-left", FocusLeft, None),
                KeyBinding::new("alt-right", FocusRight, None),
                KeyBinding::new("alt-up", FocusUp, None),
                KeyBinding::new("alt-down", FocusDown, None),
                KeyBinding::new("ctrl-1", SelectWorkspace1, None),
                KeyBinding::new("ctrl-2", SelectWorkspace2, None),
                KeyBinding::new("ctrl-3", SelectWorkspace3, None),
                KeyBinding::new("ctrl-4", SelectWorkspace4, None),
                KeyBinding::new("ctrl-5", SelectWorkspace5, None),
                KeyBinding::new("ctrl-6", SelectWorkspace6, None),
                KeyBinding::new("ctrl-7", SelectWorkspace7, None),
                KeyBinding::new("ctrl-8", SelectWorkspace8, None),
                KeyBinding::new("ctrl-9", SelectWorkspace9, None),
                KeyBinding::new("ctrl-shift-t", NewTab, None),
                KeyBinding::new("ctrl-w", CloseTab, None),
                KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
                KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
                KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
                KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
                KeyBinding::new("ctrl-shift-z", ToggleZoom, None),
                KeyBinding::new("ctrl-alt-1", LayoutEvenHorizontal, None),
                KeyBinding::new("ctrl-alt-2", LayoutEvenVertical, None),
                KeyBinding::new("ctrl-alt-3", LayoutMainVertical, None),
                KeyBinding::new("ctrl-alt-4", LayoutTiled, None),
            ]);

            let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);

            // Read window_decorations setting from config
            let config = paneflow_config::loader::load_config();
            let decorations = match config.window_decorations.as_deref() {
                Some("server") => WindowDecorations::Server,
                Some("client") | None => WindowDecorations::Client,
                Some(other) => {
                    log::warn!(
                        "Invalid window_decorations value '{}', using 'client'",
                        other
                    );
                    WindowDecorations::Client
                }
            };

            let window_result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.0), px(500.0))),
                    window_decorations: Some(decorations),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("PaneFlow".into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    app_id: Some("paneflow".into()),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(PaneFlowApp::new);
                    view.update(cx, |app, cx| {
                        app.workspaces[0].focus_first(window, cx);
                    });
                    view
                },
            );

            match window_result {
                Ok(_) => cx.activate(true),
                Err(e) => {
                    log::error!(
                        "Failed to open PaneFlow window: {e}\n\n\
                     This usually means your GPU driver does not support Vulkan (Linux) \
                     or Metal (macOS).\n\n\
                     Troubleshooting:\n\
                     - Linux: install mesa-vulkan-drivers or your GPU vendor's Vulkan ICD\n\
                     - Run `vulkaninfo` to verify Vulkan support\n\
                     - Try setting WGPU_BACKEND=gl for OpenGL fallback"
                    );
                    eprintln!(
                        "Error: Failed to open PaneFlow window.\n\n\
                     Your GPU driver may not support Vulkan. \
                     Install mesa-vulkan-drivers or run with RUST_LOG=error for details."
                    );
                    std::process::exit(1);
                }
            }
        });
}
