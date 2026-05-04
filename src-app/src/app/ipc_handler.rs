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

use gpui::{App, AppContext, Context};
use paneflow_config::schema::LayoutNode;

use crate::layout::LayoutTree;
use crate::layout::SplitDirection;
use crate::terminal::TerminalView;
use crate::workspace::{Workspace, next_workspace_id};
use crate::{Notification, PaneFlowApp, ai_types, keybindings, update};

// ---------------------------------------------------------------------------
// Terminal-routing helpers used by the IPC `surface.*` handlers (US-002:
// extracted from `main.rs`). Re-exported at the crate root via `main.rs` so
// older `crate::find_first_terminal` lookups keep resolving.
// ---------------------------------------------------------------------------

/// Write text to the first leaf pane's active terminal PTY in a layout tree.
/// US-020: silently skips markdown leaves (no PTY to write to).
pub(crate) fn send_text_to_first_leaf(node: &LayoutTree, text: &str, cx: &App) {
    match node {
        LayoutTree::Leaf(pane) => {
            if let Some(active) = pane.read(cx).active_terminal_opt() {
                active
                    .read(cx)
                    .terminal
                    .write_to_pty(text.as_bytes().to_vec());
            }
        }
        LayoutTree::Container { children, .. } => {
            if let Some(first) = children.first() {
                send_text_to_first_leaf(&first.node, text, cx);
            }
        }
    }
}

/// Find the first terminal in a layout tree (for default routing).
/// US-020: skips markdown leaves — recurses past them when searching containers.
pub(crate) fn find_first_terminal(
    node: &LayoutTree,
    cx: &App,
) -> Option<gpui::Entity<TerminalView>> {
    match node {
        LayoutTree::Leaf(pane) => pane.read(cx).active_terminal_opt().cloned(),
        LayoutTree::Container { children, .. } => children
            .iter()
            .find_map(|child| find_first_terminal(&child.node, cx)),
    }
}

/// Find a terminal view entity by its surface_id (GPUI entity ID) across all workspaces.
pub(crate) fn find_terminal_by_surface_id(
    workspaces: &[Workspace],
    surface_id: u64,
    cx: &App,
) -> Option<gpui::Entity<TerminalView>> {
    for ws in workspaces {
        if let Some(root) = &ws.root
            && let Some(t) = find_terminal_in_tree(root, surface_id, cx)
        {
            return Some(t);
        }
    }
    None
}

