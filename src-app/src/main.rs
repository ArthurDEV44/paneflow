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
//! PaneFlow — native terminal workspace for coding agents.
//!
//! App shell with sidebar workspace list, terminal panes, agent surfaces, and
//! diff/review workflows.

mod agent_launcher;
mod agent_sessions;
mod agents;
mod agents_view;
mod ai_hooks;
mod ai_types;
mod app;
mod assets;
mod claude_sessions;
mod codex_sessions;
mod config_writer;
mod diff;
mod editor;
mod fonts;
mod ipc;
mod keybindings;
mod keys;
mod layout;
mod limits;
mod markdown;
mod mouse;
mod opencode_sessions;
mod pane;
mod pane_drag;
mod project;
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
    App, Bounds, Context, CursorStyle, Decorations, Entity, FocusHandle, Focusable, HitboxBehavior,
    InteractiveElement, IntoElement, MouseButton, Pixels, Point, Render, ResizeEdge, Styled,
    Window, WindowBounds, WindowDecorations, WindowOptions, canvas, div, point, prelude::*, px,
    rgb, size, transparent_black,
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
// so callers like `crate::TOAST_HOLD_MS` keep resolving without an
// import-rewrite churn across the workspace.
pub(crate) use app::constants::{
    CLAUDE_SPINNER_FRAMES, CODEX_SPINNER_FRAMES, MAX_CLOSED_PANES, RESIZE_BORDER, SIDEBAR_WIDTH,
    TOAST_HOLD_MS,
};
// `TOAST_ENTER_MS` and `TOAST_EXIT_MS` are used only by the toast
// renderer inside `app::notifications`; not re-exported at crate root.
pub(crate) use app::drag::{WorkspaceDrag, WorkspaceDragPreview};
pub(crate) use app::notifications::{Toast, ToastAction};
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

/// Open "Move to pane…" tab context menu (EP-002 US-006). Identifies the tab
/// (its owning pane + index) and the click anchor; the destination panes are
/// resolved at render time from the workspace's split tree.
#[derive(Clone)]
pub(crate) struct TabContextMenu {
    pub(crate) source_pane: Entity<Pane>,
    pub(crate) tab_idx: usize,
    pub(crate) position: Point<Pixels>,
}

/// Open right-click menu for a Files-sidebar row (PRD files-tree EP-003
/// US-009). Carries the row's absolute path and the click anchor; "Copy
/// relative path" resolves the workspace root at render/action time.
#[derive(Clone)]
pub(crate) struct FilesContextMenu {
    pub(crate) path: std::path::PathBuf,
    pub(crate) position: Point<Pixels>,
}

/// Captured state of a closed pane for undo-close-pane (US-014).
pub(crate) struct ClosedPaneRecord {
    pub(crate) cwd: Option<std::path::PathBuf>,
    pub(crate) scrollback: Option<String>,
    pub(crate) workspace_idx: usize,
}

/// US-053: in-app self-update flow state, extracted from the `PaneFlowApp`
/// god-struct. Grouped: the background-check slot/result, the live flow
/// status, the detected install method, and the consecutive-failure counter.
struct SelfUpdateState {
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
    /// Monotonic token identifying the current `Downloading` attempt (EP-002,
    /// U-015). Bumped each time the flow enters `Downloading`; the per-attempt
    /// watchdog captures the value and only fires if it still matches — so a
    /// stale watchdog from a superseded attempt can't reset a newer one.
    download_generation: u64,
}

/// US-053: docked agent-sessions sidebar state (visibility, per-agent
/// scanned session lists, the originating pane/cwd, and group UI flags),
/// extracted from the `PaneFlowApp` god-struct.
struct AgentSessionsState {
    /// Whether the docked agent-sessions right sidebar is visible
    /// (PRD `prd-agent-sessions-sidebar-2026-Q3`, EP-001). Toggled by the
    /// tab-bar sessions button; the sidebar renders as a layout child of the
    /// root row, not a `deferred()` overlay.
    sessions_sidebar_open: bool,
    /// Claude Code sessions for the active terminal's cwd. Filled
    /// asynchronously by a background fs scan; stays empty while the scan is
    /// pending and after it resolves with no matches.
    claude_sessions: Vec<agent_sessions::SessionMeta>,
    /// Codex CLI sessions for the same cwd, populated by a parallel scan.
    codex_sessions: Vec<agent_sessions::SessionMeta>,
    /// OpenCode CLI sessions for the same cwd, populated by a third parallel
    /// scan that shells out to `opencode session list --format json` (see
    /// `opencode_sessions.rs`).
    opencode_sessions: Vec<agent_sessions::SessionMeta>,
    /// Working directory the sidebar was opened for. Used both to filter stale
    /// scan results that resolve after the sidebar was closed and as the label
    /// inside the sidebar header.
    claude_sessions_cwd: Option<String>,
    /// Weak handle to the pane whose tab-bar button opened the sidebar. Routes
    /// `claude --resume <id>` back to the *originating* pane's terminal even if
    /// focus shifts. Weak so the sidebar never keeps a closed pane alive.
    claude_sessions_pane: Option<gpui::WeakEntity<crate::pane::Pane>>,
    /// Scroll state for the sessions list. Re-created on every open so a fresh
    /// sidebar starts at offset 0.
    claude_sessions_scroll: gpui::ScrollHandle,
    /// Per-agent sidebar group state, indexed by `agent_index()`
    /// (Claude=0, Codex=1, OpenCode=2). All reset on close/open.
    /// `collapsed`: the group's caret has hidden its rows (EP-002 US-006).
    sessions_group_collapsed: [bool; 3],
    /// `show_all`: the group is past its 5-row cap via "Show more" (US-005).
    sessions_group_show_all: [bool; 3],
    /// `scanning`: a background scan for this agent is in flight, so an empty
    /// list should read as "loading" not "none" (US-004).
    sessions_scanning: [bool; 3],
}

