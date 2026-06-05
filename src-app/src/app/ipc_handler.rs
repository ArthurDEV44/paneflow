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
use crate::{PaneFlowApp, ai_types, keybindings, update};

// ---------------------------------------------------------------------------
// Terminal-routing helpers used by the IPC `surface.*` handlers (US-002:
// extracted from `main.rs`). Re-exported at the crate root via `main.rs` so
// older `crate::find_first_terminal` lookups keep resolving.
// ---------------------------------------------------------------------------

/// US-012 (cli-hardening-followup-2026-Q3): same-UID RCE-primitive
/// gate for `surface.send_text` and `surface.send_keystroke`.
/// Returns `true` when `PANEFLOW_IPC_SCRIPTING=1` (the documented
/// opt-in). Any other value -- including unset, empty, or `0` -- is
/// `false`. The env var is read on every call (cheap: a syscall on
/// glibc, an atomic on musl) so a user can toggle the gate mid-
/// session by re-launching `paneflow-ai-hook` with the env set,
/// without re-launching Paneflow itself. A one-time warn-log at
/// first-enable confirmation is emitted on the next first-success
/// path by the handler.
fn ipc_scripting_enabled() -> bool {
    scripting_enabled_from(std::env::var("PANEFLOW_IPC_SCRIPTING").ok().as_deref())
}

