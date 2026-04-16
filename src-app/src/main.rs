//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar workspace list + main content area.

mod ai_detector;
mod assets;
mod config_writer;
mod csd;
mod ipc;
mod keybindings;
mod keys;
mod mouse;
mod pane;
mod pty;
mod search;
mod settings_window;
mod split;
mod terminal;
mod terminal_element;
pub mod theme;
mod title_bar;
mod update_checker;
mod workspace;

use alacritty_terminal::grid::Dimensions;
use gpui::{
    Animation, AnimationExt, App, Bounds, ClickEvent, ClipboardItem, Context, CursorStyle,
    Decorations, Entity, FocusHandle, Focusable, HitboxBehavior, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, PathPromptOptions, Pixels, Point, Render, ResizeEdge, SharedString,
    Styled, Window, WindowBounds, WindowDecorations, WindowOptions, actions, canvas, deferred, div,
    ease_in_out, point, prelude::*, px, rgb, size, svg, transparent_black,
};
use gpui_platform::application;
use notify::Watcher;

use std::collections::VecDeque;

use paneflow_config::schema::LayoutNode;

use crate::pane::Pane;
use crate::split::{FocusDirection, LayoutTree, SplitDirection};
use crate::terminal::TerminalView;
use crate::workspace::{Workspace, next_workspace_id};

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

/// Find the first terminal in a layout tree (for default routing).
fn find_first_terminal(
    node: &LayoutTree,
    cx: &App,
) -> Option<gpui::Entity<crate::terminal::TerminalView>> {
    match node {
        LayoutTree::Leaf(pane) => {
            let pane = pane.read(cx);
            Some(pane.active_terminal().clone())
        }
        LayoutTree::Container { children, .. } => children
            .first()
            .and_then(|child| find_first_terminal(&child.node, cx)),
    }
}

/// Find a terminal view entity by its surface_id (GPUI entity ID) across all workspaces.
fn find_terminal_by_surface_id(
    workspaces: &[crate::workspace::Workspace],
    surface_id: u64,
    cx: &App,
) -> Option<gpui::Entity<crate::terminal::TerminalView>> {
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
) -> Option<gpui::Entity<crate::terminal::TerminalView>> {
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
        CopyWorkspacePath,
        RevealWorkspaceInFileManager,
        OpenWorkspaceInZed,
        OpenWorkspaceInCursor,
        OpenWorkspaceInVsCode,
        OpenWorkspaceInWindsurf,
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
        LayoutTiled,
        SplitEqualize,
        SwapPane,
        ToggleSearch,
        ToggleSearchRegex,
        JumpToPromptPrev,
        JumpToPromptNext,
        UndoClosePane,
        SearchNext,
        SearchPrev,
        DismissSearch,
        ToggleCopyMode,
        ClearScrollHistory,
        ResetTerminal
    ]
);

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

/// Sidebar width in pixels — shared between sidebar and title bar for alignment.
const SIDEBAR_WIDTH: f32 = 240.;

#[derive(Clone, Copy, PartialEq)]
enum SettingsSection {
    Shortcuts,
    Appearance,
}

/// A notification from Claude Code state changes, displayed in the bell menu.
#[derive(Clone)]
#[allow(dead_code)]
struct Notification {
    workspace_id: u64,
    workspace_title: String,
    message: String,
    kind: ai_detector::AiToolState,
    timestamp: std::time::Instant,
    read: bool,
}

struct Toast {
    message: String,
}

#[derive(Clone, Copy)]
struct WorkspaceContextMenu {
    idx: usize,
    position: Point<Pixels>,
}

/// Claude Code spinner glyphs — same characters Claude renders in the terminal.
const CLAUDE_SPINNER_FRAMES: [char; 6] = ['·', '✻', '✽', '✶', '✳', '✢'];
/// Codex spinner glyphs — pulsing dot from the dots animation variant.
const CODEX_SPINNER_FRAMES: [char; 4] = ['●', '○', '◉', '○'];
const TOAST_ENTER_MS: u64 = 180;
const TOAST_HOLD_MS: u64 = 1440;
const TOAST_EXIT_MS: u64 = 180;

/// Captured state of a closed pane for undo-close-pane (US-014).
struct ClosedPaneRecord {
    cwd: Option<std::path::PathBuf>,
    scrollback: Option<String>,
    workspace_idx: usize,
}

/// Maximum number of closed pane records to keep.
const MAX_CLOSED_PANES: usize = 5;

struct PaneFlowApp {
    workspaces: Vec<Workspace>,
    active_idx: usize,
    renaming_idx: Option<usize>,
    rename_text: String,
    /// Shared slot for config changes from the background `ConfigWatcher` thread.
    /// The watcher writes `Some(config)` on every successful reload; the main
    /// thread `take()`s it in the 50ms poll loop to apply keybindings + theme.
    pending_config:
        std::sync::Arc<std::sync::Mutex<Option<paneflow_config::schema::PaneFlowConfig>>>,
    ipc_rx: std::sync::mpsc::Receiver<ipc::IpcRequest>,
    title_bar: Entity<title_bar::TitleBar>,
    /// File watcher for `.git/HEAD` and `.git/index` across all workspaces.
    /// `None` if the OS watcher could not be created (graceful degradation).
    git_watcher: Option<notify::RecommendedWatcher>,
    /// Receiver for raw notify events from the git file watcher.
    git_event_rx: std::sync::mpsc::Receiver<notify::Result<notify::Event>>,
    /// Refcount for watched `.git` directories (multiple workspaces may share a repo).
    git_watch_counts: std::collections::HashMap<std::path::PathBuf, usize>,
    /// Active settings section, or `None` if settings is closed.
    settings_section: Option<SettingsSection>,
    /// Cached HOME directory for sidebar display (avoids per-render syscall).
    home_dir: String,
    /// Effective keybindings (defaults merged with user overrides) for settings display.
    effective_shortcuts: Vec<keybindings::ShortcutEntry>,
    /// Index of the shortcut row currently being recorded (`None` = not recording).
    recording_shortcut_idx: Option<usize>,
    /// Focus handle for the settings page (receives key events during recording/font search).
    settings_focus: FocusHandle,
    /// Cached list of monospace font family names from the system.
    mono_font_names: Vec<String>,
    /// Whether the font family dropdown is open.
    font_dropdown_open: bool,
    /// Filter text for the font dropdown.
    font_search: String,
    /// Notifications from Claude Code state changes (bell menu).
    notifications: Vec<Notification>,
    /// Whether the notification bell dropdown is open.
    notif_menu_open: bool,
    /// Workflow action menu currently open in the sidebar (`None` = closed).
    workspace_menu_open: Option<WorkspaceContextMenu>,
    /// Burger menu currently open in the title bar (`None` = closed).
    title_bar_menu_open: Option<Point<Pixels>>,
    /// Ephemeral bottom-right toast.
    toast: Option<Toast>,
    /// Dismiss timer for the active toast — dropped on new toast to cancel the old timer.
    _toast_task: Option<gpui::Task<()>>,
    /// Whether the loader animation spawn is currently running.
    loader_anim_running: bool,
    /// Source pane for swap mode, or `None` if not in swap mode.
    swap_source: Option<Entity<crate::pane::Pane>>,
    /// LIFO stack of recently closed panes for undo-close (US-014).
    closed_panes: Vec<ClosedPaneRecord>,
    /// Whether the "About PaneFlow" dialog is visible.
    show_about_dialog: bool,
    /// Shared slot for the background update checker result.
    pending_update: update_checker::SharedUpdateSlot,
    /// Resolved update status (set once the background check completes).
    update_status: Option<update_checker::UpdateStatus>,
}

