//! Cross-thread channel pumps and JSON-RPC dispatcher for `PaneFlowApp`.
//!
//! Runs on the GPUI main thread and owns three pull-based intakes:
//! - `process_ipc_requests` — drains the Unix-socket IPC receiver and routes
//!   each request through `handle_ipc` (dispatches over the `workspace.*`,
//!   `surface.*`, and `ai.*` namespaces).
//! - `process_config_changes` — picks up a hot-reloaded config deposited by
//!   the `ConfigWatcher` background thread and reapplies keybindings + theme.
//! - `process_update_check` — picks up the background update-check result
//!   once (no-op once resolved).
//!
//! Extracted from `main.rs` per US-024 of the src-app refactor PRD. `handle_ipc`
//! remains a single function here; if future additions push it over the
//! module's LOC budget, split by namespace per the PRD's fallback spec.

use gpui::{AppContext, Context};
use paneflow_config::schema::LayoutNode;

use crate::layout::SplitDirection;
use crate::terminal::TerminalView;
use crate::workspace::{Workspace, next_workspace_id};
use crate::{
    Notification, PaneFlowApp, ai_types, find_first_terminal, find_terminal_by_surface_id,
    keybindings, send_text_to_first_leaf, update,
};

impl PaneFlowApp {
    pub(crate) fn process_ipc_requests(&mut self, cx: &mut Context<Self>) {
        while let Ok(req) = self.ipc_rx.try_recv() {
            let result = self.handle_ipc(&req.method, &req.params, cx);
            let _ = req.response_tx.send(result);
        }
    }

    /// Apply any pending config change deposited by the background `ConfigWatcher`.
    pub(crate) fn process_config_changes(&mut self, cx: &mut Context<Self>) {
        let new_config = self
            .pending_config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(config) = new_config {
            keybindings::apply_keybindings(cx, &config.shortcuts);
            self.effective_shortcuts = keybindings::effective_shortcuts(&config.shortcuts);
            crate::theme::invalidate_theme_cache();
            cx.notify();
        }
    }

