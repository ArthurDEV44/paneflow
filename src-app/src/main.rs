//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar workspace list + main content area.

mod ai_types;
mod app;
mod assets;
mod config_writer;
mod fonts;
mod ipc;
mod keybindings;
mod keys;
mod layout;
mod mouse;
mod pane;
mod pty;
mod runtime_paths;
mod search;
mod settings;
mod terminal;
pub mod theme;
mod update;
mod widgets;
mod window_chrome;
mod workspace;

use crate::window_chrome::title_bar;

use gpui::{
    Animation, AnimationExt, App, Bounds, ClickEvent, Context, CursorStyle, Decorations, Entity,
    FocusHandle, HitboxBehavior, InteractiveElement, IntoElement, MouseButton, Pixels, Point,
    Render, ResizeEdge, SharedString, Styled, Window, WindowBounds, WindowDecorations,
    WindowOptions, canvas, deferred, div, ease_in_out, point, prelude::*, px, rgb, size,
    transparent_black,
};
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem, OsAction};
use gpui_platform::application;
use notify::Watcher;

use crate::layout::LayoutTree;
use crate::pane::Pane;
use crate::terminal::TerminalView;
use crate::workspace::Workspace;

// Re-export action types at the crate root so existing `crate::SplitHorizontally`
// references in sibling modules keep compiling without a crate-wide import churn.
pub use app::actions::*;

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
    kind: ai_types::AiToolState,
    timestamp: std::time::Instant,
    read: bool,
}

struct Toast {
    message: String,
    /// Optional action buttons shown inside the toast. Empty for the
    /// ordinary confirmation toasts ("Path copied", etc); populated for
    /// update-failure toasts (US-013) with Retry / "Open releases" buttons.
    actions: Vec<ToastAction>,
    /// How long the "hold" phase of the toast animation lasts, in ms.
    /// Must match the auto-dismiss timer in [`push_toast`] — otherwise the
    /// exit animation plays early and the element persists as a ghost at
    /// opacity 0 until the dismiss task fires.
    hold_ms: u64,
}

#[derive(Clone)]
enum ToastAction {
    /// "Retry" — re-dispatches the `StartSelfUpdate` action. The action
    /// handler's existing guards (busy check, attempt counter) apply.
    RetryUpdate,
    /// "Open releases" — opens the given URL in the user's browser.
    /// Used for the 4th-attempt fallback (AC: "Download manually from the
    /// releases page").
    OpenReleasesPage(String),
}

#[derive(Clone, Copy)]
struct WorkspaceContextMenu {
    idx: usize,
    position: Point<Pixels>,
}

/// Drag payload used when reordering workspace cards in the sidebar.
#[derive(Clone)]
struct WorkspaceDrag {
    id: u64,
    title: SharedString,
}

/// Floating preview entity rendered under the cursor during a workspace drag.
struct WorkspaceDragPreview {
    title: SharedString,
}