fn find_terminal_in_tree(
    node: &LayoutTree,
    surface_id: u64,
    cx: &App,
) -> Option<gpui::Entity<TerminalView>> {
    match node {
        LayoutTree::Leaf(pane) => {
            let pane = pane.read(cx);
            for terminal in &pane.tabs {
                if terminal.entity_id().as_u64() == surface_id {
                    return Some(terminal.clone());
                }
            }
            None
        }
        LayoutTree::Container { children, .. } => {
            for child in children {
                if let Some(t) = find_terminal_in_tree(&child.node, surface_id, cx) {
                    return Some(t);
                }
            }
            None
        }
    }
}

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
            // US-014: reconcile the telemetry consent state. On any change,
            // rebuild the `TelemetryClient` handle (Null ↔ Active) so future
            // emissions reflect the new choice; show a confirmation toast;
            // fire a one-time `telemetry_reenabled` breadcrumb on an explicit
            // opted-out → opted-in transition (ROPA audit trail).
            self.reconcile_telemetry_consent(&config, cx);
            cx.notify();
        }

        // US-006: drain the theme watcher's "file changed" signal. The
        // watcher invalidates the cache directly on its background thread;
        // this only schedules the GPUI repaint so the next render picks up
        // the freshly-resolved theme. `swap` is the cheapest way to read +
        // reset atomically — we don't care about preserving other writers.
        if self
            .theme_changed
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            cx.notify();
        }
    }

    /// Handle a `config.telemetry.enabled` change detected during config
    /// reconciliation. Compares against `self.telemetry_enabled_last` and
    /// performs the three mandated side effects:
    /// 1. Rebuild the client handle via `TelemetryClient::from_config`.
    /// 2. Emit `telemetry_reenabled` iff `Some(false) → Some(true)`.
    /// 3. Surface a toast mirroring the new state.
    ///
    /// Pure logic (which side effects to run) is factored into
    /// [`reconcile_telemetry`] so it can be unit-tested without GPUI or
    /// filesystem state.
    fn reconcile_telemetry_consent(
        &mut self,
        config: &paneflow_config::schema::PaneFlowConfig,
        cx: &mut Context<Self>,
    ) {
        let new_enabled = config.telemetry.as_ref().and_then(|t| t.enabled);
        let decision = reconcile_telemetry(self.telemetry_enabled_last, new_enabled);
        if !decision.rebuild {
            return;
        }

        // Swap the client handle. Distinct_id is re-read from disk — if
        // the telemetry_id file is gone since last launch, we get a
        // fresh ephemeral; otherwise the stable UUID persists.
        let distinct_id = crate::telemetry::id::telemetry_id();
        let api_key = option_env!("POSTHOG_API_KEY").unwrap_or("");
        let host = option_env!("POSTHOG_HOST").unwrap_or("https://eu.i.posthog.com");
        self.telemetry =
            std::sync::Arc::new(crate::telemetry::client::TelemetryClient::from_config(
                config,
                api_key,
                host,
                &distinct_id,
            ));

        if decision.reenabled {
            // Explicit false → true transition. `telemetry_reenabled`
            // carries no properties — its presence alone documents that
            // consent was re-granted from an opted-out state.
            self.telemetry
                .capture("telemetry_reenabled", serde_json::json!({}));
        }

        self.telemetry_enabled_last = new_enabled;

        if let Some(msg) = decision.toast_msg {
            self.show_toast(msg, cx);
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
            // Zed-style silent pre-install: as soon as we know there's
            // a new release and the install method supports an in-app
            // download, kick off the install in the background. By the
            // time the user notices the pill and clicks it, the new
            // binary is already on disk and the click handler only has
            // to invoke `cx.restart()`.
            self.try_auto_kickoff_install(cx);
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
                // US-001: parse the optional `layout` param up-front so we can
                // refuse a malformed payload with -32602 before mutating any
                // workspace state.
                let mut layout = match parse_layout_param(params) {
                    Ok(l) => l,
                    Err(e) => return e.into_value(),
                };
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

                // US-001: when a layout is provided, apply it to the freshly
                // created workspace. `apply_layout_from_json` operates on the
                // active workspace, so we have to switch focus first; we
                // restore `previous_idx` if application fails so a malformed
                // layout doesn't strand the caller on a half-initialised
                // workspace they didn't ask to land on.
                let panes = if let Some(ref mut layout) = layout {
                    let previous_idx = self.active_idx;
                    self.active_idx = idx;
                    if let Err(e) = self.apply_layout_from_json(layout, cx) {
                        // Roll back: drop the just-created workspace so the
                        // caller sees a clean -32602 and no orphan workspace.
                        if let Some(dir) = self.workspaces[idx].git_dir.clone() {
                            self.unwatch_git_dir(&dir);
                        }
                        self.workspaces.remove(idx);
                        self.active_idx = previous_idx.min(self.workspaces.len().saturating_sub(1));
                        return JsonRpcError::invalid_params(format!(
                            "layout could not be applied: {e}"
                        ))
                        .into_value();
                    }
                    self.active_workspace().map_or(1, |ws| ws.pane_count())
                } else {
                    1
                };

                self.save_session(cx);
                cx.notify();
                serde_json::json!({"index": idx, "title": name, "panes": panes})
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
                        message: message.clone(),
                        kind: ai_types::AiToolState::WaitingForInput(tool),
                        timestamp: std::time::Instant::now(),
                        read: false,
                    });
                    cx.notify();
                    let _ = message;
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
                    let _ = tool_str;
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

// ---------------------------------------------------------------------------
// JSON-RPC error envelope (US-001)
// ---------------------------------------------------------------------------

/// A structured JSON-RPC 2.0 error to be promoted into the response envelope
/// by `dispatch_to_gpui` in `ipc.rs`. Handlers signal a true protocol-level
/// error (vs. an application error returned inside `result`) by returning
/// the value produced by [`JsonRpcError::into_value`]; the dispatcher detects
/// the `_jsonrpc_error` sentinel key and rewrites the response shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

/// Sentinel key under which `dispatch_to_gpui` looks for a structured error.
/// Underscore-prefixed to make it unambiguously not a user-data field.
pub(crate) const JSONRPC_ERROR_KEY: &str = "_jsonrpc_error";

impl JsonRpcError {
    /// JSON-RPC 2.0 reserved error code for invalid method parameters.
    pub(crate) const INVALID_PARAMS: i32 = -32602;

    pub(crate) fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: Self::INVALID_PARAMS,
            message: message.into(),
        }
    }

    pub(crate) fn into_value(self) -> serde_json::Value {
        serde_json::json!({
            JSONRPC_ERROR_KEY: {
                "code": self.code,
                "message": self.message,
            }
        })
    }
}