/// Pure truth table for the gate. Extracted so the rule can be
/// unit-tested without mutating the process environment (which is
/// `unsafe` on Rust 1.85+ and races with any other thread reading
/// env in parallel test mode).
fn scripting_enabled_from(value: Option<&str>) -> bool {
    matches!(value, Some("1"))
}

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
            for terminal in pane.terminals() {
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

/// Collect every terminal entity in a layout tree, in deterministic traversal
/// order. Unlike [`find_first_terminal`] this includes ALL tabs of each pane
/// (a pane can hold several terminal surfaces). Used by `surface.list`
/// (US-002, prd-pane-context-bridge).
fn collect_surface_entities(
    node: &LayoutTree,
    cx: &App,
    out: &mut Vec<gpui::Entity<TerminalView>>,
) {
    match node {
        LayoutTree::Leaf(pane) => {
            for terminal in pane.read(cx).terminals() {
                out.push(terminal.clone());
            }
        }
        LayoutTree::Container { children, .. } => {
            for child in children {
                collect_surface_entities(&child.node, cx, out);
            }
        }
    }
}

/// Per-surface metadata for `surface.list` (US-002) and name-based resolution
/// in `surface.read` / `surface.search` (US-003/US-004).
pub(crate) struct SurfaceMeta {
    pub surface_id: u64,
    pub name: String,
    pub title: String,
    pub cwd: Option<String>,
    pub cmd: Option<String>,
    pub workspace: usize,
}

/// Window a scrollback string by line for `surface.read` (US-003). `offset`
/// counts lines skipped from the most-recent end; `lines` is the window size.
/// Returns `(text, returned_line_count, total_lines, eof)`, where `eof` is
/// `true` once the window reaches the oldest retained line. Pure → unit-tested.
pub(crate) fn paginate_scrollback(
    full: &str,
    lines: usize,
    offset: usize,
) -> (String, usize, usize, bool) {
    if full.is_empty() {
        return (String::new(), 0, 0, true);
    }
    let all: Vec<&str> = full.split('\n').collect();
    let total = all.len();
    let end = total.saturating_sub(offset);
    if end == 0 {
        // Offset is past the oldest line — nothing to return, at the top.
        return (String::new(), 0, total, true);
    }
    let start = end.saturating_sub(lines);
    let window = &all[start..end];
    (window.join("\n"), window.len(), total, start == 0)
}

/// Parse the `new_name` field of a `surface.rename` request (US-013). Trims
/// whitespace, strips control characters, and caps length; an empty/absent
/// value yields `None` (clear the custom name, reverting to auto-derived).
pub(crate) fn parse_rename_name(params: &serde_json::Value) -> Option<String> {
    const MAX_NAME_LEN: usize = 64;
    let raw = params.get("new_name").and_then(|v| v.as_str())?;
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_LEN)
        .collect();
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
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
            // US-014 (telemetry): reconcile the telemetry consent state. On any
            // change, rebuild the `TelemetryClient` handle (Null ↔ Active) so
            // future emissions reflect the new choice; show a confirmation
            // toast; fire a one-time `telemetry_reenabled` breadcrumb on an
            // explicit opted-out → opted-in transition (ROPA audit trail).
            self.reconcile_telemetry_consent(&config, cx);
            // US-014 (render cache): refresh the cached config so render paths
            // pick up the reload without a per-frame `load_config()`. Last use
            // of `config` — move it in.
            self.cached_config = config;
            // US-015: push the refreshed config to every pane's tab-bar cache.
            for ws in &self.workspaces {
                ws.propagate_config(&self.cached_config, cx);
            }
            // US-016: push to the open Settings window so its render cache +
            // shortcut list reflect this external change (fixes désync).
            Self::push_config_to_settings_window(&self.cached_config, cx);
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

    /// US-016: push a refreshed config to the open Settings window (if any).
    /// The window handle isn't stored anywhere, so we locate it among the
    /// open windows by downcast — leak-free, no second `ConfigWatcher`.
    fn push_config_to_settings_window(
        config: &paneflow_config::schema::PaneFlowConfig,
        cx: &mut Context<Self>,
    ) {
        for handle in cx.windows() {
            if let Some(settings) = handle.downcast::<crate::settings::SettingsWindow>() {
                let cfg = config.clone();
                let _ = settings.update(cx, |settings, _window, cx| {
                    settings.apply_external_config(cfg, cx);
                });
            }
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

    /// Walk every workspace's layout tree and build per-surface metadata with
    /// globally-disambiguated human-readable names (US-002). Order is
    /// deterministic: workspace index, then tree-traversal order.
    pub(crate) fn collect_surface_meta(&self, cx: &App) -> Vec<SurfaceMeta> {
        // Stage 1: gather raw signals per surface in stable order, with an
        // empty `name` placeholder filled in by stage 2. `customs` tracks each
        // surface's user-assigned name (US-013) in parallel.
        let mut metas: Vec<SurfaceMeta> = Vec::new();
        let mut customs: Vec<Option<String>> = Vec::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            if let Some(root) = &ws.root {
                let mut entities = Vec::new();
                collect_surface_entities(root, cx, &mut entities);
                for entity in entities {
                    let view = entity.read(cx);
                    let ts = &view.terminal;
                    customs.push(ts.custom_name.clone());
                    metas.push(SurfaceMeta {
                        surface_id: entity.entity_id().as_u64(),
                        name: String::new(),
                        title: ts.title.clone(),
                        cwd: ts.current_cwd.clone(),
                        cmd: ts.foreground_command(),
                        workspace: ws_idx,
                    });
                }
            }
        }

        // Stage 2: a custom name (US-013) wins verbatim; otherwise derive a
        // base name. Globally resolve to unique names, then assign.
        let inputs: Vec<(Option<String>, String, Option<String>)> = metas
            .iter()
            .zip(&customs)
            .map(|(m, custom)| {
                let base = crate::workspace::surface_naming::derive_surface_base_name(
                    m.cmd.as_deref(),
                    Some(m.title.as_str()).filter(|t| !t.is_empty()),
                );
                (custom.clone(), base, m.cwd.clone())
            })
            .collect();
        for (meta, name) in
            metas
                .iter_mut()
                .zip(crate::workspace::surface_naming::resolve_surface_names(
                    &inputs,
                ))
        {
            meta.name = name;
        }
        metas
    }

    /// Resolve a `surface.*` target from the request params to a terminal
    /// entity (US-003/US-004). Precedence: explicit `surface_id` → `name` →
    /// the active workspace's first leaf. Returns a structured `-32602` error
    /// when the target is missing, unknown, or an ambiguous name.
    fn resolve_surface(
        &self,
        params: &serde_json::Value,
        cx: &App,
    ) -> Result<gpui::Entity<TerminalView>, JsonRpcError> {
        if let Some(sid) = params.get("surface_id").and_then(|s| s.as_u64()) {
            return find_terminal_by_surface_id(&self.workspaces, sid, cx).ok_or_else(|| {
                JsonRpcError::invalid_params(format!("surface_id {sid} not found"))
            });
        }
        if let Some(name) = params
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
        {
            let meta = self.collect_surface_meta(cx);
            let matches: Vec<&SurfaceMeta> = meta.iter().filter(|m| m.name == name).collect();
            match matches.as_slice() {
                [one] => {
                    let sid = one.surface_id;
                    return find_terminal_by_surface_id(&self.workspaces, sid, cx).ok_or_else(
                        || JsonRpcError::invalid_params(format!("surface '{name}' vanished")),
                    );
                }
                [] => {
                    let available: Vec<&str> = meta.iter().map(|m| m.name.as_str()).collect();
                    return Err(JsonRpcError::invalid_params(format!(
                        "no surface named '{name}'; available: [{}]",
                        available.join(", ")
                    )));
                }
                many => {
                    let ids: Vec<String> = many.iter().map(|m| m.surface_id.to_string()).collect();
                    return Err(JsonRpcError::invalid_params(format!(
                        "surface name '{name}' is ambiguous across {} surfaces (ids: {}); pass surface_id",
                        many.len(),
                        ids.join(", ")
                    )));
                }
            }
        }
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && let Some(t) = find_first_terminal(root, cx)
        {
            return Ok(t);
        }
        Err(JsonRpcError::invalid_params("no surface available"))
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
                // US-014 (cli-hardening-followup-2026-Q3):
                // canonicalize the `cwd` field before handing it to
                // `TerminalView::with_cwd`. Validation lives in the
                // free helper `canonicalize_workspace_cwd` so the
                // contract is unit-testable in isolation (see the
                // `workspace_create_rejects_nonexistent_cwd` test
                // below).
                let cwd = match params.get("cwd").and_then(|c| c.as_str()) {
                    Some(raw) => match canonicalize_workspace_cwd(raw) {
                        Ok(canonical) => Some(canonical),
                        Err(err) => return err.into_value(),
                    },
                    None => None,
                };
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
                // US-013: deferred git-stats probe off the render thread.
                Self::spawn_initial_git_stats(ws_id, ws.cwd.clone(), cx);
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
                // US-002: additive enrichment — keep the legacy root fields
                // (`pane_count`, `workspace`) for back-compat and add a
                // per-surface `surfaces` array with disambiguated names.
                let surfaces: Vec<_> = self
                    .collect_surface_meta(cx)
                    .into_iter()
                    .map(|s| {
                        serde_json::json!({
                            "surface_id": s.surface_id,
                            "name": s.name,
                            "title": s.title,
                            "cwd": s.cwd,
                            "cmd": s.cmd,
                            "workspace": s.workspace,
                        })
                    })
                    .collect();
                let count = self.active_workspace().map_or(0, |ws| ws.pane_count());
                serde_json::json!({
                    "pane_count": count,
                    "workspace": self.active_idx,
                    "surfaces": surfaces,
                })
            }
            "surface.read" => {
                // US-003: read a surface's scrollback as plain text. Read-only;
                // no scripting gate (the send_* gate guards writes, not reads).
                let terminal = match self.resolve_surface(params, cx) {
                    Ok(t) => t,
                    Err(e) => return e.into_value(),
                };
                const DEFAULT_LINES: usize = 200;
                // Mirror `extract_scrollback`'s own 4000-line cap.
                const MAX_LINES: usize = 4000;
                let lines = params
                    .get("lines")
                    .and_then(|v| v.as_u64())
                    .map(|n| (n as usize).clamp(1, MAX_LINES))
                    .unwrap_or(DEFAULT_LINES);
                let offset = params
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(0);
                let full = terminal
                    .read(cx)
                    .terminal
                    .extract_scrollback()
                    .unwrap_or_default();
                let (text, returned, total, eof) = paginate_scrollback(&full, lines, offset);
                // US-025: an offset past the oldest retained line is a client
                // error, not a silent empty read. The old `saturating_sub`
                // path returned `("", 0, total, true)`, indistinguishable from
                // "you legitimately scrolled to the very top" (offset == total).
                if offset > total {
                    return JsonRpcError::invalid_params(format!(
                        "offset {offset} out of range (total_lines={total})"
                    ))
                    .into_value();
                }
                serde_json::json!({
                    "text": text,
                    "lines": returned,
                    "total_lines": total,
                    "eof": eof,
                })
            }
            "surface.search" => {
                // US-004: locate a pattern in a surface's scrollback without
                // pulling the whole buffer. Plain-text, case-insensitive.
                let pattern = params.get("pattern").and_then(|p| p.as_str()).unwrap_or("");
                if pattern.is_empty() {
                    return JsonRpcError::invalid_params("missing or empty 'pattern' parameter")
                        .into_value();
                }
                if pattern.len() > crate::search::MAX_QUERY_LEN {
                    return JsonRpcError::invalid_params(format!(
                        "pattern exceeds {} bytes",
                        crate::search::MAX_QUERY_LEN
                    ))
                    .into_value();
                }
                let terminal = match self.resolve_surface(params, cx) {
                    Ok(t) => t,
                    Err(e) => return e.into_value(),
                };
                const DEFAULT_MAX: usize = 50;
                const HARD_MAX: usize = 1000;
                let max_matches = params
                    .get("max_matches")
                    .and_then(|v| v.as_u64())
                    .map(|n| (n as usize).clamp(1, HARD_MAX))
                    .unwrap_or(DEFAULT_MAX);
                let (matches, truncated) = terminal
                    .read(cx)
                    .terminal
                    .search_scrollback(pattern, max_matches);
                let arr: Vec<_> = matches
                    .into_iter()
                    .map(|(line, text)| serde_json::json!({"line": line, "text": text}))
                    .collect();
                serde_json::json!({"matches": arr, "truncated": truncated})
            }
            "surface.rename" => {
                // US-013: assign (or clear) a surface's custom name. `new_name`
                // is trimmed + capped; empty/absent clears it (back to the
                // auto-derived name). Targeting reuses `resolve_surface`
                // (surface_id / current name / active).
                let terminal = match self.resolve_surface(params, cx) {
                    Ok(t) => t,
                    Err(e) => return e.into_value(),
                };
                let new_name = parse_rename_name(params);
                terminal.update(cx, |view, _cx| {
                    view.terminal.custom_name = new_name.clone();
                });
                self.save_session(cx);
                cx.notify();
                serde_json::json!({"renamed": true, "name": new_name})
            }
            "surface.send_text" => {
                // US-012 (cli-hardening-followup-2026-Q3): same-UID
                // RCE primitive gate. See ipc.rs module doc for the
                // blast-radius rationale. Opt-in via env var; default
                // off. Returning JSON-RPC error code mirrors a
                // disabled-method shape so generic clients can
                // surface a clean "feature disabled" message.
                if !ipc_scripting_enabled() {
                    return JsonRpcError {
                        code: -32601,
                        message:
                            "surface.send_text disabled; set PANEFLOW_IPC_SCRIPTING=1 to enable"
                                .to_string(),
                    }
                    .into_value();
                }
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
                // US-012 (cli-hardening-followup-2026-Q3): same gate
                // as `surface.send_text`. Even when enabled, CRLF
                // bytes are rejected so a multi-keystroke payload
                // cannot smuggle a newline-terminated PTY command.
                if !ipc_scripting_enabled() {
                    return JsonRpcError {
                        code: -32601,
                        message: "surface.send_keystroke disabled; set PANEFLOW_IPC_SCRIPTING=1 to enable".to_string(),
                    }
                    .into_value();
                }
                let keystroke = params
                    .get("keystroke")
                    .and_then(|k| k.as_str())
                    .unwrap_or("");
                if keystroke.is_empty() {
                    return serde_json::json!({"error": "Missing 'keystroke' parameter"});
                }
                if keystroke.contains('\r') || keystroke.contains('\n') {
                    return JsonRpcError::invalid_params(
                        "keystroke must not contain CR or LF bytes",
                    )
                    .into_value();
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
                let tool_str = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
                    .unwrap_or("claude");
                // Validate tool name: alphanumeric + hyphens, max 64 chars
                if tool_str.len() > 64
                    || !tool_str
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                {
                    return serde_json::json!({"error": "Invalid tool name"});
                }
                let tool = ai_types::AiTool::from_name(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // session_start is a no-op on `agent_sessions`: a
                    // freshly-spawned shell with no prompt in flight
                    // should NOT show any badge in the sidebar. The
                    // first `ai.prompt_submit` / `ai.tool_use` will
                    // create the row with `AgentState::Thinking`.
                    // Stale-PID sweep covers session cleanup if the
                    // process dies before its first prompt.
                    let _ = (pid, tool, ws);
                    serde_json::json!({"registered": true})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            "ai.prompt_submit" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let pid = read_session_pid(params);
                let tool = read_tool(params);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    upsert_session_state(ws, pid, tool, ai_types::AgentState::Thinking, None);
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
                let active_tool_name = hook
                    .and_then(|h| h.get("tool_name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("tool_name").and_then(|v| v.as_str()))
                    .map(|s| s.chars().take(128).collect::<String>());
                let pid = read_session_pid(params);
                let tool = read_tool(params);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // tool_use implies the session is actively thinking —
                    // promote it (or keep it) even if the prior state was
                    // Finished from a stale prompt-end.
                    upsert_session_state(
                        ws,
                        pid,
                        tool,
                        ai_types::AgentState::Thinking,
                        active_tool_name,
                    );
                    cx.notify();
                    if !self.loader_anim_running {
                        self.start_loader_animation(cx);
                    }
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
                let pid = read_session_pid(params);
                let tool = read_tool(params);
                let message: String = hook
                    .and_then(|h| h.get("message"))
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("message").and_then(|v| v.as_str()))
                    .unwrap_or("Needs input")
                    .chars()
                    .take(512)
                    .collect();

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    upsert_session_state(
                        ws,
                        pid,
                        tool,
                        ai_types::AgentState::WaitingForInput,
                        None,
                    );
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
                let pid = read_session_pid(params);
                let tool = read_tool(params);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    upsert_session_state(ws, pid, tool, ai_types::AgentState::Finished, None);
                    cx.notify();

                    // Auto-clear the session 5 s after stop unless something
                    // else (new prompt_submit, tool_use) bumps it back to
                    // Thinking. Targets the exact (workspace_id, pid) so
                    // sibling sessions in the same workspace are untouched.
                    let ws_id = workspace_id;
                    let target_pid = pid;
                    cx.spawn(
                        async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                            smol::Timer::after(std::time::Duration::from_secs(5)).await;
                            cx.update(|cx| {
                                let _ = this.update(cx, |app, cx| {
                                    if let Some(ws) =
                                        app.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                                        && let Some(pid_key) = target_pid
                                        && matches!(
                                            ws.agent_sessions.get(&pid_key).map(|s| &s.state),
                                            Some(ai_types::AgentState::Finished)
                                        )
                                    {
                                        ws.agent_sessions.remove(&pid_key);
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
                let pid = read_session_pid(params);
                let tool = ai_types::AiTool::from_name(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // Prefer exact PID removal; fall back to removing one
                    // session matching the tool name (back-compat for older
                    // shims that didn't carry `pid` on session_end). Last
                    // resort keeps `agent_sessions` consistent with the
                    // pre-refactor "one session per tool" assumption.
                    let removed = if let Some(p) = pid
                        && ws.agent_sessions.remove(&p).is_some()
                    {
                        true
                    } else {
                        let pid_to_remove = ws
                            .agent_sessions
                            .iter()
                            .find(|(_, s)| s.tool == tool)
                            .map(|(k, _)| *k);
                        if let Some(k) = pid_to_remove {
                            ws.agent_sessions.remove(&k);
                            true
                        } else {
                            false
                        }
                    };
                    if removed {
                        cx.notify();
                    }
                    serde_json::json!({"cleared": removed})
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
// AI session helpers (multi-session refactor)
// ---------------------------------------------------------------------------

/// Read the session PID from an `ai.*` IPC param object. Returns `None`
/// when the field is missing or zero — older shims (pre multi-session
/// refactor) don't include `pid` on every lifecycle frame, so the
/// caller must tolerate `None` and degrade to tool-name-based matching.
fn read_session_pid(params: &serde_json::Value) -> Option<u32> {
    params
        .get("pid")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&p| p > 0)
}

/// Read the `tool` field from an `ai.*` IPC param object, falling back
/// to `hook_payload.tool`, defaulting to Claude (matches the server's
/// historical behavior for legacy shims that don't stamp the field).
fn read_tool(params: &serde_json::Value) -> ai_types::AiTool {
    let hook = params.get("hook_payload");
    let tool_str = params
        .get("tool")
        .and_then(|v| v.as_str())
        .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
        .unwrap_or("claude");
    ai_types::AiTool::from_name(tool_str)
}

/// US-026: floor of the reserved synthetic-PID namespace. Legacy `ai.*` frames
/// that carry no real `pid` get a placeholder key in `[BASE, u32::MAX]`, a band
/// no real OS PID reaches on any supported platform, so the two keyspaces never
/// overlap.
const SYNTHETIC_SESSION_PID_BASE: u32 = 0xFFFF_0000;

/// Insert or update a session in `ws.agent_sessions`. When `pid` is
/// known, the session is keyed by PID (the desired path — supports
/// many concurrent sessions of the same tool). When `pid` is `None`
/// (older shim), falls back to matching any existing session of the
/// same tool and updating it in place; if none exists, a synthetic
/// PID slot is allocated from the negative u32 space so the row is
/// still tracked. This keeps the UI consistent during a rolling shim
/// upgrade where some frames carry `pid` and others don't.
fn upsert_session_state(
    ws: &mut crate::workspace::Workspace,
    pid: Option<u32>,
    tool: ai_types::AiTool,
    state: ai_types::AgentState,
    active_tool_name: Option<String>,
) {
    let key = match pid {
        Some(p) => p,
        None => {
            if let Some((existing_pid, _)) = ws.agent_sessions.iter().find(|(_, s)| s.tool == tool)
            {
                *existing_pid
            } else {
                // US-026: allocate from a reserved high band that is disjoint
                // from every supported platform's real PID range (Linux pid_max
                // 4 194 304; macOS 99 999; Windows DWORDs are multiples of 4 and
                // never approach this in practice). Treating this band as a
                // separate synthetic namespace keeps a legacy placeholder from
                // being confused with — or clobbered by — a real OS PID. The
                // walk stops at the band floor instead of descending into the
                // real-PID range.
                let mut k: u32 = u32::MAX;
                while k > SYNTHETIC_SESSION_PID_BASE && ws.agent_sessions.contains_key(&k) {
                    k -= 1;
                }
                k
            }
        }
    };

    ws.agent_sessions
        .entry(key)
        .and_modify(|s| {
            s.tool = tool;
            s.state = state.clone();
            s.active_tool_name = active_tool_name.clone();
        })
        .or_insert_with(|| {
            let mut session = ai_types::AgentSession::new(tool, state);
            session.active_tool_name = active_tool_name;
            session
        });
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

/// US-014 (cli-hardening-followup-2026-Q3): validate and canonicalize the
/// `cwd` field of a `workspace.create` IPC request.
///
/// US-026: this is **not** a confinement jail. A same-UID client may
/// legitimately open a workspace at any directory it can already reach, and
/// `canonicalize` resolves `../` and symlinks to wherever they actually point
/// (`"../../etc"` → `/etc`) without restricting the result to any root — so it
/// does not, and cannot, prevent "walking outside the workspace". Its job is
/// narrower: turn a relative or symlinked path into a concrete absolute one and
/// reject upfront the inputs that would otherwise fail confusingly at PTY
/// spawn — a path that does not exist or is unreadable, a path containing NUL
/// bytes (rejected by `canonicalize` itself; most OSes would silently truncate
/// it), or a path to a regular file (the first chdir would fail) — each with a
/// structured `-32602` so the client knows the request was refused.
///
/// Successful canonicalization is logged at `info!` for audit trail
/// (relative-path resolution and symlink traversal visibility).
pub(crate) fn canonicalize_workspace_cwd(raw: &str) -> Result<std::path::PathBuf, JsonRpcError> {
    let canonical = std::fs::canonicalize(raw).map_err(|e| {
        JsonRpcError::invalid_params(format!("cwd does not exist or is unreadable: {raw} ({e})"))
    })?;
    let meta = std::fs::metadata(&canonical).map_err(|e| {
        JsonRpcError::invalid_params(format!("cwd metadata read failed for {raw}: {e}"))
    })?;
    if !meta.is_dir() {
        return Err(JsonRpcError::invalid_params(format!(
            "cwd is not a directory: {raw}"
        )));
    }
    log::info!("ipc::workspace.create: canonical cwd resolved {raw:?} -> {canonical:?}");
    Ok(canonical)
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

    // -----------------------------------------------------------------
    // US-012 (cli-hardening-followup-2026-Q3) — surface.send_text gate
    // -----------------------------------------------------------------

    /// AC #6: `surface.send_text` MUST be gated behind
    /// `PANEFLOW_IPC_SCRIPTING=1`. The handler is on `PaneFlowApp`
    /// and not driveable from a unit test, so we cover the contract
    /// in two pieces: (a) the pure `scripting_enabled_from()` truth
    /// table -- no env mutation needed, race-free under `cargo
    /// test`'s default multi-threaded harness; (b) the exact
    /// `JsonRpcError` shape the handler constructs at
    /// `ipc_handler.rs:451-459`.
    #[test]
    fn send_text_rejected_when_scripting_disabled() {
        // (a) Pure truth table: only the literal "1" enables.
        assert!(
            !super::scripting_enabled_from(None),
            "unset env must read as disabled"
        );
        assert!(
            !super::scripting_enabled_from(Some("")),
            "empty string must read as disabled"
        );
        assert!(
            !super::scripting_enabled_from(Some("0")),
            "explicit 0 must read as disabled"
        );
        assert!(
            !super::scripting_enabled_from(Some("true")),
            "truthy strings other than \"1\" must read as disabled"
        );
        assert!(
            super::scripting_enabled_from(Some("1")),
            "the documented opt-in value must enable"
        );

        // (b) JSON-RPC envelope shape returned by the handler.
        let err = JsonRpcError {
            code: -32601,
            message: "surface.send_text disabled; set PANEFLOW_IPC_SCRIPTING=1 to enable"
                .to_string(),
        };
        let envelope = promote_response(err.into_value(), serde_json::json!(42));
        assert_eq!(envelope["error"]["code"], -32601);
        assert!(envelope.get("result").is_none());
        assert_eq!(envelope["id"], 42);
    }

    /// AC #4 corollary: even when scripting IS enabled,
    /// `surface.send_keystroke` must reject CR/LF bytes with
    /// `-32602 Invalid params` to defuse the CRLF-injection bypass.
    /// Mirrors the rejection at `ipc_handler.rs:503-508`.
    #[test]
    fn send_keystroke_crlf_rejection_shape() {
        let err = JsonRpcError::invalid_params("keystroke must not contain CR or LF bytes");
        let envelope = promote_response(err.into_value(), serde_json::json!("req-1"));
        assert_eq!(envelope["error"]["code"], JsonRpcError::INVALID_PARAMS);
        assert!(
            envelope["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("CR or LF"),
        );
    }

    // -----------------------------------------------------------------
    // US-014 (cli-hardening-followup-2026-Q3) — workspace.create cwd
    // canonicalization
    // -----------------------------------------------------------------

    /// AC #6: a non-existent `cwd` must surface as JSON-RPC `-32602
    /// Invalid params` without attempting to spawn a PTY. Exercises
    /// the free helper `canonicalize_workspace_cwd` directly so the
    /// contract is verified in isolation from `PaneFlowApp`.
    #[test]
    fn workspace_create_rejects_nonexistent_cwd() {
        let bogus = "/nonexistent/path/paneflow-us-014-fixture-xyz";
        assert!(
            !std::path::Path::new(bogus).exists(),
            "fixture precondition: path must not exist"
        );
        let err = super::canonicalize_workspace_cwd(bogus).expect_err("must reject missing cwd");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.contains("does not exist"),
            "error must mention non-existence, got: {}",
            err.message
        );
    }

    /// AC #3: a `cwd` that resolves to a regular file (not a
    /// directory) must surface as `-32602 cwd is not a directory`.
    #[test]
    fn workspace_create_rejects_file_cwd() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        let path = tmp.path().to_string_lossy().into_owned();
        let err =
            super::canonicalize_workspace_cwd(&path).expect_err("must reject regular-file cwd");
        assert_eq!(err.code, JsonRpcError::INVALID_PARAMS);
        assert!(
            err.message.contains("not a directory"),
            "error must mention not-a-directory, got: {}",
            err.message
        );
    }

    /// Sanity: a real, existing directory must canonicalize successfully.
    #[test]
    fn workspace_create_accepts_existing_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let resolved = super::canonicalize_workspace_cwd(tmp.path().to_str().expect("utf-8 path"))
            .expect("real dir must canonicalize");
        // canonicalize resolves to an absolute path.
        assert!(resolved.is_absolute());
        assert!(resolved.is_dir());
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

    // -----------------------------------------------------------------
    // US-003 (prd-pane-context-bridge) — surface.read pagination
    // -----------------------------------------------------------------

    #[test]
    fn paginate_empty_buffer_is_eof() {
        assert_eq!(
            super::paginate_scrollback("", 200, 0),
            (String::new(), 0, 0, true)
        );
    }

    #[test]
    fn paginate_default_window_returns_tail() {
        // offset 0 returns the most-recent `lines` lines, not at eof.
        let (text, returned, total, eof) = super::paginate_scrollback("a\nb\nc\nd\ne", 2, 0);
        assert_eq!(text, "d\ne");
        assert_eq!(returned, 2);
        assert_eq!(total, 5);
        assert!(!eof);
    }

    #[test]
    fn paginate_offset_walks_back_up_the_buffer() {
        // Skip the 2 most-recent lines, then take 2 → "b\nc".
        let (text, returned, total, eof) = super::paginate_scrollback("a\nb\nc\nd\ne", 2, 2);
        assert_eq!(text, "b\nc");
        assert_eq!(returned, 2);
        assert_eq!(total, 5);
        assert!(!eof);
    }

    #[test]
    fn paginate_window_covering_whole_buffer_is_eof() {
        let (text, returned, total, eof) = super::paginate_scrollback("a\nb\nc", 10, 0);
        assert_eq!(text, "a\nb\nc");
        assert_eq!(returned, 3);
        assert_eq!(total, 3);
        assert!(eof, "reaching the oldest line sets eof");
    }

    #[test]
    fn paginate_offset_past_top_returns_empty_at_eof() {
        let (text, returned, total, eof) = super::paginate_scrollback("a\nb\nc", 2, 10);
        assert!(text.is_empty());
        assert_eq!(returned, 0);
        assert_eq!(total, 3);
        assert!(eof);
    }

    #[test]
    fn paginate_total_drives_us025_offset_guard() {
        // US-025: the surface.read handler rejects `offset > total` with a
        // structured -32602. `offset == total` is the valid "scrolled to the
        // very top" boundary (empty window, eof) and must NOT be rejected.
        // paginate exposes the true `total` either way, so the guard can fire.
        let (_, _, total_at_top, eof_at_top) = super::paginate_scrollback("a\nb\nc", 2, 3);
        assert_eq!(total_at_top, 3);
        assert!(eof_at_top);
        assert!(3 <= total_at_top, "offset == total is in range (boundary)");

        let (_, _, total_past, _) = super::paginate_scrollback("a\nb\nc", 2, 4);
        assert_eq!(total_past, 3);
        assert!(
            4 > total_past,
            "offset > total is out of range → handler returns -32602"
        );
    }

    // -----------------------------------------------------------------
    // US-013 (prd-pane-context-bridge) — surface.rename name parsing
    // -----------------------------------------------------------------

    #[test]
    fn parse_rename_name_trims_and_accepts() {
        let p = serde_json::json!({"new_name": "  build logs  "});
        assert_eq!(super::parse_rename_name(&p).as_deref(), Some("build logs"));
    }

    #[test]
    fn parse_rename_name_empty_or_absent_clears() {
        assert_eq!(super::parse_rename_name(&serde_json::json!({})), None);
        assert_eq!(
            super::parse_rename_name(&serde_json::json!({"new_name": "   "})),
            None
        );
        assert_eq!(
            super::parse_rename_name(&serde_json::json!({"new_name": ""})),
            None
        );
    }

    #[test]
    fn parse_rename_name_strips_control_chars_and_caps_length() {
        let p = serde_json::json!({"new_name": "ab\ncd\u{7}ef"});
        assert_eq!(super::parse_rename_name(&p).as_deref(), Some("abcdef"));
        let long = "x".repeat(200);
        let p = serde_json::json!({ "new_name": long });
        assert_eq!(super::parse_rename_name(&p).map(|s| s.len()), Some(64));
    }
}
