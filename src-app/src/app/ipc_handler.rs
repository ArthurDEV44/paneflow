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

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, Focusable};
use paneflow_config::schema::LayoutNode;

use crate::agent_launcher::TerminalAgent;
use crate::ai_types::AgentSession;
use crate::layout::LayoutTree;
use crate::layout::{MAX_PANES, SplitDirection};
use crate::pane::Pane;
use crate::terminal::TerminalView;
use crate::workspace::{MAX_WORKSPACES, Workspace, next_workspace_id};
use crate::{PaneFlowApp, ai_types, keybindings, update};

/// Prompt-prefill readiness window for `workspace.up` (US-010,
/// prd-cli-agent-orchestration).
///
/// The agent CLI's input box is not ready the instant its launch command is
/// written, so a too-early `send_text` is lost into a not-ready buffer. A
/// single fixed delay (the prior approach) silently lost prompts whenever an
/// agent started slower than the timer — the dominant failure at N-pane scale
/// (PRD Risk #2). Instead the prefill waits a `FLOOR` (preserving the prior
/// fast-path behaviour, by which the launch-command echo has settled), then
/// EXTENDS while the pane is still actively producing output (its
/// `output_generation` keeps advancing — the agent is still painting), firing
/// once that output goes idle ("settled") or `MAX` is hit. On `MAX` without a
/// settle the prompt is injected best-effort with a warning (AC4). The wait is
/// bounded and runs concurrently per pane (one detached task each), so an
/// N-pane `up` still prefills in ~one window, not N.
const UP_PREFILL_FLOOR: Duration = Duration::from_millis(1800);
const UP_PREFILL_MAX: Duration = Duration::from_millis(8000);
const UP_PREFILL_POLL: Duration = Duration::from_millis(200);

/// A validated pane plan for `workspace.up`: the cwd is already canonicalized,
/// so the spawn phase is infallible with respect to directories (US-012).
struct PlannedPane {
    cwd: Option<PathBuf>,
    command: Option<String>,
    prompt: Option<String>,
    env: Option<HashMap<String, String>>,
    focus: bool,
}

/// Parse a JSON `{ "K": "V", … }` object into an env map, dropping non-string
/// values. Returns `None` for absent/empty so the global `terminal.env` default
/// still applies underneath (parity with `SurfaceDefinition::env`).
fn parse_env_object(value: Option<&serde_json::Value>) -> Option<HashMap<String, String>> {
    let obj = value?.as_object()?;
    let map: HashMap<String, String> = obj
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect();
    (!map.is_empty()).then_some(map)
}

/// Build the layout tree for `workspace.up` from a preset name. Mirrors the
/// keyboard layout presets (`handle_layout_*`): `even_h` = side by side
/// (Vertical divider), `even_v` = stacked (Horizontal divider), `main_vertical`
/// = the focused pane at 60% with the rest stacked, `tiled` = tmux grid.
/// Unknown names fall back to `even_h`.
fn build_up_layout(preset: &str, panes: Vec<Entity<Pane>>, focus_idx: usize) -> Option<LayoutTree> {
    match preset {
        "even_v" => LayoutTree::from_panes_equal(SplitDirection::Horizontal, panes),
        "main_vertical" => {
            let main = panes.get(focus_idx).or_else(|| panes.first())?.clone();
            let others: Vec<_> = panes.into_iter().filter(|p| *p != main).collect();
            LayoutTree::main_vertical(main, others)
        }
        "tiled" => LayoutTree::tiled(panes),
        _ => LayoutTree::from_panes_equal(SplitDirection::Vertical, panes),
    }
}

/// EP-004 US-020 — fire a best-effort OS desktop notification on agent turn-end,
/// but only when Paneflow is NOT the focused window (the repositioning intent is
/// "notify on turn-end, not while you're watching"). Runs the platform notifier
/// as a bounded subprocess on the background executor: no new crate dependency,
/// never blocks the render thread, and a missing notifier just fails silently.
fn fire_turn_end_notification(workspace_title: &str, executor: gpui::BackgroundExecutor) {
    fire_desktop_notification(format!("{workspace_title}: agent finished"), executor);
}

/// US-016 (orchestration-v2): the WaitingForInput desktop notification
/// carries the agent's actual question, not a generic "needs input" — the
/// user decides from the notification whether it's worth coming back.
fn fire_attention_notification(
    workspace_title: &str,
    message: Option<&str>,
    executor: gpui::BackgroundExecutor,
) {
    let body = attention_notification_body(workspace_title, message);
    fire_desktop_notification(body, executor);
}

/// US-016/US-020: bound + sanitize an agent question before it is stored on
/// the session (single choke point — peek badge, expanded panel and desktop
/// notification all read the stored value). 512-char truncation, then the
/// shared markdown bidi/zero-width strip: an RLO in untrusted hook text could
/// otherwise visually reverse the displayed question ("texte brut sanitizé",
/// US-020 AC5 — same precedent as the markdown viewer). Pure → unit-tested.
fn sanitize_notification_message(raw: &str) -> String {
    crate::markdown::strip_bidi_zero_width(raw.chars().take(512).collect())
}

/// Pure body composition (unit-tested): "title: question", falling back to
/// the legacy generic body when the hook carried no message — never an
/// empty body (US-016 AC3).
fn attention_notification_body(workspace_title: &str, message: Option<&str>) -> String {
    match message.filter(|m| !m.trim().is_empty()) {
        Some(m) => format!("{workspace_title}: {m}"),
        None => format!("{workspace_title}: agent needs input"),
    }
}

/// EP-004 US-010: the `Errored` desktop notification — distinct body so a
/// crash never reads as "agent finished". Same choke point + window-focus
/// gate as the other two notification kinds.
fn fire_agent_exit_notification(
    workspace_title: &str,
    exit_code: i32,
    executor: gpui::BackgroundExecutor,
) {
    fire_desktop_notification(
        agent_exit_notification_body(workspace_title, exit_code),
        executor,
    );
}

/// Pure body composition (unit-tested) — PRD US-010 AC #6.
fn agent_exit_notification_body(workspace_title: &str, exit_code: i32) -> String {
    format!("{workspace_title}: agent exited (exit {exit_code})")
}

/// EP-004 US-011: the `Stalled` desktop notification. Called by the
/// periodic sweep (`event_handlers.rs::sweep_stale_pids`) on the ONE
/// `Thinking → Stalled` transition of a stall episode — the dedup is
/// structural (the sweep only flips `Thinking`, so a stalled session can't
/// re-trigger until a hook event revives it first).
pub(crate) fn fire_stalled_notification(
    workspace_title: &str,
    silent_secs: u64,
    executor: gpui::BackgroundExecutor,
) {
    fire_desktop_notification(
        stalled_notification_body(workspace_title, silent_secs),
        executor,
    );
}

/// Pure body composition (unit-tested) — PRD US-011 AC #4.
fn stalled_notification_body(workspace_title: &str, silent_secs: u64) -> String {
    format!("{workspace_title}: agent silent for {silent_secs} s")
}

/// Shared desktop-notification path (turn-end + attention): fires only when
/// the Paneflow window is NOT focused; body sanitization stays in the per-OS
/// command builders (`--` on Linux, AppleScript strip on macOS, Windows
/// stub).
fn fire_desktop_notification(body: String, executor: gpui::BackgroundExecutor) {
    if crate::agents::notifications::window_active() {
        return;
    }
    let Some(command) = turn_end_notify_command(&body) else {
        return;
    };
    executor
        .spawn(async move {
            let _ = paneflow_process::run_with_timeout(command, Duration::from_secs(10), 64 * 1024);
        })
        .detach();
}

/// Strip control characters from an untrusted notification body before it is
/// embedded in an AppleScript string literal. AppleScript literals have no
/// `\n`/`\r` escape and cannot span a raw newline, so an un-stripped control
/// char in a crafted workspace title would break the literal (CWE-78 — a parse
/// error, not RCE, but the brittleness is removed at the source). Pure and not
/// `cfg`-gated so it is unit-testable on every host; only the macOS notifier
/// consumes it, hence the off-macOS dead-code allow.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn sanitize_applescript_body(body: &str) -> String {
    body.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(target_os = "linux")]
fn turn_end_notify_command(body: &str) -> Option<std::process::Command> {
    let mut command = std::process::Command::new("notify-send");
    // `--` terminates GLib option parsing: a workspace title beginning with `-`
    // must be taken as the summary/body, never mis-parsed as a notify-send flag.
    // `--icon` (before `--`) gives the notification PaneFlow's icon instead of a
    // generic glyph; the `paneflow` icon name matches the `.desktop` Icon= key.
    command
        .arg("--app-name=Paneflow")
        .arg("--icon=paneflow")
        .arg("--")
        .arg("Paneflow")
        .arg(body);
    Some(command)
}