/// Promote a handler return value into a full JSON-RPC 2.0 response.
///
/// If the value carries the `_jsonrpc_error` sentinel, it's emitted as a
/// `{ "jsonrpc", "error", "id" }` envelope; otherwise it's wrapped under
/// `result`. Pure / no I/O so it can be unit-tested without GPUI.
pub(crate) fn promote_response(
    handler_result: serde_json::Value,
    id: serde_json::Value,
) -> serde_json::Value {
    if let Some(err) = handler_result.get(JSONRPC_ERROR_KEY) {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(-32603);
        let message = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown error")
            .to_string();
        return serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": code, "message": message },
            "id": id,
        });
    }
    serde_json::json!({
        "jsonrpc": "2.0",
        "result": handler_result,
        "id": id,
    })
}

/// Parse the optional `layout` field from a `workspace.create` params object.
///
/// Returns `Ok(None)` if the field is absent or `null` (preserves the
/// existing single-pane default). Returns `Err(JsonRpcError)` with code
/// `-32602` if the field is present but not a valid `LayoutNode`.
pub(crate) fn parse_layout_param(
    params: &serde_json::Value,
) -> Result<Option<LayoutNode>, JsonRpcError> {
    let Some(raw) = params.get("layout") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    serde_json::from_value::<LayoutNode>(raw.clone())
        .map(Some)
        .map_err(|e| JsonRpcError::invalid_params(format!("invalid layout: {e}")))
}

// ---------------------------------------------------------------------------
// Telemetry reconciliation (US-014)
// ---------------------------------------------------------------------------

/// Decision outcome for the telemetry-consent reconciler (US-014).
/// Separated from the GPUI-bound `reconcile_telemetry_consent` so the
/// transition matrix is unit-testable in isolation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TelemetryReconciliation {
    /// Whether the `TelemetryClient` handle must be rebuilt. Only the
    /// `None ↔ Some(_)` and `Some(true) ↔ Some(false)` transitions
    /// require a rebuild; identical states are a no-op.
    pub rebuild: bool,
    /// Whether to emit the one-time `telemetry_reenabled` breadcrumb.
    /// Fires exclusively on the `Some(false) → Some(true)` transition
    /// — a user explicitly re-granting consent after having declined.
    /// None → Some(true) (first answer) does NOT count as a re-enable.
    pub reenabled: bool,
    /// Toast copy reflecting the resolved state. `None` if no toast is
    /// warranted (identical state transitions).
    pub toast_msg: Option<&'static str>,
}