/// US-053: Git Diff mode state (mounted single/multi-repo views + their
/// caches, the worktree/scope/project pickers, and the file-tree filter),
/// extracted from the `PaneFlowApp` god-struct.
struct DiffModeState {
    /// US-005 (prd-git-diff-mode-2026-Q3.md): the mounted Git Diff mode
    /// view, when `mode == AppMode::Diff`. Lazily (re)built by
    /// `rebuild_diff_view` on mode entry and on workspace switch;
    /// `None` when no git repo backs the active workspace. Dropping it
    /// releases the DiffView's filesystem watchers.
    diff_view: Option<gpui::Entity<crate::diff::DiffView>>,
    /// US-014 (prd-git-diff-mode-2026-Q3.md): the Multi-project host,
    /// mounted when `diff_scope == MultiProject`. Separate from
    /// `diff_view` (the single-repo host for Project / Worktree).
    multi_diff_view: Option<gpui::Entity<crate::diff::MultiRepoDiffView>>,
    /// US-016 warm-resume: cache of mounted single-repo `DiffView` entities
    /// (Project / Worktree scopes), keyed by repo + scope + worktree set. A
    /// CLI↔Diff toggle (or a workspace switch back to a visited repo) reuses
    /// the cached entity instead of cold-rebuilding it, so the diff shows in
    /// one frame with its computed rows instead of flashing "Computing diff…".
    /// Non-displayed entries are suspended (watchers released — US-016), so at
    /// most one diff entity ever holds live watchers. Mirrors the
    /// `agents_terminal_view_cache` pointer/owner split; bounded by
    /// `DIFF_VIEW_CACHE_CAP` and pruned to open repos on workspace close.
    diff_view_cache: std::collections::HashMap<
        crate::app::diff_view_actions::DiffViewKey,
        gpui::Entity<crate::diff::DiffView>,
    >,
    /// US-016: the cache key the current `diff_view` pointer is bound to (which
    /// cache entry it clones). `None` outside Diff mode, in Multi-project scope,
    /// or when no git repo backs the active workspace.
    diff_view_key: Option<crate::app::diff_view_actions::DiffViewKey>,
    /// US-016: retained Multi-project host + the signature of the repo-group set
    /// it was built for. Reused across CLI↔Diff toggles while the open project
    /// set is unchanged; rebuilt when projects open/close. `multi_diff_view` is
    /// the display pointer into this slot.
    multi_diff_view_retained: Option<(u64, gpui::Entity<crate::diff::MultiRepoDiffView>)>,
    /// Diff sidebar: branch sections (keyed by branch name) the user has
    /// collapsed in the multi-branch changed-files panel. Ephemeral UI state
    /// (resets on remount), so a `HashSet` of names is enough.
    diff_collapsed_branches: std::collections::HashSet<String>,
    /// `true` while the Worktree-scope on-disk worktree discovery
    /// (`spawn_worktree_discovery`) is in flight, so the diff sidebar can show a
    /// "Discovering worktrees…" note instead of looking like columns are missing
    /// during the brief cold-mount window.
    diff_discovering: bool,
    /// Worktree-scope branch curation: per repo, the set of worktree paths (raw
    /// path strings) the user explicitly chose to show as columns. NO entry for a
    /// repo ⇒ show ALL its worktrees (the default). An entry ⇒ build columns for
    /// exactly those worktrees, so branches the user didn't pick are never diffed
    /// (not merely hidden). Edited by the branches picker; in-memory per session.
    diff_chosen_worktrees:
        std::collections::HashMap<std::path::PathBuf, std::collections::HashSet<String>>,
    /// Whether the Worktree-scope branches multi-select popover is open.
    diff_worktree_picker_open: bool,
    /// All worktrees of `diff_available_repo`, fetched off-thread for the branches
    /// picker so it can offer branches not currently shown. Populated lazily when
    /// the picker opens.
    diff_available_worktrees: Vec<crate::diff::DiffWorktree>,
    /// The repo [`Self::diff_available_worktrees`] was fetched for (guards against
    /// showing a stale list after a workspace/repo switch).
    diff_available_repo: Option<std::path::PathBuf>,
    /// US-011: the active Git Diff view scope (Project / Multi-project /
    /// Worktree). Defaults to Project; `rebuild_diff_view` branches on it.
    diff_scope: crate::diff::DiffScope,
    /// US-012: whether the scope-selector popover is open.
    diff_scope_picker_open: bool,
    /// Whether the project-selector popover (Project / Worktree scopes) is
    /// open. Lets the user pick which open workspace's repo the single-repo
    /// diff follows, without leaving Diff mode.
    diff_project_picker_open: bool,
    /// US-008 (prd-git-diff-mode-2026-Q3.md): path of the file row
    /// selected in the diff git panel (presentation-only until the
    /// scroll-to-file wiring lands). `None` = nothing selected.
    diff_selected_file: Option<String>,
    /// US-008: whether the git panel's "Changes" section is collapsed.
    diff_files_collapsed: bool,
    /// Changed-files panel layout: `false` = flat list (default), `true` =
    /// collapsible directory tree (compact-folder chains merged). Toggled from
    /// the "Changes" header.
    diff_files_tree: bool,
    /// Collapsed directory nodes in tree mode, keyed `col_idx\0<dir path>` so a
    /// directory present in two branch sections collapses independently.
    diff_collapsed_dirs: std::collections::HashSet<String>,
    /// US-008: persistent type-to-filter field for the diff changed-files
    /// panel. Observed at construction so each keystroke re-renders the
    /// sidebar (which recomputes the visible matches by path substring).
    diff_file_filter: gpui::Entity<crate::widgets::text_input::TextInput>,
}