#[cfg(target_os = "macos")]
fn turn_end_notify_command(body: &str) -> Option<std::process::Command> {
    // `body` is embedded in an AppleScript string literal. Strip control chars
    // FIRST (a raw newline cannot live in the literal — no `\n` escape — so a
    // crafted title would break it, CWE-78), then escape backslash and
    // double-quote. Args are passed directly (no shell), so this is sufficient.
    let escaped = sanitize_applescript_body(body)
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = format!("display notification \"{escaped}\" with title \"Paneflow\"");
    let mut command = std::process::Command::new("osascript");
    command.arg("-e").arg(script);
    Some(command)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn turn_end_notify_command(_body: &str) -> Option<std::process::Command> {
    // Windows: no dependency-free toast path yet (BurntToast / WinRT both add
    // weight). Documented stub — no notification fired (US-020 AC allows this).
    None
}

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

/// Parse the optional `managed_worktree` object a `workspace.up` pane spec or
/// a spawn-capable `surface.split` carries (EP-002/EP-003, orchestration-v2):
/// the CLI created a git worktree for this pane and hands the ownership
/// record over so the workspace tears it down at close (US-009). `path` and
/// `repo_root` are both required — anything else is ignored (no record, no
/// teardown: fail toward "never touch what we can't prove we own").
fn parse_managed_worktree(
    value: Option<&serde_json::Value>,
) -> Option<crate::workspace::worktree::ManagedWorktree> {
    let mw = value.filter(|v| !v.is_null())?;
    let path = mw.get("path").and_then(|p| p.as_str()).unwrap_or("");
    let repo_root = mw.get("repo_root").and_then(|p| p.as_str()).unwrap_or("");
    if path.is_empty() || repo_root.is_empty() {
        return None;
    }
    Some(crate::workspace::worktree::ManagedWorktree {
        path: PathBuf::from(path),
        repo_root: PathBuf::from(repo_root),
        branch: mw
            .get("branch")
            .and_then(|b| b.as_str())
            .unwrap_or_default()
            .to_string(),
        teardown: crate::workspace::worktree::TeardownPolicy::parse(
            mw.get("teardown").and_then(|t| t.as_str()).unwrap_or(""),
        ),
    })
}

/// Locate the pane (and tab index) hosting a surface, across all workspaces.
/// Returns `(workspace_index, pane, tab_index)`. Unlike
/// [`find_terminal_by_surface_id`] this yields the *container*, which is what
/// `surface.focus` (focus + tab activation) and the targeted `surface.split`
/// (split at that leaf) need (US-001/US-002, prd-orchestration-v2).
pub(crate) fn find_pane_by_surface_id(
    workspaces: &[Workspace],
    surface_id: u64,
    cx: &App,
) -> Option<(usize, gpui::Entity<Pane>, usize)> {
    for (ws_idx, ws) in workspaces.iter().enumerate() {
        if let Some(root) = &ws.root
            && let Some((pane, tab_idx)) = find_pane_in_tree(root, surface_id, cx)
        {
            return Some((ws_idx, pane, tab_idx));
        }
    }
    None
}

fn find_pane_in_tree(
    node: &LayoutTree,
    surface_id: u64,
    cx: &App,
) -> Option<(gpui::Entity<Pane>, usize)> {
    match node {
        LayoutTree::Leaf(pane) => {
            // Index into `tabs` (not the terminals-only iterator): markdown
            // tabs interleave, and `selected_idx` addresses the full tab list.
            let tab_idx = pane.read(cx).tabs.iter().position(|tab| {
                tab.as_terminal()
                    .is_some_and(|t| t.entity_id().as_u64() == surface_id)
            })?;
            Some((pane.clone(), tab_idx))
        }
        LayoutTree::Container { children, .. } => children
            .iter()
            .find_map(|child| find_pane_in_tree(&child.node, surface_id, cx)),
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

/// EP-001 US-001 (agent-control-plane): one workspace's fleet inputs, borrowed
/// for the pure [`build_fleet_rows`]. Keeps row-building free of GPUI so it can
/// be unit-tested directly.
struct WsFleet<'a> {
    idx: usize,
    sessions: &'a HashMap<u32, AgentSession>,
    detected: &'a HashSet<String>,
}

/// EP-001 US-001: build the sorted `fleet.list` agent rows. Pure.
///
/// Hooked sessions are emitted with full state (`hooked: true`); then every
/// detected binary whose tool has NO hook session is appended once as
/// `unknown_running` (`hooked: false`) so the 6 hookless agents stay visible.
/// Sorted by `(workspace, tool display_rank, pid)` for a stable order across the
/// `HashMap`'s nondeterministic iteration.
fn build_fleet_rows(
    workspaces: &[WsFleet],
    name_by_sid: &HashMap<u64, String>,
    now: std::time::Instant,
) -> Vec<serde_json::Value> {
    let mut rows: Vec<(usize, usize, u32, serde_json::Value)> = Vec::new();
    for ws in workspaces {
        let mut tools_seen: HashSet<TerminalAgent> = HashSet::new();
        for (pid, s) in ws.sessions {
            tools_seen.insert(s.tool);
            let surface_name = s
                .surface_id
                .and_then(|sid| name_by_sid.get(&sid).map(String::as_str));
            rows.push((
                ws.idx,
                s.tool.display_rank(),
                *pid,
                serde_json::json!({
                    "pid": *pid,
                    "tool": s.tool.binary(),
                    "state": s.state.wire_str(),
                    "hooked": true,
                    "surface_id": s.surface_id,
                    "surface_name": surface_name,
                    "workspace": ws.idx,
                    "active_tool_name": s.active_tool_name,
                    "message": s.message,
                    "waiting_ms": s
                        .waiting_since
                        .map(|w| now.saturating_duration_since(w).as_millis() as u64),
                    "idle_ms": now.saturating_duration_since(s.last_activity).as_millis() as u64,
                }),
            ));
        }
        // Detected-but-unhooked: a binary the /proc scan saw whose tool has no
        // hook session. Appended last within the workspace (pid sentinel).
        let mut unhooked: Vec<TerminalAgent> = ws
            .detected
            .iter()
            .filter_map(|b| TerminalAgent::from_binary(b))
            .filter(|t| !tools_seen.contains(t))
            .collect();
        unhooked.sort_by_key(|t| t.display_rank());
        for tool in unhooked {
            rows.push((
                ws.idx,
                tool.display_rank(),
                u32::MAX,
                serde_json::json!({
                    "pid": serde_json::Value::Null,
                    "tool": tool.binary(),
                    "state": "unknown_running",
                    "hooked": false,
                    "surface_id": serde_json::Value::Null,
                    "surface_name": serde_json::Value::Null,
                    "workspace": ws.idx,
                    "active_tool_name": serde_json::Value::Null,
                    "message": serde_json::Value::Null,
                    "waiting_ms": serde_json::Value::Null,
                    "idle_ms": serde_json::Value::Null,
                }),
            ));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    rows.into_iter().map(|(_, _, _, v)| v).collect()
}

/// EP-001 US-002 (agent-control-plane): the `surface.status` response for a
/// pane, given the session living in it (if any). Pure. `idle` when `None`.
fn surface_status_value(
    sid: u64,
    session: Option<&AgentSession>,
    output_generation: u64,
    now: std::time::Instant,
) -> serde_json::Value {
    match session {
        Some(s) => serde_json::json!({
            "surface_id": sid,
            "state": s.state.wire_str(),
            "tool": s.tool.binary(),
            "active_tool_name": s.active_tool_name,
            "message": s.message,
            "waiting_ms": s
                .waiting_since
                .map(|w| now.saturating_duration_since(w).as_millis() as u64),
            "idle_ms": now.saturating_duration_since(s.last_activity).as_millis() as u64,
            "output_generation": output_generation,
        }),
        None => serde_json::json!({
            "surface_id": sid,
            "state": "idle",
            "output_generation": output_generation,
        }),
    }
}

impl PaneFlowApp {
    pub(crate) fn process_ipc_requests(&mut self, cx: &mut Context<Self>) {
        while let Ok(req) = self.ipc_rx.try_recv() {
            // U-053: the socket thread bounds each request at 5 s. If it
            // already timed out it set `cancelled` and returned an error to
            // the client; skip the request entirely so a slow non-idempotent
            // mutation (workspace.create, surface.split) doesn't run after the
            // client gave up — a retry would otherwise create duplicate
            // workspaces/panes. The dropped response channel makes a late
            // result a no-op regardless, so skipping only avoids wasted work
            // and the duplicate side effect.
            if req.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                continue;
            }
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
            crate::theme::sync_markdown_global_theme(cx);
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
            // The embedded settings page reads `self.cached_config` directly and
            // its shortcut list is refreshed above (`effective_shortcuts`), so an
            // external `paneflow.json` edit reflects without any extra push.
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
            crate::theme::sync_markdown_global_theme(cx);
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
        if self.self_update.update_status.is_some() {
            return; // Already resolved
        }
        let status = self
            .self_update
            .pending_update
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(status) = status
            && !matches!(status, update::checker::UpdateStatus::Checking)
        {
            self.self_update.update_status = Some(status);
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

    /// `workspace.up` — materialize a declarative multi-pane agent workspace in
    /// one call (US-008/US-009/US-010, prd-cli-agent-orchestration). Unlike
    /// `workspace.create` + `layout`, this honors a per-pane cwd / launch
    /// command / prompt: each pane spawns in its own directory, optionally runs
    /// an agent CLI, and optionally gets a prompt pre-filled (never submitted).
    ///
    /// Security: same-UID peer-cred is the gate (the socket is 0600 + peer-UID).
    /// Launching a CLI here is no more privileged than the user's own shell, and
    /// every pane is freshly created by this call (no injection into a
    /// pre-existing foreign agent), so it does NOT require the
    /// `PANEFLOW_IPC_SCRIPTING` gate `surface.send_text` carries — that gate
    /// guards lateral injection into another agent's live session.
    ///
    /// Atomic: every pane's cwd is canonicalized BEFORE anything spawns, so a
    /// bad directory returns -32602 with no half-built workspace (US-012).
    fn handle_workspace_up(
        &mut self,
        params: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> serde_json::Value {
        if self.workspaces.len() >= MAX_WORKSPACES {
            return JsonRpcError::invalid_params("Workspace limit reached").into_value();
        }
        let name = params
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("Workspace")
            .to_string();
        let preset = params
            .get("layout")
            .and_then(|l| l.as_str())
            .unwrap_or("even_h");
        let pane_specs = match params.get("panes").and_then(|p| p.as_array()) {
            Some(a) if !a.is_empty() => a,
            _ => {
                return JsonRpcError::invalid_params("`panes` must be a non-empty array")
                    .into_value();
            }
        };
        if pane_specs.len() > MAX_PANES {
            return JsonRpcError::invalid_params(format!(
                "layout exceeds maximum pane count ({MAX_PANES})"
            ))
            .into_value();
        }

        // Phase 1 (no mutation): validate + canonicalize every cwd up-front so a
        // bad path fails atomically with -32602 before any pane spawns (US-012).
        // EP-002 (orchestration-v2): collect the worktrees the CLI created for
        // these panes — the workspace records ownership so close tears them
        // down (US-009) and session restore keeps the record across a crash.
        let mut managed_worktrees: Vec<crate::workspace::worktree::ManagedWorktree> = Vec::new();
        let mut planned: Vec<PlannedPane> = Vec::with_capacity(pane_specs.len());
        for (i, spec) in pane_specs.iter().enumerate() {
            if let Some(mw) = parse_managed_worktree(spec.get("managed_worktree")) {
                managed_worktrees.push(mw);
            }
            let cwd = match spec.get("cwd").and_then(|c| c.as_str()) {
                Some(raw) => match canonicalize_workspace_cwd(raw) {
                    Ok(canonical) => Some(canonical),
                    Err(_) => {
                        return JsonRpcError::invalid_params(format!(
                            "pane {i}: cwd '{raw}' does not exist or is not a directory"
                        ))
                        .into_value();
                    }
                },
                None => None,
            };
            planned.push(PlannedPane {
                cwd,
                command: spec
                    .get("command")
                    .and_then(|c| c.as_str())
                    .map(str::to_string),
                prompt: spec
                    .get("prompt")
                    .and_then(|c| c.as_str())
                    .map(str::to_string),
                env: parse_env_object(spec.get("env")),
                focus: spec.get("focus").and_then(|f| f.as_bool()).unwrap_or(false),
            });
        }

        // Phase 2: spawn every pane (cwd + env honored). `self.workspaces` is
        // untouched until the tree is built, so a failed layout strands nothing.
        let ws_id = next_workspace_id();
        let mut panes: Vec<Entity<Pane>> = Vec::with_capacity(planned.len());
        let mut launches: Vec<(Entity<TerminalView>, Option<String>, Option<String>)> =
            Vec::with_capacity(planned.len());
        for pp in &planned {
            let terminal = cx.new(|cx| {
                TerminalView::with_cwd_and_env(ws_id, pp.cwd.clone(), None, pp.env.clone(), cx)
            });
            let pane = self.create_pane(terminal.clone(), ws_id, cx);
            launches.push((terminal, pp.command.clone(), pp.prompt.clone()));
            panes.push(pane);
        }

        let focus_idx = planned.iter().position(|p| p.focus).unwrap_or(0);
        let Some(tree) = build_up_layout(preset, panes, focus_idx) else {
            return JsonRpcError::invalid_params("could not build layout from panes").into_value();
        };

        let ws_cwd = planned
            .iter()
            .find_map(|p| p.cwd.clone())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut ws = Workspace::with_layout_and_id(ws_id, &name, ws_cwd, tree);
        ws.managed_worktrees = managed_worktrees;
        self.watch_git_dir(&ws);
        Self::spawn_initial_git_stats(ws_id, ws.cwd.clone(), cx);
        self.workspaces.push(ws);
        let idx = self.workspaces.len() - 1;
        self.active_idx = idx;

        // Phase 3: launch each agent (typed-ahead into the shell is fine) and
        // schedule the prompt prefill after a bounded readiness wait. The
        // prompt is written WITHOUT a carriage return — human-in-loop: the user
        // reviews and submits it themselves (US-010).
        // EP-003 (orchestration-v2): collect the spawned terminals' surface ids
        // in pane order — `paneflow flow` maps them back to its DAG steps.
        let mut surface_ids: Vec<u64> = Vec::with_capacity(launches.len());
        for (i, (terminal, command, prompt)) in launches.into_iter().enumerate() {
            surface_ids.push(terminal.entity_id().as_u64());
            if let Some(cmd) = command.filter(|c| !c.is_empty()) {
                terminal.read(cx).send_command(&cmd);
            }
            if let Some(prompt) = prompt.filter(|p| !p.is_empty()) {
                Self::schedule_prompt_prefill(&terminal, prompt, i, cx);
            }
        }

        let panes_n = self.active_workspace().map_or(0, |ws| ws.pane_count());
        self.save_session(cx);
        cx.notify();
        serde_json::json!({
            "index": idx, "title": name, "panes": panes_n, "surface_ids": surface_ids
        })
    }

    /// Prefill a prompt into a pane once its output settles (US-010,
    /// cli-agent-orchestration): FLOOR delay, then poll `output_generation`
    /// until idle (two equal reads) or MAX elapses; then write the prompt
    /// WITHOUT a carriage return — human-in-loop, the user submits. Shared by
    /// `workspace.up` and the spawn-capable `surface.split` (EP-003).
    pub(crate) fn schedule_prompt_prefill(
        terminal: &Entity<TerminalView>,
        prompt: String,
        pane_label: usize,
        cx: &mut Context<Self>,
    ) {
        let weak = terminal.downgrade();
        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
            smol::Timer::after(UP_PREFILL_FLOOR).await;
            // `AsyncApp::update` returns the closure value directly, so
            // this is `Option<u64>`: `None` once the pane is gone.
            let gen_now = |cx: &mut gpui::AsyncApp| -> Option<u64> {
                cx.update(|cx| {
                    weak.upgrade()
                        .map(|t| t.read(cx).terminal.output_generation)
                })
            };
            let mut last = gen_now(cx);
            let mut waited = UP_PREFILL_FLOOR;
            let mut settled = false;
            while waited < UP_PREFILL_MAX {
                smol::Timer::after(UP_PREFILL_POLL).await;
                waited += UP_PREFILL_POLL;
                let now = gen_now(cx);
                match (last, now) {
                    // Output unchanged across a poll interval -> the
                    // agent is idle and ready for input.
                    (Some(a), Some(b)) if a == b => {
                        settled = true;
                        break;
                    }
                    // Pane closed mid-wait: nothing to prefill.
                    (_, None) => break,
                    _ => last = now,
                }
            }
            cx.update(|cx| {
                if let Some(t) = weak.upgrade() {
                    if !settled {
                        log::warn!(
                            "prompt prefill: pane {pane_label} still producing output after \
                             {UP_PREFILL_MAX:?}; prompt prefilled best-effort"
                        );
                    }
                    t.read(cx).send_text(&prompt);
                }
            });
        })
        .detach();
    }

    /// US-017 (orchestration-v2): resolve which surface (pane terminal) a
    /// session's PID lives in, by walking the process ancestor chain to a
    /// known `terminal.child_pid`. Direct children (agents launched by
    /// `paneflow up`) hit the fast path synchronously; deeper chains walk
    /// `/proc`/libproc OFF the render thread and deposit the result back.
    /// A synthetic session key (legacy no-pid frames) or an unresolvable
    /// chain leaves `surface_id = None` — workspace-level badge only, never
    /// a wrong pane.
    pub(crate) fn schedule_surface_resolution(
        &mut self,
        ws_id: u64,
        session_key: u32,
        cx: &mut Context<Self>,
    ) {
        if session_key >= SYNTHETIC_SESSION_PID_BASE {
            return;
        }
        let already = self
            .workspaces
            .iter()
            .find(|ws| ws.id == ws_id)
            .and_then(|ws| ws.agent_sessions.get(&session_key))
            .is_none_or(|s| s.surface_id.is_some());
        if already {
            return;
        }
        // child_pid → surface entity id, across every workspace (the hook's
        // workspace_id can lag a moved pane; the chain decides).
        let mut candidates: HashMap<u32, u64> = HashMap::new();
        for ws in &self.workspaces {
            if let Some(root) = &ws.root {
                for pane in root.collect_leaves() {
                    for terminal in pane.read(cx).terminals() {
                        let pid = terminal.read(cx).terminal.child_pid;
                        if pid > 0 {
                            candidates.insert(pid, terminal.entity_id().as_u64());
                        }
                    }
                }
            }
        }
        // Fast path: the agent IS the pane's direct child (`up`-launched).
        if let Some(&sid) = candidates.get(&session_key) {
            self.set_session_surface(ws_id, session_key, sid, cx);
            return;
        }
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let resolved = smol::unblock(move || {
                    crate::workspace::pid_resolve::resolve_surface_for_pid(session_key, &candidates)
                })
                .await;
                if let Some(sid) = resolved {
                    let _ = cx.update(|cx| {
                        this.update(cx, |app, cx| {
                            app.set_session_surface(ws_id, session_key, sid, cx);
                        })
                    });
                }
            },
        )
        .detach();
    }

    fn set_session_surface(&mut self, ws_id: u64, key: u32, sid: u64, cx: &mut Context<Self>) {
        if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == ws_id)
            && let Some(session) = ws.agent_sessions.get_mut(&key)
            && session.surface_id != Some(sid)
        {
            session.surface_id = Some(sid);
            // EP-004 US-010: a NEW session resolving this pane evicts a stale
            // `Errored` row left by the previous (dead) agent on the same
            // surface — "pas d'erreur collante": relaunching the agent in the
            // pane replaces the crash signal with the live state. Deliberately
            // NOT tool-scoped: launching codex where claude crashed also
            // clears the dot — the surface is visibly back in use, whatever
            // the tool, and the dead row has no further eviction path.
            ws.agent_sessions.retain(|k, s| {
                *k == key || s.surface_id != Some(sid) || s.state != ai_types::AgentState::Errored
            });
            self.sync_attention(cx);
            // EP-001 US-003 (cli-cockpit): a late surface resolution can flip
            // a pane's busy verdict — refresh the Composer chip.
            self.agent_sessions_changed(cx);
            cx.notify();
        }
    }

    /// US-018/US-020 (orchestration-v2): push the WaitingForInput state down
    /// into the panes. Recomputed idempotently from `agent_sessions` after
    /// every transition (hooks, sweep, auto-clear, resolution) — the panes'
    /// `attention` maps can never drift from the session truth. Amplifies
    /// the waiting pane; inactive panes are never degraded.
    pub(crate) fn sync_attention(&self, cx: &mut Context<Self>) {
        let mut waiting: HashMap<u64, Option<String>> = HashMap::new();
        // EP-004 US-010: Errored surfaces ride the same idempotent push, in a
        // PARALLEL set (never overloading the waiting map — a tab is either
        // asking for input or crashed, and the dot colors must not mix).
        let mut errored: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for ws in &self.workspaces {
            for session in ws.agent_sessions.values() {
                let Some(sid) = session.surface_id else {
                    continue;
                };
                match session.state {
                    ai_types::AgentState::WaitingForInput => {
                        waiting.insert(sid, session.message.clone());
                    }
                    ai_types::AgentState::Errored => {
                        errored.insert(sid);
                    }
                    _ => {}
                }
            }
        }
        for ws in &self.workspaces {
            if let Some(root) = &ws.root {
                for pane in root.collect_leaves() {
                    let subset: HashMap<gpui::EntityId, Option<String>> = pane
                        .read(cx)
                        .terminals()
                        .filter_map(|t| {
                            waiting
                                .get(&t.entity_id().as_u64())
                                .map(|msg| (t.entity_id(), msg.clone()))
                        })
                        .collect();
                    let errored_subset: std::collections::HashSet<gpui::EntityId> = pane
                        .read(cx)
                        .terminals()
                        .filter(|t| errored.contains(&t.entity_id().as_u64()))
                        .map(|t| t.entity_id())
                        .collect();
                    pane.update(cx, |p, cx| {
                        p.set_attention(subset, cx);
                        p.set_errored(errored_subset, cx);
                    });
                }
            }
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
            "workspace.up" => self.handle_workspace_up(params, cx),
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
                        // US-009 (orchestration-v2): same teardown as the UI
                        // close path — clean managed worktrees removed in the
                        // background, dirty ones kept, branch never deleted.
                        let worktrees = std::mem::take(&mut self.workspaces[idx].managed_worktrees);
                        Self::spawn_worktree_teardown(worktrees, cx);
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
                // EP-001 US-003 (agent-control-plane): expose the output
                // generation counter so a client detects pane-idle without a
                // timer heuristic (kills the flow engine's settling poll).
                let output_generation = terminal.read(cx).terminal.output_generation;
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
                    "output_generation": output_generation,
                })
            }
            "fleet.list" => {
                // EP-001 US-001 (agent-control-plane): snapshot every running
                // agent across all workspaces. Read-only, no scripting gate.
                let name_by_sid: HashMap<u64, String> = self
                    .collect_surface_meta(cx)
                    .into_iter()
                    .map(|m| (m.surface_id, m.name))
                    .collect();
                let fleets: Vec<WsFleet> = self
                    .workspaces
                    .iter()
                    .enumerate()
                    .map(|(idx, ws)| WsFleet {
                        idx,
                        sessions: &ws.agent_sessions,
                        detected: &ws.detected_agents,
                    })
                    .collect();
                let agents = build_fleet_rows(&fleets, &name_by_sid, std::time::Instant::now());
                serde_json::json!({ "agents": agents })
            }
            "surface.status" => {
                // EP-001 US-002 (agent-control-plane): one pane's agent state.
                // Read-only; `idle` when no agent session lives in the pane.
                let terminal = match self.resolve_surface(params, cx) {
                    Ok(t) => t,
                    Err(e) => return e.into_value(),
                };
                let sid = terminal.entity_id().as_u64();
                let output_generation = terminal.read(cx).terminal.output_generation;
                let session = self
                    .workspaces
                    .iter()
                    .flat_map(|ws| ws.agent_sessions.values())
                    .find(|s| s.surface_id == Some(sid));
                surface_status_value(sid, session, output_generation, std::time::Instant::now())
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
            "surface.focus" => {
                // US-001 (orchestration-v2): give a targeted pane the focus.
                // Navigation only (no PTY write), so — like `workspace.select`
                // and unlike `surface.send_*` — it does NOT require the
                // `PANEFLOW_IPC_SCRIPTING` gate.
                let Some(sid) = params.get("surface_id").and_then(|s| s.as_u64()) else {
                    return serde_json::json!({"error": "Missing 'surface_id' parameter"});
                };
                let Some((ws_idx, pane, tab_idx)) =
                    find_pane_by_surface_id(&self.workspaces, sid, cx)
                else {
                    return serde_json::json!({"error": "Surface not found"});
                };
                // Switch workspace + activate the hosting tab synchronously…
                self.active_idx = ws_idx;
                pane.update(cx, |p, cx| {
                    if p.selected_idx != tab_idx {
                        p.selected_idx = tab_idx;
                    }
                    cx.notify();
                });
                // …but the keyboard focus needs a `&mut Window`, which the IPC
                // dispatch doesn't carry. Defer one tick and re-enter through
                // the main window handle (locate it among `cx.windows()` by
                // downcast); deferring keeps the re-entrant `PaneFlowApp` update
                // out of this in-flight one.
                cx.defer(move |cx| {
                    for handle in cx.windows() {
                        if let Some(main) = handle.downcast::<PaneFlowApp>() {
                            let _ = main.update(cx, |_, window, cx| {
                                pane.read(cx).focus_handle(cx).focus(window, cx);
                            });
                        }
                    }
                });
                self.save_session(cx);
                cx.notify();
                serde_json::json!({"focused": true, "surface_id": sid, "workspace": ws_idx})
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
                // US-005 (orchestration-v2): `submit: true` appends a CR after
                // the text — the ONLY sanctioned submission path. It is
                // unreachable when the scripting gate is off (the whole method
                // returns -32601 above, before params are even read), so a
                // submission can never happen silently; the default stays
                // strict inject-without-CR.
                let submit = params
                    .get("submit")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                // Route by surface_id if provided, otherwise use first leaf
                if let Some(sid) = params.get("surface_id").and_then(|s| s.as_u64()) {
                    if let Some(terminal) = find_terminal_by_surface_id(&self.workspaces, sid, cx) {
                        terminal.read(cx).send_text(text);
                        if submit {
                            terminal.read(cx).send_text("\r");
                        }
                        return serde_json::json!({
                            "sent": true, "length": text.len(), "submitted": submit
                        });
                    }
                    return serde_json::json!({"error": "Surface not found"});
                }
                if let Some(ws) = self.active_workspace()
                    && let Some(root) = &ws.root
                {
                    send_text_to_first_leaf(root, text, cx);
                    if submit {
                        send_text_to_first_leaf(root, "\r", cx);
                    }
                    return serde_json::json!({
                        "sent": true, "length": text.len(), "submitted": submit
                    });
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
                // EP-003 (orchestration-v2): `surface.split` can spawn a fully
                // configured pane — optional `cwd` (canonicalized, -32602 when
                // bad), `command` (launched like workspace.up panes), `env`,
                // `name`, `prompt` (server-side prefill, never submitted) and
                // `managed_worktree` (ownership registration, US-009). Same
                // trust model as workspace.up: the pane is freshly created by
                // this call (no lateral injection into a live agent session),
                // so no scripting gate. All fields absent = legacy bare split.
                let spawn_cwd = match params.get("cwd").and_then(|c| c.as_str()) {
                    Some(raw) => match canonicalize_workspace_cwd(raw) {
                        Ok(canonical) => Some(canonical),
                        Err(err) => return err.into_value(),
                    },
                    None => None,
                };
                let spawn_env = parse_env_object(params.get("env"));
                let spawn_command = params
                    .get("command")
                    .and_then(|c| c.as_str())
                    .filter(|c| !c.is_empty())
                    .map(str::to_string);
                let spawn_name = params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .filter(|n| !n.is_empty())
                    .map(str::to_string);
                let spawn_prompt = params
                    .get("prompt")
                    .and_then(|p| p.as_str())
                    .filter(|p| !p.is_empty())
                    .map(str::to_string);

                // US-002 (orchestration-v2): an optional `surface_id` targets
                // the leaf hosting that surface — in whatever workspace it
                // lives — instead of the active workspace's first leaf. Absent
                // = the legacy first-leaf behavior, so existing clients are
                // untouched.
                let (ws_idx, target_pane) =
                    if let Some(sid) = params.get("surface_id").and_then(|s| s.as_u64()) {
                        let Some((ws_idx, target_pane, _tab)) =
                            find_pane_by_surface_id(&self.workspaces, sid, cx)
                        else {
                            return serde_json::json!({"error": "Surface not found"});
                        };
                        (ws_idx, Some(target_pane))
                    } else {
                        (self.active_idx, None)
                    };
                let Some(ws) = self.workspaces.get(ws_idx) else {
                    return serde_json::json!({"error": "No active workspace"});
                };
                let ws_id = ws.id;
                if ws.root.as_ref().is_none_or(|r| r.leaf_count() >= MAX_PANES) {
                    return serde_json::json!({"error": "Maximum pane count reached"});
                }
                let new_terminal = cx.new(|cx| {
                    TerminalView::with_cwd_and_env(
                        ws_id,
                        spawn_cwd.clone(),
                        None,
                        spawn_env.clone(),
                        cx,
                    )
                });
                if let Some(name) = spawn_name {
                    new_terminal.update(cx, |view, _cx| {
                        view.terminal.custom_name = Some(name);
                    });
                }
                let surface_id = new_terminal.entity_id().as_u64();
                let new_pane = self.create_pane(new_terminal.clone(), ws_id, cx);
                let Some(root) = self.workspaces[ws_idx].root.as_mut() else {
                    return serde_json::json!({"error": "Workspace has no root"});
                };
                match target_pane {
                    Some(target) => {
                        if !root.split_at_pane(&target, direction, new_pane) {
                            // The pane vanished between lookup and mutation (a
                            // close raced this request); nothing was inserted.
                            return serde_json::json!({"error": "Surface not found"});
                        }
                    }
                    None => root.split_first_leaf(direction, new_pane),
                }
                if let Some(mw) = parse_managed_worktree(params.get("managed_worktree")) {
                    self.workspaces[ws_idx].managed_worktrees.push(mw);
                }
                if let Some(cmd) = spawn_command {
                    new_terminal.read(cx).send_command(&cmd);
                }
                if let Some(prompt) = spawn_prompt {
                    Self::schedule_prompt_prefill(&new_terminal, prompt, usize::MAX, cx);
                }
                let panes = self.workspaces[ws_idx].pane_count();
                self.save_session(cx);
                cx.notify();
                serde_json::json!({
                    "split": true, "direction": dir_str, "panes": panes,
                    "surface_id": surface_id
                })
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
                let tool = crate::agent_launcher::TerminalAgent::from_binary(tool_str);

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
                } else if self.agents_thread_mut_by_env_id(workspace_id).is_some() {
                    // Same no-op policy for an Agents thread: the spinner
                    // only appears once a prompt is actually in flight.
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
                let Some(tool) = read_tool(params) else {
                    // An unknown binary name can't map to a TerminalAgent —
                    // reject instead of mislabeling the session as Claude
                    // (the pre-fusion `from_name` fallback did exactly that).
                    return serde_json::json!({"error": "Unknown tool"});
                };

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    let key =
                        upsert_session_state(ws, pid, tool, ai_types::AgentState::Thinking, None);
                    // US-016: a new prompt invalidates the previous question.
                    if let Some(s) = ws.agent_sessions.get_mut(&key) {
                        s.message = None;
                    }
                    cx.notify();
                    if !self.loader_anim_running {
                        self.start_loader_animation(cx);
                    }
                    self.schedule_surface_resolution(workspace_id, key, cx);
                    self.sync_attention(cx);
                    // EP-001 US-003 (cli-cockpit): the target just turned
                    // busy — refresh the Composer chip (no flush can apply).
                    self.agent_sessions_changed(cx);
                    serde_json::json!({"status": "running"})
                } else if let Some(t) = self.agents_thread_mut_by_env_id(workspace_id) {
                    // The row spinner self-animates (declarative GPUI
                    // Animation in `thread_row`) — no loader-loop start here.
                    t.status = crate::project::ThreadStatus::Thinking;
                    if pid.is_some() {
                        t.agent_pid = pid;
                    }
                    cx.notify();
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
                let Some(tool) = read_tool(params) else {
                    // An unknown binary name can't map to a TerminalAgent —
                    // reject instead of mislabeling the session as Claude
                    // (the pre-fusion `from_name` fallback did exactly that).
                    return serde_json::json!({"error": "Unknown tool"});
                };

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // tool_use implies the session is actively thinking —
                    // promote it (or keep it) even if the prior state was
                    // Finished from a stale prompt-end.
                    let key = upsert_session_state(
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
                    self.schedule_surface_resolution(workspace_id, key, cx);
                    self.sync_attention(cx);
                    // EP-001 US-003 (cli-cockpit): see the prompt_submit arm.
                    self.agent_sessions_changed(cx);
                    serde_json::json!({"status": "running"})
                } else if let Some(t) = self.agents_thread_mut_by_env_id(workspace_id) {
                    // tool_use keeps (or promotes) the thread spinner —
                    // same Finished-revival rationale as the workspace arm.
                    t.status = crate::project::ThreadStatus::Thinking;
                    if pid.is_some() {
                        t.agent_pid = pid;
                    }
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
                let pid = read_session_pid(params);
                let Some(tool) = read_tool(params) else {
                    // An unknown binary name can't map to a TerminalAgent —
                    // reject instead of mislabeling the session as Claude
                    // (the pre-fusion `from_name` fallback did exactly that).
                    return serde_json::json!({"error": "Unknown tool"});
                };
                let message = sanitize_notification_message(
                    hook.and_then(|h| h.get("message"))
                        .and_then(|v| v.as_str())
                        .or_else(|| params.get("message").and_then(|v| v.as_str()))
                        .unwrap_or("Needs input"),
                );

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    let key = upsert_session_state(
                        ws,
                        pid,
                        tool,
                        ai_types::AgentState::WaitingForInput,
                        None,
                    );
                    // US-016: keep the agent's question — the peek overlay
                    // and the desktop notification surface it. Untrusted
                    // text: stored and displayed verbatim, never interpreted.
                    if let Some(s) = ws.agent_sessions.get_mut(&key) {
                        s.message = Some(message.clone());
                    }
                    let ws_title = ws.title.clone();
                    cx.notify();
                    fire_attention_notification(
                        &ws_title,
                        Some(&message),
                        cx.background_executor().clone(),
                    );
                    self.schedule_surface_resolution(workspace_id, key, cx);
                    self.sync_attention(cx);
                    // EP-001 US-003 (cli-cockpit): WaitingForInput is a safe
                    // prefill target — flush this pane's queued prompt now
                    // (main thread: transition and flush are serialized).
                    self.agent_sessions_changed(cx);
                    serde_json::json!({"status": "waiting"})
                } else if let Some(t) = self.agents_thread_mut_by_env_id(workspace_id) {
                    t.status = crate::project::ThreadStatus::WaitingForInput;
                    if pid.is_some() {
                        t.agent_pid = pid;
                    }
                    // Notification body uses the cleaned title so a CLI
                    // spinner glyph baked into the OSC title never leaks
                    // into the desktop notification.
                    let title = crate::project::clean_sidebar_title(&t.title)
                        .unwrap_or_else(|| t.title.clone());
                    cx.notify();
                    fire_attention_notification(
                        &title,
                        Some(&message),
                        cx.background_executor().clone(),
                    );
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
                let Some(tool) = read_tool(params) else {
                    // An unknown binary name can't map to a TerminalAgent —
                    // reject instead of mislabeling the session as Claude
                    // (the pre-fusion `from_name` fallback did exactly that).
                    return serde_json::json!({"error": "Unknown tool"});
                };

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // U-014: key the auto-clear on the RESOLVED session key, not
                    // the raw `pid`. A legacy no-pid frame is stored under a
                    // fallback/synthetic key by `upsert_session_state`; the old
                    // code captured `pid` (None) and the `let Some(pid_key)`
                    // guard short-circuited, so that session's Finished state
                    // never auto-cleared and leaked into the sidebar forever.
                    let session_key =
                        upsert_session_state(ws, pid, tool, ai_types::AgentState::Finished, None);
                    // US-016: the turn ended — the question is answered, no
                    // ghost message may survive into the next state.
                    if let Some(s) = ws.agent_sessions.get_mut(&session_key) {
                        s.message = None;
                    }
                    // EP-004 US-020: notify the user the turn ended if they're
                    // looking elsewhere. Read the title before the borrow ends.
                    let ws_title = ws.title.clone();
                    cx.notify();
                    fire_turn_end_notification(&ws_title, cx.background_executor().clone());
                    self.sync_attention(cx);
                    // EP-001 US-003 (cli-cockpit): the turn ended — flush any
                    // queued prompt for this pane (prefill only).
                    self.agent_sessions_changed(cx);

                    // Auto-clear the session 5 s after stop unless something
                    // else (new prompt_submit, tool_use) bumps it back to
                    // Thinking. Targets the exact (workspace_id, session_key) so
                    // sibling sessions in the same workspace are untouched.
                    let ws_id = workspace_id;
                    cx.spawn(
                        async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                            smol::Timer::after(std::time::Duration::from_secs(5)).await;
                            cx.update(|cx| {
                                let _ = this.update(cx, |app, cx| {
                                    if let Some(ws) =
                                        app.workspaces.iter_mut().find(|ws| ws.id == ws_id)
                                        && matches!(
                                            ws.agent_sessions.get(&session_key).map(|s| &s.state),
                                            Some(ai_types::AgentState::Finished)
                                        )
                                    {
                                        ws.agent_sessions.remove(&session_key);
                                        app.sync_attention(cx);
                                        app.agent_sessions_changed(cx);
                                        cx.notify();
                                    }
                                });
                            });
                        },
                    )
                    .detach();

                    serde_json::json!({"status": "idle"})
                } else if let Some(t) = self.agents_thread_mut_by_env_id(workspace_id) {
                    // Codex-style: the spinner drops the moment the turn
                    // ends and the relative timestamp returns. No Finished
                    // hold state — `ThreadStatus` has no such variant and
                    // the row's timestamp is the natural rest indicator.
                    t.status = crate::project::ThreadStatus::Idle;
                    t.agent_pid = None;
                    let title = crate::project::clean_sidebar_title(&t.title)
                        .unwrap_or_else(|| t.title.clone());
                    // Snapshot what the off-thread ai-title backfill needs
                    // before the &mut borrow on `self` ends.
                    let thread_id = t.id;
                    let cwd = t.cwd.clone();
                    let session_agent = t.terminal_agent.and_then(|a| a.session_agent());
                    let bound_session = t.session_id.clone();
                    let title_locked = t.title_user_set;
                    cx.notify();
                    fire_turn_end_notification(&title, cx.background_executor().clone());
                    // Parity with `/resume`: at turn end the session's LLM
                    // `ai-title` exists on disk — adopt it as the sidebar
                    // label, unless the user pinned the name via a rename.
                    if !title_locked && let Some(agent) = session_agent {
                        self.spawn_thread_title_backfill(thread_id, cwd, agent, bound_session, cx);
                    }
                    serde_json::json!({"status": "idle"})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            // EP-004 US-010: the shim reports the wrapped agent binary's REAL
            // exit status (`ChildExit` only ever carries the shell's). Always
            // emitted BEFORE the shim's `ai.session_end`, both blocking — see
            // `paneflow-shim::main` for the ordering contract.
            "ai.exit" => {
                let Some(workspace_id) = params.get("workspace_id").and_then(|v| v.as_u64()) else {
                    return serde_json::json!({"error": "Missing workspace_id"});
                };
                let Some(exit_code) = params
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .and_then(|n| i32::try_from(n).ok())
                else {
                    return serde_json::json!({"error": "Missing or invalid exit_code"});
                };
                let pid = read_session_pid(params);
                let Some(tool) = read_tool(params) else {
                    // An unknown binary name can't map to a TerminalAgent —
                    // reject instead of mislabeling the session as Claude
                    // (the pre-fusion `from_name` fallback did exactly that).
                    return serde_json::json!({"error": "Unknown tool"});
                };

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // 0 / SIGINT-and-friends → Finished (a human interrupt is
                    // NOT an error, FR-06); everything else → Errored. The
                    // classifier is pure and unit-tested in `ai_types`.
                    let state = ai_types::state_for_exit(exit_code);
                    let errored = state == ai_types::AgentState::Errored;
                    let key = upsert_session_state(ws, pid, tool, state, None);
                    // The binary is gone — whatever question it was asking is
                    // moot (same ghost-message rationale as `ai.stop`).
                    if let Some(s) = ws.agent_sessions.get_mut(&key) {
                        s.message = None;
                    }
                    let ws_title = ws.title.clone();
                    cx.notify();
                    if errored {
                        fire_agent_exit_notification(
                            &ws_title,
                            exit_code,
                            cx.background_executor().clone(),
                        );
                        // A crash-on-launch session may have had no prior
                        // frame: try resolving its pane while the shim (the
                        // PID anchor) is still alive, so the Errored dot can
                        // land on a tab. No-op if already resolved.
                        self.schedule_surface_resolution(workspace_id, key, cx);
                    }
                    // Finished (exit 0 / interrupt) intentionally fires no
                    // notification — `ai.stop` already announced the turn
                    // end, and the shim's `ai.session_end` lands right after
                    // this frame to clear the row.
                    self.sync_attention(cx);
                    self.agent_sessions_changed(cx);
                    serde_json::json!({"status": if errored { "errored" } else { "finished" }})
                } else if let Some(t) = self.agents_thread_mut_by_env_id(workspace_id) {
                    // Agents view (out of this PRD's cockpit scope): the
                    // thread model has no Errored status — treat like
                    // `ai.session_end` so the spinner never sticks.
                    t.status = crate::project::ThreadStatus::Idle;
                    t.agent_pid = None;
                    cx.notify();
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
                // Unknown tool string → `None`: the PID-based removal below
                // still works, only the tool-name fallback is skipped.
                let tool = crate::agent_launcher::TerminalAgent::from_binary(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // Prefer exact PID removal; fall back to removing one
                    // session matching the tool name (back-compat for older
                    // shims that didn't carry `pid` on session_end). Last
                    // resort keeps `agent_sessions` consistent with the
                    // pre-refactor "one session per tool" assumption.
                    //
                    // EP-004 US-010: an `Errored` session is SPARED — the
                    // shim's `ai.exit` lands just before this frame, and
                    // removing the row here would wipe the crash signal the
                    // instant it appeared. The Errored row is evicted later
                    // by a new session resolving the same pane
                    // (`set_session_surface`) or by the sweep once its pane
                    // closes (`sweep_stale_pids`).
                    let is_errored =
                        |s: &ai_types::AgentSession| s.state == ai_types::AgentState::Errored;
                    let removed = if let Some(p) = pid
                        && ws.agent_sessions.get(&p).is_some_and(|s| !is_errored(s))
                        && ws.agent_sessions.remove(&p).is_some()
                    {
                        true
                    } else if pid.is_some_and(|p| ws.agent_sessions.contains_key(&p)) {
                        // Exact-PID match exists but is Errored: keep it, and
                        // do NOT fall through to the tool-name removal (it
                        // would evict an unrelated sibling session).
                        false
                    } else {
                        let pid_to_remove = ws
                            .agent_sessions
                            .iter()
                            .find(|(_, s)| Some(s.tool) == tool && !is_errored(s))
                            .map(|(k, _)| *k);
                        if let Some(k) = pid_to_remove {
                            ws.agent_sessions.remove(&k);
                            true
                        } else {
                            false
                        }
                    };
                    if removed {
                        self.sync_attention(cx);
                        // EP-001 US-003 (cli-cockpit): a removed session
                        // leaves a bare shell — always a safe prefill target.
                        self.agent_sessions_changed(cx);
                        cx.notify();
                    }
                    serde_json::json!({"cleared": removed})
                } else if let Some(t) = self.agents_thread_mut_by_env_id(workspace_id) {
                    let was_active = t.status != crate::project::ThreadStatus::Idle;
                    t.status = crate::project::ThreadStatus::Idle;
                    t.agent_pid = None;
                    if was_active {
                        cx.notify();
                    }
                    serde_json::json!({"cleared": was_active})
                } else {
                    serde_json::json!({"error": format!("Unknown workspace_id: {workspace_id}")})
                }
            }
            _ => {
                serde_json::json!({"error": format!("Unknown method: {method}")})
            }
        }
    }

    /// Resolve an `ai.*` frame's `workspace_id` to the Agents thread it was
    /// emitted from, when the id sits in the Agents PTY-env namespace
    /// ([`crate::project::AGENTS_THREAD_ENV_ID_BASE`]). CLI-mode workspace
    /// ids live below the base and return `None` here, so the workspace arm
    /// and this fallback can never both match one frame. Searches project
    /// threads and free chats — both spawn through the same mount path.
    fn agents_thread_mut_by_env_id(&mut self, env_id: u64) -> Option<&mut crate::project::Thread> {
        let thread_id = crate::project::thread_id_from_env_id(env_id)?;
        self.agents_thread_mut_by_id(thread_id)
    }
}

// ---------------------------------------------------------------------------
// AI session helpers (multi-session refactor)
// ---------------------------------------------------------------------------

/// Read the session PID from an `ai.*` IPC param object. Returns `None`
/// when the field is missing or zero — older shims (pre multi-session
/// refactor) don't include `pid` on every lifecycle frame, so the
/// caller must tolerate `None` and degrade to tool-name-based matching.
///
/// EP-004 security hardening: the upper half of u32 (`> i32::MAX`) is
/// REJECTED from clients. That band is reserved for server-allocated
/// synthetic keys and — critically — `sweep_stale_pids` keeps every key in
/// it forever (it can't be probed with `kill(pid, 0)`). Accepting it from
/// the (same-UID, untrusted) socket would let a forger accumulate
/// unbounded permanent sessions. Real OS PIDs sit far below this bound on
/// every supported platform (Linux pid_max 4 194 304; macOS 99 999;
/// Windows DWORD ids in practice), so a legitimate frame is never dropped;
/// a forged real-range PID is self-limiting (probed and reaped ≤ 30 s).
fn read_session_pid(params: &serde_json::Value) -> Option<u32> {
    params
        .get("pid")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&p| p > 0 && p <= i32::MAX as u32)
}

/// Read the `tool` field from an `ai.*` IPC param object, falling back
/// to `hook_payload.tool`, defaulting to `"claude"` when absent (matches
/// the server's historical behavior for legacy shims that don't stamp the
/// field). The string is the agent's BINARY name — the wire id shared with
/// the shim's `detect_tool_from_stem` — resolved via
/// [`TerminalAgent::from_binary`]. `None` for an unknown string: the frame
/// is then ignored by the caller instead of silently retyped as Claude
/// (the historical `from_name` fallback mislabeled every future agent).
fn read_tool(params: &serde_json::Value) -> Option<crate::agent_launcher::TerminalAgent> {
    let hook = params.get("hook_payload");
    let tool_str = params
        .get("tool")
        .and_then(|v| v.as_str())
        .or_else(|| hook.and_then(|h| h.get("tool")).and_then(|v| v.as_str()))
        .unwrap_or("claude");
    crate::agent_launcher::TerminalAgent::from_binary(tool_str)
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
/// Returns the resolved session key the entry was stored under — the real
/// `pid` when known, or the fallback/synthetic key chosen for a legacy
/// no-pid frame. Callers that need to act on the same row later (e.g. the
/// `ai.stop` auto-clear, U-014) must use THIS key, not the raw `pid`, or a
/// no-pid session is stored under a synthetic key yet never cleared.
fn upsert_session_state(
    ws: &mut crate::workspace::Workspace,
    pid: Option<u32>,
    tool: crate::agent_launcher::TerminalAgent,
    state: ai_types::AgentState,
    active_tool_name: Option<String>,
) -> u32 {
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

    // EP-002 US-004 (cli-cockpit): this is the single choke point for every
    // state write, so the Attention Queue's wait stamp lives here — stamped
    // on entering WaitingForInput, preserved across re-notifications,
    // cleared on any other transition.
    //
    // EP-004 US-011: `last_activity` is refreshed here too — every `ai.*`
    // lifecycle frame routes through this function, so the Stalled sweep's
    // silence clock resets on any hook activity. This also makes Stalled
    // non-sticky for free: the next frame overwrites `state` AND the clock.
    let now = std::time::Instant::now();
    // Pin the process start time for real-PID sessions so the sweep can
    // tell a recycled PID from the original agent (an opaque value, only
    // compared for equality). Probed once — a `Some` is immutable for the
    // process's lifetime; a `None` (transient EPERM) retries on the next
    // frame.
    let probe_start = |k: u32| {
        if k <= i32::MAX as u32 {
            super::event_handlers::pid_start_time(k)
        } else {
            None
        }
    };
    ws.agent_sessions
        .entry(key)
        .and_modify(|s| {
            s.waiting_since =
                ai_types::next_waiting_since(Some((&s.state, s.waiting_since)), &state, now);
            s.tool = tool;
            s.state = state.clone();
            s.active_tool_name = active_tool_name.clone();
            s.last_activity = now;
            if s.proc_start.is_none() {
                s.proc_start = probe_start(key);
            }
        })
        .or_insert_with(|| {
            let mut session = ai_types::AgentSession::new(tool, state);
            session.waiting_since = ai_types::next_waiting_since(None, &session.state, now);
            session.active_tool_name = active_tool_name;
            // Same `now` as the and_modify arm — `AgentSession::new` stamps
            // its own Instant, which would skew (sub-µs) from the wait stamp.
            session.last_activity = now;
            session.proc_start = probe_start(key);
            session
        });
    key
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

    // US-020 / CWE-78: a control char (newline) cannot survive into the macOS
    // AppleScript literal — it is stripped at the source — while quotes are
    // preserved for the downstream backslash/quote escape.
    #[test]
    fn sanitize_applescript_body_strips_control_chars_keeps_quotes() {
        assert_eq!(sanitize_applescript_body("a\r\nb\tc"), "abc");
        let with_quote = sanitize_applescript_body("title\"\n");
        assert_eq!(with_quote, "title\"");
        assert!(!with_quote.contains('\n'));
        assert_eq!(sanitize_applescript_body("plain title"), "plain title");
    }

    // EP-004 security hardening: client-supplied PIDs above i32::MAX are
    // rejected — that band is server-reserved (synthetic keys) AND immune to
    // the stale-PID sweep, so accepting it would allow unbounded permanent
    // session accumulation from forged frames on the same-UID socket.
    #[test]
    fn read_session_pid_rejects_server_reserved_high_band() {
        let pid = |v: serde_json::Value| read_session_pid(&serde_json::json!({ "pid": v }));
        assert_eq!(pid(serde_json::json!(1234)), Some(1234));
        assert_eq!(pid(serde_json::json!(i32::MAX as u32)), Some(2147483647));
        assert_eq!(pid(serde_json::json!(i32::MAX as u32 + 1)), None);
        assert_eq!(
            pid(serde_json::json!(0xFFFF_0000u32)),
            None,
            "synthetic band floor"
        );
        assert_eq!(pid(serde_json::json!(u32::MAX)), None);
        assert_eq!(pid(serde_json::json!(0)), None);
        assert_eq!(read_session_pid(&serde_json::json!({})), None);
    }

    // EP-004 US-010/US-011: the two new notification bodies are distinct
    // from each other and from the legacy "agent finished" / "needs input"
    // shapes — the whole point of the epic is that the four causes read
    // differently in a desktop toast.
    #[test]
    fn agent_exit_body_carries_workspace_and_code() {
        assert_eq!(
            agent_exit_notification_body("api", 1),
            "api: agent exited (exit 1)"
        );
        assert_eq!(
            agent_exit_notification_body("ws", -1073741510),
            "ws: agent exited (exit -1073741510)"
        );
    }

    #[test]
    fn stalled_body_carries_workspace_and_silence() {
        assert_eq!(
            stalled_notification_body("api", 300),
            "api: agent silent for 300 s"
        );
    }

    // US-008: `workspace.up` env parsing. Non-string values are dropped (a
    // shell env value can only be a string) and an absent/empty object yields
    // `None` so the global `terminal.env` default still applies underneath.
    #[test]
    fn parse_env_object_keeps_strings_and_drops_the_rest() {
        let env = parse_env_object(Some(&serde_json::json!({
            "RUST_LOG": "info",
            "PORT": 8080,
            "FLAG": true
        })))
        .expect("non-empty string map");
        assert_eq!(env.get("RUST_LOG").map(String::as_str), Some("info"));
        assert!(
            !env.contains_key("PORT"),
            "non-string value must be dropped"
        );
        assert!(!env.contains_key("FLAG"));
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn parse_env_object_absent_or_empty_is_none() {
        assert!(parse_env_object(None).is_none());
        assert!(parse_env_object(Some(&serde_json::json!({}))).is_none());
        // An object with only non-string values collapses to an empty map -> None.
        assert!(parse_env_object(Some(&serde_json::json!({ "N": 1 }))).is_none());
    }

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
    // US-016 (orchestration-v2) — contextual attention notification
    // -----------------------------------------------------------------

    /// US-020 AC5: the stored question is bounded (512 chars) and stripped
    /// of bidi/zero-width controls at the storage choke point — an RLO in
    /// hook text must not visually reverse the peek badge or the desktop
    /// notification (same sanitizer as the markdown viewer).
    #[test]
    fn notification_message_is_bounded_and_bidi_stripped() {
        let spoofed = "Allow \u{202E}?fr- mr\u{202C} ?";
        let clean = super::sanitize_notification_message(spoofed);
        assert!(!clean.contains('\u{202E}'), "RLO stripped");
        assert!(!clean.contains('\u{202C}'), "PDF stripped");
        assert!(clean.contains("Allow"), "visible text kept: {clean}");

        let long = "é".repeat(600);
        assert_eq!(
            super::sanitize_notification_message(&long).chars().count(),
            512,
            "char-bounded, multibyte-safe"
        );
    }

    /// AC2/AC3: the WaitingForInput desktop notification carries the
    /// agent's question; an empty/absent message falls back to the generic
    /// body — never an empty notification.
    #[test]
    fn attention_body_carries_the_question_with_fallback() {
        assert_eq!(
            super::attention_notification_body("backend", Some("Allow `cargo test`?")),
            "backend: Allow `cargo test`?"
        );
        assert_eq!(
            super::attention_notification_body("backend", None),
            "backend: agent needs input"
        );
        assert_eq!(
            super::attention_notification_body("backend", Some("   ")),
            "backend: agent needs input",
            "whitespace-only message falls back too"
        );
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

    // EP-001 US-001: fleet rows are pure — a conductor's snapshot.
    #[test]
    fn build_fleet_rows_empty_is_empty() {
        let sessions = HashMap::new();
        let detected = HashSet::new();
        let fleets = [WsFleet {
            idx: 0,
            sessions: &sessions,
            detected: &detected,
        }];
        let rows = build_fleet_rows(&fleets, &HashMap::new(), std::time::Instant::now());
        assert!(rows.is_empty());
    }

    #[test]
    fn build_fleet_rows_lists_hooked_session_with_surface_name() {
        use crate::agent_launcher::TerminalAgent;
        use crate::ai_types::{AgentSession, AgentState};
        let mut sessions = HashMap::new();
        let mut s = AgentSession::new(TerminalAgent::ClaudeCode, AgentState::WaitingForInput);
        s.surface_id = Some(42);
        sessions.insert(1234u32, s);
        let detected = HashSet::new();
        let fleets = [WsFleet {
            idx: 0,
            sessions: &sessions,
            detected: &detected,
        }];
        let mut names = HashMap::new();
        names.insert(42u64, "backend".to_string());
        let rows = build_fleet_rows(&fleets, &names, std::time::Instant::now());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["pid"], 1234);
        assert_eq!(rows[0]["tool"], "claude");
        assert_eq!(rows[0]["state"], "waiting_for_input");
        assert_eq!(rows[0]["hooked"], true);
        assert_eq!(rows[0]["surface_id"], 42);
        assert_eq!(rows[0]["surface_name"], "backend");
    }

    #[test]
    fn build_fleet_rows_appends_unhooked_only_when_tool_has_no_session() {
        use crate::agent_launcher::TerminalAgent;
        use crate::ai_types::{AgentSession, AgentState};
        // Claude is hooked AND detected; Copilot is only detected (no hooks).
        let mut sessions = HashMap::new();
        sessions.insert(
            10u32,
            AgentSession::new(TerminalAgent::ClaudeCode, AgentState::Thinking),
        );
        let mut detected = HashSet::new();
        detected.insert(TerminalAgent::ClaudeCode.binary().to_string());
        detected.insert(TerminalAgent::Copilot.binary().to_string());
        let fleets = [WsFleet {
            idx: 0,
            sessions: &sessions,
            detected: &detected,
        }];
        let rows = build_fleet_rows(&fleets, &HashMap::new(), std::time::Instant::now());
        // Claude once (hooked), Copilot once (unhooked) — Claude NOT doubled.
        assert_eq!(rows.len(), 2);
        let hooked: Vec<_> = rows.iter().filter(|r| r["hooked"] == true).collect();
        assert_eq!(hooked.len(), 1);
        assert_eq!(hooked[0]["tool"], "claude");
        let unhooked: Vec<_> = rows.iter().filter(|r| r["hooked"] == false).collect();
        assert_eq!(unhooked.len(), 1);
        assert_eq!(unhooked[0]["tool"], "copilot");
        assert_eq!(unhooked[0]["state"], "unknown_running");
        assert_eq!(unhooked[0]["pid"], serde_json::Value::Null);
    }

    // EP-001 US-002: surface.status is pure — idle vs live session.
    #[test]
    fn surface_status_value_idle_when_no_session() {
        let v = surface_status_value(7, None, 99, std::time::Instant::now());
        assert_eq!(v["surface_id"], 7);
        assert_eq!(v["state"], "idle");
        assert_eq!(v["output_generation"], 99);
        assert!(v.get("tool").is_none());
    }

    #[test]
    fn surface_status_value_reports_session_state() {
        use crate::agent_launcher::TerminalAgent;
        use crate::ai_types::{AgentSession, AgentState};
        let s = AgentSession::new(TerminalAgent::Codex, AgentState::Thinking);
        let v = surface_status_value(7, Some(&s), 12, std::time::Instant::now());
        assert_eq!(v["state"], "thinking");
        assert_eq!(v["tool"], "codex");
        assert_eq!(v["output_generation"], 12);
    }
}