/// Pure state-transition matrix for the telemetry consent toggle.
/// Called from `PaneFlowApp::reconcile_telemetry_consent` after the
/// background `ConfigWatcher` has deposited a fresh config.
pub(crate) fn reconcile_telemetry(old: Option<bool>, new: Option<bool>) -> TelemetryReconciliation {
    if old == new {
        return TelemetryReconciliation {
            rebuild: false,
            reenabled: false,
            toast_msg: None,
        };
    }
    let toast_msg = Some(match new {
        Some(true) => "Télémétrie activée",
        Some(false) => "Télémétrie désactivée",
        None => "Télémétrie : la demande réapparaîtra au prochain lancement",
    });
    TelemetryReconciliation {
        rebuild: true,
        reenabled: old == Some(false) && new == Some(true),
        toast_msg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exhaustive 3x3 transition matrix over `Option<bool>`. Each case is
    // asserted explicitly; a future variant added to the tri-state would
    // force this test to be updated.

    #[test]
    fn identical_state_is_a_noop() {
        for state in [None, Some(false), Some(true)] {
            let r = reconcile_telemetry(state, state);
            assert!(!r.rebuild, "no rebuild for identical {state:?}");
            assert!(!r.reenabled);
            assert!(r.toast_msg.is_none());
        }
    }

    #[test]
    fn none_to_some_true_rebuilds_but_does_not_flag_reenabled() {
        let r = reconcile_telemetry(None, Some(true));
        assert!(r.rebuild);
        assert!(
            !r.reenabled,
            "first-ever consent (None → true) is not a re-enable"
        );
        assert_eq!(r.toast_msg, Some("Télémétrie activée"));
    }

    #[test]
    fn none_to_some_false_rebuilds() {
        let r = reconcile_telemetry(None, Some(false));
        assert!(r.rebuild);
        assert!(!r.reenabled);
        assert_eq!(r.toast_msg, Some("Télémétrie désactivée"));
    }

    #[test]
    fn some_false_to_some_true_flags_reenabled() {
        let r = reconcile_telemetry(Some(false), Some(true));
        assert!(r.rebuild);
        assert!(
            r.reenabled,
            "opted-out → opted-in is the only transition that emits telemetry_reenabled"
        );
        assert_eq!(r.toast_msg, Some("Télémétrie activée"));
    }

    #[test]
    fn some_true_to_some_false_rebuilds_no_reenabled() {
        let r = reconcile_telemetry(Some(true), Some(false));
        assert!(r.rebuild);
        assert!(!r.reenabled);
        assert_eq!(r.toast_msg, Some("Télémétrie désactivée"));
    }

    #[test]
    fn some_true_to_none_rebuilds() {
        let r = reconcile_telemetry(Some(true), None);
        assert!(r.rebuild);
        assert!(!r.reenabled);
        assert_eq!(
            r.toast_msg,
            Some("Télémétrie : la demande réapparaîtra au prochain lancement")
        );
    }

    #[test]
    fn some_false_to_none_rebuilds_no_reenabled() {
        let r = reconcile_telemetry(Some(false), None);
        assert!(r.rebuild);
        assert!(!r.reenabled);
        assert_eq!(
            r.toast_msg,
            Some("Télémétrie : la demande réapparaîtra au prochain lancement")
        );
    }

    // -----------------------------------------------------------------
    // US-001 — workspace.create `layout` param parsing + JSON-RPC error
    // envelope promotion
    // -----------------------------------------------------------------

    #[test]
    fn parse_layout_param_absent_returns_none() {
        let params = serde_json::json!({"name": "ws"});
        assert!(parse_layout_param(&params).expect("ok").is_none());
    }

    #[test]
    fn parse_layout_param_null_returns_none() {
        // null is treated like absent — caller still gets the
        // single-pane default behavior.
        let params = serde_json::json!({"layout": null});
        assert!(parse_layout_param(&params).expect("ok").is_none());
    }

    #[test]
    fn parse_layout_param_valid_pane_returns_some() {
        let params = serde_json::json!({
            "layout": { "type": "pane", "surfaces": [] }
        });
        let layout = parse_layout_param(&params).expect("ok").expect("some");
        assert_eq!(layout.leaf_count(), 1);
    }

    #[test]
    fn parse_layout_param_valid_split_returns_some() {
        let params = serde_json::json!({
            "layout": {
                "type": "split",
                "direction": "vertical",
                "ratios": [0.5, 0.5],
                "children": [
                    { "type": "pane", "surfaces": [] },
                    { "type": "pane", "surfaces": [] }
                ]
            }
        });
        let layout = parse_layout_param(&params).expect("ok").expect("some");
        assert_eq!(layout.leaf_count(), 2);
    }

    #[test]
    fn parse_layout_param_string_payload_returns_invalid_params() {
        let params = serde_json::json!({"layout": "not an object"});
        let err = parse_layout_param(&params).expect_err("err");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.starts_with("invalid layout:"),
            "got {:?}",
            err.message
        );
    }

    #[test]
    fn parse_layout_param_unknown_tag_returns_invalid_params() {
        let params = serde_json::json!({"layout": { "type": "unknown_kind" }});
        let err = parse_layout_param(&params).expect_err("err");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
    }

    #[test]
    fn promote_response_wraps_value_under_result_by_default() {
        let id = serde_json::json!(7);
        let resp = promote_response(serde_json::json!({"index": 0, "title": "ws"}), id);
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 7);
        assert_eq!(resp["result"]["index"], 0);
        assert!(resp.get("error").is_none());
    }

    #[test]
    fn promote_response_extracts_jsonrpc_error_sentinel() {
        let err_val = JsonRpcError::invalid_params("bad layout").into_value();
        let resp = promote_response(err_val, serde_json::json!("req-1"));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], "req-1");
        assert!(resp.get("result").is_none());
        assert_eq!(resp["error"]["code"], -32602);
        assert_eq!(resp["error"]["message"], "bad layout");
    }

    #[test]
    fn promote_response_preserves_legacy_application_error_strings() {
        // Existing handlers return `{"error": "string"}` — those must keep
        // flowing through the `result` field, not be promoted.
        let id = serde_json::json!(null);
        let legacy = serde_json::json!({"error": "Workspace limit reached"});
        let resp = promote_response(legacy, id);
        assert_eq!(resp["result"]["error"], "Workspace limit reached");
        assert!(resp.get("error").is_none());
    }
}