/// US-053: Agents-view sidebar state extracted from the `PaneFlowApp`
/// god-struct (terminal-only Agents view: rename, context menu, skills
/// page, search filter, and the per-thread terminal cache).
struct AgentsViewState {
    /// US-011 (prd-agents-view.md): which sidebar row is currently in
    /// inline-rename mode (mirrors [`Self::renaming_idx`] but for the
    /// Agents domain). `None` when no rename is active.
    pub(crate) agents_renaming: Option<crate::app::agents_sidebar::AgentsRenameTarget>,
    /// Inline rename input. `Some` only while a rename is in flight;
    /// dropped on commit / cancel. Mirrors the Composer's TextArea
    /// pattern so users get a real text input (cursor, selection,
    /// IME, copy/paste, click-to-position, double-click word select)
    /// instead of a fake `{text}|` shimmer. One entity is enough
    /// because [`Self::agents_renaming`] enforces a single in-flight
    /// rename at a time.
    pub(crate) agents_rename_input: Option<gpui::Entity<crate::widgets::text_area::TextArea>>,
    /// US-011: the in-progress rename text. Empty when not renaming.
    pub(crate) agents_rename_text: String,
    /// US-011: open right-click context menu (project header or
    /// thread row). `None` when no menu is open.
    pub(crate) agents_menu_open: Option<crate::app::agents_sidebar::AgentsContextMenu>,
    /// US-011: pending delete confirmation. The actual mutation
    /// happens only after the user confirms in the dialog.
    pub(crate) agents_confirm_delete: Option<crate::app::agents_sidebar::AgentsDeleteTarget>,
    /// US-012 (prd-agents-view.md): live search/filter query for the
    /// Agents sidebar. Empty string == "no filter, show full list".
    /// Case-insensitive substring match is applied at render time.
    pub(crate) agents_filter: String,
    /// US-012: focus handle for the sidebar search input. Held on
    /// `PaneFlowApp` so the input's key handler can route Backspace /
    /// Escape / Down without competing with the global app key chain.
    pub(crate) agents_filter_focus: FocusHandle,
    /// `true` while the Agents-view sidebar's "Skills" affordance is
    /// active. Takes precedence over the thread / picker surfaces in
    /// `render_agents_main_body`. Cleared by `select_thread` and
    /// anywhere else that navigates away from the skills page.
    pub(crate) agents_skills_visible: bool,
    /// Active tab on the Skills page. Persists across re-opens of
    /// the Skills view within the session; resets on app restart.
    pub(crate) agents_skills_tab: crate::agents_view::SkillsTab,
    /// Name of the skill whose Copy button was just clicked. The
    /// card flips its label to "Copied" while this matches; a 2 s
    /// timer reverts the slot. Single-slot — only one "Copied"
    /// indicator visible at a time, which is fine for a click-driven
    /// affordance.
    pub(crate) agents_skills_copied: Option<String>,
    /// True while the bottom-of-sidebar "Settings" popover is open.
    /// Shared between CLI and Agents sidebars — only one popover is
    /// ever visible because only one sidebar is rendered at a time.
    pub(crate) sidebar_actions_menu_open: bool,
    /// Cache of every Terminal Thread surface mounted this session,
    /// keyed by [`crate::project::Thread::id`]. The Agents view is
    /// terminal-only: selecting a thread reuses the existing
    /// [`crate::terminal::view::TerminalView`] entity so the shell
    /// process, scrollback, and I/O threads survive the round trip.
    /// Drop happens on thread deletion (via `remove_thread`'s cache
    /// cleanup) or on app shutdown.
    pub(crate) agents_terminal_view_cache:
        std::collections::HashMap<u64, gpui::Entity<crate::terminal::view::TerminalView>>,
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
    /// US-011: monotonic save-coalescing token. Every `save_session` bumps it
    /// and the off-thread writer skips its disk write when a newer save has
    /// been scheduled meanwhile, collapsing a burst (e.g. closing 20
    /// workspaces) into a single write — none of it on the render thread.
    save_seq: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// US-014: parsed `paneflow.json` cached on the main thread so render paths
    /// never call the blocking `load_config()` (fs read + JSON parse) per frame.
    /// Hydrated at startup, invalidated in [`Self::process_config_changes`] when
    /// the background `ConfigWatcher` reports a reload. Render code reads this;
    /// click handlers that must observe a config write *they just made* still
    /// read fresh from disk (the cache lags the write by the watcher debounce).
    cached_config: paneflow_config::schema::PaneFlowConfig,
    ipc_rx: std::sync::mpsc::Receiver<ipc::IpcRequest>,
    ipc_status: ipc::IpcStatus,
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
    /// Scroll state for the inline settings page.
    settings_scroll: gpui::ScrollHandle,
    settings_drag: Option<crate::widgets::scrollbar::ScrollDragState>,
    /// Cached HOME directory for sidebar display (avoids per-render syscall).
    home_dir: String,
    /// Scroll state for the persistent sidebar workspace list.
    /// Driven by GPUI's `overflow_y_scroll + track_scroll`; the
    /// visible scroll bar has been removed but the handle is still
    /// useful so the list keeps a stable wheel-scroll offset across
    /// re-renders.
    sidebar_scroll: gpui::ScrollHandle,
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
    /// Workflow action menu currently open in the sidebar (`None` = closed).
    workspace_menu_open: Option<WorkspaceContextMenu>,
    /// "Move to pane…" tab context menu (EP-002 US-006), or `None` when closed.
    tab_menu_open: Option<TabContextMenu>,
    /// Pane to focus on the next render (EP-003 US-009). Set by the
    /// `DropSplit` handler — which runs in a subscription callback without a
    /// `Window` — and consumed in `render`, which has one. One-shot.
    pending_pane_focus: Option<Entity<Pane>>,
    /// Profile menu currently open at the right of the title bar.
    /// Stores the click position so the menu can anchor near the profile
    /// button. `None` = closed.
    profile_menu_open: Option<Point<Pixels>>,
    /// US-053: agent-sessions sidebar state (see `AgentSessionsState`).
    agent_sessions: AgentSessionsState,
    /// Whether the docked Files right sidebar is visible (PRD
    /// `prd-files-tree-sidebar-2026-Q3`, EP-001). Mutually exclusive with
    /// `sessions_sidebar_open`. Never persisted — always `false` on launch.
    files_sidebar_open: bool,
    /// In-memory tree state for the open Files sidebar (root + expanded set +
    /// lazily-cached directory listings). Empty when the sidebar is closed.
    files_tree: app::files_tree::FilesTreeState,
    /// Scroll state for the Files tree body. Re-created on every open so a
    /// fresh sidebar starts at offset 0.
    files_tree_scroll: gpui::ScrollHandle,
    /// Recursive `notify` watcher on the Files tree root (EP-002 US-005).
    /// `None` when the sidebar is closed or the watch could not be installed
    /// (US-006 graceful degradation — the tree then refreshes on expand).
    files_watcher: Option<notify::RecommendedWatcher>,
    /// Receiver for raw watch events, drained + debounced by the background
    /// loop in `bootstrap`. `Some` only while a watcher is installed.
    files_event_rx: Option<std::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
    /// Open right-click context menu for a Files-sidebar row (EP-003 US-009),
    /// or `None` when closed. Mutually exclusive with the other popovers.
    files_menu_open: Option<FilesContextMenu>,
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
    /// Scroll state for the theme picker list (visible scrollbar overlay).
    theme_picker_scroll: gpui::ScrollHandle,
    theme_picker_drag: Option<crate::widgets::scrollbar::ScrollDragState>,
    /// US-053: self-update flow state (see `SelfUpdateState`).
    self_update: SelfUpdateState,
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
    /// US-006: shared "theme file changed" signal flipped by the theme
    /// watcher's debounce thread (event-driven invalidation). The 50 ms
    /// IPC poll loop in `process_config_changes` drains this flag and
    /// calls `cx.notify()` so the next render picks up the new theme.
    /// `Arc<AtomicBool>` — Send + Sync, lock-free.
    theme_changed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// US-053: Git Diff mode state (see `DiffModeState`).
    diff_mode: DiffModeState,
    /// US-008 (prd-agents-view.md): top-level UI mode. `Cli` = the
    /// traditional terminal multiplexer; `Agents` = the projects +
    /// threads sidebar and chat thread view. Toggled by the
    /// `OpenAgentsView` action (Ctrl/Cmd+Shift+A) and by the title-bar
    /// icon (US-009). Persisted to / restored from `session.json`
    /// (US-009 wires the restore branch).
    pub(crate) mode: paneflow_config::schema::AppMode,
    /// US-007 (prd-agents-view.md): in-memory list of Agents-view
    /// projects, persisted to `session.json` via [`save_session`].
    /// Empty until the user creates their first project (US-011).
    pub(crate) projects: Vec<crate::project::Project>,
    /// US-007 (prd-agents-view.md): index into [`Self::projects`] of
    /// the currently active project. `0` when no projects exist
    /// (the sidebar reads `projects.is_empty()` to decide whether
    /// to render anything).
    pub(crate) active_project_idx: usize,
    /// US-007 (prd-agents-view.md): index into the active project's
    /// thread list. `None` when no thread is selected (e.g. the
    /// user just opened the project and hasn't picked a thread yet).
    pub(crate) active_thread_idx: Option<usize>,
    /// US-053: Agents-view sidebar state (rename/menu/skills/filter +
    /// the terminal-thread cache), extracted from the god-struct.
    pub(crate) agents_view: AgentsViewState,
    /// "Close all workspaces" guard. `true` while the confirmation
    /// dialog is up; flipped back to `false` on cancel/confirm. Cheap
    /// `bool` instead of `Option<()>` because the action is global --
    /// there's only ever one pending confirm at a time.
    pub(crate) confirm_close_all_workspaces: bool,
    /// US-048: memoized sidebar display order (worktree grouping). Recomputed
    /// only when the workspace set / order / repo roots change, keyed by a
    /// cheap content signature — `render_sidebar` runs on every app `notify()`,
    /// so the old per-frame `HashMap` + `Vec` rebuild was pure waste. Interior
    /// mutability because the render fn borrows `&self`.
    pub(crate) sidebar_order_cache: std::cell::RefCell<crate::app::sidebar::SidebarOrderCache>,
}