impl Render for WorkspaceDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        div()
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(ui.text)
            .child(self.title.clone())
    }
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
    /// Notification bell dropdown anchor (click position). `None` = closed.
    /// Uses the same `Option<Point<Pixels>>` pattern as `profile_menu_open` /
    /// `title_bar_menu_open` so every dropdown is rendered via `deferred()`
    /// and clamped to the window bounds.
    notif_menu_open: Option<Point<Pixels>>,
    /// Workflow action menu currently open in the sidebar (`None` = closed).
    workspace_menu_open: Option<WorkspaceContextMenu>,
    /// Burger menu currently open in the title bar (`None` = closed).
    title_bar_menu_open: Option<Point<Pixels>>,
    /// Profile menu currently open at the right of the title bar.
    /// Stores the click position so the menu can anchor near the profile
    /// button. `None` = closed.
    profile_menu_open: Option<Point<Pixels>>,
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
    /// Whether the command-palette-style theme picker is visible.
    show_theme_picker: bool,
    /// Typeahead filter for the theme picker (case-insensitive substring).
    theme_picker_query: String,
    /// Index into the *filtered* theme list for the currently highlighted row.
    theme_picker_selected_idx: usize,
    /// Focus handle routing key events to the theme picker while it's open.
    theme_picker_focus: FocusHandle,
    /// Shared slot for the background update checker result.
    pending_update: update::checker::SharedUpdateSlot,
    /// Resolved update status (set once the background check completes).
    update_status: Option<update::checker::UpdateStatus>,
    /// Live state of the in-app self-update flow (download → install → restart).
    self_update_status: update::SelfUpdateStatus,
    /// How the running binary was installed. Detected once at startup —
    /// drives the update pill's label/click behaviour (US-012) and the
    /// in-app updater's branch selection.
    install_method: update::install_method::InstallMethod,
    /// Count of consecutive in-app update failures since process start
    /// (US-013). Bumped on every classified error; after 3 failures the
    /// 4th click skips the network and shows the "download manually"
    /// escape hatch toast.
    ///
    /// Never decremented. The only success path for an update calls
    /// `cx.restart()`, which replaces this process — the fresh
    /// `PaneFlowApp::new` initializes the counter back to 0. So "failures
    /// since last success" and "failures since process start" coincide by
    /// construction; the PRD's "three consecutive failures" requirement
    /// holds without an explicit reset.
    update_attempt_count: u32,
    /// State of the "Custom Buttons" management modal opened from the
    /// workspace context menu. `None` = closed.
    custom_buttons_modal: Option<app::custom_buttons_modal::CustomButtonsModal>,
    /// Focus handle routing key events to the custom-buttons modal while open.
    custom_buttons_modal_focus: FocusHandle,
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

    fn show_toast(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.push_toast(message.into(), Vec::new(), TOAST_HOLD_MS, cx);
    }

    /// Surface an update failure as a toast with a "Retry" action button
    /// (US-013). Hold is extended so the user has time to click the button
    /// before auto-dismiss.
    fn show_update_error_toast(&mut self, err: &update::UpdateError, cx: &mut Context<Self>) {
        self.push_toast(
            err.user_message(),
            vec![ToastAction::RetryUpdate],
            TOAST_HOLD_MS * 4,
            cx,
        );
    }

    /// Centralised bookkeeping for a failed update attempt (US-013):
    /// classify the error, log it, update state, show the retry toast,
    /// and bump the attempt counter (which gates the 4th-click escape
    /// hatch).
    fn record_update_failure(
        &mut self,
        context: &str,
        err: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        log::error!("self-update/{context}: {err:#}");
        let tag = update::UpdateError::classify(err);
        self.self_update_status = update::SelfUpdateStatus::Errored(tag.clone());
        self.update_attempt_count = self.update_attempt_count.saturating_add(1);
        self.show_update_error_toast(&tag, cx);
        cx.notify();
    }

    fn push_toast(
        &mut self,
        message: String,
        actions: Vec<ToastAction>,
        hold_ms: u64,
        cx: &mut Context<Self>,
    ) {
        self.toast = Some(Toast {
            message,
            actions,
            hold_ms,
        });
        cx.notify();

        // Dropping the previous task cancels its timer automatically.
        let total = TOAST_ENTER_MS + hold_ms + TOAST_EXIT_MS;
        self._toast_task = Some(cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(std::time::Duration::from_millis(total)).await;
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

    // --- Sidebar rendering ---
}

// ---------------------------------------------------------------------------
// CSD window resize helpers
// ---------------------------------------------------------------------------

/// Width of the invisible border zone used for edge/corner resize handles.
const RESIZE_BORDER: Pixels = px(10.0);

/// Re-export from csd module for use in the render function below.
use crate::window_chrome::csd::resize_edge;

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
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_center()
                        .gap(px(10.))
                        .w(px(460.))
                        .px(px(24.))
                        .child(
                            div()
                                .text_color(rgb(0xffffff))
                                .text_size(px(20.))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("Welcome to PaneFlow"),
                        )
                        .child(
                            div()
                                .text_color(ui.muted)
                                .text_size(px(13.))
                                .child(
                                    "The next-generation IDE for the AI era — \
                                     a GPU-native terminal with workspace-aware panes, \
                                     live git status, and first-class support for Claude Code and Codex.",
                                ),
                        )
                        .child(
                            div()
                                .mt(px(6.))
                                .text_color(ui.muted)
                                .text_size(px(12.))
                                .child("Click + in the sidebar to create your first workspace."),
                        ),
                )
                .into_any_element()
        };

        // Update title bar with current workspace name
        let ws_name = self.active_workspace().map(|ws| ws.title.clone());
        let update_info = match &self.update_status {
            Some(update::checker::UpdateStatus::Available { version, .. }) => {
                let kind = match &self.install_method {
                    update::install_method::InstallMethod::SystemPackage { manager } => {
                        let system_kind = match manager {
                            update::install_method::PackageManager::Apt => {
                                title_bar::SystemPackageKind::Apt
                            }
                            update::install_method::PackageManager::Dnf => {
                                title_bar::SystemPackageKind::Dnf
                            }
                            update::install_method::PackageManager::Other => {
                                title_bar::SystemPackageKind::Other
                            }
                        };
                        title_bar::UpdatePillKind::SystemManaged(system_kind)
                    }
                    _ => {
                        let state = match &self.self_update_status {
                            update::SelfUpdateStatus::Idle => title_bar::SelfUpdatePillState::Idle,
                            update::SelfUpdateStatus::Downloading => {
                                title_bar::SelfUpdatePillState::Downloading
                            }
                            update::SelfUpdateStatus::Installing => {
                                title_bar::SelfUpdatePillState::Installing
                            }
                            update::SelfUpdateStatus::Errored(_) => {
                                title_bar::SelfUpdatePillState::Errored
                            }
                        };
                        title_bar::UpdatePillKind::InApp(state)
                    }
                };
                Some(title_bar::UpdateInfo {
                    version: version.clone(),
                    kind,
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
        let ui_font = crate::terminal::element::resolve_font_family(
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
            // US-012: macOS menu-bar actions. `Quit` mirrors `CloseWindow`.
            // `About` is a placeholder; clicking it logs until we ship a
            // real About surface. `Copy` / `Paste` delegate to the existing
            // terminal clipboard actions so Edit > Copy works when a
            // terminal pane is focused (matches the ⌘C keybinding from
            // US-010). `SelectAll` is a no-op until the terminal exposes
            // a select-all action.
            .on_action(cx.listener(|this: &mut Self, _: &Quit, _window, cx| {
                this.save_session(cx);
                cx.quit();
            }))
            .on_action(cx.listener(|_this: &mut Self, _: &About, _window, _cx| {
                log::info!("About PaneFlow: v{}", env!("CARGO_PKG_VERSION"));
            }))
            .on_action(cx.listener(|_this: &mut Self, _: &Copy, _window, cx| {
                cx.dispatch_action(&TerminalCopy);
            }))
            .on_action(cx.listener(|_this: &mut Self, _: &Paste, _window, cx| {
                cx.dispatch_action(&TerminalPaste);
            }))
            .on_action(
                cx.listener(|_this: &mut Self, _: &SelectAll, _window, _cx| {
                    log::debug!("Edit > Select All dispatched (terminal select-all not yet wired)");
                }),
            )
            .on_action(cx.listener(|_this: &mut Self, _: &OpenHelp, _window, _cx| {
                if let Err(e) = open::that("https://github.com/ArthurDEV44/paneflow#readme") {
                    log::warn!("Help > PaneFlow Help: could not open browser: {e}");
                }
            }))
            .on_action(cx.listener(Self::handle_start_self_update))
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
            let has_actions = !toast.actions.is_empty();
            // Error toasts (those with action buttons) get a warning glyph
            // + wider panel so the message + button have room. Ordinary
            // confirmation toasts keep the tight 320-px "✓ …" layout.
            let (icon, max_w) = if has_actions {
                ("!", px(420.))
            } else {
                ("✓", px(320.))
            };

            let header = div()
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
                        .child(icon),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(ui.text)
                        .child(toast.message.clone()),
                );

            let action_row = if has_actions {
                let mut row = div().flex().flex_row().gap(px(8.)).mt(px(8.)).pl(px(28.));
                for (idx, action) in toast.actions.iter().enumerate() {
                    let (label, button_id): (&str, String) = match action {
                        ToastAction::RetryUpdate => ("Retry", format!("toast-retry-{idx}")),
                        ToastAction::OpenReleasesPage(_) => {
                            ("Open releases", format!("toast-releases-{idx}"))
                        }
                    };
                    let action_clone = action.clone();
                    let btn = div()
                        .id(SharedString::from(button_id))
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(ui.accent)
                        .text_color(ui.accent)
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.7))
                        .child(label)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| match &action_clone {
                            ToastAction::RetryUpdate => {
                                window.dispatch_action(Box::new(StartSelfUpdate), cx);
                            }
                            ToastAction::OpenReleasesPage(url) => {
                                let _ = open::that(url);
                            }
                        });
                    row = row.child(btn);
                }
                Some(row)
            } else {
                None
            };

            app_content = app_content.child(
                deferred(
                    div()
                        .id("copy-toast")
                        .absolute()
                        .right(px(20.))
                        .bottom(px(20.))
                        .max_w(max_w)
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
                        .flex_col()
                        .child(header)
                        .children(action_row)
                        .with_animations(
                            SharedString::from("copy-toast-anim"),
                            vec![
                                Animation::new(std::time::Duration::from_millis(TOAST_ENTER_MS))
                                    .with_easing(ease_in_out),
                                Animation::new(std::time::Duration::from_millis(toast.hold_ms)),
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
                        ))
                        .child(self.render_context_menu_item(
                            "title-bar-menu-themes".into(),
                            "Themes…",
                            None,
                            ui,
                            cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.title_bar_menu_open = None;
                                this.open_theme_picker(window, cx);
                                cx.stop_propagation();
                            }),
                        )),
                )
                .priority(3),
            );
        }

        if let Some(anchor) = self.profile_menu_open {
            app_content = app_content.child(self.render_profile_menu(anchor, window, cx));
        }

        if let Some(anchor) = self.notif_menu_open {
            app_content = app_content.child(self.render_notif_menu(anchor, window, cx));
        }

        if self.show_theme_picker {
            app_content = app_content.child(self.render_theme_picker(cx));
        }

        if self.custom_buttons_modal.is_some() {
            app_content = app_content.child(self.render_custom_buttons_modal(cx));
        }

        if self.show_about_dialog {
            app_content = app_content.child(self.render_about_dialog(cx));
        }

        if let Some(menu) = self.workspace_menu_open
            && menu.idx < self.workspaces.len()
        {
            let idx = menu.idx;
            let can_delete = !self.workspaces.is_empty();

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

            // Estimated menu height: 8 items × 25px + 2 separators × 7px + 8px padding
            let menu_height = px(228.);
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

            // Manage Custom Buttons — opens the per-workspace button editor modal.
            context_menu = context_menu.child(self.render_context_menu_item(
                "workspace-context-custom-buttons".into(),
                "Manage Custom Buttons…",
                None,
                ui,
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.open_custom_buttons_modal(idx, window, cx);
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

/// Detect the legacy `.run`-installer layout and log a migration hint.
///
/// Build the copy-pasteable upgrade command for a system-package install.
///
/// `version` is safe to interpolate into a shell string without escaping: it
/// comes from `UpdateStatus::Available { version }`, which is set from a
/// `semver::Version::to_string()` — the semver parser rejects any input that
/// would survive into `;`/`$()`/whitespace/bidi, so malformed GitHub tags
/// short-circuit to `UpdateStatus::Failed` long before this function runs.
///
/// Version format notes:
/// - apt pinning uses `name=upstream-debrev`. `cargo-deb` emits `-1` as the
///   debian revision by default, so `paneflow=<v>-1` targets the exact tag.
/// - dnf accepts `name-upstream` as a NEVR prefix match. The `<v>` we pass is
///   already the raw upstream version from GitHub Releases.
/// - `PackageManager::Other` gets a plain-English hint rather than a command,
///   because we don't know the syntax (eopkg/xbps/apk all differ).
fn system_package_update_command(
    manager: Option<&update::install_method::PackageManager>,
    version: &str,
) -> String {
    match manager {
        Some(update::install_method::PackageManager::Apt) => {
            format!("sudo apt update && sudo apt install paneflow={version}-1")
        }
        Some(update::install_method::PackageManager::Dnf) => {
            format!("sudo dnf upgrade paneflow-{version}")
        }
        Some(update::install_method::PackageManager::Other) | None => {
            "Update PaneFlow via your system's package manager".to_string()
        }
    }
}

/// The old `.run` installer (removed in US-007) dropped a standalone binary
/// at `~/.local/bin/paneflow`. The new tar.gz installer instead drops a
/// `~/.local/paneflow.app/` directory and symlinks `~/.local/bin/paneflow`
/// into it. We warn when the old layout is detected so users know why the
/// in-app updater can no longer fetch a `.run` asset (there are none).
/// Install the macOS menu bar.
///
/// US-012: three top-level menus — PaneFlow / Edit / Window — populated with
/// the actions listed in the PRD. The `PaneFlow` menu name matches the
/// `CFBundleName` from the future US-013 Info.plist (AC6). Keyboard shortcuts
/// are derived from the global keybindings table (e.g. Quit shows `⌘Q`
/// because US-010's `MACOS_ONLY_DEFAULTS` binds `cmd-q → quit`; Window items
/// show `⌘⇧N` / `⌘⇧Q` / `⌘Tab` from US-009's `secondary-*` bindings).
/// Copy / Paste / Select All carry an `OsAction` hint so macOS routes them
/// through the native responder chain and renders `⌘C` / `⌘V` / `⌘A`.
#[cfg(target_os = "macos")]
fn install_macos_menu_bar(cx: &mut App) {
    cx.set_menus(vec![
        Menu::new("PaneFlow").items(vec![
            MenuItem::action("About PaneFlow", About),
            MenuItem::separator(),
            MenuItem::action("Quit PaneFlow", Quit),
        ]),
        Menu::new("Edit").items(vec![
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("Window").items(vec![
            MenuItem::action("New Workspace", NewWorkspace),
            MenuItem::action("Close Workspace", CloseWorkspace),
            MenuItem::separator(),
            MenuItem::action("Next Workspace", NextWorkspace),
        ]),
        // macOS convention: every app ships a Help menu (even if it only
        // points to an online doc/repo). Without one, Apple's HIG-conforming
        // users perceive the app as unfinished. "PaneFlow Help" dispatches
        // `OpenHelp` which opens the GitHub README in the default browser.
        Menu::new("Help").items(vec![MenuItem::action("PaneFlow Help", OpenHelp)]),
    ]);
}

/// Detect whether the Apple Silicon binary is running under Rosetta 2
/// translation on an Intel Mac (or, more commonly, an Intel binary on
/// Apple Silicon — which Apple translates transparently). Either way it
/// warns once at startup so a user who grabbed the wrong `.dmg` knows
/// why GPU performance is degraded instead of silently eating the hit.
///
/// Edge case 4 of the macOS port PRD. Uses `sysctl.proc_translated`: returns
/// `1` for a translated process, `0` native, ENOENT → native Intel kernel
/// (no Rosetta available at all). Failure to read the sysctl is silent —
/// this warning is diagnostic, not load-bearing.
#[cfg(target_os = "macos")]
fn warn_if_rosetta_translated() {
    use std::ffi::CString;
    use std::mem::size_of;

    let name = match CString::new("sysctl.proc_translated") {
        Ok(n) => n,
        Err(_) => return,
    };
    let mut translated: i32 = 0;
    let mut size = size_of::<i32>();
    // SAFETY: `sysctlbyname` reads a small integer into a stack buffer whose
    // size is passed by pointer. `name.as_ptr()` is a valid NUL-terminated
    // C string from a CString we just constructed. `translated` and `size`
    // are live stack variables for the duration of the call. Zero-initialized
    // buffer means a kernel short-write can't expose uninitialized memory.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut translated as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 && translated == 1 {
        log::warn!(
            "running under Rosetta 2 translation — GPU rendering will be \
             degraded. For best performance, download the matching \
             architecture from https://github.com/ArthurDEV44/paneflow/releases"
        );
    }
}

fn warn_if_legacy_run_install() {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };
    let app_dir = home.join(".local/paneflow.app");
    let legacy_bin = home.join(".local/bin/paneflow");

    let legacy_bin_is_regular_file = legacy_bin
        .symlink_metadata()
        .map(|m| m.file_type().is_file())
        .unwrap_or(false);

    if !app_dir.exists() && legacy_bin_is_regular_file {
        log::warn!(
            "legacy .run install detected at {} — see README for migration \
             to the .tar.gz / .deb / .AppImage formats",
            legacy_bin.display()
        );
    }
}

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

    warn_if_legacy_run_install();
    #[cfg(target_os = "macos")]
    warn_if_rosetta_translated();

    application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            // Load config early — needed for keybindings and window decorations
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            widgets::text_input::register_keybindings(cx);

            // US-012: macOS native menu bar. On Linux/Windows the call is
            // elided — GPUI's non-macOS platforms don't render a menu bar
            // and AC5 forbids any Linux UI change.
            #[cfg(target_os = "macos")]
            install_macos_menu_bar(cx);

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

            // US-011: reserve space on the left of the custom titlebar
            // for macOS traffic lights. The three red/yellow/green circles
            // live at x≈12-78px; the brand text starts at x=80 (see
            // title_bar.rs). `..Default::default()` is load-bearing on
            // non-macOS (GPUI's TitlebarOptions may grow platform-specific
            // fields we don't set); clippy only flags it needless under
            // target_os = "macos" where traffic_light_position makes the
            // field list complete.
            #[cfg_attr(target_os = "macos", allow(clippy::needless_update))]
            let titlebar_options = gpui::TitlebarOptions {
                title: Some("PaneFlow".into()),
                appears_transparent: true,
                #[cfg(target_os = "macos")]
                traffic_light_position: Some(point(px(12.0), px(12.0))),
                ..Default::default()
            };

            let window_result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.0), px(500.0))),
                    window_decorations: Some(decorations),
                    titlebar: Some(titlebar_options),
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
                        if !app.workspaces.is_empty() {
                            app.workspaces[0].focus_first(window, cx);
                        }
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
