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
    App, Bounds, ClickEvent, Context, Entity, Focusable, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, PathPromptOptions, Render, SharedString, Styled, Window,
    WindowBounds, WindowDecorations, WindowOptions, actions, div, prelude::*, px, rgb, size, svg,
};
use gpui_platform::application;

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
        LayoutEvenVertical
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
}

impl PaneFlowApp {
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

        // Poll git diff stats for all workspaces every 3s
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(3)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let mut changed = false;
                            for ws in &mut app.workspaces {
                                let new_stats = crate::workspace::GitDiffStats::from_cwd(&ws.cwd);
                                if new_stats != ws.git_stats {
                                    ws.git_stats = new_stats;
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

        Self {
            workspaces: vec![ws],
            active_idx: 0,
            renaming_idx: None,
            rename_text: String::new(),
            last_config_mtime,
            ipc_rx,
            title_bar,
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
                    serde_json::json!({
                        "index": self.active_idx,
                        "title": ws.title,
                        "cwd": ws.cwd,
                        "panes": ws.pane_count(),
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

        div()
            .flex()
            .flex_col()
            .size_full()
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
            )
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