/// Global flag for swap mode, checked by TerminalView to intercept Escape.
/// A process-global `AtomicBool` (rather than threading state through every
/// `TerminalView`) because the check sits on the keystroke hot path.
pub static SWAP_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl PaneFlowApp {
    /// Add a workspace's `.git` directory to the file watcher.
    /// Uses refcounting so multiple workspaces sharing a repo don't conflict.
    /// Silently skipped if the workspace is not in a git repo or watcher is unavailable.
    fn watch_git_dir(&mut self, ws: &Workspace) {
        if let Some(ref git_dir) = ws.git_dir {
            let current = self.git_watch_counts.get(git_dir).copied().unwrap_or(0);
            if current == 0 {
                // First workspace watching this git dir — register with OS.
                // U-018: only commit the refcount when `watch()` succeeds. The
                // old form incremented to 1 before checking, so a transient
                // failure pinned the count at 1 and every later workspace
                // sharing the repo saw count>1 and never retried the
                // registration — the dir stayed permanently unwatched. On
                // failure we return without recording the entry so a later
                // workspace re-attempts the watch.
                if let Some(ref mut watcher) = self.git_watcher
                    && let Err(e) = watcher.watch(git_dir, notify::RecursiveMode::NonRecursive)
                {
                    log::warn!("git watcher: failed to watch {}: {e}", git_dir.display());
                    return;
                }
            }
            *self.git_watch_counts.entry(git_dir.clone()).or_insert(0) += 1;
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
        self.self_update.self_update_status = update::SelfUpdateStatus::Errored(tag.clone());
        self.self_update.update_attempt_count =
            self.self_update.update_attempt_count.saturating_add(1);
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

        // EP-003 US-009: focus the pane created by a drop-to-split. Deferred
        // here from the `DropSplit` subscription handler (no `Window` there).
        if let Some(pane) = self.pending_pane_focus.take() {
            pane.read(cx).focus_handle(cx).focus(window, cx);
        }
        let main_content = if matches!(self.mode, paneflow_config::schema::AppMode::Agents) {
            // US-008 (prd-agents-view.md): mode is the source of truth
            // for which screen renders. The Agents view is terminal-only
            // — `render_agents_main` shows the selected thread's PTY, the
            // agent picker, or an empty state.
            self.render_agents_main(cx)
        } else if matches!(self.mode, paneflow_config::schema::AppMode::Diff) {
            // US-003 (prd-git-diff-mode-2026-Q3.md). NOTE: this site is
            // an `if matches!`, not a `match`, so the compiler does NOT
            // force a Diff arm — it must be added by hand or the diff
            // mode would silently fall through to the terminal view.
            self.render_diff_main(cx)
        } else if self.settings_section.is_some() {
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
        // Pill state for in-app installer flows (AppImage, TarGz, AppBundle,
        // MSI, pkexec dnf|apt). Shared between the SystemPackage branch and
        // the catch-all so both reflect the live install state machine; if
        // the SystemPackage branch ignored this, the pkexec dnf/apt path
        // would render "Update via dnf" frozen for the entire install while
        // is_busy() silently dropped clicks.
        let in_app_state = match &self.self_update.self_update_status {
            update::SelfUpdateStatus::Idle => title_bar::SelfUpdatePillState::Idle,
            update::SelfUpdateStatus::Downloading => title_bar::SelfUpdatePillState::Downloading,
            update::SelfUpdateStatus::Installing => title_bar::SelfUpdatePillState::Installing,
            update::SelfUpdateStatus::ReadyToRestart => {
                title_bar::SelfUpdatePillState::ReadyToRestart
            }
            update::SelfUpdateStatus::Errored(_) => title_bar::SelfUpdatePillState::Errored,
        };
        let update_info = match &self.self_update.update_status {
            Some(update::checker::UpdateStatus::Available { version, .. }) => {
                let kind = match &self.self_update.install_method {
                    update::install_method::InstallMethod::SystemPackage { manager } => {
                        match manager {
                            // Dnf / Apt: in-app pkexec install. Pill follows
                            // the install state machine like every other
                            // in-app installer.
                            update::install_method::PackageManager::Dnf
                            | update::install_method::PackageManager::Apt => {
                                title_bar::UpdatePillKind::InApp(in_app_state)
                            }
                            // Clipboard-only paths: kickoff_self_update_install
                            // returns early after copying the upgrade command,
                            // self_update_status never leaves Idle.
                            update::install_method::PackageManager::RpmOstree => {
                                title_bar::UpdatePillKind::SystemManaged(
                                    title_bar::SystemPackageKind::RpmOstree,
                                )
                            }
                            update::install_method::PackageManager::Other => {
                                title_bar::UpdatePillKind::SystemManaged(
                                    title_bar::SystemPackageKind::Other,
                                )
                            }
                        }
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
                    _ => title_bar::UpdatePillKind::InApp(in_app_state),
                };
                Some(title_bar::UpdateInfo {
                    version: version.clone(),
                    kind,
                })
            }
            _ => None,
        };
        // Push the matching sidebar width (220 px CLI / 280 px Agents)
        // so the title bar's brand slot stays aligned with the sidebar
        // edge across mode swaps.
        let sidebar_px = match self.mode {
            paneflow_config::schema::AppMode::Agents => {
                crate::app::agents_view_actions::AGENTS_SIDEBAR_WIDTH
            }
            paneflow_config::schema::AppMode::Diff => {
                crate::app::diff_view_actions::DIFF_SIDEBAR_WIDTH
            }
            paneflow_config::schema::AppMode::Cli => SIDEBAR_WIDTH,
        };
        self.title_bar.update(cx, |tb, _| {
            tb.workspace_name = ws_name;
            tb.sidebar_width = px(sidebar_px);
            tb.update_available = update_info;
            tb.ipc_state = self.ipc_status.state();
        });

        // --- CSD resize backdrop ---
        let decorations = window.window_decorations();

        match decorations {
            Decorations::Client { .. } => window.set_client_inset(RESIZE_BORDER),
            Decorations::Server => window.set_client_inset(px(0.0)),
        }

        // The inner app content (title bar + sidebar + main). UI tree
        // uses IBM Plex Sans (bundled, registered at boot via
        // `Assets::load_fonts`). TerminalElement resolves its own
        // monospace family from `paneflow.json#font_family`, so the
        // terminal output is unaffected.
        let mut app_content = div()
            .font_family("IBM Plex Sans")
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
            .on_action(cx.listener(Self::handle_open_multi_diff))
            .on_action(cx.listener(Self::handle_open_diff_view))
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
                    this.save_session_blocking(cx);
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
                this.save_session_blocking(cx);
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
            .on_action(cx.listener(Self::handle_dismiss_update))
            .on_action(cx.listener(Self::handle_open_agents_view))
            // EP-001 US-003: Escape cancels an in-flight tab drag. Capture
            // phase runs ancestor-before-descendant, so this pre-empts the
            // focused terminal's own Escape->PTY forwarding — but only while a
            // drag is active; otherwise we leave the key untouched so normal
            // terminal Escape behaviour is unaffected. Drop-outside-target is
            // handled by GPUI itself (it clears the active drag on mouse-up
            // over a non-target), so no extra wiring is needed there.
            .capture_key_down(cx.listener(|_this, e: &gpui::KeyDownEvent, window, cx| {
                if cx.has_active_drag() && e.keystroke.key == "escape" {
                    cx.stop_active_drag(window);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_move(|_e, _, cx| cx.stop_propagation())
            // Title bar (Entity with drag-to-move support)
            .child(self.title_bar.clone())
            // Sidebar + main content area. US-008: branch on the
            // top-level UI mode so the CLI sidebar (workspace list)
            // and the Agents sidebar (projects + threads, US-010)
            // swap atomically with the main content.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .overflow_hidden()
                    .child(match self.mode {
                        paneflow_config::schema::AppMode::Agents => {
                            self.render_agents_sidebar(window, cx)
                        }
                        paneflow_config::schema::AppMode::Diff => {
                            self.render_diff_sidebar(window, cx)
                        }
                        paneflow_config::schema::AppMode::Cli => {
                            self.render_sidebar(cx).into_any_element()
                        }
                    })
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .bg(rgb(0x212121))
                            .overflow_hidden()
                            .child(main_content),
                    )
                    // Docked agent-sessions sidebar (right edge). A layout child
                    // — not an overlay — so it reflows the content and persists
                    // while the user works (PRD agent-sessions-sidebar EP-001).
                    .when(self.agent_sessions.sessions_sidebar_open, |row| {
                        row.child(self.render_sessions_sidebar(cx))
                    })
                    // Docked Files sidebar (right edge) — same layout child as
                    // the sessions sidebar, mutually exclusive with it (PRD
                    // files-tree EP-001).
                    .when(self.files_sidebar_open, |row| {
                        row.child(self.render_files_sidebar(cx))
                    }),
            );

        if let Some(toast) = &self.toast {
            app_content = app_content.child(self.render_toast(toast, ui));
        }

        if let Some(anchor) = self.profile_menu_open {
            app_content = app_content.child(self.render_profile_menu(anchor, window, cx));
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

        // EP-002 US-006: "Move to pane…" tab context menu.
        if let Some(menu) = self.tab_menu_open.clone() {
            app_content = app_content.child(self.render_tab_context_menu(menu, ui, window, cx));
        }

        // files-tree EP-003 US-009: per-file copy-path context menu.
        if let Some(menu) = self.files_menu_open.clone() {
            app_content = app_content.child(self.render_files_context_menu(menu, ui, window, cx));
        }

        // US-011 (prd-agents-view.md): Agents-mode right-click context
        // menu (project header or thread row) + delete-confirmation
        // dialog. Both render only when the corresponding state field
        // is `Some`; the dispatcher fns guard against stale indices.
        if let Some(menu) = self.agents_view.agents_menu_open
            && let Some(el) =
                crate::app::agents_sidebar::render_open_agents_menu(self, menu, ui, window, cx)
        {
            app_content = app_content.child(el);
        }
        if let Some(target) = self.agents_view.agents_confirm_delete {
            app_content =
                app_content.child(self.render_agents_confirm_delete_dialog(target, ui, cx));
        }
        if self.confirm_close_all_workspaces {
            app_content = app_content.child(self.render_close_all_confirm_dialog(ui, cx));
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
// `--update-and-exit` (US-005 e2e auto-update harness)
// ---------------------------------------------------------------------------

/// Synchronous self-update entry point invoked by the e2e harness
/// (`scripts/test-update-e2e.sh`). Mirrors the GUI flow's check + per-format
/// install steps but never initializes GPUI — so it runs cleanly in headless
/// CI containers without Xvfb. Honours `PANEFLOW_UPDATE_FEED_URL`
/// ([`update::checker::update_feed_url`]) so the harness can point the
/// checker at a localhost fixture.
///
/// Returns the process exit code (see `--update-and-exit` doc-comment in
/// `main` for the full table). The split between exit-3 (feed unreachable)
/// and exit-1 (other) satisfies AC6 — the harness asserts a specific code,
/// not a substring of the generic "update failed" toast.
fn run_update_and_exit() -> i32 {
    use crate::update::checker::{UpdateStatus, check_github_release};
    use crate::update::install_method::{self, InstallMethod};

    let method = install_method::detect();
    log::info!("--update-and-exit: install method = {method:?}");

    // The harness MUST NOT emit telemetry — the test runs are not user
    // sessions and would skew funnels. Use a Null client (no-op
    // capture, no HTTP).
    let null_telemetry = crate::telemetry::client::TelemetryClient::Null;
    let status = check_github_release(&null_telemetry);
    let (version, asset_url) = match status {
        UpdateStatus::Available {
            version,
            asset_url: Some(url),
            ..
        } => (version, url),
        UpdateStatus::Available {
            asset_url: None, ..
        } => {
            eprintln!("paneflow-update: no asset matched the install method — nothing to install");
            return 5;
        }
        UpdateStatus::UpToDate => {
            eprintln!("paneflow-update: already up to date");
            return 2;
        }
        UpdateStatus::Failed => {
            // The checker logs whether the failure was DNS/HTTP/parse via
            // `log::warn!`; we can't easily distinguish here without a
            // structured error, so print the explicit feed-unreachable
            // hint per AC6 — the dominant failure mode the harness
            // exercises (kill miniserve before invocation).
            eprintln!(
                "paneflow-update: feed unreachable at {} — check PANEFLOW_UPDATE_FEED_URL",
                crate::update::checker::update_feed_url()
            );
            return 3;
        }
        UpdateStatus::Checking => {
            eprintln!("paneflow-update: checker returned Checking — should never happen");
            return 1;
        }
    };

    log::info!("--update-and-exit: installing v{version} from {asset_url}");

    match method {
        InstallMethod::TarGz { .. } => match crate::update::linux::targz::run_update(&asset_url) {
            Ok(new_bin) => {
                println!("paneflow-update: ok new={}", new_bin.display());
                0
            }
            Err(err) => {
                let classified = crate::update::error::UpdateError::classify(&err);
                if matches!(
                    classified,
                    crate::update::error::UpdateError::IntegrityMismatch { .. }
                ) {
                    eprintln!("paneflow-update: hash mismatch — {err}");
                    return 4;
                }
                eprintln!("paneflow-update: install failed — {err}");
                1
            }
        },
        InstallMethod::AppImage { source_path, .. } => {
            // AC3a deferred: appimageupdatetool isn't part of the default
            // CI image, and it has no in-process SHA verify path (the tool
            // fetches via embedded zsync metadata). The tar.gz path covers
            // the same regression surface (download + SHA verify + atomic
            // swap + restart-path). Leaving the wiring in place so a
            // follow-up can opt in by installing the tool.
            match crate::update::linux::appimage::run_update(&source_path, &asset_url) {
                Ok(new_bin) => {
                    println!("paneflow-update: ok new={}", new_bin.display());
                    0
                }
                Err(err) => {
                    eprintln!("paneflow-update: AppImage install failed — {err}");
                    1
                }
            }
        }
        // SystemPackage (.deb/.rpm/dnf/apt) updates need pkexec + a
        // running polkit agent — neither belongs in `--update-and-exit`,
        // which is designed to be deterministic and non-interactive.
        // AppBundle/WindowsMsi: the e2e harness is Linux-only (US-014
        // covers Windows e2e separately).
        other => {
            eprintln!(
                "paneflow-update: --update-and-exit does not support install method {other:?}"
            );
            5
        }
    }
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

fn main() {
    // Handle --help and --version before initializing GPUI
    let args: Vec<String> = std::env::args().collect();
    // US-038: detect the `mcp` subcommand BEFORE the global flag scans. Those
    // scans look at *every* arg, so `paneflow mcp install --help` would
    // otherwise match the global `--help` and print the top-level help instead
    // of routing to the `mcp` handler (which forwards `--help` to its own
    // subcommand parser). Gating the global scans on `!is_mcp_subcommand`
    // hands `paneflow mcp …` straight to the dispatcher below.
    let is_mcp_subcommand = args.get(1).map(String::as_str) == Some("mcp");
    if !is_mcp_subcommand && args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "PaneFlow {version} — native terminal workspace for coding agents\n\
             \n\
             Usage: paneflow [OPTIONS]\n\
             \x20      paneflow mcp <install|status|uninstall>\n\
             \n\
             Options:\n\
             \x20 -h, --help       Print this help message\n\
             \x20 -v, --version    Print version\n\
             \x20 --update-and-exit  Check for an update and exit (CI harness)\n\
             \n\
             Agent workflow:\n\
             \x20 Launch Claude Code, Codex, opencode, Pi, or any CLI agent in panes\n\
             \x20 Use `paneflow mcp install` so capable agents can read pane output\n\
             \n\
             Keybindings:\n\
             \x20 Ctrl+Shift+D/E   Split horizontal/vertical\n\
             \x20 Ctrl+Shift+W     Close pane\n\
             \x20 Alt+Arrow        Focus adjacent pane\n\
             \x20 Ctrl+Shift+N     New workspace\n\
             \x20 Ctrl+Tab         Next workspace\n\
             \x20 Ctrl+1-9         Switch to workspace N\n\
             \n\
             Config paths and IPC endpoints are documented in the README.\n\
             https://github.com/ArthurDEV44/paneflow",
            version = env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if !is_mcp_subcommand && args.iter().any(|a| a == "--version" || a == "-v") {
        println!("paneflow {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // US-011 (cli-hardening-followup-2026-Q3): scrub the `CLAUDECODE`
    // env var BEFORE any thread::spawn / tokio runtime / smol /
    // GPUI init reads or mutates env. Rust 1.85 made
    // `std::env::remove_var` `unsafe` precisely because it races
    // with concurrent `getenv` calls; the only race-free place to
    // mutate process env is the top of `main()` before any other
    // thread exists. Subsequent calls from `spawn_acp_agent` are
    // now idempotent no-ops, preserving the per-spawn safety net.
    paneflow_acp::scrub_claudecode_env();

    // Quiet by default: a plain `cargo run` (or a shipped binary) shows only
    // warnings + errors. `RUST_LOG=info` restores the startup/runtime
    // diagnostics (GPU selection, IPC, session restore, …) and `RUST_LOG=debug`
    // adds the per-operation diff/git trace — matching the documented
    // "RUST_LOG=info cargo run # with logging" workflow.
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("warn,wgpu_hal=off,naga=warn,zbus=warn,tracing::span=warn"),
    )
    .init();

    // US-003: install the process-wide kill-on-parent-death guard
    // BEFORE any agent CLI or ConPTY spawns so children inherit the
    // Job Object (Windows). On Linux + macOS this is a no-op shim
    // pending upstream pre_exec exposure in paneflow-acp / portable-pty.
    if let Err(err) = agents::parent_guard::install_process_job() {
        log::warn!(
            "parent_guard: failed to install Job Object -- kill -9 of Paneflow may orphan agent CLIs ({err})"
        );
    }

    // Patch PATH BEFORE GPUI starts so `which::which("bunx")` in
    // `paneflow_acp::discovery` finds binaries installed under `~/.bun/bin`
    // when Paneflow is launched from a `.desktop` file / Finder / Start Menu
    // (those inherit a minimal systemd-user / launchd / Explorer PATH that
    // does not source the user's shell rc). Must run before any other
    // thread spawns — see safety note on `augment_path_for_gui_launch`.
    runtime_paths::augment_path_for_gui_launch();

    // US-005: synchronous update flow for the e2e harness. Runs the same
    // checker + per-format installer the GUI calls, but without ever
    // initializing GPUI — exits with status 0 on a successful swap, 2 on
    // "no update needed", 3 on a feed-unreachable error (AC6's explicit
    // "feed unreachable" requirement vs the generic "update failed"),
    // 4 on integrity / hash mismatch, 5 on unsupported install method,
    // 1 on any other error. Pair with `PANEFLOW_UPDATE_FEED_URL` to
    // point the checker at a localhost fixture.
    if !is_mcp_subcommand && args.iter().any(|a| a == "--update-and-exit") {
        std::process::exit(run_update_and_exit());
    }

    // EP-002 US-004: `paneflow mcp <subcommand>` runs as a scriptable CLI
    // and exits — it never initializes GPUI / opens a window. Placed after
    // `augment_path_for_gui_launch` (so agent-CLI detection sees `~/.bun/bin`
    // etc.) and after `--update-and-exit`, before any GUI bootstrap. The
    // install engine lives in the GPU-free `paneflow-mcp-install` crate; we
    // extract the bridge first so the path written into agent configs is
    // guaranteed to exist (best-effort — the engine refuses cleanly if not).
    // Diagnostics go to stderr (env_logger), the per-agent report to stdout.
    if args.get(1).map(String::as_str) == Some("mcp") {
        let bridge_path = match ai_hooks::extract::ensure_bridge_extracted() {
            Ok(p) => Some(p),
            Err(e) => {
                log::warn!("paneflow mcp: bridge extraction failed ({e:#})");
                // Fall back to the resolved-but-maybe-missing path so the
                // engine can emit the precise "binary missing at <path>"
                // refusal rather than a vaguer "data dir unresolved".
                runtime_paths::bridge_binary_path()
            }
        };
        std::process::exit(paneflow_mcp_install::run_cli(&args[2..], bridge_path));
    }

    warn_if_legacy_run_install();
    #[cfg(target_os = "macos")]
    warn_if_rosetta_translated();

    // EP-001 US-003: materialize the embedded `paneflow-mcp` bridge to its
    // stable, non-versioned path so a registered MCP server keeps resolving
    // across Paneflow updates. SHA-compared + atomic: a no-op when the
    // on-disk bytes already match the embedded version. Non-fatal — the GUI
    // must still open if data_dir is unwritable; `paneflow mcp install`
    // (EP-002) refuses cleanly later rather than write a dangling path.
    match ai_hooks::extract::ensure_bridge_extracted() {
        Ok(path) => log::info!("paneflow: MCP bridge ready at {}", path.display()),
        Err(e) => log::warn!(
            "paneflow: MCP bridge extraction failed ({e:#}); `paneflow mcp install` will be unavailable until resolved"
        ),
    }

    application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            // Load config early — needed for keybindings and window decorations
            let config = paneflow_config::loader::load_config();
            keybindings::apply_keybindings(cx, &config.shortcuts);
            widgets::text_input::register_keybindings(cx);
            // US-016: agents-view composer textarea bindings.
            widgets::text_area::register_keybindings(cx);

            // Register every embedded `.ttf` under `assets/fonts/` BEFORE
            // any window opens, so GPUI's text system can resolve the
            // `Lilex` family (mono, 4 weights) and `IBM Plex Sans`
            // family (sans, 4 weights) Paneflow ships as the default
            // primaries — same strategy Zed uses with `.ZedMono` /
            // `.ZedSans` (`zed/assets/settings/default.json:29,57`).
            // Picking embedded families as the **primary** instead of
            // system families (Menlo / Cascadia Mono / DejaVu) sidesteps
            // the c3e2331 failure mode: Core Text inside a signed .app
            // bundle could return valid glyph_ids for a system family
            // and rasterize them as empty bitmaps; GPUI's per-Font
            // fallback chain only walks on missing-glyph not on
            // empty-raster, so the system primary "rendered" zero glyphs
            // and nothing fell through. With Lilex as the registered
            // primary, GPUI owns the font tables end-to-end. Iterates
            // the rust-embed registry (Zed pattern,
            // `zed/crates/assets/src/assets.rs:42`) so adding a new font
            // face is "drop a .ttf into assets/fonts/" with no Rust
            // change needed.
            if let Err(e) = assets::Assets.load_fonts(cx) {
                log::warn!(
                    "Assets::load_fonts failed: {e}; text rendering may fail on \
                     systems without a system monospace font"
                );
            }

            // Bootstrap Zed's `GlobalTheme` so the `markdown` crate's
            // paint pass can resolve `cx.theme().colors()` (borders,
            // panel backgrounds, etc.). Without this call, opening a
            // thread panics with `no state of type theme::GlobalTheme
            // exists`. `LoadThemes::JustBase` skips the JSON theme
            // bundles and `theme_settings` integration — Paneflow's own
            // `crate::theme` module remains the source of truth for
            // application chrome; this global only feeds the markdown
            // renderer's secondary decorations. The `::theme` (root)
            // path disambiguates against this crate's local
            // `crate::theme` module.
            ::theme::init(::theme::LoadThemes::JustBase, cx);

            // Register a minimal `ThemeSettingsProvider`. The `markdown`
            // crate re-uses Zed's `ui` components (`Label`, `CopyButton`,
            // `Checkbox`, `Tooltip`, etc.) in its paint pass; those
            // components call `theme::theme_settings(cx)` which expects
            // `GlobalThemeSettingsProvider` to be registered. Without
            // this call, rendering any markdown body that contains a
            // code block or task-list checkbox panics with `no state of
            // type GlobalThemeSettingsProvider exists`.
            //
            // We don't pull the heavy `theme_settings` crate (which
            // would drag in `settings`, `language`, etc.); instead we
            // implement the trait directly with fixed values that match
            // Paneflow's UI (IBM Plex Sans / Lilex, 13 px). The
            // markdown renderer only reads font_family and font size
            // for its embedded ui components — anything else flows
            // through the `MarkdownStyle` we pass to `MarkdownElement`.
            struct PaneflowThemeSettingsProvider {
                ui_font: gpui::Font,
                buffer_font: gpui::Font,
            }
            impl ::theme::ThemeSettingsProvider for PaneflowThemeSettingsProvider {
                fn ui_font<'a>(&'a self, _: &'a gpui::App) -> &'a gpui::Font {
                    &self.ui_font
                }
                fn buffer_font<'a>(&'a self, _: &'a gpui::App) -> &'a gpui::Font {
                    &self.buffer_font
                }
                fn ui_font_size(&self, _: &gpui::App) -> gpui::Pixels {
                    gpui::px(13.)
                }
                fn buffer_font_size(&self, _: &gpui::App) -> gpui::Pixels {
                    gpui::px(13.)
                }
                fn ui_density(&self, _: &gpui::App) -> ::theme::UiDensity {
                    ::theme::UiDensity::Default
                }
            }
            ::theme::set_theme_settings_provider(
                Box::new(PaneflowThemeSettingsProvider {
                    ui_font: gpui::Font {
                        family: "IBM Plex Sans".into(),
                        features: Default::default(),
                        fallbacks: None,
                        weight: Default::default(),
                        style: Default::default(),
                    },
                    buffer_font: gpui::Font {
                        family: "Lilex".into(),
                        features: Default::default(),
                        fallbacks: None,
                        weight: Default::default(),
                        style: Default::default(),
                    },
                }),
                cx,
            );

            // Override the table-cell colors the `markdown` crate reads
            // from the global theme. Its table renderer paints header
            // rows with `cx.theme().colors().title_bar_background` and
            // alternating body rows with `panel_background` — Zed's
            // defaults are blue-tinted (One Dark). We replace both
            // with neutral greys so tables render monochrome and blend
            // with Paneflow's terminal-mode palette.
            {
                use ::theme::ActiveTheme as _;
                let mut new_theme = (**cx.theme()).clone();
                new_theme.styles.colors.title_bar_background = gpui::rgb(0x1f1f1f).into();
                new_theme.styles.colors.panel_background = gpui::rgb(0x1c1c1c).into();
                new_theme.styles.colors.border = gpui::rgb(0x2f2f2f).into();
                new_theme.styles.colors.border_variant = gpui::rgb(0x2a2a2a).into();
                ::theme::GlobalTheme::update_theme(cx, std::sync::Arc::new(new_theme));
            }

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
                            app.save_session_blocking(cx);
                            // US-013 AC #2 — final chance to flush
                            // `app_exited` when the OS close button or a
                            // keyboard shortcut closes the last window.
                            app.emit_app_exited_and_flush();
                            cx.quit();
                            false
                        }
                    });
                    // US-116 (prd-agent-ui-refactor-2026-Q3.md): track
                    // window-activation state in a process-wide
                    // AtomicBool the agents notifications module reads
                    // when deciding whether to fire an OS toast. The
                    // observer keeps a Subscription alive that drops
                    // with the entity, so no manual teardown is needed.
                    view.update(cx, |_, cx| {
                        let subscription = cx.observe_window_activation(
                            window,
                            |_, window, _cx| {
                                crate::agents::notifications::set_window_active(
                                    window.is_window_active(),
                                );
                            },
                        );
                        // Detach: the closure side-effect is what we
                        // want, and we never need to manually drop
                        // this subscription -- the entity outlives
                        // every render that could care.
                        subscription.detach();
                    });
                    // Prime the gate with the current state in case the
                    // first activation tick is delayed past the first
                    // runtime event (e.g. a tool the user launched
                    // before app focus stabilises).
                    crate::agents::notifications::set_window_active(window.is_window_active());

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