/// Global flag for swap mode, checked by TerminalView to intercept Escape.
/// Follows the same AtomicBool pattern as `SUPPRESS_REPAINTS`.
pub static SWAP_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
                if let Some(ref mut watcher) = self.git_watcher
                    && let Err(e) = watcher.watch(git_dir, notify::RecursiveMode::NonRecursive)
                {
                    log::warn!("git watcher: failed to watch {}: {e}", git_dir.display());
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
        workspace_id: u64,
        cx: &mut Context<Self>,
    ) -> Entity<Pane> {
        cx.subscribe(&terminal, Self::handle_terminal_event)
            .detach();
        let pane = cx.new(|cx| Pane::new(terminal, workspace_id, cx));
        cx.subscribe(&pane, Self::handle_pane_event).detach();
        pane
    }

    fn handle_title_bar_event(
        &mut self,
        _title_bar: Entity<title_bar::TitleBar>,
        event: &title_bar::TitleBarEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            title_bar::TitleBarEvent::ToggleMenu(position) => {
                self.workspace_menu_open = None;
                self.notif_menu_open = false;
                self.title_bar_menu_open = if self.title_bar_menu_open.is_some() {
                    None
                } else {
                    Some(*position)
                };
                cx.notify();
            }
        }
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
                    let ws_id = ws.id;
                    let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
                    let new_pane = self.create_pane(terminal, ws_id, cx);
                    if let Some(ws) = self.active_workspace_mut() {
                        ws.root = Some(LayoutTree::Leaf(new_pane));
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
                let (source_cwd, initial_size) = {
                    let view = pane.read(cx).active_terminal().read(cx);
                    let cwd = view.terminal.cwd_now();
                    let term = view.terminal.term.lock();
                    let (cols, rows) = (term.columns(), term.screen_lines());
                    let size = match direction {
                        crate::split::SplitDirection::Horizontal => (cols, (rows / 2).max(1)),
                        crate::split::SplitDirection::Vertical => ((cols / 2).max(1), rows),
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
                self.save_session(cx);
                cx.notify();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Terminal event handling — push-based port detection and CWD tracking
    // -----------------------------------------------------------------------

    fn handle_terminal_event(
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

    /// Probe registered AI agent PIDs with `kill(pid, 0)` and clean up
    /// stale entries where the process no longer exists (ESRCH).
    fn sweep_stale_pids(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for ws in &mut self.workspaces {
            if ws.agent_pids.is_empty() {
                continue;
            }
            let before = ws.agent_pids.len();
            ws.agent_pids.retain(|_tool, &mut pid| {
                if pid > i32::MAX as u32 {
                    return false; // Invalid PID range — treat as stale
                }
                let ret = unsafe { libc::kill(pid as i32, 0) };
                if ret == -1 {
                    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if errno == libc::ESRCH {
                        // Process does not exist — stale
                        return false;
                    }
                    // EPERM or other: process exists but we can't signal it — keep
                }
                true
            });
            if ws.agent_pids.len() < before {
                changed = true;
                // If all agent PIDs were cleared and state is still active, reset to Inactive
                if ws.agent_pids.is_empty() && ws.ai_state != ai_detector::AiToolState::Inactive {
                    ws.ai_state = ai_detector::AiToolState::Inactive;
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
    fn start_loader_animation(&mut self, cx: &mut Context<Self>) {
        self.loader_anim_running = true;
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(16)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            let any_thinking = app.workspaces.iter().any(|ws| {
                                matches!(ws.ai_state, ai_detector::AiToolState::Thinking(_))
                            });
                            if !any_thinking {
                                app.loader_anim_running = false;
                                return false;
                            }
                            // 0.9s per revolution ≈ 0.1164 rad/frame at 60fps
                            let delta = std::f32::consts::TAU / (0.9 * 60.0);
                            for ws in &app.workspaces {
                                if matches!(ws.ai_state, ai_detector::AiToolState::Thinking(_)) {
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
                        // Update workspace
                        let ws = &mut app.workspaces[ws_idx];
                        ws.cwd = new_cwd.clone();
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

    fn new(cx: &mut Context<Self>) -> Self {
        let title_bar = cx.new(title_bar::TitleBar::new);
        cx.subscribe(&title_bar, Self::handle_title_bar_event)
            .detach();
        let ipc_rx = ipc::start_server();

        // ConfigWatcher: background thread detects file changes (300ms debounce),
        // stores parsed config in a shared slot for the 50ms poll loop to pick up.
        // Note: `start()` moves the OS watcher into a background thread, so the
        // `ConfigWatcher` struct itself can be safely dropped after starting.
        let pending_config = std::sync::Arc::new(std::sync::Mutex::new(
            None::<paneflow_config::schema::PaneFlowConfig>,
        ));
        let pending_config_writer = std::sync::Arc::clone(&pending_config);
        let _config_watcher = paneflow_config::watcher::ConfigWatcher::new(std::sync::Arc::new(
            move |cfg: paneflow_config::schema::PaneFlowConfig| {
                *pending_config_writer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = Some(cfg);
            },
        ));
        if let Err(e) = _config_watcher.start() {
            log::warn!("config watcher failed to start: {e}; config hot-reload disabled");
        }

        // Background update check (startup-only, non-blocking)
        let pending_update = update_checker::spawn_check();

        // Restore session or create a single default workspace
        let (workspaces, active_idx) = if let Some(session) = Self::load_session() {
            log::info!(
                "restoring session: {} workspace(s)",
                session.workspaces.len()
            );
            Self::restore_workspaces(&session, cx)
        } else {
            let ws_id = next_workspace_id();
            let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
            cx.subscribe(&terminal, Self::handle_terminal_event)
                .detach();
            let pane = cx.new(|cx| Pane::new(terminal, ws_id, cx));
            cx.subscribe(&pane, Self::handle_pane_event).detach();
            let dir_name = std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "Terminal 1".into());
            let ws = Workspace::with_id(ws_id, dir_name, pane);
            (vec![ws], 0)
        };

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
        // Watch all workspaces' .git directories
        if let Some(ref mut watcher) = git_watcher {
            for ws in &workspaces {
                if let Some(ref git_dir) = ws.git_dir {
                    if let Err(e) = watcher.watch(git_dir, notify::RecursiveMode::NonRecursive) {
                        log::warn!("git watcher: failed to watch {}: {e}", git_dir.display());
                    } else {
                        *git_watch_counts.entry(git_dir.clone()).or_insert(0) += 1;
                    }
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
                    smol::Timer::after(std::time::Duration::from_millis(200)).await;

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

        // Poll IPC requests + config changes every 50ms
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_millis(50)).await;
                    let result = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                            app.process_ipc_requests(cx);
                            app.process_config_changes(cx);
                            app.process_update_check(cx);
                        })
                    });
                    if result.is_err() {
                        break;
                    }
                }
            },
        )
        .detach();

        // Config hot-reload is now driven by ConfigWatcher (notify crate, 300ms debounce).
        // Changes are picked up in the 50ms IPC poll loop below via process_config_changes().

        // Fallback: poll git metadata for all workspaces every 30s.
        // Primary detection is event-driven (US-003 notify watcher above).
        // This timer catches edge cases where file system events are missed.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(30)).await;

                    // Phase 1: collect CWDs (cheap, main thread)
                    let cwds = cx.update(|cx| {
                        this.update(cx, |app: &mut Self, _cx: &mut Context<Self>| {
                            app.workspaces
                                .iter()
                                .map(|ws| ws.cwd.clone())
                                .collect::<Vec<String>>()
                        })
                    });
                    let cwds = match cwds {
                        Ok(c) => c,
                        Err(_) => break,
                    };

                    // Phase 2: run git probes off main thread
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

                    // Phase 3: apply results (cheap, main thread)
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
            },
        )
        .detach();

        // Stale PID sweep: every 30s, probe registered AI agent PIDs with
        // kill(pid, 0) to detect crashed processes and clean up sidebar state.
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(30)).await;
                    if cx
                        .update(|cx| {
                            this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                                app.sweep_stale_pids(cx);
                            })
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            },
        )
        .detach();

        // Port scanning and CWD detection are now event-driven:
        // - TerminalEvent::ActivityBurst → schedule_port_scan()
        // - TerminalEvent::CwdChanged → handle_cwd_change()
        // See handle_terminal_event() for the push-based implementation.

        Self {
            workspaces,
            active_idx,
            renaming_idx: None,
            rename_text: String::new(),
            pending_config,
            ipc_rx,
            title_bar,
            git_watcher,
            git_event_rx,
            git_watch_counts,
            settings_section: None,
            home_dir: std::env::var("HOME").unwrap_or_default(),
            effective_shortcuts: keybindings::effective_shortcuts(
                &paneflow_config::loader::load_config().shortcuts,
            ),
            recording_shortcut_idx: None,
            settings_focus: cx.focus_handle(),
            mono_font_names: Vec::new(),
            font_dropdown_open: false,
            font_search: String::new(),
            notifications: Vec::new(),
            notif_menu_open: false,
            workspace_menu_open: None,
            title_bar_menu_open: None,
            toast: None,
            _toast_task: None,
            loader_anim_running: false,
            swap_source: None,
            closed_panes: Vec::new(),
            show_about_dialog: false,
            pending_update,
            update_status: None,
        }
    }

    // ── Session persistence ──────────────────────────────────────────

    fn save_session(&self, cx: &App) {
        let state = paneflow_config::schema::SessionState {
            version: 1,
            active_workspace: self.active_idx,
            workspaces: self
                .workspaces
                .iter()
                .map(|ws| paneflow_config::schema::WorkspaceSession {
                    title: ws.title.clone(),
                    cwd: ws.cwd.clone(),
                    layout: ws.serialize_layout(cx),
                })
                .collect(),
        };
        let Some(path) = paneflow_config::loader::session_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&state) {
            Ok(json) => {
                // Atomic write: write to temp file, then rename. This prevents
                // corruption if the process is killed mid-write (US-013).
                let tmp_path = path.with_extension("json.tmp");
                match std::fs::write(&tmp_path, &json) {
                    Ok(()) => {
                        if let Err(e) = std::fs::rename(&tmp_path, &path) {
                            log::warn!("session save rename failed: {e}");
                            let _ = std::fs::remove_file(&tmp_path);
                        }
                    }
                    Err(e) => {
                        log::warn!("session save failed: {e}");
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                }
            }
            Err(e) => log::warn!("session serialize failed: {e}"),
        }
    }

    fn load_session() -> Option<paneflow_config::schema::SessionState> {
        let path = paneflow_config::loader::session_path()?;
        let data = std::fs::read_to_string(&path).ok()?;
        let state: paneflow_config::schema::SessionState = serde_json::from_str(&data).ok()?;
        if state.workspaces.is_empty() {
            return None;
        }
        Some(state)
    }

    /// Rebuild workspaces from a saved session. Each workspace's layout tree
    /// is reconstructed via `LayoutTree::from_layout_node` with CWD-aware
    /// terminal spawning. Returns the workspace list and active index.
    fn restore_workspaces(
        session: &paneflow_config::schema::SessionState,
        cx: &mut Context<Self>,
    ) -> (Vec<Workspace>, usize) {
        use std::path::PathBuf;

        let mut workspaces = Vec::new();

        for ws_session in &session.workspaces {
            let cwd = PathBuf::from(&ws_session.cwd);
            let ws_id = next_workspace_id();

            if let Some(mut layout) = ws_session.layout.clone() {
                paneflow_config::loader::validate_layout(&mut layout);
                let mut pane_deque: VecDeque<Entity<Pane>> = VecDeque::new();
                let ws_cwd = cwd.clone();
                let tree = LayoutTree::from_layout_node(&layout, &mut pane_deque, &mut |node| {
                    let surfaces = match node {
                        LayoutNode::Pane { surfaces } => surfaces.as_slice(),
                        _ => &[],
                    };
                    Self::spawn_pane_from_surfaces(ws_id, surfaces, &ws_cwd, cx)
                });
                workspaces.push(Workspace::with_layout_and_id(
                    ws_id,
                    ws_session.title.clone(),
                    cwd,
                    tree,
                ));
            } else {
                // No saved layout — single terminal in the workspace CWD
                let terminal =
                    cx.new(|cx| TerminalView::with_cwd(ws_id, Some(cwd.clone()), None, cx));
                cx.subscribe(&terminal, Self::handle_terminal_event)
                    .detach();
                let pane = cx.new(|cx| Pane::new(terminal, ws_id, cx));
                cx.subscribe(&pane, Self::handle_pane_event).detach();
                workspaces.push(Workspace::with_cwd_and_id(
                    ws_id,
                    ws_session.title.clone(),
                    cwd,
                    pane,
                ));
            }
        }

        let active_idx = session
            .active_workspace
            .min(workspaces.len().saturating_sub(1));
        (workspaces, active_idx)
    }

    /// Create a `Pane` (with one tab per surface) from serialized surface
    /// definitions. Falls back to a single terminal in `fallback_cwd` when
    /// the surface list is empty.
    fn spawn_pane_from_surfaces(
        workspace_id: u64,
        surfaces: &[paneflow_config::schema::SurfaceDefinition],
        fallback_cwd: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> Entity<Pane> {
        use std::path::PathBuf;

        let mut focus_idx: usize = 0;
        let terminals: Vec<Entity<TerminalView>> = if surfaces.is_empty() {
            let t = cx.new(|cx| {
                TerminalView::with_cwd(workspace_id, Some(fallback_cwd.to_path_buf()), None, cx)
            });
            cx.subscribe(&t, Self::handle_terminal_event).detach();
            vec![t]
        } else {
            surfaces
                .iter()
                .enumerate()
                .map(|(i, surface)| {
                    let cwd = surface
                        .cwd
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| fallback_cwd.to_path_buf());
                    let t = cx.new(|cx| TerminalView::with_cwd(workspace_id, Some(cwd), None, cx));
                    // Restore saved scrollback into the terminal grid
                    if let Some(ref scrollback) = surface.scrollback {
                        t.read(cx).terminal.restore_scrollback(scrollback);
                    }
                    cx.subscribe(&t, Self::handle_terminal_event).detach();
                    if surface.focus == Some(true) {
                        focus_idx = i;
                    }
                    t
                })
                .collect()
        };

        let first = terminals[0].clone();
        let pane = cx.new(|cx| {
            let mut p = Pane::new(first, workspace_id, cx);
            for tab in &terminals[1..] {
                p.add_tab(tab.clone(), cx);
            }
            p.selected_idx = focus_idx.min(terminals.len() - 1);
            p
        });
        cx.subscribe(&pane, Self::handle_pane_event).detach();
        pane
    }

    fn process_ipc_requests(&mut self, cx: &mut Context<Self>) {
        while let Ok(req) = self.ipc_rx.try_recv() {
            let result = self.handle_ipc(&req.method, &req.params, cx);
            let _ = req.response_tx.send(result);
        }
    }

    /// Apply any pending config change deposited by the background `ConfigWatcher`.
    fn process_config_changes(&mut self, cx: &mut Context<Self>) {
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
    fn process_update_check(&mut self, cx: &mut Context<Self>) {
        if self.update_status.is_some() {
            return; // Already resolved
        }
        let status = self
            .pending_update
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(status) = status
            && !matches!(status, update_checker::UpdateStatus::Checking)
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
                let tool = ai_detector::AiTool::from_name(tool_name);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_detector::AiToolState::Thinking(tool);
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
                let tool = ai_detector::AiTool::from_name(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    // Keep Thinking state (no transition if already Thinking)
                    if !matches!(ws.ai_state, ai_detector::AiToolState::Thinking(_)) {
                        ws.ai_state = ai_detector::AiToolState::Thinking(tool);
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
                let tool = ai_detector::AiTool::from_name(tool_str);
                let message: String = hook
                    .and_then(|h| h.get("message"))
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("message").and_then(|v| v.as_str()))
                    .unwrap_or("Needs input")
                    .chars()
                    .take(512)
                    .collect();

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_detector::AiToolState::WaitingForInput(tool);
                    ws.active_tool_name = None;
                    let title = ws.title.clone();
                    self.notifications.push(Notification {
                        workspace_id,
                        workspace_title: title,
                        message,
                        kind: ai_detector::AiToolState::WaitingForInput(tool),
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
                let tool = ai_detector::AiTool::from_name(tool_str);

                if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.id == workspace_id) {
                    ws.ai_state = ai_detector::AiToolState::Finished(tool);
                    ws.active_tool_name = None;
                    let title = ws.title.clone();
                    self.notifications.push(Notification {
                        workspace_id,
                        workspace_title: title,
                        message: format!("{} finished", tool.label()),
                        kind: ai_detector::AiToolState::Finished(tool),
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
                                        && matches!(
                                            ws.ai_state,
                                            ai_detector::AiToolState::Finished(_)
                                        )
                                    {
                                        ws.ai_state = ai_detector::AiToolState::Inactive;
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
                    ws.ai_state = ai_detector::AiToolState::Inactive;
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
        self.notif_menu_open = false;
        self.workspace_menu_open = None;
        self.title_bar_menu_open = None;
        if idx < self.workspaces.len() && idx != self.active_idx {
            self.active_idx = idx;
            self.workspaces[idx].focus_first(window, cx);
            self.save_session(cx);
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
        let ws_id = next_workspace_id();
        let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
        let pane = self.create_pane(terminal, ws_id, cx);
        let ws = Workspace::with_id(ws_id, format!("Terminal {n}"), pane);
        self.watch_git_dir(&ws);
        self.workspaces.push(ws);
        self.active_idx = self.workspaces.len() - 1;
        self.workspaces[self.active_idx].focus_first(window, cx);
        self.save_session(cx);
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
            multiple: true,
            prompt: None,
        });
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                if let Ok(Ok(Some(paths))) = receiver.await {
                    let _ = cx.update(|cx| {
                        this.update(cx, |app, cx| {
                            for path in paths {
                                if app.workspaces.len() >= MAX_WORKSPACES {
                                    break;
                                }
                                let n = app.workspaces.len() + 1;
                                let dir = path.clone();
                                let title = dir
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| format!("Terminal {n}"));
                                let ws_id = next_workspace_id();
                                let terminal = cx
                                    .new(|cx| TerminalView::with_cwd(ws_id, Some(path), None, cx));
                                let pane = app.create_pane(terminal, ws_id, cx);
                                let ws = Workspace::with_cwd_and_id(ws_id, title, dir, pane);
                                app.watch_git_dir(&ws);
                                app.workspaces.push(ws);
                            }
                            app.active_idx = app.workspaces.len() - 1;
                            app.save_session(cx);
                            cx.notify();
                        })
                    });
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
        // Inherit CWD from the focused pane's active terminal (fresh /proc read)
        let source_cwd = self
            .active_workspace()
            .and_then(|ws| ws.root.as_ref())
            .and_then(|root| root.focused_pane(window, cx))
            .and_then(|pane| pane.read(cx).active_terminal().read(cx).terminal.cwd_now());
        let ws_id = self.active_workspace().map(|ws| ws.id).unwrap_or(0);
        let new_terminal = cx.new(|cx| TerminalView::with_cwd(ws_id, source_cwd, None, cx));
        let new_pane = self.create_pane(new_terminal, ws_id, cx);
        if let Some(ws) = self.active_workspace_mut()
            && let Some(root) = &mut ws.root
            && root.split_at_focused(direction, new_pane.clone(), window, cx)
        {
            new_pane.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.save_session(cx);
        cx.notify();
    }

    fn handle_split_h(&mut self, _: &SplitHorizontally, w: &mut Window, cx: &mut Context<Self>) {
        self.split(SplitDirection::Horizontal, w, cx);
    }
    fn handle_split_v(&mut self, _: &SplitVertically, w: &mut Window, cx: &mut Context<Self>) {
        self.split(SplitDirection::Vertical, w, cx);
    }

    fn handle_close_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        // Capture state of the pane being closed for undo (US-014).
        // Must happen BEFORE the tree mutation that drops the pane entity.
        let workspace_idx = self.active_idx;
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
        {
            let closing_pane = if ws.is_zoomed() {
                root.first_leaf()
            } else {
                root.focused_pane(window, cx)
            };
            if let Some(pane) = closing_pane {
                let tv = pane.read(cx).active_terminal();
                let tv_ref = tv.read(cx);
                let record = ClosedPaneRecord {
                    cwd: tv_ref
                        .terminal
                        .current_cwd
                        .as_ref()
                        .map(std::path::PathBuf::from)
                        .or_else(|| tv_ref.terminal.cwd_now()),
                    scrollback: tv_ref.terminal.extract_scrollback(),
                    workspace_idx,
                };
                if self.closed_panes.len() >= MAX_CLOSED_PANES {
                    self.closed_panes.remove(0);
                }
                self.closed_panes.push(record);
            }
        }

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
            let ws_id = ws.id;
            if self.workspaces.len() > 1 {
                self.close_workspace_at(self.active_idx, window, cx);
            } else {
                // Last workspace: spawn a fresh pane instead of destroying
                let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
                let new_pane = self.create_pane(terminal, ws_id, cx);
                if let Some(ws) = self.active_workspace_mut() {
                    ws.root = Some(LayoutTree::Leaf(new_pane));
                }
                self.workspaces[self.active_idx].focus_first(window, cx);
            }
        }

        self.save_session(cx);
        cx.notify();
    }

    fn handle_undo_close_pane(
        &mut self,
        _: &UndoClosePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self.closed_panes.pop() else {
            return; // No closed panes to restore
        };

        // Switch to the workspace where the pane was closed, if it still exists
        if record.workspace_idx < self.workspaces.len() {
            self.active_idx = record.workspace_idx;
        }

        let ws_id = self.active_workspace().map(|ws| ws.id).unwrap_or(0);
        let cwd = record.cwd;
        let new_terminal = cx.new(|cx| TerminalView::with_cwd(ws_id, cwd, None, cx));

        // Restore scrollback into the new terminal's grid
        if let Some(ref scrollback) = record.scrollback {
            new_terminal
                .read(cx)
                .terminal
                .restore_scrollback(scrollback);
        }

        let new_pane = self.create_pane(new_terminal, ws_id, cx);

        // Insert via split from the currently focused pane
        if let Some(ws) = self.active_workspace_mut()
            && let Some(ref mut root) = ws.root
            && root.split_at_focused(SplitDirection::Horizontal, new_pane.clone(), window, cx)
        {
            new_pane.read(cx).focus_handle(cx).focus(window, cx);
        } else if let Some(ws) = self.active_workspace_mut() {
            // No existing root (empty workspace) — set as the root
            ws.root = Some(LayoutTree::Leaf(new_pane.clone()));
            new_pane.read(cx).focus_handle(cx).focus(window, cx);
        }

        self.save_session(cx);
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
        self.save_session(cx);
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
            && let Some(saved) = ws.saved_layout.take()
        {
            ws.root = Some(saved);
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
        self.save_session(cx);
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

        let ws_id = self.active_workspace().map(|ws| ws.id).unwrap_or(0);
        let app_ref = &mut *self;
        let tree = LayoutTree::from_layout_node(layout, &mut pane_deque, &mut |_node| {
            let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
            app_ref.create_pane(terminal, ws_id, cx)
        });

        let Some(ws) = self.active_workspace_mut() else {
            return Err("No active workspace".into());
        };
        ws.root = Some(tree);
        self.save_session(cx);
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
            && let Some(saved) = ws.saved_layout.take()
        {
            ws.root = Some(saved);
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
        self.save_session(cx);
        cx.notify();
    }

    fn handle_layout_tiled(
        &mut self,
        _: &LayoutTiled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_layout_preset(LayoutTree::tiled, window, cx);
    }

    fn handle_split_equalize(
        &mut self,
        _: &SplitEqualize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ws) = self.active_workspace_mut()
            && let Some(ref root) = ws.root
        {
            root.equalize_ratios();
            self.save_session(cx);
            cx.notify();
        }
    }

    fn handle_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && let Some(pane) = root.focused_pane(window, cx)
        {
            let ws_id = ws.id;
            let terminal = cx.new(|cx| TerminalView::new(ws_id, cx));
            cx.subscribe(&terminal, Self::handle_terminal_event)
                .detach();
            pane.update(cx, |p, cx| {
                p.add_tab(terminal, cx);
            });
            pane.read(cx).focus_handle(cx).focus(window, cx);
            self.save_session(cx);
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
            self.save_session(cx);
            cx.notify();
        }
    }

    fn handle_swap_pane(&mut self, _: &SwapPane, window: &mut Window, cx: &mut Context<Self>) {
        if self.swap_source.is_some() {
            // Already in swap mode — toggle off (cancel)
            self.swap_source = None;
            SWAP_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
        } else if let Some(ws) = self.active_workspace()
            && let Some(root) = &ws.root
            && root.leaf_count() > 1
        {
            // Enter swap mode: record the currently focused pane
            if let Some(pane) = root.focused_pane(window, cx) {
                self.swap_source = Some(pane);
                SWAP_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        cx.notify();
    }

    fn cancel_swap_mode(&mut self, cx: &mut Context<Self>) {
        if self.swap_source.is_some() {
            self.swap_source = None;
            SWAP_MODE.store(false, std::sync::atomic::Ordering::Relaxed);
            cx.notify();
        }
    }

    fn handle_focus(&mut self, dir: FocusDirection, window: &mut Window, cx: &mut Context<Self>) {
        // When swap mode is active, perform the swap instead of just moving focus
        if let Some(source) = self.swap_source.take() {
            SWAP_MODE.store(false, std::sync::atomic::Ordering::Relaxed);

            if let Some(ws) = self.active_workspace()
                && let Some(root) = &ws.root
            {
                // Move focus to find the target pane
                root.focus_in_direction(dir, window, cx);
                if let Some(target) = root.focused_pane(window, cx)
                    && target != source
                {
                    // Swap the panes in the tree
                    if let Some(ws) = self.active_workspace_mut()
                        && let Some(ref mut root) = ws.root
                    {
                        root.swap_panes(&source, &target);
                    }
                    // Focus the original source pane (now at the target's position)
                    source.read(cx).focus_handle(cx).focus(window, cx);
                }
            }
            self.save_session(cx);
            cx.notify();
            return;
        }

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

    fn handle_copy_workspace_path(
        &mut self,
        _: &CopyWorkspacePath,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_workspace_path(self.active_idx, cx);
    }

    fn handle_reveal_workspace_in_file_manager(
        &mut self,
        _: &RevealWorkspaceInFileManager,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reveal_workspace_in_file_manager(self.active_idx, cx);
    }

    fn handle_open_workspace_in_zed(
        &mut self,
        _: &OpenWorkspaceInZed,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workspace_in_editor(self.active_idx, "zed", "Zed", cx);
    }

    fn handle_open_workspace_in_cursor(
        &mut self,
        _: &OpenWorkspaceInCursor,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workspace_in_editor(self.active_idx, "cursor", "Cursor", cx);
    }

    fn handle_open_workspace_in_vscode(
        &mut self,
        _: &OpenWorkspaceInVsCode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workspace_in_editor(self.active_idx, "code", "VS Code", cx);
    }

    fn handle_open_workspace_in_windsurf(
        &mut self,
        _: &OpenWorkspaceInWindsurf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workspace_in_editor(self.active_idx, "windsurf", "Windsurf", cx);
    }

    fn close_workspace_at(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Guard: don't close the last workspace
        if self.workspaces.len() <= 1 || idx >= self.workspaces.len() {
            return;
        }
        self.workspace_menu_open = None;
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
        self.save_session(cx);
        cx.notify();
    }

    fn copy_workspace_path(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(ws) = self.workspaces.get(idx) else {
            return;
        };

        cx.write_to_clipboard(ClipboardItem::new_string(ws.cwd.clone()));
        self.show_toast("Path copied", cx);
        self.workspace_menu_open = None;
        cx.notify();
    }

    fn reveal_workspace_in_file_manager(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(ws) = self.workspaces.get(idx) else {
            return;
        };

        if let Err(err) = std::process::Command::new("xdg-open").arg(&ws.cwd).spawn() {
            log::warn!("failed to reveal workspace path in file manager: {err}");
        }

        self.workspace_menu_open = None;
        cx.notify();
    }

    fn open_workspace_in_editor(
        &mut self,
        idx: usize,
        command: &str,
        label: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(ws) = self.workspaces.get(idx) else {
            return;
        };

        if let Err(err) = std::process::Command::new(command)
            .current_dir(&ws.cwd)
            .arg(".")
            .spawn()
        {
            log::warn!("failed to open workspace in {label}: {err}");
        }

        self.workspace_menu_open = None;
        cx.notify();
    }

    fn shortcut_for_description(&self, description: &str) -> Option<&str> {
        self.effective_shortcuts
            .iter()
            .find(|entry| entry.description == description)
            .map(|entry| entry.key.as_str())
    }

    fn render_context_menu_item(
        &self,
        id: SharedString,
        label: &str,
        shortcut: Option<SharedString>,
        ui: crate::theme::UiColors,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(ui.text)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(on_click)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label.to_string()),
            )
            .when_some(shortcut, |d, shortcut| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(ui.muted)
                        .child(shortcut),
                )
            })
    }

    fn show_toast(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.toast = Some(Toast {
            message: message.into(),
        });
        cx.notify();

        // Dropping the previous task cancels its timer automatically.
        self._toast_task = Some(cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(std::time::Duration::from_millis(
                    TOAST_ENTER_MS + TOAST_HOLD_MS + TOAST_EXIT_MS,
                ))
                .await;
                let _ = cx.update(|cx| {
                    this.update(cx, |app: &mut Self, cx: &mut Context<Self>| {
                        app.toast = None;
                        app._toast_task = None;
                        cx.notify();
                    })
                });
            },
        ));
    }

    fn commit_rename(&mut self, cx: &App) {
        if let Some(idx) = self.renaming_idx.take() {
            let text = std::mem::take(&mut self.rename_text);
            if !text.is_empty()
                && let Some(ws) = self.workspaces.get_mut(idx)
            {
                ws.title = text;
                self.save_session(cx);
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
        icon_path: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_color(ui.muted)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle).text_color(ui.text)
            })
            .on_click(move |e, w, cx| on_click(e, w, cx))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(icon_path)
                    .text_color(ui.muted),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();
        let mut sidebar = div()
            .relative()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .bg(theme.title_bar_background)
            .border_r_1()
            .border_color(ui.border)
            .flex()
            .flex_col();

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
                        .text_color(ui.text)
                        .child("WORKSPACES"),
                )
                .child(
                    // Right side — action buttons
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.))
                        .child({
                            let has_unread = self.notifications.iter().any(|n| !n.read);
                            div()
                                .relative()
                                .child(self.sidebar_action_btn(
                                    "sidebar-bell",
                                    "icons/bell.svg",
                                    cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.title_bar_menu_open = None;
                                        this.notif_menu_open = !this.notif_menu_open;
                                        cx.notify();
                                    }),
                                ))
                                .when(has_unread, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(2.))
                                            .right(px(2.))
                                            .w(px(6.))
                                            .h(px(6.))
                                            .rounded_full()
                                            .bg(rgb(0xf38ba8)),
                                    )
                                })
                        })
                        .child(self.sidebar_action_btn(
                            "sidebar-settings",
                            "icons/settings.svg",
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_settings_window(window, cx);
                            }),
                        ))
                        .child(self.sidebar_action_btn(
                            "sidebar-new-ws",
                            "icons/plus.svg",
                            cx.listener(|this, _: &ClickEvent, w, cx| {
                                this.create_workspace_with_picker(w, cx);
                            }),
                        )),
                ),
        );

        // ── Notification dropdown menu ──
        if self.notif_menu_open {
            let mut menu = div()
                .id("notif-menu")
                .occlude()
                .absolute()
                .top(px(64.))
                .left(px(6.))
                .w(px(SIDEBAR_WIDTH - 12.))
                .max_h(px(300.))
                .overflow_y_scroll()
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .rounded(px(6.))
                .shadow_lg()
                .flex()
                .flex_col()
                .p(px(4.));

            if self.notifications.is_empty() {
                menu = menu.child(
                    div()
                        .px(px(10.))
                        .py(px(12.))
                        .text_xs()
                        .text_color(ui.muted)
                        .child("No notifications"),
                );
            } else {
                // Newest first
                for (ni, notif) in self.notifications.iter().enumerate().rev() {
                    let ws_id = notif.workspace_id;
                    let is_unread = !notif.read;
                    let notif_idx = ni;
                    menu = menu.child(
                        div()
                            .id(SharedString::from(format!("notif-{ni}")))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(is_unread, |d| {
                                let ui = crate::theme::ui_colors();
                                d.bg(ui.subtle)
                            })
                            .hover(|s| {
                                let ui = crate::theme::ui_colors();
                                s.bg(ui.surface)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                // Find workspace by stable ID
                                if let Some(idx) =
                                    this.workspaces.iter().position(|ws| ws.id == ws_id)
                                {
                                    this.select_workspace(idx, window, cx);
                                }
                                if notif_idx < this.notifications.len() {
                                    this.notifications[notif_idx].read = true;
                                }
                                this.notif_menu_open = false;
                                cx.notify();
                            }))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if is_unread { ui.text } else { ui.muted })
                                    .child(notif.workspace_title.clone()),
                            )
                            .child({
                                let msg_color = match notif.kind {
                                    ai_detector::AiToolState::WaitingForInput(_) => {
                                        gpui::Hsla::from(rgb(0xf9e2af))
                                    }
                                    ai_detector::AiToolState::Finished(_) => {
                                        gpui::Hsla::from(rgb(0xa6e3a1))
                                    }
                                    _ => ui.muted,
                                };
                                div()
                                    .text_xs()
                                    .text_color(msg_color)
                                    .child(notif.message.clone())
                            }),
                    );
                }
            }

            sidebar = sidebar.child(deferred(menu));
        }

        // Workspace list — scrollable area
        let mut list = div()
            .id("workspace-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.))
            .py_2();

        for (i, ws) in self.workspaces.iter().enumerate() {
            let is_active = i == self.active_idx;

            let title = ws.title.clone();
            // Format cwd as ~/... (collapse home dir)
            let cwd_display = {
                if !self.home_dir.is_empty() && ws.cwd.starts_with(&self.home_dir) {
                    format!("~{}", &ws.cwd[self.home_dir.len()..])
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

            let mut card = div()
                .id(SharedString::from(format!("ws-{i}")))
                .mx(px(6.))
                .px(px(10.))
                .py(px(8.))
                .when(is_active, |d| d.bg(ui.surface))
                .rounded(px(6.))
                .cursor_pointer()
                .when(!is_active, |d| {
                    d.hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle)
                    })
                })
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.workspace_menu_open = None;
                    this.title_bar_menu_open = None;
                    let is_double = matches!(e, ClickEvent::Mouse(m) if m.down.click_count == 2);
                    if is_double {
                        this.commit_rename(cx); // commit any previous rename
                        this.rename_text = this.workspaces[idx].title.clone();
                        this.renaming_idx = Some(idx);
                    } else {
                        this.commit_rename(cx);
                        this.select_workspace(idx, window, cx);
                    }
                    cx.notify();
                }))
                .on_aux_click(cx.listener(move |this, e: &ClickEvent, _window, cx| {
                    if e.is_right_click()
                        && let Some(position) = e.mouse_position()
                    {
                        this.commit_rename(cx);
                        this.title_bar_menu_open = None;
                        this.workspace_menu_open = Some(WorkspaceContextMenu { idx, position });
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _window, cx| {
                    if this.renaming_idx != Some(idx) {
                        return;
                    }
                    let key = e.keystroke.key.as_str();
                    match key {
                        "enter" => {
                            this.commit_rename(cx);
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
                            if let Some(ch) = &e.keystroke.key_char
                                && !ch.is_empty()
                                && !e.keystroke.modifiers.control
                                && !e.keystroke.modifiers.platform
                            {
                                this.rename_text.push_str(ch);
                                cx.notify();
                            }
                        }
                    }
                }))
                .flex()
                .flex_col()
                .gap_1();

            // ── Row 1: Title + action menu ──
            let title_el = if self.renaming_idx == Some(i) {
                div()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(gpui::FontWeight::BOLD)
                    .bg(ui.overlay)
                    .px_1()
                    .rounded_sm()
                    .child(format!("{}|", self.rename_text))
            } else {
                div()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title)
            };
            let title_row = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .min_w_0()
                .child(title_el);

            card = card.child(title_row);

            // ── Row 2: Git branch ──
            if !ws.git_branch.is_empty() {
                card = card.child(
                    div()
                        .text_color(rgb(0x89b4fa)) // Catppuccin Blue
                        .text_xs()
                        .truncate()
                        .child(format!(" {}", ws.git_branch)),
                );
            }

            // ── Row 3: Subtitle — pane count as status ──
            card = card.child(div().text_color(ui.muted).text_xs().child(pane_label));

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
                        .text_color(ui.muted)
                        .text_xs()
                        .child("No changes detected"),
                );
            }

            // ── Row 4: Active ports — clickable URL badges ──
            if !ws.active_ports.is_empty() {
                let mut ports_row = div().flex().flex_row().flex_wrap().gap(px(4.));

                for (pi, port) in ws.active_ports.iter().take(4).enumerate() {
                    let info = ws.service_labels.get(port);
                    let is_frontend = info.is_some_and(|i| i.is_frontend);
                    let label = if let Some(i) = info
                        && let Some(ref l) = i.label
                    {
                        format!("{l} :{port}")
                    } else {
                        format!(":{port}")
                    };

                    if is_frontend {
                        let url = info
                            .and_then(|i| i.url.clone())
                            .unwrap_or_else(|| format!("http://localhost:{port}"));
                        ports_row = ports_row.child(
                            div()
                                .id(SharedString::from(format!("port-{idx}-{pi}")))
                                .px(px(6.))
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(ui.subtle)
                                .text_size(px(11.))
                                .text_color(ui.accent)
                                .cursor_pointer()
                                .hover(|s| s.text_color(rgb(0xa0e8ff)))
                                .on_click(move |_, _, _| {
                                    let _ =
                                        std::process::Command::new("xdg-open").arg(&url).spawn();
                                })
                                .child(label),
                        );
                    } else {
                        ports_row =
                            ports_row.child(div().text_xs().text_color(ui.muted).child(label));
                    }
                }

                if ws.active_ports.len() > 4 {
                    ports_row = ports_row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(0xffffff))
                            .child(format!("+{} more", ws.active_ports.len() - 4)),
                    );
                }

                card = card.child(ports_row);
            }

            // ── Row: AI tool status (Claude Code / Codex) ──
            match ws.ai_state {
                ai_detector::AiToolState::Thinking(tool) => {
                    let (frames, color): (&[char], u32) = match tool {
                        ai_detector::AiTool::Claude => (&CLAUDE_SPINNER_FRAMES, 0xd97757),
                        ai_detector::AiTool::Codex => (&CODEX_SPINNER_FRAMES, 0x10a37f),
                    };
                    let angle = ws.loader_angle.get();
                    let idx = ((angle / std::f32::consts::TAU) * frames.len() as f32) as usize
                        % frames.len();
                    let spinner = frames[idx];
                    card = card.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .w(px(14.))
                                    .h(px(14.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(color))
                                    .text_xs()
                                    .child(format!("{spinner}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(color))
                                    .child(format!("{} thinking…", tool.label())),
                            ),
                    );
                }
                ai_detector::AiToolState::WaitingForInput(tool) => {
                    card = card.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                svg()
                                    .size(px(14.))
                                    .flex_none()
                                    .path("icons/bell.svg")
                                    .text_color(rgb(0xf9e2af)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xf9e2af))
                                    .child(format!("{} needs input", tool.label())),
                            ),
                    );
                }
                ai_detector::AiToolState::Finished(tool) => {
                    card = card.child(
                        div().flex().flex_row().items_center().gap(px(6.)).child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(0xa6e3a1))
                                .child(format!("✓ {} done", tool.label())),
                        ),
                    );
                }
                ai_detector::AiToolState::Inactive => {}
            }

            // ── Row 5: Working directory (monospace-style) ──
            card = card.child(div().text_color(ui.muted).text_xs().child(cwd_display));

            list = list.child(card);
        }

        sidebar = sidebar.child(list);

        sidebar
    }

    // ── Settings page: sidebar + content ──────────────────────────────

    fn render_settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let section = self.settings_section.unwrap_or(SettingsSection::Shortcuts);
        let ui = crate::theme::ui_colors();

        // Header bar: title + close button
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(24.))
            .pt(px(20.))
            .pb(px(16.))
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(ui.text)
                    .child("Settings"),
            )
            .child(
                div()
                    .id("settings-close")
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(ui.subtle))
                    .text_size(px(14.))
                    .text_color(ui.muted)
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.close_settings(cx);
                        cx.notify();
                    }))
                    .child("Close"),
            );

        // Content area based on active section
        let content = match section {
            SettingsSection::Shortcuts => self.render_shortcuts_content(cx).into_any_element(),
            SettingsSection::Appearance => self.render_appearance_content(cx).into_any_element(),
        };

        div()
            .id("settings-page")
            .track_focus(&self.settings_focus)
            .on_key_down(cx.listener(Self::handle_settings_key_down))
            .flex()
            .flex_col()
            .size_full()
            .bg(ui.base)
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    // Left sidebar
                    .child(self.render_settings_sidebar(section, ui, cx))
                    // Right content
                    .child(
                        div()
                            .id("settings-content")
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(24.))
                            .py(px(12.))
                            .child(content),
                    ),
            )
    }

    fn render_settings_sidebar(
        &self,
        active: SettingsSection,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sections = [
            ("Shortcuts", SettingsSection::Shortcuts),
            ("Appearance", SettingsSection::Appearance),
        ];

        let mut nav = div()
            .flex()
            .flex_col()
            .w(px(180.))
            .h_full()
            .border_r_1()
            .border_color(ui.border)
            .bg(ui.base)
            .pt(px(4.));

        for (label, section) in sections {
            let is_active = section == active;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("nav-{label}")))
                    .mx(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .cursor(CursorStyle::PointingHand)
                    .when(is_active, |d| d.bg(ui.overlay).text_color(ui.text))
                    .when(!is_active, |d| {
                        d.text_color(ui.muted).hover(|s| s.bg(ui.subtle))
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.settings_section = Some(section);
                        // Reset editing state when switching tabs
                        this.font_dropdown_open = false;
                        this.font_search.clear();
                        if this.recording_shortcut_idx.is_some() {
                            this.recording_shortcut_idx = None;
                            let config = paneflow_config::loader::load_config();
                            keybindings::apply_keybindings(cx, &config.shortcuts);
                        }
                        cx.notify();
                    }))
                    .child(label),
            );
        }

        nav
    }

    fn render_shortcuts_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let recording_idx = self.recording_shortcut_idx;

        // Section header + reset button
        let section_header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .pb(px(16.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child("KEYBOARD SHORTCUTS"),
            )
            .child(
                div()
                    .id("reset-shortcuts")
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor(CursorStyle::PointingHand)
                    .bg(ui.subtle)
                    .hover(|s| s.bg(ui.overlay))
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        config_writer::reset_shortcuts();
                        let config = paneflow_config::loader::load_config();
                        keybindings::apply_keybindings(cx, &config.shortcuts);
                        this.effective_shortcuts =
                            keybindings::effective_shortcuts(&config.shortcuts);
                        this.recording_shortcut_idx = None;
                        cx.notify();
                    }))
                    .child("Reset to defaults"),
            );

        let mut list = div().flex().flex_col();

        for (i, entry) in self.effective_shortcuts.iter().enumerate() {
            let is_recording = recording_idx == Some(i);

            let key_badge = if is_recording {
                div()
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(ui.overlay)
                    .text_size(px(12.))
                    .text_color(ui.accent)
                    .child("Press a key...")
            } else {
                div()
                    .px(px(8.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(ui.subtle)
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .child(entry.key.clone())
            };

            let row_bg = if is_recording { ui.overlay } else { ui.base };

            list = list.child(
                div()
                    .id(("shortcut", i))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(4.))
                    .bg(row_bg)
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(ui.overlay))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.recording_shortcut_idx = Some(i);
                        cx.clear_key_bindings();
                        this.settings_focus.focus(window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(ui.text)
                            .child(entry.description.clone()),
                    )
                    .child(key_badge),
            );
        }

        div().flex().flex_col().child(section_header).child(list)
    }

    fn render_appearance_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = paneflow_config::loader::load_config();
        let ui = crate::theme::ui_colors();
        let current_font =
            crate::terminal_element::resolve_font_family(config.font_family.as_deref());
        let current_theme = config
            .theme
            .clone()
            .unwrap_or_else(|| "Catppuccin Mocha".to_string());

        let section_header = div()
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(ui.text)
            .child("APPEARANCE");

        // ── Theme ──
        let theme_label = div()
            .text_size(px(13.))
            .text_color(ui.muted)
            .pb(px(6.))
            .child("Theme");

        let themes = [
            ("Catppuccin Mocha", "Default (Dark)"),
            ("PaneFlow Light", "Light"),
        ];
        let mut theme_row_inner = div().flex().flex_row().gap(px(8.));

        for (theme_id, label) in themes {
            let is_active = current_theme == theme_id;
            let theme_id_owned = theme_id.to_string();
            theme_row_inner = theme_row_inner.child(
                div()
                    .id(SharedString::from(format!("theme-{theme_id}")))
                    .px(px(14.))
                    .py(px(8.))
                    .rounded(px(6.))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(13.))
                    .when(is_active, |d| d.bg(ui.accent).text_color(ui.base))
                    .when(!is_active, |d| {
                        d.bg(ui.subtle)
                            .text_color(ui.muted)
                            .hover(|s| s.bg(ui.overlay))
                    })
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _w, cx| {
                        config_writer::save_config_value(
                            "theme",
                            serde_json::Value::String(theme_id_owned.clone()),
                        );
                        crate::theme::invalidate_theme_cache();
                        cx.notify();
                    }))
                    .child(label),
            );
        }

        let theme_row = div()
            .flex()
            .flex_col()
            .pb(px(20.))
            .child(theme_label)
            .child(theme_row_inner);

        // ── Font Family ──
        let font_label = div()
            .text_size(px(13.))
            .text_color(ui.muted)
            .pb(px(6.))
            .child("Font Family");

        let font_value_text = if self.font_dropdown_open {
            if self.font_search.is_empty() {
                "Search fonts...".to_string()
            } else {
                format!("{}|", self.font_search)
            }
        } else {
            current_font.clone()
        };

        let font_value_color = if self.font_dropdown_open {
            ui.accent
        } else {
            ui.text
        };

        let font_badge = div()
            .id("font-family-badge")
            .px(px(12.))
            .py(px(6.))
            .rounded(px(4.))
            .bg(ui.overlay)
            .cursor(CursorStyle::PointingHand)
            .hover(|s| s.bg(ui.subtle))
            .text_size(px(13.))
            .text_color(font_value_color)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.font_dropdown_open = !this.font_dropdown_open;
                this.font_search.clear();
                if this.font_dropdown_open && this.mono_font_names.is_empty() {
                    this.mono_font_names = config_writer::load_mono_fonts();
                }
                this.settings_focus.focus(window, cx);
                cx.notify();
            }))
            .child(font_value_text);

        let mut font_row = div()
            .flex()
            .flex_col()
            .pb(px(20.))
            .child(font_label)
            .child(font_badge);

        // Dropdown list
        if self.font_dropdown_open {
            let search = self.font_search.to_lowercase();
            let filtered: Vec<&String> = self
                .mono_font_names
                .iter()
                .filter(|name| search.is_empty() || name.to_lowercase().contains(&search))
                .collect();

            let mut dropdown = div()
                .id("font-dropdown")
                .flex()
                .flex_col()
                .mt(px(4.))
                .rounded(px(6.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .max_h(px(250.))
                .overflow_y_scroll();

            for (i, name) in filtered.iter().enumerate() {
                let name_owned = (*name).clone();
                let is_current = **name == current_font;
                dropdown = dropdown.child(
                    div()
                        .id(("font", i))
                        .px(px(12.))
                        .py(px(6.))
                        .cursor(CursorStyle::PointingHand)
                        .text_size(px(13.))
                        .when(is_current, |d| d.text_color(ui.accent).bg(ui.subtle))
                        .when(!is_current, |d| {
                            d.text_color(ui.text).hover(|s| s.bg(ui.subtle))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            config_writer::save_config_value(
                                "font_family",
                                serde_json::Value::String(name_owned.clone()),
                            );
                            this.font_dropdown_open = false;
                            this.font_search.clear();
                            cx.notify();
                        }))
                        .child((*name).clone()),
                );
            }

            if filtered.is_empty() {
                dropdown = dropdown.child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("No matching fonts"),
                );
            }

            font_row = font_row.child(dropdown);
        }

        // ── Font Preview ──
        let preview = div()
            .pb(px(20.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(ui.muted)
                    .pb(px(6.))
                    .child("Preview"),
            )
            .child(
                div()
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(6.))
                    .bg(ui.preview_bg)
                    .border_1()
                    .border_color(ui.border)
                    .font_family(current_font.clone())
                    .text_size(px(14.))
                    .text_color(ui.text)
                    .child("The quick brown fox jumps over the lazy dog\nABCDEFGHIJKLM 0123456789 {}[]()"),
            );

        // ── Reset to defaults ──
        let reset_btn = div()
            .id("reset-appearance")
            .px(px(10.))
            .py(px(4.))
            .rounded(px(4.))
            .cursor(CursorStyle::PointingHand)
            .bg(ui.subtle)
            .hover(|s| s.bg(ui.overlay))
            .text_size(px(12.))
            .text_color(ui.text)
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                config_writer::save_config_value("font_family", serde_json::Value::Null);
                config_writer::save_config_value("theme", serde_json::Value::Null);
                crate::theme::invalidate_theme_cache();
                this.font_dropdown_open = false;
                cx.notify();
            }))
            .child("Reset to defaults");

        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .pb(px(16.))
                    .child(section_header)
                    .child(reset_btn),
            )
            .child(theme_row)
            .child(font_row)
            .child(preview)
    }

    fn handle_settings_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Font dropdown search
        if self.font_dropdown_open {
            let key = event.keystroke.key.as_str();
            match key {
                "escape" => {
                    self.font_dropdown_open = false;
                    self.font_search.clear();
                    cx.notify();
                }
                "backspace" => {
                    self.font_search.pop();
                    cx.notify();
                }
                _ => {
                    if let Some(ch) = &event.keystroke.key_char
                        && !ch.is_empty()
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.platform
                    {
                        self.font_search.push_str(ch);
                        cx.notify();
                    }
                }
            }
            return;
        }

        // Shortcut recording (only on Shortcuts tab)
        if self.settings_section == Some(SettingsSection::Shortcuts) {
            self.handle_shortcut_recording(event, _window, cx);
        }
    }

    fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = None;
        self.title_bar_menu_open = None;
        self.font_dropdown_open = false;
        self.font_search.clear();
        if self.recording_shortcut_idx.is_some() {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
        }
        crate::terminal::SUPPRESS_REPAINTS.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn open_settings_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notif_menu_open = false;
        self.workspace_menu_open = None;
        self.title_bar_menu_open = None;
        settings_window::open_or_focus(window, cx);
        cx.notify();
    }

    fn handle_shortcut_recording(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(idx) = self.recording_shortcut_idx else {
            return;
        };

        // Ignore bare modifier presses (Shift alone, Ctrl alone, etc.)
        if keybindings::is_bare_modifier(&event.keystroke) {
            return;
        }

        // Escape cancels recording
        if event.keystroke.key == "escape" {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        }

        // Get the action name for this shortcut index
        let Some(action_name) = keybindings::action_name_at(idx) else {
            self.recording_shortcut_idx = None;
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            cx.notify();
            return;
        };

        // Format keystroke to GPUI string (e.g. "ctrl-shift-d")
        let new_key = event.keystroke.to_string();

        // Save to config file
        config_writer::save_shortcut(&new_key, action_name);

        // Re-apply keybindings from updated config
        let config = paneflow_config::loader::load_config();
        keybindings::apply_keybindings(cx, &config.shortcuts);
        self.effective_shortcuts = keybindings::effective_shortcuts(&config.shortcuts);
        self.recording_shortcut_idx = None;
        cx.notify();
    }
}

