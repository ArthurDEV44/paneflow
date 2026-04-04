//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar workspace list + main content area.

mod assets;
mod ipc;
mod keys;
mod split;
mod terminal;
mod terminal_element;
pub mod theme;
mod title_bar;
mod workspace;

use gpui::{
    actions, div, prelude::*, px, rgb, size, App, Bounds, ClickEvent, Context, Entity,
    Focusable, InteractiveElement, IntoElement, KeyBinding, Render, SharedString, Styled, Window,
    WindowBounds, WindowDecorations, WindowOptions,
};
use gpui_platform::application;

use crate::split::{FocusDirection, SplitDirection, SplitNode};
use crate::terminal::TerminalView;
use crate::workspace::Workspace;

/// Write text to the first leaf terminal's PTY in a split tree.
fn send_text_to_first_leaf(node: &SplitNode, text: &str, cx: &App) {
    match node {
        SplitNode::Leaf(terminal) => {
            terminal.read(cx).terminal.write_to_pty(text.as_bytes().to_vec());
        }
        SplitNode::Split { first, .. } => send_text_to_first_leaf(first, text, cx),
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
        ScrollPageDown
    ]
);

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

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
    fn new(cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(TerminalView::new);
        let ws = Workspace::new("Terminal 1", terminal);
        let title_bar = cx.new(title_bar::TitleBar::new);
        let last_config_mtime = crate::theme::config_mtime();
        let ipc_rx = ipc::start_server();

        // Poll IPC requests every 10ms
        cx.spawn(async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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
        })
        .detach();

        // Poll config file for theme changes every 500ms
        cx.spawn(async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
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
        })
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
                let ws = Workspace::new(name, terminal);
                self.workspaces.push(ws);
                let idx = self.workspaces.len() - 1;
                cx.notify();
                serde_json::json!({"index": idx, "title": name})
            }
            "workspace.select" => {
                let idx = params
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(0) as usize;
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
                let count = self
                    .active_workspace()
                    .map_or(0, |ws| ws.pane_count());
                serde_json::json!({"pane_count": count, "workspace": self.active_idx})
            }
            "surface.send_text" => {
                let text = params
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
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
                let Some(ws) = self.active_workspace_mut() else {
                    return serde_json::json!({"error": "No active workspace"});
                };
                let Some(root) = ws.root.as_mut() else {
                    return serde_json::json!({"error": "Workspace has no root"});
                };
                root.split_first_leaf(direction, new_terminal);
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

    fn create_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        const MAX_WORKSPACES: usize = 20;
        if self.workspaces.len() >= MAX_WORKSPACES {
            return;
        }
        let n = self.workspaces.len() + 1;
        let terminal = cx.new(TerminalView::new);
        let ws = Workspace::new(format!("Terminal {n}"), terminal);
        self.workspaces.push(ws);
        self.active_idx = self.workspaces.len() - 1;
        self.workspaces[self.active_idx].focus_first(window, cx);
        cx.notify();
    }

    // --- Split/close/focus handlers (operate on active workspace) ---

    fn split(&mut self, direction: SplitDirection, window: &mut Window, cx: &mut Context<Self>) {
        const MAX_PANES: usize = 32;
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && root.leaf_count() >= MAX_PANES
        {
            return;
        }
        let new_terminal = cx.new(TerminalView::new);
        if let Some(ws) = self.active_workspace_mut()
            && let Some(root) = &mut ws.root
            && root.split_at_focused(direction, new_terminal.clone(), window, cx)
        {
            new_terminal.read(cx).focus_handle(cx).focus(window, cx);
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
            && let Some(root) = ws.root.take()
        {
            let (new_root, _closed) = root.close_focused(window, cx);
            ws.root = new_root;
            if let Some(ref root) = ws.root {
                root.focus_first(window, cx);
            }
        }
        cx.notify();
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
        self.create_workspace(w, cx);
    }

    fn handle_close_workspace(
        &mut self,
        _: &CloseWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Guard: don't close the last workspace
        if self.workspaces.len() <= 1 {
            return;
        }
        // Remove the active workspace — Drop on SplitNode sends Msg::Shutdown to PTYs
        self.workspaces.remove(self.active_idx);
        // Clamp active_idx
        if self.active_idx >= self.workspaces.len() {
            self.active_idx = self.workspaces.len() - 1;
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

    fn handle_select_ws(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = div()
            .w(px(220.))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(0x181825))
            .border_r_1()
            .border_color(rgb(0x313244))
            .flex()
            .flex_col();

        // Header
        sidebar = sidebar.child(
            div()
                .p_3()
                .child(
                    div()
                        .text_color(rgb(0xcdd6f4))
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("PaneFlow"),
                ),
        );

        // Workspace list
        let mut list = div().flex_1().overflow_hidden().flex().flex_col();

        for (i, ws) in self.workspaces.iter().enumerate() {
            let is_active = i == self.active_idx;
            let bg = if is_active {
                rgb(0x313244) // Surface0 — highlighted
            } else {
                rgb(0x181825) // Mantle — default
            };

            let title = ws.title.clone();
            let cwd_display = ws
                .cwd
                .rsplit('/')
                .next()
                .unwrap_or(&ws.cwd)
                .to_string();
            let pane_count = ws.pane_count();

            let idx = i;
            list = list.child(
                div()
                    .id(SharedString::from(format!("ws-{i}")))
                    .px_3()
                    .py_1()
                    .bg(bg)
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x45475a)))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        let is_double = matches!(e, ClickEvent::Mouse(m) if m.down.click_count == 2);
                        if is_double {
                            // Double-click → start rename
                            this.rename_text = this.workspaces[idx].title.clone();
                            this.renaming_idx = Some(idx);
                        } else {
                            this.commit_rename();
                            this.select_workspace(idx, window, cx);
                        }
                        cx.notify();
                    }))
                    .child(if self.renaming_idx == Some(i) {
                        // Inline rename mode — show current text with visual cue
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
                            .text_color(rgb(0xcdd6f4))
                            .text_sm()
                            .font_weight(if is_active {
                                gpui::FontWeight::BOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .child(title)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .child(
                                div()
                                    .text_color(rgb(0x6c7086))
                                    .text_xs()
                                    .child(cwd_display),
                            )
                            .child(
                                div()
                                    .text_color(rgb(0x6c7086))
                                    .text_xs()
                                    .child(format!("{pane_count} pane{}", if pane_count != 1 { "s" } else { "" })),
                            ),
                    ),
            );
        }

        sidebar = sidebar.child(list);

        // "+" button at bottom
        sidebar = sidebar.child(
            div()
                .p_2()
                .border_t_1()
                .border_color(rgb(0x313244))
                .child(
                    div()
                        .id("new-workspace-btn")
                        .px_3()
                        .py_1()
                        .text_color(rgb(0x89b4fa))
                        .text_sm()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(0x313244)))
                        .rounded_md()
                        .text_center()
                        .on_click(cx.listener(|this, _e: &ClickEvent, window, cx| {
                            this.create_workspace(window, cx);
                        }))
                        .child("+ New Workspace"),
                ),
        );

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
                    .child(div().text_color(rgb(0x6c7086)).child("No terminal panes open"))
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
        });

        div()
            .flex()
            .flex_col()
            .size_full()
            .on_action(cx.listener(Self::handle_split_h))
            .on_action(cx.listener(Self::handle_split_v))
            .on_action(cx.listener(Self::handle_close_pane))
            .on_action(cx.listener(Self::handle_focus_left))
            .on_action(cx.listener(Self::handle_focus_right))
            .on_action(cx.listener(Self::handle_focus_up))
            .on_action(cx.listener(Self::handle_focus_down))
            .on_action(cx.listener(Self::handle_new_workspace))
            .on_action(cx.listener(Self::handle_close_workspace))
            .on_action(cx.listener(Self::handle_next_workspace))
            .on_action(cx.listener(Self::handle_ws1))
            .on_action(cx.listener(Self::handle_ws2))
            .on_action(cx.listener(Self::handle_ws3))
            .on_action(cx.listener(Self::handle_ws4))
            .on_action(cx.listener(Self::handle_ws5))
            .on_action(cx.listener(Self::handle_ws6))
            .on_action(cx.listener(Self::handle_ws7))
            .on_action(cx.listener(Self::handle_ws8))
            .on_action(cx.listener(Self::handle_ws9))
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
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            "info,wgpu_hal=off,wgpu_core=warn,naga=warn,zbus=warn,tracing::span=warn",
        ),
    )
    .init();

    application().with_assets(assets::Assets).run(|cx: &mut App| {
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
            KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
            KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
            KeyBinding::new("shift-pageup", ScrollPageUp, Some("Terminal")),
            KeyBinding::new("shift-pagedown", ScrollPageDown, Some("Terminal")),
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
