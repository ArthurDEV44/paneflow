// Test-only allow for the CLAUDE.md-mandated clippy restrictions. These
// lints are also demoted to `allow` at crate level in `src-app/Cargo.toml`
// for pre-existing GPUI UI-code unwraps (US-007 "or equivalent" escape),
// so today this belt is effectively redundant — but it stays in place so
// that when the eventual cleanup story re-promotes the Cargo.toml lints
// to `warn`, tests continue to pass without another edit here.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::unwrap_in_result,
        clippy::panic
    )
)]
//! PaneFlow v2 — GPUI Native Terminal Multiplexer
//!
//! App shell with sidebar workspace list + main content area.

mod ai_hooks;
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
mod telemetry;
mod terminal;
pub mod theme;
mod update;
mod widgets;
mod window_chrome;
mod workspace;

use crate::window_chrome::title_bar;

use gpui::{
    App, Bounds, ClickEvent, Context, CursorStyle, Decorations, Entity, FocusHandle,
    HitboxBehavior, InteractiveElement, IntoElement, MouseButton, Pixels, Point, Render,
    ResizeEdge, Styled, Window, WindowBounds, WindowDecorations, WindowOptions, canvas, deferred,
    div, point, prelude::*, px, rgb, size, transparent_black,
};
use gpui_platform::application;
use notify::Watcher;

use crate::pane::Pane;
use crate::terminal::TerminalView;
use crate::workspace::Workspace;

// Re-export action types at the crate root so existing `crate::SplitHorizontally`
// references in sibling modules keep compiling without a crate-wide import churn.
pub use app::actions::*;
// US-002: items extracted out of `main.rs` are re-exported at crate root
// so callers like `crate::Notification` / `crate::TOAST_HOLD_MS` keep
// resolving without an import-rewrite churn across the workspace.
pub(crate) use app::constants::{
    CLAUDE_SPINNER_FRAMES, CODEX_SPINNER_FRAMES, MAX_CLOSED_PANES, RESIZE_BORDER, SIDEBAR_WIDTH,
    TOAST_HOLD_MS,
};
// `TOAST_ENTER_MS` and `TOAST_EXIT_MS` are used only by the toast
// renderer inside `app::notifications`; not re-exported at crate root.
pub(crate) use app::drag::{WorkspaceDrag, WorkspaceDragPreview};
pub(crate) use app::notifications::{Notification, Toast, ToastAction};
// Free helpers extracted to bootstrap.rs but still callable as
// `crate::system_package_update_command` etc. from sibling modules.
#[cfg(target_os = "macos")]
pub(crate) use app::bootstrap::{install_macos_menu_bar, warn_if_rosetta_translated};
pub(crate) use app::bootstrap::{system_package_update_command, warn_if_legacy_run_install};

// Terminal-routing helpers (`find_first_terminal`, `find_terminal_by_surface_id`,
// `send_text_to_first_leaf`) live in `app::ipc_handler` — its only consumer.

// ---------------------------------------------------------------------------
// Root application view
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SettingsSection {
    Shortcuts,
    Appearance,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkspaceContextMenu {
    pub(crate) idx: usize,
    pub(crate) position: Point<Pixels>,
}

/// Captured state of a closed pane for undo-close-pane (US-014).
pub(crate) struct ClosedPaneRecord {
    pub(crate) cwd: Option<std::path::PathBuf>,
    pub(crate) scrollback: Option<String>,
    pub(crate) workspace_idx: usize,
}

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
    /// Live telemetry handle (US-012/US-013). `Null` when consent is missing
    /// or `PANEFLOW_NO_TELEMETRY` is set — every `capture`/`flush` call is a
    /// no-op in that state, so callers never branch on consent.
    telemetry: std::sync::Arc<crate::telemetry::client::TelemetryClient>,
    /// Monotonic clock at process start, used to compute
    /// `session_duration_seconds` for the `app_exited` event. Wall-clock-change
    /// proof — a system clock jump mid-session never produces a negative value.
    launch_instant: std::time::Instant,
    /// Last observed `config.telemetry.enabled` value, cached so the config
    /// watcher's reconcile path can detect a transition (US-014) without
    /// re-reading the file.
    telemetry_enabled_last: Option<bool>,
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

    /// Centralised bookkeeping for a failed update attempt (US-013):
    /// classify the error, log it, update state, show the retry toast,
    /// and bump the attempt counter (which gates the 4th-click escape
    /// hatch).
    pub(crate) fn record_update_failure(
        &mut self,
        context: &str,
        err: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        log::error!("self-update/{context}: {err:#}");
        let tag = update::UpdateError::classify(err);
        // US-013 AC #4 — single choke-point for the failure telemetry: the
        // classified `UpdateError` collapses into a canonical
        // `error_category` label so no message string ever leaves the
        // machine. Called before `show_update_error_toast` so the event is
        // queued even if toast rendering panics.
        self.emit_update_failure(&tag);
        self.self_update_status = update::SelfUpdateStatus::Errored(tag.clone());
        self.update_attempt_count = self.update_attempt_count.saturating_add(1);
        self.show_update_error_toast(&tag, cx);
        cx.notify();
    }

    // --- Sidebar rendering ---
}

// ---------------------------------------------------------------------------
// CSD window resize helpers — `RESIZE_BORDER` lives in `app::constants`.
// ---------------------------------------------------------------------------

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
                            update::install_method::PackageManager::RpmOstree => {
                                title_bar::SystemPackageKind::RpmOstree
                            }
                            update::install_method::PackageManager::Other => {
                                title_bar::SystemPackageKind::Other
                            }
                        };
                        title_bar::UpdatePillKind::SystemManaged(system_kind)
                    }
                    // Flatpak / Snap / `PANEFLOW_UPDATE_EXPLANATION` —
                    // packager owns updates, render the same generic
                    // SystemHint pill. The explanation copy is surfaced
                    // by the click handler in `self_update_flow.rs`.
                    update::install_method::InstallMethod::ExternallyManaged { .. } => {
                        title_bar::UpdatePillKind::SystemManaged(
                            title_bar::SystemPackageKind::Other,
                        )
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
                            update::SelfUpdateStatus::ReadyToRestart { .. } => {
                                title_bar::SelfUpdatePillState::ReadyToRestart
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
                    this.emit_app_exited_and_flush();
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
                this.emit_app_exited_and_flush();
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
            app_content = app_content.child(self.render_toast(toast, ui));
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
            app_content =
                app_content.child(self.render_workspace_context_menu(menu, ui, window, cx));
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
                            let app = view.read(cx);
                            app.save_session(cx);
                            // US-013 AC #2 — final chance to flush
                            // `app_exited` when the OS close button or a
                            // keyboard shortcut closes the last window.
                            app.emit_app_exited_and_flush();
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