// ---------------------------------------------------------------------------
// CSD window resize helpers
// ---------------------------------------------------------------------------

/// Width of the invisible border zone used for edge/corner resize handles.
const RESIZE_BORDER: Pixels = px(10.0);

/// Re-export from csd module for use in the render function below.
use csd::resize_edge;

impl Render for PaneFlowApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let main_content = if self.settings_section.is_some() {
            self.render_settings_page(cx).into_any_element()
        } else if let Some(ws) = self.active_workspace() {
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
                            .text_color(rgb(0xffffff))
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
                .child(div().text_color(rgb(0xffffff)).child("No workspaces"))
                .into_any_element()
        };

        // Update title bar with current workspace name
        let ws_name = self.active_workspace().map(|ws| ws.title.clone());
        let update_info = match &self.update_status {
            Some(update_checker::UpdateStatus::Available { version, url }) => {
                Some(title_bar::UpdateInfo {
                    version: version.clone(),
                    url: url.clone(),
                })
            }
            _ => None,
        };
        self.title_bar.update(cx, |tb, _| {
            tb.workspace_name = ws_name;
            tb.sidebar_width = px(SIDEBAR_WIDTH);
            tb.update_available = update_info;
        });

        // --- CSD resize backdrop ---
        let decorations = window.window_decorations();

        match decorations {
            Decorations::Client { .. } => window.set_client_inset(RESIZE_BORDER),
            Decorations::Server => window.set_client_inset(px(0.0)),
        }

        // The inner app content (title bar + sidebar + main)
        let ui_font = crate::terminal_element::resolve_font_family(
            paneflow_config::loader::load_config()
                .font_family
                .as_deref(),
        );
        let mut app_content = div()
            .font_family(ui_font)
            .relative()
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
            .on_action(cx.listener(Self::handle_copy_workspace_path))
            .on_action(cx.listener(Self::handle_reveal_workspace_in_file_manager))
            .on_action(cx.listener(Self::handle_open_workspace_in_zed))
            .on_action(cx.listener(Self::handle_open_workspace_in_cursor))
            .on_action(cx.listener(Self::handle_open_workspace_in_vscode))
            .on_action(cx.listener(Self::handle_open_workspace_in_windsurf))
            .on_action(cx.listener(Self::handle_next_workspace))
            .on_action(cx.listener(Self::handle_toggle_zoom))
            .on_action(cx.listener(Self::handle_layout_even_h))
            .on_action(cx.listener(Self::handle_layout_even_v))
            .on_action(cx.listener(Self::handle_layout_main_v))
            .on_action(cx.listener(Self::handle_layout_tiled))
            .on_action(cx.listener(Self::handle_split_equalize))
            .on_action(cx.listener(Self::handle_swap_pane))
            .on_action(cx.listener(Self::handle_undo_close_pane))
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
                cx.listener(|this: &mut Self, _: &CloseWindow, _window, cx| {
                    this.save_session(cx);
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
                            .bg(rgb(0x212121))
                            .overflow_hidden()
                            .child(main_content),
                    ),
            );

        if let Some(toast) = &self.toast {
            app_content = app_content.child(
                deferred(
                    div()
                        .id("copy-toast")
                        .absolute()
                        .right(px(20.))
                        .bottom(px(20.))
                        .max_w(px(320.))
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .bg(ui.overlay)
                        .border_1()
                        .border_color(ui.border)
                        .shadow_lg()
                        .text_sm()
                        .text_color(ui.text)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.))
                        .child(
                            div()
                                .w(px(18.))
                                .h(px(18.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(11.))
                                .text_color(ui.accent)
                                .child("✓"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(ui.text)
                                .child(toast.message.clone()),
                        )
                        .with_animations(
                            SharedString::from("copy-toast-anim"),
                            vec![
                                Animation::new(std::time::Duration::from_millis(TOAST_ENTER_MS))
                                    .with_easing(ease_in_out),
                                Animation::new(std::time::Duration::from_millis(TOAST_HOLD_MS)),
                                Animation::new(std::time::Duration::from_millis(TOAST_EXIT_MS))
                                    .with_easing(ease_in_out),
                            ],
                            |toast_el, stage, delta| match stage {
                                0 => {
                                    let lift = 8.0 * (1.0 - delta);
                                    toast_el.opacity(delta).bottom(px(20.0 + lift))
                                }
                                1 => toast_el.opacity(1.0).bottom(px(20.0)),
                                _ => {
                                    let drop = 8.0 * delta;
                                    toast_el.opacity(1.0 - delta).bottom(px(20.0 + drop))
                                }
                            },
                        ),
                )
                .priority(2),
            );
        }

        if let Some(position) = self.title_bar_menu_open {
            app_content = app_content.child(
                deferred(
                    div()
                        .id("title-bar-menu")
                        .occlude()
                        .absolute()
                        .left(position.x)
                        .top(position.y)
                        .w(px(180.))
                        .bg(ui.overlay)
                        .border_1()
                        .border_color(ui.border)
                        .rounded(px(8.))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .p(px(4.))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.title_bar_menu_open = None;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .child(self.render_context_menu_item(
                            "title-bar-menu-about".into(),
                            "About PaneFlow",
                            None,
                            ui,
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.title_bar_menu_open = None;
                                this.show_about_dialog = true;
                                cx.notify();
                                cx.stop_propagation();
                            }),
                        ))
                        .child(self.render_context_menu_item(
                            "title-bar-menu-settings".into(),
                            "Settings",
                            None,
                            ui,
                            cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.open_settings_window(window, cx);
                                cx.stop_propagation();
                            }),
                        )),
                )
                .priority(3),
            );
        }

        if self.show_about_dialog {
            let version = env!("CARGO_PKG_VERSION");
            app_content = app_content.child(
                deferred(
                    div()
                        .id("about-dialog-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui::hsla(0., 0., 0., 0.5))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.show_about_dialog = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .id("about-dialog")
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(16.))
                                .w(px(300.))
                                .px(px(24.))
                                .py(px(24.))
                                .bg(ui.overlay)
                                .border_1()
                                .border_color(ui.border)
                                .rounded(px(12.))
                                .shadow_lg()
                                .child(
                                    div()
                                        .text_color(ui.text)
                                        .text_size(px(16.))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child("PaneFlow"),
                                )
                                .child(
                                    div()
                                        .text_color(ui.muted)
                                        .text_size(px(13.))
                                        .child(format!("Version {version}")),
                                )
                                .child(
                                    div()
                                        .id("about-dialog-ok")
                                        .px(px(24.))
                                        .py(px(6.))
                                        .mt(px(4.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .bg(ui.accent)
                                        .text_color(ui.base)
                                        .text_size(px(13.))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .hover(|s| s.opacity(0.85))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.show_about_dialog = false;
                                            cx.notify();
                                        }))
                                        .child("Ok"),
                                ),
                        ),
                )
                .priority(10),
            );
        }

        if let Some(menu) = self.workspace_menu_open
            && menu.idx < self.workspaces.len()
        {
            let idx = menu.idx;
            let can_delete = self.workspaces.len() > 1;

            // Data-driven editor entries: (id, label, command, shortcut_description)
            let editors: &[(&str, &str, &str, &str)] = &[
                ("zed", "Open in Zed", "zed", "Open in Zed"),
                ("cursor", "Open in Cursor", "cursor", "Open in Cursor"),
                ("vscode", "Open in VS Code", "code", "Open in VS Code"),
                (
                    "windsurf",
                    "Open in Windsurf",
                    "windsurf",
                    "Open in Windsurf",
                ),
            ];

            // Estimated menu height: 7 items × 25px + 2 separators × 7px + 8px padding
            let menu_height = px(203.);
            let win_h = window.window_bounds().get_bounds().size.height;
            // Flip: if not enough space below the click, show the menu above it
            let menu_y = if menu.position.y + menu_height > win_h {
                (menu.position.y - menu_height).max(px(0.))
            } else {
                menu.position.y
            };

            let mut context_menu = div()
                .id("workspace-context-menu")
                .occlude()
                .absolute()
                .left(menu.position.x)
                .top(menu_y)
                .w(px(248.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .rounded(px(8.))
                .shadow_lg()
                .flex()
                .flex_col()
                .p(px(4.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.workspace_menu_open = None;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());

            for &(id, label, command, shortcut_desc) in editors {
                let shortcut = self
                    .shortcut_for_description(shortcut_desc)
                    .map(|s| SharedString::from(s.to_string()));
                let command = command.to_string();
                let label_owned = label.to_string();
                context_menu = context_menu.child(self.render_context_menu_item(
                    SharedString::from(format!("workspace-context-{id}")),
                    label,
                    shortcut,
                    ui,
                    cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.open_workspace_in_editor(idx, &command, &label_owned, cx);
                        cx.stop_propagation();
                    }),
                ));
            }

            // ── Separator ──
            context_menu = context_menu.child(div().mx(px(-4.)).my(px(3.)).h(px(1.)).bg(ui.border));

            // Reveal in file manager
            let reveal_shortcut = self
                .shortcut_for_description("Reveal in file manager")
                .map(|s| SharedString::from(s.to_string()));
            context_menu = context_menu.child(self.render_context_menu_item(
                "workspace-context-reveal".into(),
                "Reveal in File Manager",
                reveal_shortcut,
                ui,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.reveal_workspace_in_file_manager(idx, cx);
                    cx.stop_propagation();
                }),
            ));

            // Copy path
            let copy_shortcut = self
                .shortcut_for_description("Copy path")
                .map(|s| SharedString::from(s.to_string()));
            context_menu = context_menu.child(self.render_context_menu_item(
                "workspace-context-copy".into(),
                "Copy Path",
                copy_shortcut,
                ui,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.copy_workspace_path(idx, cx);
                    cx.stop_propagation();
                }),
            ));

            // ── Separator ──
            context_menu = context_menu.child(div().mx(px(-4.)).my(px(3.)).h(px(1.)).bg(ui.border));

            // Delete workspace (conditionally disabled)
            let close_shortcut = self
                .shortcut_for_description("Close workspace")
                .map(|s| SharedString::from(s.to_string()));
            context_menu = context_menu.child(
                div()
                    .id("workspace-context-delete")
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .when(can_delete, |d| d.cursor_pointer())
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .when(can_delete, |d| d.text_color(ui.text))
                    .when(can_delete, |d| {
                        d.hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle)
                        })
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        if can_delete {
                            this.close_workspace_at(idx, window, cx);
                        } else {
                            this.workspace_menu_open = None;
                            cx.notify();
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_x_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child("Delete"),
                    )
                    .when_some(close_shortcut, |d, shortcut| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(px(10.))
                                .text_color(ui.muted)
                                .child(shortcut),
                        )
                    }),
            );

            app_content = app_content.child(deferred(context_menu).priority(3));
        }

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
    // Handle --help and --version before initializing GPUI
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "PaneFlow {version} — GPU-accelerated terminal multiplexer\n\
             \n\
             Usage: paneflow [OPTIONS]\n\
             \n\
             Options:\n\
             \x20 -h, --help       Print this help message\n\
             \x20 -v, --version    Print version\n\
             \n\
             Configuration: ~/.config/paneflow/paneflow.json\n\
             IPC socket:    $XDG_RUNTIME_DIR/paneflow/paneflow.sock\n\
             \n\
             Keybindings:\n\
             \x20 Ctrl+Shift+D/E   Split horizontal/vertical\n\
             \x20 Ctrl+Shift+W     Close pane\n\
             \x20 Alt+Arrow        Focus adjacent pane\n\
             \x20 Ctrl+Shift+N     New workspace\n\
             \x20 Ctrl+Tab         Next workspace\n\
             \x20 Ctrl+1-9         Switch to workspace N\n\
             \n\
             https://github.com/ArthurDEV44/paneflow",
            version = env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("paneflow {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "info,wgpu_hal=off,wgpu_core=warn,naga=warn,zbus=warn,tracing::span=warn",
    ))
    .init();

    application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            // Load config early — needed for keybindings and window decorations
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);

            let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
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
                    window.on_window_should_close(cx, {
                        let view = view.clone();
                        move |_window, cx| {
                            view.read(cx).save_session(cx);
                            cx.quit();
                            false
                        }
                    });
                    view.update(cx, |app, cx| {
                        app.workspaces[0].focus_first(window, cx);
                    });
                    view
                },
            );

            match window_result {
                Ok(_) => cx.activate(true),
                Err(e) => {
                    log::error!("Failed to open PaneFlow window: {e}");
                    eprintln!(
                        "Error: PaneFlow requires a GPU with Vulkan support.\n\n\
                         Install mesa-vulkan-drivers (AMD/Intel) or your GPU's proprietary driver.\n\n\
                         Install commands:\n\
                         \x20 Debian/Ubuntu:  sudo apt install mesa-vulkan-drivers\n\
                         \x20 Fedora/RHEL:    sudo dnf install mesa-vulkan-drivers\n\
                         \x20 Arch:           sudo pacman -S vulkan-radeon vulkan-intel or nvidia-utils\n\n\
                         Run `vulkaninfo` to verify Vulkan support.\n\
                         If drivers are already installed, run with RUST_LOG=error for details.\n\n\
                         Underlying error: {e}"
                    );
                    std::process::exit(1);
                }
            }
        });
}