    /// Pick up the background update check result (runs once, then stops polling).
    pub(crate) fn process_update_check(&mut self, cx: &mut Context<Self>) {
        if self.update_status.is_some() {
            return; // Already resolved
        }
        let status = self
            .pending_update
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(status) = status
            && !matches!(status, update::checker::UpdateStatus::Checking)
        {
            self.update_status = Some(status);
            cx.notify();
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
                        "layout": layout.and_then(|l| serde_json::to_value(l).ok()),
                    })
                } else {
                    serde_json::json!(null)
                }
            }
            "workspace.create" => {
                // Cap workspace count to prevent unbounded growth from malicious
                // or buggy IPC clients (CWE-400). Matches the keyboard-action cap
                // in `workspace_ops::create_workspace`.
                const MAX_WORKSPACES: usize = 20;
                if self.workspaces.len() >= MAX_WORKSPACES {
                    return serde_json::json!({"error": "Workspace limit reached"});
                }
                let name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("Terminal");
                let cwd = params
                    .get("cwd")
                    .and_then(|c| c.as_str())
                    .map(std::path::PathBuf::from);
                let ws_id = next_workspace_id();
                let ws = if let Some(dir) = cwd {
                    let terminal =
                        cx.new(|cx| TerminalView::with_cwd(ws_id, Some(dir.clone()), None, cx));
                    let pane = self.create_pane(terminal, ws_id, cx);
                    Workspace::with_cwd_and_id(ws_id, name, dir, pane)
                } else {
                    let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
                    let pane = self.create_pane(terminal, ws_id, cx);
                    Workspace::with_id(ws_id, name, pane)
                };
                self.watch_git_dir(&ws);
                self.workspaces.push(ws);
                let idx = self.workspaces.len() - 1;
                self.save_session(cx);
                cx.notify();
                serde_json::json!({"index": idx, "title": name})
            }
            "workspace.select" => {
                let idx = params.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                if idx < self.workspaces.len() {
                    self.active_idx = idx;
                    self.save_session(cx);
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
                        self.save_session(cx);
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
                // Route by surface_id if provided, otherwise use first leaf
                if let Some(sid) = params.get("surface_id").and_then(|s| s.as_u64()) {
                    if let Some(terminal) = find_terminal_by_surface_id(&self.workspaces, sid, cx) {
                        terminal.read(cx).send_text(text);
                        return serde_json::json!({"sent": true, "length": text.len()});
                    }
                    return serde_json::json!({"error": "Surface not found"});
                }
                if let Some(ws) = self.active_workspace()
                    && let Some(root) = &ws.root
                {
                    send_text_to_first_leaf(root, text, cx);
                    return serde_json::json!({"sent": true, "length": text.len()});
                }
                serde_json::json!({"error": "No active terminal"})
            }
            "surface.send_keystroke" => {
                let keystroke = params
                    .get("keystroke")
                    .and_then(|k| k.as_str())
                    .unwrap_or("");
                if keystroke.is_empty() {
                    return serde_json::json!({"error": "Missing 'keystroke' parameter"});
                }
                // Route by surface_id if provided, otherwise use active terminal
                let terminal = if let Some(sid) = params.get("surface_id").and_then(|s| s.as_u64())
                {
                    find_terminal_by_surface_id(&self.workspaces, sid, cx)
                } else if let Some(ws) = self.active_workspace()
                    && let Some(root) = &ws.root
                {
                    // Use first leaf as default
                    find_first_terminal(root, cx)
                } else {
                    None
                };
                match terminal {
                    Some(t) => match t.read(cx).send_keystroke(keystroke) {
                        Ok(()) => serde_json::json!({"sent": true}),
                        Err(e) => serde_json::json!({"error": e}),
                    },
                    None => serde_json::json!({"error": "No active terminal"}),
                }
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
                let ws_id = ws.id;
                if root.leaf_count() >= MAX_PANES {
                    return serde_json::json!({"error": "Maximum pane count reached"});
                }
                let new_terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
                let new_pane = self.create_pane(new_terminal, ws_id, cx);
                let Some(ws) = self.active_workspace_mut() else {
                    return serde_json::json!({"error": "No active workspace"});
                };
                let Some(root) = ws.root.as_mut() else {
                    return serde_json::json!({"error": "Workspace has no root"});
                };
                root.split_first_leaf(direction, new_pane);
                let panes = ws.pane_count();
                self.save_session(cx);
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
            // -----------------------------------------------------------------
            // AI hook lifecycle methods (from paneflow-hook via IPC socket)
            // -----------------------------------------------------------------
            "ai.session_start" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let Some(pid) = params
                    .get("pid")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .filter(|&p| p > 0)
                else {
                    return serde_json::json!({"error": "Missing or invalid pid"});
                };
                // Tool name: check top-level "tool" param, then hook_payload.tool, default "claude"
                let hook = params.get("hook_payload");
                let tool = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                // Validate tool name: alphanumeric + hyphens, max 64 chars
                if tool.len() > 64 || !tool.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return serde_json::json!({"error": "Invalid tool name"});
                }
                let tool = tool.to_string();

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // Cap agent PIDs to prevent unbounded HashMap growth from
                    // malicious or buggy IPC clients (CWE-400).
                    const MAX_AGENT_PIDS: usize = 16;
                    if ws.agent_pids.len() >= MAX_AGENT_PIDS && !ws.agent_pids.contains_key(&tool) {
                        return serde_json::json!({"error": "Agent PID limit reached"});
                    }
                    ws.agent_pids.insert(tool, pid);
                    serde_json::json!({"registered": true})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            "ai.prompt_submit" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let hook = params.get("hook_payload");
                let tool_name = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                let tool = ai_types::AiTool::from_name(tool_name);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_types::AiToolState::Thinking(tool);
                    // Clear pending notifications for this workspace
                    self.notifications
                        .retain(|n| n.workspace_id != workspace_id);
                    cx.notify();
                    if !self.loader_anim_running {
                        self.start_loader_animation(cx);
                    }
                    serde_json::json!({"status": "running"})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            "ai.tool_use" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let hook = params.get("hook_payload");
                let tool_name = hook
                    .and_then(|h| h.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("tool_name").and_then(|v| v.as_str()));
                let tool_str = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                let tool = ai_types::AiTool::from_name(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // Keep Thinking state (no transition if already Thinking)
                    if !matches!(ws.ai_state, ai_types::AiToolState::Thinking(_)) {
                        ws.ai_state = ai_types::AiToolState::Thinking(tool);
                    }
                    ws.active_tool_name =
                        tool_name.map(|s| s.chars().take(128).collect::<String>());
                    cx.notify();
                    serde_json::json!({"status": "running"})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            "ai.notification" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let hook = params.get("hook_payload");
                let tool_str = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                let tool = ai_types::AiTool::from_name(tool_str);
                let message: String = hook
                    .and_then(|h| h.get("message"))
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("message").and_then(|v| v.as_str()))
                    .unwrap_or("Needs input")
                    .chars()
                    .take(512)
                    .collect();

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_types::AiToolState::WaitingForInput(tool);
                    ws.active_tool_name = None;
                    let title = ws.title.clone();
                    self.notifications.push(Notification {
                        workspace_id,
                        workspace_title: title,
                        message,
                        kind: ai_types::AiToolState::WaitingForInput(tool),
                        timestamp: std::time::Instant::now(),
                        read: false,
                    });
                    cx.notify();
                    serde_json::json!({"status": "waiting"})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            "ai.stop" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let hook = params.get("hook_payload");
                let tool_str = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                let tool = ai_types::AiTool::from_name(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_types::AiToolState::Finished(tool);
                    ws.active_tool_name = None;
                    let title = ws.title.clone();
                    self.notifications.push(Notification {
                        workspace_id,
                        workspace_title: title,
                        message: format!("{} finished", tool.label()),
                        kind: ai_types::AiToolState::Finished(tool),
                        timestamp: std::time::Instant::now(),
                        read: false,
                    });
                    cx.notify();

                    // Auto-reset to Inactive after 5 seconds
                    let ws_id = workspace_id;
                    cx.spawn(
                        async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                            smol::Timer::after(std::time::Duration::from_secs(5)).await;
                            cx.update(|cx| {
                                let _ = this.update(cx, |app, cx| {
                                    if let Some(ws) =
                                        app.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                                        && matches!(ws.ai_state, ai_types::AiToolState::Finished(_))
                                    {
                                        ws.ai_state = ai_types::AiToolState::Inactive;
                                        cx.notify();
                                    }
                                });
                            });
                        },
                    )
                    .detach();

                    serde_json::json!({"status": "idle"})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            "ai.session_end" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let hook = params.get("hook_payload");
                let tool_str = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                if tool_str.len() > 64
                    || !tool_str
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return serde_json::json!({"error": "Invalid tool name"});
                }

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_types::AiToolState::Inactive;
                    ws.active_tool_name = None;
                    ws.agent_pids.remove(tool_str);
                    self.notifications
                        .retain(|n| n.workspace_id != workspace_id);
                    cx.notify();
                    serde_json::json!({"cleared": true})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            _ => {
                serde_json::json!({"error": format!("Unknown method: {method}")})
            }
        }
    }
}
