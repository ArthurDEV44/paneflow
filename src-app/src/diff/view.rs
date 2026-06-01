//! `DiffView` — GPUI entity hosting the multi-worktree diff viewer.
//!
//! Pipeline: each sibling worktree becomes a column whose diff
//! (`merge-base..working-tree`) is computed off the main thread
//! (`smol::unblock`) and applied back via `this.update` from a spawned task —
//! never mutated inside `render`. A per-view `generation` counter discards
//! stale results when a refresh is superseded (US-007 last-write-wins).
//!
//! EP-004: N columns render side by side in one tab (US-012) with a shared
//! base-branch selector (US-013), per-column hide/show (US-014), and live
//! refresh on working-tree / HEAD / index / base-ref changes via an
//! entity-owned `notify` watcher (US-015). Rendering is virtualized by
//! `uniform_list` in both unified and split modes (US-006/US-009).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::Either;
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, CursorStyle, DragMoveEvent, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, Hsla, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render,
    ScrollHandle, SharedString, Styled, Window, anchored, deferred, div, point, prelude::*, px,
    relative,
};
use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::pane_drag::{DropEdge, SPLIT_EDGE_BAND, TabDragPreview, compute_drop_edge};
use crate::widgets::text_input::TextInput;

use super::align::CellKind;
use super::arrange::{Arrange, Axis};
use super::element::{DiffBody, DiffElement};

/// Drag payload for a diff branch pane dragged by its column header (inc 5).
/// Carries the source column index; dropping on another pane's edge restructures
/// the [`Arrange`] tree to split toward that edge. Cloned cheaply by GPUI for
/// the duration of the drag.
#[derive(Clone)]
pub struct DiffColumnDrag {
    pub source_idx: usize,
}
use super::rows::{
    DisplayRow, RowKind, RowPalette, SplitRow, build_display_rows, build_split_rows,
    display_row_height, split_row_height,
};

/// When jumping to a hunk, leave this much room above its first changed line so
/// the pinned sticky file header (24px) does not cover it.
const HUNK_JUMP_MARGIN: f32 = 28.0;

/// Below this estimated per-column width the split view auto-falls back to
/// unified (mirroring Zed's `too_narrow_for_split`).
const MIN_SPLIT_COLUMN_PX: f32 = 360.0;

/// Live-refresh debounce. Long enough to coalesce a build's file churn into one
/// re-diff per window, short enough to feel live (US-015).
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);

/// After a reload, ignore further watcher events for this long. Prevents a
/// reload's own churn (or a concurrent build) from immediately re-triggering and
/// starving the in-flight load (the perpetual "Computing diff…" loop).
const REFRESH_COOLDOWN: Duration = Duration::from_millis(1000);

/// Syntax highlighting (prd-diff-syntax-highlight-2026-Q3.md). ON.
///
/// History: a full-file `syntect` pass cost 0.3–2.8 s/file (×4 builders ≈
/// ~30 s/column), so it shipped gated. Replaced by tree-sitter
/// ([`super::highlighter`]) — the same engine Zed uses — whose parse is
/// ms-scale, so the eager full-file highlight is now cheap enough to run at
/// build time off-thread. `super::syntax::DiffSyntax` supplies theme-derived
/// (ANSI) colors; unknown grammars fall back to monochrome.
const SYNTAX_HIGHLIGHT_ENABLED: bool = true;

/// Delay before pre-filling a freshly-launched embedded review CLI's input (tmux
/// send-keys style). Long enough for `claude` / `codex` / `opencode` / `pi` to
/// boot their readline; the clipboard fallback covers a missed window.
const REVIEW_PREFILL_DELAY_MS: u64 = 1800;

/// Embedded review-terminal region height (px): default + drag clamp bounds.
/// Opens roughly half the view so the CLI/shell has real room (drag to resize).
const REVIEW_DEFAULT_HEIGHT: f32 = 520.0;
const REVIEW_MIN_HEIGHT: f32 = 120.0;
const REVIEW_MAX_HEIGHT: f32 = 1000.0;

/// Inline (unified) vs side-by-side. Unified is the default — it mirrors Zed's
/// git-panel Diff view (single gutter, one merged line number, colored hunk
/// bar). The toggle flips to Split; a too-narrow column also falls back to
/// Unified (US-011).
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Split,
    Unified,
}

/// One worktree column seed: its working-tree root and current branch name.
#[derive(Clone)]
pub struct DiffWorktree {
    pub path: PathBuf,
    pub branch: String,
    /// US-011 (prd-git-diff-mode-2026-Q3.md): the open workspace this worktree
    /// belongs to, or `None` for a worktree discovered on disk (git worktree
    /// list) that has no open workspace. Render-only — never serialized — so
    /// the field is additive and does not touch session persistence. Read by
    /// the Multi-project stale-reconcile pass (US-016, EP-006); carried now so
    /// the seed already distinguishes open vs discovered worktrees.
    #[allow(dead_code)]
    pub workspace_id: Option<u64>,
}

/// Lightweight per-file summary for the Diff-mode git panel (US-008,
/// prd-git-diff-mode-2026-Q3.md). Derived from the off-thread
/// [`super::git::FileDiff`] build; carries only what the sidebar renders so the
/// heavy base/new text stays in the column's row sets, not duplicated here.
#[derive(Clone)]
pub struct FileEntry {
    pub path: String,
    pub change: super::git::FileChange,
    /// Original path for a detected rename, mirrored from
    /// [`super::git::FileDiff::old_path`] so the sidebar can show `old → new`.
    pub old_path: Option<String>,
    pub added: u32,
    pub removed: u32,
    pub is_binary: bool,
}

/// Changed-files list state the git panel renders, read off the first visible
/// column (US-008; the Project scope mounts exactly one column).
#[derive(Clone)]
pub enum FileListState {
    Loading,
    Loaded(Rc<Vec<FileEntry>>),
    Failed(String),
}

/// Aggregate diffstat over a set of per-column file lists, as
/// `(branches, files, added, removed)`. Only columns that are `Loaded` AND
/// non-empty are counted — a branch identical to its base contributes nothing
/// and must never inflate the "N branches vs base" count. Single source of
/// truth for the multi-branch aggregate strip ([`DiffView::render_aggregate_strip`])
/// and the sidebar "Changes" header, so the two folds can never drift apart
/// (the bug that showed +10191 in the strip but +10183 in the sidebar — two
/// separate folds over slightly different column sets).
pub fn aggregate_file_lists(
    lists: &[(String, usize, std::path::PathBuf, FileListState)],
) -> (usize, usize, u32, u32) {
    lists
        .iter()
        .filter_map(|(_, _, _, st)| match st {
            FileListState::Loaded(files) if !files.is_empty() => Some(files),
            _ => None,
        })
        .fold((0usize, 0usize, 0u32, 0u32), |(b, fc, a, r), files| {
            let (fa, fr) = files
                .iter()
                .fold((0u32, 0u32), |(a, r), f| (a + f.added, r + f.removed));
            (b + 1, fc + files.len(), a + fa, r + fr)
        })
}

/// Async lifecycle of a single column's diff. Loaded carries both row sets so
/// toggling the view mode is instant (no recompute, US-011 AC).
enum ColumnState {
    Loading,
    Loaded {
        unified: Rc<Vec<DisplayRow>>,
        split: Rc<Vec<SplitRow>>,
        file_count: usize,
        /// US-008: per-file summary for the git panel (shared `Rc` so the
        /// sidebar reads it without cloning the whole list each frame).
        files: Rc<Vec<FileEntry>>,
        /// `(file path, header row index)` for the unified / split row sets, so
        /// a sidebar file click can scroll the body to that file
        /// ([`DiffView::jump_to_file`]). Built once per load, off-thread.
        anchors_unified: Rc<Vec<(String, usize)>>,
        anchors_split: Rc<Vec<(String, usize)>>,
        /// US-001/US-002 (prd-ai-in-diff-2026-Q3.md): the raw per-file diffs
        /// retained so "copy hunk/file" (US-003) and the agent review payload
        /// (US-005) serialize an exact unified diff at action time (no stable
        /// hunk ID — hunks are resolved from these on demand). Bounded by the
        /// same per-file caps as the rows; shared `Rc` so reads never clone the
        /// base/new text.
        files_full: Rc<Vec<super::git::FileDiff>>,
    },
    Failed(String),
}

/// Off-thread build result for one column. `Send` (only owned data) so it can
/// cross the `smol::unblock` boundary; the non-`Send` `Rc` wrapping happens back
/// on the main thread when the result is applied.
enum Built {
    Failed(String),
    Loaded {
        unified: Vec<DisplayRow>,
        split: Vec<SplitRow>,
        file_count: usize,
        files: Vec<FileEntry>,
        anchors_unified: Vec<(String, usize)>,
        anchors_split: Vec<(String, usize)>,
        /// US-001/US-002: raw per-file diffs retained for copy/review, moved out
        /// of the off-thread `diff.files` after the rows are built from it.
        files_full: Vec<super::git::FileDiff>,
        /// US-016: captured in the same off-thread pass as the diff, so a later
        /// `revalidate` compares against it without re-shelling at harvest time.
        fingerprint: super::git::ColumnFingerprint,
    },
}

/// prd-ai-in-diff-2026-Q3.md (terminal-launch revision): a review CLI running in
/// a real terminal embedded under a column's diff. The diff view owns the
/// `TerminalView` entity directly (no workspace pane); dropping it shuts the PTY.
struct ReviewTerminal {
    label: SharedString,
    terminal: Entity<crate::terminal::TerminalView>,
}

/// Hover tooltip body for the icon-only column-header buttons (Review, terminal).
struct DiffHeaderTooltip {
    label: SharedString,
}

impl Render for DiffHeaderTooltip {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = crate::theme::active_theme();
        let ui = crate::theme::ui_colors();
        div()
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(theme.title_bar_background)
            .border_1()
            .border_color(ui.border)
            .text_color(ui.text)
            .text_size(px(11.))
            .child(self.label.clone())
    }
}

struct Column {
    branch: String,
    path: PathBuf,
    /// The open workspace this column's worktree belongs to (seed
    /// [`DiffWorktree::workspace_id`]), used to tag the embedded review terminal.
    workspace_id: Option<u64>,
    state: ColumnState,
    /// Scroll handle for the custom [`DiffElement`] path (hosted in an
    /// `overflow_y_scroll` div). Also the offset source/target for cross-column
    /// scroll sync ([`DiffView::sync_scroll`]) and for `jump_to_file`.
    el_scroll: ScrollHandle,
    /// US-014: hidden columns are skipped in render and in refresh (no wasted
    /// diffing); re-showing reloads them.
    visible: bool,
    /// File paths whose hunks are collapsed (header-only) in the body. Persists
    /// across live-refresh reloads. Toggled per-file by clicking a file header
    /// in the body, or in bulk by the toolbar collapse/expand-all chip.
    collapsed: std::collections::HashSet<String>,
    /// Collapse-filtered row sets + their file-header anchors, derived from
    /// `state` + `collapsed` by [`Column::recompute_display`] only on load /
    /// toggle (never per frame), so collapse is O(1) at paint and the body +
    /// `jump_to_file` index against what is actually shown.
    disp_unified: Rc<Vec<DisplayRow>>,
    disp_split: Rc<Vec<SplitRow>>,
    disp_anchors_unified: Rc<Vec<(String, usize)>>,
    disp_anchors_split: Rc<Vec<(String, usize)>>,
    /// US-016 warm-resume: the git fingerprint (HEAD + base + status hash) this
    /// column's rows were built against, captured off-thread at load time.
    /// `DiffView::revalidate` compares a fresh fingerprint on diff-mode re-entry
    /// to re-diff ONLY columns that actually changed. `None` until first load
    /// (and on a failed load), so such a column always reloads on resume.
    fingerprint: Option<super::git::ColumnFingerprint>,
    /// Per-column comparison base override. `None` ⇒ this column diffs against
    /// the view's shared `base_ref` (e.g. `develop`); `Some(ref)` ⇒ it diffs
    /// against that ref instead — the per-commit toggle sets `Some("HEAD~1")` so
    /// one branch column can show "just my latest commit's work" while its
    /// siblings keep the whole-branch-vs-develop view.
    base_override: Option<String>,
    /// Per-column last-write-wins guard (US-007). Bumped each time THIS column
    /// is (re)loaded; the spawned task captures it and discards its result if a
    /// newer load for the same column superseded it. Per-column (not a single
    /// view-wide counter) so a subset reload — e.g. `revalidate` reloading only
    /// the columns whose fingerprint moved — never discards an in-flight full
    /// reload of the OTHER columns.
    generation: u64,
    /// Review CLIs launched on this column's branch, rendered as real terminals
    /// under the diff body (prd-ai-in-diff-2026-Q3.md). Empty until the user runs
    /// Review; replaced on a re-run; dropped (PTY shutdown) when the column is.
    review_terminals: Vec<ReviewTerminal>,
    /// User-resizable height (px) of this column's embedded review region.
    review_height: f32,
}

impl Column {
    fn new_loading(branch: String, path: PathBuf, workspace_id: Option<u64>) -> Self {
        Self {
            branch,
            path,
            workspace_id,
            state: ColumnState::Loading,
            el_scroll: ScrollHandle::new(),
            visible: true,
            collapsed: std::collections::HashSet::new(),
            disp_unified: Rc::new(Vec::new()),
            disp_split: Rc::new(Vec::new()),
            disp_anchors_unified: Rc::new(Vec::new()),
            disp_anchors_split: Rc::new(Vec::new()),
            fingerprint: None,
            base_override: None,
            generation: 0,
            review_terminals: Vec::new(),
            review_height: REVIEW_DEFAULT_HEIGHT,
        }
    }

    /// Rebuild the collapse-filtered views from the loaded rows + `collapsed`.
    /// No-op until the column is `Loaded`. When nothing is collapsed the full
    /// rows are shared as-is (no allocation); otherwise collapsed files keep
    /// only their header (marked `▸`).
    fn recompute_display(&mut self) {
        let computed = match &self.state {
            ColumnState::Loaded {
                unified,
                split,
                anchors_unified,
                anchors_split,
                ..
            } => {
                if self.collapsed.is_empty() {
                    Some((
                        unified.clone(),
                        split.clone(),
                        anchors_unified.clone(),
                        anchors_split.clone(),
                    ))
                } else {
                    let (du, au) =
                        apply_collapse_unified(unified, anchors_unified, &self.collapsed);
                    let (ds, as_) = apply_collapse_split(split, anchors_split, &self.collapsed);
                    Some((Rc::new(du), Rc::new(ds), Rc::new(au), Rc::new(as_)))
                }
            }
            _ => None,
        };
        if let Some((u, s, au, as_)) = computed {
            self.disp_unified = u;
            self.disp_split = s;
            self.disp_anchors_unified = au;
            self.disp_anchors_split = as_;
        }
    }
}

/// Multi-worktree diff viewer pane.
pub struct DiffView {
    repo_root: PathBuf,
    /// Resolved base ref every column diffs against (US-013 selector).
    base_ref: String,
    /// Local branches offered by the base selector.
    branches: Vec<String>,
    base_picker_open: bool,
    /// Live type-to-filter field inside the base-branch popover. Owned so the
    /// popover can be a real searchable list (the DiffView observes it to
    /// recompute matches on every keystroke).
    base_filter: Entity<TextInput>,
    columns: Vec<Column>,
    focus_handle: FocusHandle,
    element_id: SharedString,
    /// US-016 watcher epoch. Bumped by [`Self::suspend`]; [`Self::start_watchers`]
    /// captures it at spawn and (a) refuses to install the freshly-built watcher
    /// if the epoch advanced while it was building off-thread, and (b) stops the
    /// debounce loop once the epoch advances. Closes the build-race where a
    /// `suspend` between the watcher-build spawn and its completion would
    /// otherwise leave a live watcher (and event loop) on a hidden/cached entity,
    /// or a double watcher after `resume`.
    watch_epoch: u64,
    mode: ViewMode,
    /// When true (default), all visible columns scroll in lockstep: the
    /// vertical offset of `scroll_driver` is broadcast to the rest each render,
    /// turning N parked viewers into one comparison surface (the whole point of
    /// the side-by-side worktree view). Toggleable from the toolbar.
    sync_scroll: bool,
    /// Index of the column the user last scrolled — the offset source the sync
    /// broadcast follows. Set by each column's `on_scroll_wheel`. Sourcing only
    /// from the explicit driver (never from clamped followers) keeps the sync
    /// drift-free across columns of differing height.
    scroll_driver: usize,
    /// Column whose changed-file list feeds the sidebar and whose body
    /// `jump_to_file` scrolls. Set by clicking a column header.
    selected_column: usize,
    /// Toolbar collapse/expand-all state: drives the chip label and the bulk
    /// action applied across every visible column.
    all_collapsed: bool,
    /// Entity-owned filesystem watchers (US-015). Dropped on tab close, which
    /// unregisters the OS handles and ends the debounce loop.
    _watchers: Vec<RecommendedWatcher>,
    /// US-016 warm-resume: `true` while the diff surface is hidden (CLI/Agents
    /// mode, or cached and not displayed). [`Self::suspend`] sets it and releases
    /// the watchers; [`Self::resume`] clears it, re-arms the watcher, and
    /// revalidates. While set, the deferred `bootstrap` completion does NOT arm a
    /// watcher (it would leak one for an invisible repo).
    suspended: bool,
    /// US-016: `true` once `bootstrap` has resolved the base + branches. Guards
    /// `resume`: if bootstrap is still in flight, clearing `suspended` is enough
    /// (bootstrap will arm the watcher + load itself); otherwise resume arms +
    /// revalidates directly.
    bootstrapped: bool,
    /// Inc 5: how the visible columns are arranged on screen — a splittable
    /// tree over column indices (side-by-side / stacked / nested), driven by
    /// drag-and-drop. Reconciled against the live columns each render, so the
    /// `Vec<Column>` and all its index-based logic stay untouched.
    arrange: Arrange,
    /// Transient drag state: `(hovered column idx, resolved drop edge)` while a
    /// `DiffColumnDrag` is in flight, so the hovered pane's overlay can preview
    /// the split. `None` edge = center (move/reorder). Cleared on drop.
    drag_target: Option<(usize, Option<DropEdge>)>,
    /// US-002/US-003 (prd-ai-in-diff-2026-Q3.md): open body context menu
    /// (right-click), carrying the resolved scope + window-space anchor. `None`
    /// when closed.
    body_menu: Option<DiffBodyMenu>,
    /// US-003: last pointer position over a column body `(col idx, window point)`,
    /// so the `Ctrl+Shift+C` action resolves the hunk under the cursor without a
    /// continuous row recompute on every move.
    last_body_pos: Option<(usize, Point<Pixels>)>,
    /// US-003: transient "copied" confirmation pill, auto-cleared by a spawned
    /// timer. Self-hosted so the diff view needs no PaneFlowApp toast handle.
    flash: Option<SharedString>,
    /// Which branch column's Review CLI multi-select popover is open (by column
    /// index), or `None` when closed. Anchored to that branch's header.
    review_menu_open: Option<usize>,
    /// Per-CLI "include in the review" toggles, aligned to [`ReviewCli::all`]
    /// order. Re-synced (default all-on) when the menu opens.
    review_picks: Vec<bool>,
    /// Active review-region resize drag: `(col_idx, start_pointer_y_px,
    /// start_height_px)`. `None` when not dragging.
    review_resizing: Option<(usize, f32, f32)>,
    /// `(col_idx, unified row)` of the changed line under the cursor while that
    /// column has a review CLI running — painted as hover-highlighted + clickable
    /// (left-click sends it to the CLI). `None` when not over an actionable line.
    hover_line: Option<(usize, usize)>,
    /// When true, the column-header `×` emits [`DiffViewEvent::CloseColumn`] (the
    /// host deselects the branch from the scope) instead of locally hiding the
    /// column. Set for the Worktree scope, where a branch is either shown or not —
    /// no in-between "hidden but tracked" state with a "N hidden" pill.
    close_removes: bool,
}

/// Events a [`DiffView`] raises to its host (`PaneFlowApp`). Today: the user
/// asked to drop a branch column from the Worktree scope via its header `×`.
pub enum DiffViewEvent {
    CloseColumn { path: PathBuf },
}

/// US-002/US-003: an open right-click menu on the diff body, anchored at the
/// click point and pre-resolved to the file (+ optional hunk + clicked line)
/// under the cursor.
struct DiffBodyMenu {
    position: Point<Pixels>,
    col_idx: usize,
    scope: DiffBodyScope,
}

/// A changed line resolved under the pointer, for sending into the embedded
/// review CLI's input (prd-ai-in-diff-2026-Q3.md).
#[derive(Clone)]
struct ClickedLine {
    path: String,
    lineno: u32,
    content: String,
    removed: bool,
}

/// Which file (+ optional hunk) a body point resolves to. Indices are into the
/// column's `files_full` and that file's `hunks`, resolved at action time from
/// the live rows (no stable hunk ID).
#[derive(Clone, Copy)]
struct DiffBodyScope {
    file_idx: usize,
    hunk_idx: Option<usize>,
}

impl DiffView {
    /// Build a diff view seeded with a repo's sibling worktrees, kick off the
    /// per-worktree diffs off the main thread, and start the live-refresh watch.
    pub fn new(repo_root: PathBuf, worktrees: Vec<DiffWorktree>, cx: &mut Context<Self>) -> Self {
        Self::with_base(repo_root, worktrees, None, cx)
    }

    /// Like [`Self::new`] but seeds the base ref. The multi-project host passes
    /// the last-chosen base so switching repos keeps the comparison base; when
    /// `base` is `None`, `bootstrap` resolves the default (develop→main→master).
    pub fn with_base(
        repo_root: PathBuf,
        worktrees: Vec<DiffWorktree>,
        base: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let element_id = SharedString::from(format!("diff-view-{}", repo_root.display()));
        let columns: Vec<Column> = worktrees
            .into_iter()
            .map(|w| Column::new_loading(w.branch, w.path, w.workspace_id))
            .collect();
        // Initial arrangement: every column side by side (mirrors the old fixed
        // flex row). Drag-and-drop reshapes this; `reconcile` keeps it in sync
        // with hide/show/reload.
        let arrange = Arrange::row(&(0..columns.len()).collect::<Vec<_>>());
        // Searchable base-branch filter. Observe it so each keystroke re-renders
        // the DiffView (and thus recomputes the filtered branch list) — the
        // TextInput only notifies itself otherwise.
        let base_filter = cx.new(|cx| TextInput::new("", "Filter branches…", cx));
        cx.observe(&base_filter, |_, _, cx| cx.notify()).detach();
        let mut view = Self {
            repo_root,
            // Seeded base (multi-project shared base) or empty until `bootstrap`
            // resolves the default off-thread — the git subprocesses must not
            // block the GPUI main thread at tab open. An empty base renders a
            // "pick a base" prompt rather than spinning on a bogus ref.
            base_ref: base.unwrap_or_default(),
            branches: Vec::new(),
            base_picker_open: false,
            base_filter,
            columns,
            focus_handle: cx.focus_handle(),
            element_id,
            watch_epoch: 0,
            mode: ViewMode::Unified,
            sync_scroll: true,
            scroll_driver: 0,
            selected_column: 0,
            all_collapsed: false,
            _watchers: Vec::new(),
            suspended: false,
            bootstrapped: false,
            arrange,
            drag_target: None,
            body_menu: None,
            last_body_pos: None,
            flash: None,
            review_menu_open: None,
            review_picks: Vec::new(),
            review_resizing: None,
            hover_line: None,
            close_removes: false,
        };
        view.bootstrap(cx);
        view
    }

    /// Host opt-in: make the column-header `×` deselect the branch (emit
    /// [`DiffViewEvent::CloseColumn`]) rather than hide it in place. Used by the
    /// Worktree scope.
    pub fn set_close_removes(&mut self, v: bool) {
        self.close_removes = v;
    }

    /// Working-tree paths of the currently visible columns, in column order.
    /// Lets the host materialize the "currently shown" branch set (including
    /// on-disk-discovered columns) when deselecting one.
    pub fn column_paths(&self) -> Vec<PathBuf> {
        self.columns
            .iter()
            .filter(|c| c.visible)
            .map(|c| c.path.clone())
            .collect()
    }

    /// Resolve the base ref + branch list off the main thread, then kick off the
    /// per-column diffs and the live-refresh watcher. Doing the git subprocesses
    /// AND the (recursive, ~20k-dir) inotify registration walk off the GPUI
    /// thread is what prevents the multi-second "not responding" freeze that
    /// `new()` used to cause at tab open.
    fn bootstrap(&mut self, cx: &mut Context<Self>) {
        let first = self.columns.first().map(|c| c.path.clone());
        let n = self.columns.len();
        // Honor a seeded base (multi-project shared base); else resolve default.
        let preset = self.base_ref.clone();
        cx.spawn(async move |this, cx| {
            log::debug!("diff: bootstrap START ({n} columns); resolving base off-thread");
            let t = Instant::now();
            let (base, branches) = match first {
                Some(p) => {
                    smol::unblock(move || {
                        // Honor a seeded base (multi-project shared base) only if
                        // it actually exists in THIS repo — else fall back to the
                        // repo's own default (develop→main→master). Empty when
                        // nothing resolves, so the toolbar prompts for a base
                        // instead of failing every column on a non-existent ref.
                        let base = if !preset.is_empty() && super::git::ref_exists(&p, &preset) {
                            preset
                        } else {
                            super::git::default_base_ref(&p).unwrap_or_default()
                        };
                        let branches = super::git::list_base_ref_candidates(&p);
                        (base, branches)
                    })
                    .await
                }
                None => (preset, Vec::new()),
            };
            log::debug!(
                "diff: bootstrap resolved base={base:?}, {} branches in {:?}; -> start_loading + start_watchers",
                branches.len(),
                t.elapsed()
            );
            let _ = cx.update(|cx| {
                this.update(cx, |view: &mut Self, cx| {
                    view.base_ref = base;
                    view.branches = branches;
                    view.bootstrapped = true;
                    view.start_loading(cx);
                    // US-016: if the surface was hidden (parked to CLI) before
                    // bootstrap resolved, do NOT arm a watcher for an invisible
                    // repo — `resume` arms it when the user returns.
                    if !view.suspended {
                        view.start_watchers(cx);
                    }
                })
            });
        })
        .detach();
    }

    /// Tab-strip title, e.g. `Diff: paneflow`.
    pub fn title(&self) -> String {
        let name = self
            .repo_root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.repo_root.display().to_string());
        format!("Diff: {name}")
    }

    fn effective_mode(&self, window: &Window) -> ViewMode {
        if self.mode == ViewMode::Unified {
            return ViewMode::Unified;
        }
        let cols = self.visible_count().max(1) as f32;
        let per_col = f32::from(window.viewport_size().width) / cols;
        if per_col < MIN_SPLIT_COLUMN_PX {
            ViewMode::Unified
        } else {
            ViewMode::Split
        }
    }

    fn visible_count(&self) -> usize {
        self.columns.iter().filter(|c| c.visible).count()
    }

    /// (Re)load every visible column's diff off the main thread. One background
    /// task per column — a slow worktree never blocks the others; the
    /// `generation` guard discards results superseded by a newer load (US-016
    /// keeps the task count bounded to the visible columns).
    fn start_loading(&mut self, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible)
            .map(|(i, _)| i)
            .collect();
        self.start_loading_columns(&indices, cx);
    }

    /// (Re)load a specific set of columns' diffs off the main thread. The full
    /// [`Self::start_loading`] passes every visible column; US-016
    /// [`Self::revalidate`] passes only the columns whose git fingerprint moved
    /// while the surface was hidden. One background task per column — a slow
    /// worktree never blocks the others; the `generation` guard discards results
    /// superseded by a newer load (US-007 last-write-wins).
    fn start_loading_columns(&mut self, indices: &[usize], cx: &mut Context<Self>) {
        let shared_base = self.base_ref.clone();
        // Snapshot the active theme on the main thread; `TerminalTheme` is `Copy`
        // so each column's background task gets its own copy to derive syntax
        // colors from, without touching the theme cache off-thread.
        let theme = crate::theme::active_theme();
        log::debug!(
            "diff: start_loading base={shared_base:?} ({} of {} columns)",
            indices.len(),
            self.columns.len()
        );
        for &i in indices {
            // Bump THIS column's generation + resolve its effective base (per-column
            // override, else the shared base) under one `get_mut`. Per-column gen so
            // a subset reload (e.g. `revalidate`) never discards an in-flight load of
            // the OTHER columns. Do NOT blank an already-loaded column to `Loading`
            // on a refresh — keep its content until the new diff swaps in (no flash).
            let (generation, base, path, branch) = match self.columns.get_mut(i) {
                Some(col) if col.visible => {
                    col.generation = col.generation.wrapping_add(1);
                    let base = col
                        .base_override
                        .clone()
                        .unwrap_or_else(|| shared_base.clone());
                    (col.generation, base, col.path.clone(), col.branch.clone())
                }
                _ => continue,
            };
            // No base resolved (no develop/main/master, or the user cleared it):
            // prompt instead of spawning a diff against a non-existent ref.
            if base.is_empty() {
                if let Some(col) = self.columns.get_mut(i) {
                    col.state = ColumnState::Failed("Select a base branch".to_string());
                }
                continue;
            }
            log::debug!("diff: col {i} ({branch}) task SPAWNED (gen={generation})");
            cx.spawn(async move |this, cx| {
                // The whole pipeline — git diff, row building, AND the syntect
                // pass — runs off the GPUI main thread; only the `Rc` wrap +
                // assignment happen back on it (NFR: 0 ms main-thread git/diff).
                log::debug!("diff: col {i} ({branch}) task STARTED (polled)");
                let bc = branch.clone();
                let built = smol::unblock(move || {
                    // US-016: snapshot the fingerprint BEFORE reading the tree, so a
                    // commit landing mid-build makes the stored fingerprint LAG the
                    // rows — `revalidate` then sees HEAD moved and reloads (a harmless
                    // extra reload) rather than matching a stale fingerprint and
                    // showing pre-commit rows as current (the unsafe direction).
                    let fingerprint = super::git::column_fingerprint(&path, &base);
                    let t0 = Instant::now();
                    let diff = super::git::compute_worktree_diff(&path, &base);
                    log::debug!(
                        "diff: col {i} ({bc}) computed {} files in {:?} (error={:?})",
                        diff.files.len(),
                        t0.elapsed(),
                        diff.error
                    );
                    if let Some(e) = diff.error {
                        return Built::Failed(e);
                    }
                    let t1 = Instant::now();
                    let syntax = SYNTAX_HIGHLIGHT_ENABLED
                        .then(|| super::syntax::DiffSyntax::from_theme(&theme));
                    let (unified, _) = build_display_rows(&diff.files, syntax.as_ref());
                    let (split, _) = build_split_rows(&diff.files, syntax.as_ref());
                    // File path -> header row index, in file order, so a sidebar
                    // click can scroll the body to that file. Header rows are
                    // emitted one per file in `diff.files` order, so zipping
                    // realigns them (the zip naturally truncates if the row cap
                    // dropped trailing headers).
                    let anchors_unified: Vec<(String, usize)> = diff
                        .files
                        .iter()
                        .map(|f| f.path.clone())
                        .zip(
                            unified
                                .iter()
                                .enumerate()
                                .filter(|(_, r)| r.kind == RowKind::FileHeader)
                                .map(|(i, _)| i),
                        )
                        .collect();
                    let anchors_split: Vec<(String, usize)> = diff
                        .files
                        .iter()
                        .map(|f| f.path.clone())
                        .zip(
                            split
                                .iter()
                                .enumerate()
                                .filter(|(_, r)| matches!(r, SplitRow::Header(_)))
                                .map(|(i, _)| i),
                        )
                        .collect();
                    // US-008: lightweight per-file summary for the git panel,
                    // built here (off-thread) from the same FileDiffs.
                    let files = diff
                        .files
                        .iter()
                        .map(|f| {
                            let (added, removed) = f.line_counts();
                            FileEntry {
                                path: f.path.clone(),
                                change: f.change,
                                old_path: f.old_path.clone(),
                                added,
                                removed,
                                is_binary: f.is_binary,
                            }
                        })
                        .collect();
                    log::debug!(
                        "diff: col {i} ({bc}) built {} unified / {} split rows in {:?}",
                        unified.len(),
                        split.len(),
                        t1.elapsed()
                    );
                    Built::Loaded {
                        unified,
                        split,
                        file_count: diff.files.len(),
                        files,
                        anchors_unified,
                        anchors_split,
                        // Move the raw FileDiffs out for copy/review (US-001..005);
                        // every `&diff.files` consumer above has finished borrowing.
                        files_full: diff.files,
                        fingerprint,
                    }
                })
                .await;
                log::debug!("diff: col {i} ({branch}) off-thread done, applying on main thread");
                cx.update(|cx| {
                    let _ = this.update(cx, |view: &mut Self, cx| {
                        let Some(col) = view.columns.get_mut(i) else {
                            return;
                        };
                        if col.generation != generation {
                            log::warn!(
                                "diff: col {i} ({branch}) DISCARDED — task gen={generation} != col gen={}",
                                col.generation
                            );
                            return; // superseded by a newer load of this column
                        }
                        let new_state = match built {
                            Built::Failed(e) => {
                                log::warn!("diff: col {i} ({branch}) FAILED: {e}");
                                ColumnState::Failed(e)
                            }
                            Built::Loaded {
                                unified,
                                split,
                                file_count,
                                files,
                                anchors_unified,
                                anchors_split,
                                files_full,
                                fingerprint,
                            } => {
                                log::debug!("diff: col {i} ({branch}) LOADED ({file_count} files)");
                                // US-016: stamp the fingerprint these rows were
                                // built against, for warm-resume revalidation.
                                col.fingerprint = Some(fingerprint);
                                ColumnState::Loaded {
                                    unified: Rc::new(unified),
                                    split: Rc::new(split),
                                    file_count,
                                    files: Rc::new(files),
                                    anchors_unified: Rc::new(anchors_unified),
                                    anchors_split: Rc::new(anchors_split),
                                    files_full: Rc::new(files_full),
                                }
                            }
                        };
                        col.state = new_state;
                        // Rebuild the collapse-filtered views from the fresh rows
                        // (carries any per-file collapse across the reload).
                        col.recompute_display();
                        cx.notify();
                    });
                });
            })
            .detach();
        }
        // Repaint now so any column set to `Failed` (empty base) above shows its
        // prompt immediately; loaded columns also repaint when their task applies.
        cx.notify();
    }

    /// Watch the shared `.git` (for HEAD / index / base-ref moves) and each
    /// worktree's source subtrees — but NEVER build/VCS dirs like `target/`.
    /// A `cargo`/rust-analyzer build churns `target/` (tens of thousands of
    /// files) continuously; watching it recursively floods the event loop and
    /// starves the diff result updates (the "stuck on Computing diff…" + lag).
    /// Real working-tree edits, HEAD/index moves, and base-ref advances still
    /// land here. Build is off-thread (the registration walk must not block the
    /// GPUI event loop — that was the earlier "not responding" freeze).
    fn start_watchers(&mut self, cx: &mut Context<Self>) {
        let mut worktrees: Vec<PathBuf> = self.columns.iter().map(|c| c.path.clone()).collect();
        worktrees.sort();
        worktrees.dedup();
        let repo_root = self.repo_root.clone();
        // US-016: the epoch this watcher build belongs to. `suspend` bumps the
        // epoch, so if a suspend (or a later re-arm) raced this off-thread build,
        // the freshly-built watcher is dropped rather than installed on a hidden
        // entity, and the event loop below stops the moment the epoch advances.
        let epoch = self.watch_epoch;

        let (tx, mut rx) = mpsc::unbounded::<notify::Result<Event>>();

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                log::debug!("diff: start_watchers building watcher off-thread");
                let watcher = smol::unblock(move || build_watcher(tx, worktrees, repo_root)).await;
                let Some(watcher) = watcher else {
                    log::warn!("diff: watcher build returned None");
                    return;
                };
                // Install only if our epoch is still current — else a suspend
                // landed while we were building; drop the watcher (unregisters its
                // OS handles) and abandon this loop.
                // `AsyncApp::update` returns the closure's value directly (R), so
                // the inner `WeakEntity::update` Result is unwrapped here and the
                // outer call yields the bool as-is.
                let installed = cx.update(|cx| {
                    this.update(cx, |view: &mut Self, _| {
                        if view.watch_epoch != epoch {
                            return false;
                        }
                        view._watchers.push(watcher);
                        true
                    })
                    .unwrap_or(false)
                });
                if !installed {
                    log::debug!("diff: watcher build superseded (epoch advanced) — dropped");
                    return;
                }
                log::debug!("diff: watcher live, entering event loop");

                let mut relevant_events = 0u64;
                while let Some(res) = rx.next().await {
                    // Drop build/VCS churn (`target/`, object store, logs,
                    // node_modules) — otherwise a `cargo build` triggers a
                    // re-diff storm. Only real working-tree / ref changes pass.
                    if !watch_event_relevant(&res) {
                        continue;
                    }
                    relevant_events += 1;
                    if let Ok(ev) = &res {
                        log::debug!(
                            "diff: watcher relevant event #{relevant_events} ({:?} {:?}) -> debounce",
                            ev.kind,
                            ev.paths.first()
                        );
                    }
                    // Coalesce the burst before re-diffing once.
                    let deadline = Instant::now() + REFRESH_DEBOUNCE;
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match futures::future::select(rx.next(), smol::Timer::after(remaining))
                            .await
                        {
                            Either::Left((Some(_), _)) => {}
                            Either::Left((None, _)) => return,
                            Either::Right(_) => break,
                        }
                    }
                    log::debug!("diff: watcher debounce done -> start_loading (reload)");
                    // Stop if a suspend advanced the epoch (don't reload a hidden
                    // entity); the WeakEntity being dead also stops the loop.
                    let alive = cx.update(|cx| {
                        this.update(cx, |view: &mut Self, cx| {
                            if view.watch_epoch != epoch {
                                return false;
                            }
                            view.start_loading(cx);
                            true
                        })
                        .unwrap_or(false)
                    });
                    if !alive {
                        break;
                    }
                    // Cooldown: drain + discard events for a beat after a reload,
                    // so any churn the reload itself (or a concurrent build) kicks
                    // up can't immediately re-trigger and starve the in-flight
                    // load. Belt-and-suspenders with the no-`.git` watch set.
                    let cooldown = Instant::now() + REFRESH_COOLDOWN;
                    loop {
                        let remaining = cooldown.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match futures::future::select(rx.next(), smol::Timer::after(remaining))
                            .await
                        {
                            Either::Left((Some(_), _)) => {} // discard
                            Either::Left((None, _)) => return,
                            Either::Right(_) => break,
                        }
                    }
                }
            },
        )
        .detach();
    }

    /// Per-branch changed-file lists for the multi-branch diff sidebar: one entry
    /// per visible column as `(branch, column index, worktree path, file-list
    /// state)`. The worktree path is the stable, globally-unique key the sidebar
    /// uses for per-section collapse state — branch NAMES collide across repos in
    /// Multi-project scope (every repo has a `main`). Reads the same `Rc`-shared
    /// file vecs, so it is allocation-cheap per frame.
    pub fn column_file_lists(&self) -> Vec<(String, usize, PathBuf, FileListState)> {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible)
            .map(|(i, c)| {
                let state = match &c.state {
                    ColumnState::Loading => FileListState::Loading,
                    ColumnState::Failed(e) => FileListState::Failed(e.clone()),
                    ColumnState::Loaded { files, .. } => FileListState::Loaded(files.clone()),
                };
                (c.branch.clone(), i, c.path.clone(), state)
            })
            .collect()
    }

    /// Index of the column whose file list currently drives the sidebar/diffstat
    /// (so the sidebar can mark the active branch's section).
    pub fn selected_column(&self) -> usize {
        self.selected_column
    }

    /// Select `col_idx` (focus its file list) AND scroll its body to `path`.
    /// Used by the multi-branch sidebar so clicking a file in ANY branch section
    /// focuses that branch and lands on the file — `jump_to_file` keys off the
    /// just-set `selected_column`.
    pub fn select_and_jump(
        &mut self,
        col_idx: usize,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_column(col_idx, cx);
        self.jump_to_file(path, window, cx);
    }

    /// Toggle a column between the shared base (e.g. `develop`) and "just my
    /// latest commit" (`HEAD~1`), reloading ONLY that column. One branch can show
    /// its last-commit delta while its siblings keep the whole-branch-vs-base
    /// view — the 80/20 of commit-granular review without a full commit walk.
    fn toggle_column_base(&mut self, idx: usize, cx: &mut Context<Self>) {
        match self.columns.get_mut(idx) {
            Some(col) => {
                col.base_override = match col.base_override {
                    None => Some("HEAD~1".to_string()),
                    Some(_) => None,
                };
            }
            None => return,
        }
        self.start_loading_columns(&[idx], cx);
    }

    /// Set the shared base from free-text (base popover Enter with no list match):
    /// accepts an arbitrary ref / tag / SHA, validated off-thread via `git
    /// rev-parse` against the first column's worktree. Applied only if it
    /// resolves, so a typo can't blank every column against a non-existent ref.
    fn resolve_and_set_base(&mut self, raw: String, cx: &mut Context<Self>) {
        let raw = raw.trim().to_string();
        if raw.is_empty() {
            return;
        }
        let Some(probe_dir) = self.columns.first().map(|c| c.path.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let raw2 = raw.clone();
            let ok = smol::unblock(move || super::git::ref_exists(&probe_dir, &raw2)).await;
            if !ok {
                log::debug!("diff: base '{raw}' did not resolve to a ref; ignored");
                return;
            }
            let _ = cx.update(|cx| this.update(cx, |view: &mut Self, cx| view.set_base(raw, cx)));
        })
        .detach();
    }

    /// US-016 warm-resume: release the OS filesystem watchers + end the debounce
    /// loop while the diff surface is hidden (CLI/Agents mode, or cached and not
    /// displayed) — WITHOUT discarding the computed rows. Clearing `_watchers`
    /// drops every `RecommendedWatcher` (unregistering the OS handles) and with
    /// them the channel sender the watcher loop holds, so its `rx.next()` yields
    /// `None` and the spawned debounce task returns (no orphan task, no `defer`
    /// into a hidden entity). The loaded `ColumnState` rows stay resident (RAM
    /// only, already capped by the `MAX_FILE_*` guards) so [`Self::resume`] can
    /// show them in one frame. Honors the US-016 AC — only watchers/background
    /// tasks for the now-hidden repo are dropped, never the view's data.
    pub fn suspend(&mut self, _cx: &mut Context<Self>) {
        if self.suspended {
            return;
        }
        self.suspended = true;
        // Bump the watcher epoch so any in-flight off-thread watcher build (or a
        // running debounce loop) retires instead of installing/refreshing on this
        // now-hidden entity. Then drop the live watchers.
        self.watch_epoch = self.watch_epoch.wrapping_add(1);
        self._watchers.clear();
    }

    /// US-016 warm-resume: re-arm the live-refresh watcher after a [`Self::suspend`],
    /// then revalidate cheaply — re-diff ONLY the columns whose git fingerprint
    /// moved while hidden (a commit, a `git add`, a working-tree edit, a new
    /// untracked file, or a base-ref advance an AI agent made from a CLI pane).
    /// Unchanged columns keep their loaded rows untouched (no "Computing diff…"
    /// flash, no recompute); when nothing moved, resume is just a watcher re-arm.
    /// No-op if not suspended. If `bootstrap` has not finished yet, clearing
    /// `suspended` is enough — bootstrap arms the watcher + loads once it
    /// resolves (and now sees `suspended == false`).
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if !self.suspended {
            return;
        }
        self.suspended = false;
        if !self.bootstrapped {
            return; // bootstrap still in flight; it arms + loads itself
        }
        self.start_watchers(cx);
        if self.base_ref.is_empty() {
            return; // no base resolved — nothing to revalidate
        }
        self.revalidate(cx);
    }

    /// US-016: off the main thread, fingerprint every visible column and re-diff
    /// only those whose fingerprint changed since they were last loaded. The
    /// cheap probe runs in one `smol::unblock` (never on the GPUI thread); the
    /// targeted reload reuses [`Self::start_loading_columns`], so unchanged
    /// columns are never touched (no flash, no recompute).
    fn revalidate(&mut self, cx: &mut Context<Self>) {
        let shared_base = self.base_ref.clone();
        // Carry each column's EFFECTIVE base (per-column override, else shared) so
        // the fingerprint matches what that column was actually diffed against —
        // else a column on `HEAD~1` would falsely revalidate every resume.
        let probes: Vec<(
            usize,
            PathBuf,
            String,
            Option<super::git::ColumnFingerprint>,
        )> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.visible)
            .map(|(i, c)| {
                (
                    i,
                    c.path.clone(),
                    c.base_override
                        .clone()
                        .unwrap_or_else(|| shared_base.clone()),
                    c.fingerprint.clone(),
                )
            })
            .collect();
        if probes.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let changed: Vec<usize> = smol::unblock(move || {
                probes
                    .into_iter()
                    .filter(|(_, path, base, stored)| {
                        stored.as_ref() != Some(&super::git::column_fingerprint(path, base))
                    })
                    .map(|(i, _, _, _)| i)
                    .collect()
            })
            .await;
            if changed.is_empty() {
                log::debug!("diff: resume revalidate — no column changed, warm reuse");
                return;
            }
            log::debug!(
                "diff: resume revalidate — reloading {} changed column(s)",
                changed.len()
            );
            let _ = cx.update(|cx| {
                this.update(cx, |view: &mut Self, cx| {
                    // The user may have toggled back to CLI while the probe ran —
                    // don't spawn diff work on a now-hidden entity.
                    if view.suspended {
                        return;
                    }
                    view.start_loading_columns(&changed, cx);
                })
            });
        })
        .detach();
    }

    /// Toggle the base-branch popover. Opening clears + focuses the filter so
    /// the user can type-to-search immediately; closing returns focus to the
    /// diff body so keyboard scrolling keeps working.
    fn toggle_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.base_picker_open = !self.base_picker_open;
        if self.base_picker_open {
            self.base_filter.update(cx, |inp, cx| {
                inp.content = SharedString::default();
                inp.selected_range = 0..0;
                cx.notify();
            });
            let fh = self.base_filter.read(cx).focus_handle.clone();
            window.focus(&fh, cx);
        } else {
            window.focus(&self.focus_handle, cx);
        }
        cx.notify();
    }

    /// Close the base popover (mouse-down-out / Escape / after a pick) and hand
    /// focus back to the diff body.
    fn close_base_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.base_picker_open {
            self.base_picker_open = false;
            window.focus(&self.focus_handle, cx);
            cx.notify();
        }
    }

    /// US-013: switch the shared base ref and re-key every column.
    fn set_base(&mut self, base: String, cx: &mut Context<Self>) {
        if base == self.base_ref {
            self.base_picker_open = false;
            cx.notify();
            return;
        }
        self.base_ref = base;
        self.base_picker_open = false;
        self.start_loading(cx);
        cx.notify();
    }

    /// US-014: hide a column (drop its data, skip future refreshes).
    fn hide_column(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(col) = self.columns.get_mut(idx) {
            col.visible = false;
            col.state = ColumnState::Loading; // dropped data; reloads on re-show
            cx.notify();
        }
    }

    /// US-014: re-show every hidden column and reload them.
    fn show_all_columns(&mut self, cx: &mut Context<Self>) {
        for col in &mut self.columns {
            col.visible = true;
        }
        self.start_loading(cx);
        cx.notify();
    }

    /// The base ref currently diffed against (read by the multi-project host to
    /// seed sibling repos with the same base).
    pub fn base_ref(&self) -> &str {
        &self.base_ref
    }

    /// Append worktree columns not already present (dedup by normalized path),
    /// load them, and re-arm the watcher to cover the new trees. Used by
    /// Worktree-scope on-disk discovery and live workspace-add so a new branch
    /// shows up without re-mounting the whole view (which would flash every
    /// column back to Loading).
    pub fn add_columns(&mut self, worktrees: Vec<DiffWorktree>, cx: &mut Context<Self>) {
        let existing: std::collections::HashSet<String> =
            self.columns.iter().map(|c| norm_key(&c.path)).collect();
        let mut added = false;
        for w in worktrees {
            if existing.contains(&norm_key(&w.path)) {
                continue;
            }
            self.columns
                .push(Column::new_loading(w.branch, w.path, w.workspace_id));
            added = true;
        }
        if added {
            // start_loading keeps already-loaded columns' content until their
            // fresh diff swaps in, so only the new columns visibly start from
            // Loading. Re-arm the watcher off-thread to include the new trees.
            self.start_loading(cx);
            self._watchers.clear();
            self.start_watchers(cx);
        }
    }

    /// Select the column whose changed-file list feeds the sidebar and whose
    /// body `jump_to_file` scrolls. Bound to a column-header click.
    fn select_column(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.selected_column != idx {
            self.selected_column = idx;
            cx.notify();
        }
    }

    /// Toggle cross-column scroll synchronization (toolbar control).
    fn toggle_sync(&mut self, cx: &mut Context<Self>) {
        self.sync_scroll = !self.sync_scroll;
        cx.notify();
    }

    /// Scroll the selected column's body so `path`'s file header is at the top.
    /// With sync on, the per-render broadcast carries the other columns to the
    /// same row offset (file-aligned where the columns share that file).
    pub fn jump_to_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.effective_mode(window);
        let target = self
            .columns
            .get(self.selected_column)
            .filter(|c| c.visible)
            .and_then(|col| {
                // Index against the *displayed* anchors so the jump lands right
                // even when files above are collapsed.
                let anchors = match mode {
                    ViewMode::Unified => &col.disp_anchors_unified,
                    ViewMode::Split => &col.disp_anchors_split,
                };
                let idx = anchors.iter().find(|(p, _)| p == path).map(|(_, i)| *i)?;
                // Cumulative top offset of the header row — rows have variable
                // heights (taller file-header cards), so summing is exact.
                let y = match mode {
                    ViewMode::Unified => col
                        .disp_unified
                        .get(..idx)
                        .map(|s| s.iter().map(display_row_height).sum::<f32>())
                        .unwrap_or(0.0),
                    ViewMode::Split => col
                        .disp_split
                        .get(..idx)
                        .map(|s| s.iter().map(split_row_height).sum::<f32>())
                        .unwrap_or(0.0),
                };
                Some((col.el_scroll.clone(), y))
            });
        let Some((handle, y)) = target else {
            return;
        };
        let x = handle.offset().x;
        handle.set_offset(point(x, px(-y)));
        // Drive the sync broadcast from the selected column this frame.
        self.scroll_driver = self.selected_column;
        cx.notify();
    }

    /// The selected column if visible, else the first visible column — the one
    /// the toolbar's diffstat / hunk-nav act on.
    fn selected_or_first_visible(&self) -> Option<usize> {
        if self
            .columns
            .get(self.selected_column)
            .is_some_and(|c| c.visible)
        {
            Some(self.selected_column)
        } else {
            self.columns.iter().position(|c| c.visible)
        }
    }

    /// Cumulative top offsets (px) of each hunk's first changed row in a
    /// column's displayed rows. A "hunk start" is a change row (Added/Removed)
    /// whose predecessor is not a change — so consecutive removed+added lines
    /// count as one hunk. Computed on demand (nav is user-driven, not per-frame).
    fn hunk_tops(col: &Column, mode: ViewMode) -> Vec<f32> {
        let mut tops = Vec::new();
        let mut acc = 0.0f32;
        let mut prev_change = false;
        match mode {
            ViewMode::Unified => {
                for r in col.disp_unified.iter() {
                    let is_change = matches!(r.kind, RowKind::Added | RowKind::Removed);
                    if is_change && !prev_change {
                        tops.push(acc);
                    }
                    prev_change = is_change;
                    acc += display_row_height(r);
                }
            }
            ViewMode::Split => {
                for r in col.disp_split.iter() {
                    let is_change = matches!(
                        r,
                        SplitRow::Pair { left, right }
                            if matches!(left.kind, CellKind::Added | CellKind::Removed)
                                || matches!(right.kind, CellKind::Added | CellKind::Removed)
                    );
                    if is_change && !prev_change {
                        tops.push(acc);
                    }
                    prev_change = is_change;
                    acc += split_row_height(r);
                }
            }
        }
        tops
    }

    /// Jump the selected column to the next/previous hunk relative to its
    /// current scroll position (cycles at the ends). Stateless: the target is
    /// derived from where the viewport is, so it stays correct after manual
    /// scrolling. The synced columns follow via the per-render broadcast.
    fn goto_hunk(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.effective_mode(window);
        let Some(ci) = self.selected_or_first_visible() else {
            return;
        };
        let Some((handle, tops, cur_y)) = self.columns.get(ci).map(|col| {
            let cur_y = f32::from(-col.el_scroll.offset().y).max(0.0);
            (col.el_scroll.clone(), Self::hunk_tops(col, mode), cur_y)
        }) else {
            return;
        };
        if tops.is_empty() {
            return;
        }
        // A jumped-to hunk is parked HUNK_JUMP_MARGIN px below the viewport top,
        // so the hunk "at" the current position is the one near
        // `cur_y + HUNK_JUMP_MARGIN` — not `cur_y`. Pivot on that: otherwise
        // `forward` keeps matching the already-parked hunk (its top is still
        // > cur_y), and the down arrow looks dead while up works.
        let pivot = cur_y + HUNK_JUMP_MARGIN;
        let target = if forward {
            tops.iter()
                .copied()
                .find(|&t| t > pivot + 4.0)
                .unwrap_or(tops[0])
        } else {
            tops.iter()
                .rev()
                .copied()
                .find(|&t| t < pivot - 4.0)
                .unwrap_or_else(|| *tops.last().unwrap_or(&0.0))
        };
        let x = handle.offset().x;
        handle.set_offset(point(x, px((HUNK_JUMP_MARGIN - target).min(0.0))));
        self.selected_column = ci;
        self.scroll_driver = ci;
        cx.notify();
    }

    /// Body click: focus the column, and if it landed on a file-header row,
    /// toggle that file's collapse. Maps the click Y to a displayed row via the
    /// scroll handle's painted bounds + offset (uniform [`ROW_HEIGHT`]).
    fn handle_body_click(
        &mut self,
        col_idx: usize,
        ev: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_column(col_idx, cx);
        let mode = self.effective_mode(window);
        // prd-ai-in-diff-2026-Q3.md: left-click a changed line sends it to the
        // review CLI to ask about it — launching Claude Code first if no session
        // is open. Context/header rows fall through to header-collapse.
        if let Some(line) = self.resolve_clicked_line(col_idx, ev.position(), mode) {
            self.ask_review_about_line(col_idx, line, window, cx);
            return;
        }
        let row = {
            let Some(col) = self.columns.get(col_idx) else {
                return;
            };
            let bounds = col.el_scroll.bounds();
            let y = ev.position().y;
            if y < bounds.top() || y > bounds.bottom() {
                return;
            }
            let target = f32::from(y - bounds.top() - col.el_scroll.offset().y).max(0.0);
            // Variable row heights (taller file-header cards): walk cumulative
            // heights to find the clicked row.
            let mut acc = 0.0f32;
            let mut hit = None;
            match mode {
                ViewMode::Unified => {
                    for (i, r) in col.disp_unified.iter().enumerate() {
                        let h = display_row_height(r);
                        if target < acc + h {
                            hit = Some(i);
                            break;
                        }
                        acc += h;
                    }
                }
                ViewMode::Split => {
                    for (i, r) in col.disp_split.iter().enumerate() {
                        let h = split_row_height(r);
                        if target < acc + h {
                            hit = Some(i);
                            break;
                        }
                        acc += h;
                    }
                }
            }
            match hit {
                Some(r) => r,
                None => return, // click past the last row
            }
        };
        let path = {
            let Some(col) = self.columns.get(col_idx) else {
                return;
            };
            let anchors = match mode {
                ViewMode::Unified => &col.disp_anchors_unified,
                ViewMode::Split => &col.disp_anchors_split,
            };
            anchors
                .iter()
                .find(|(_, i)| *i == row)
                .map(|(p, _)| p.clone())
        };
        let Some(path) = path else {
            return; // not a file header — nothing to collapse
        };
        if let Some(col) = self.columns.get_mut(col_idx) {
            if !col.collapsed.remove(&path) {
                col.collapsed.insert(path);
            }
            col.recompute_display();
            cx.notify();
        }
    }

    /// US-002: map a window-space point over column `col_idx`'s body to its row
    /// index, walking the same variable row heights as [`Self::handle_body_click`].
    fn row_at_point(&self, col_idx: usize, point: Point<Pixels>, mode: ViewMode) -> Option<usize> {
        let col = self.columns.get(col_idx)?;
        let bounds = col.el_scroll.bounds();
        if point.y < bounds.top() || point.y > bounds.bottom() {
            return None;
        }
        let target = f32::from(point.y - bounds.top() - col.el_scroll.offset().y).max(0.0);
        let mut acc = 0.0f32;
        match mode {
            ViewMode::Unified => {
                for (i, r) in col.disp_unified.iter().enumerate() {
                    let h = display_row_height(r);
                    if target < acc + h {
                        return Some(i);
                    }
                    acc += h;
                }
            }
            ViewMode::Split => {
                for (i, r) in col.disp_split.iter().enumerate() {
                    let h = split_row_height(r);
                    if target < acc + h {
                        return Some(i);
                    }
                    acc += h;
                }
            }
        }
        None
    }

    /// US-002: resolve a body point to the file (+ optional enclosing hunk) under
    /// it. Returns `None` for a click in a gap, on a collapsed/blank area, or when
    /// the column is not loaded. Hunk resolution is unified-mode only (the split
    /// view resolves to file scope); a click on a context/header line yields a
    /// file scope with no hunk.
    fn resolve_body_scope(
        &self,
        col_idx: usize,
        point: Point<Pixels>,
        mode: ViewMode,
    ) -> Option<DiffBodyScope> {
        let row = self.row_at_point(col_idx, point, mode)?;
        let col = self.columns.get(col_idx)?;
        let ColumnState::Loaded { files_full, .. } = &col.state else {
            return None;
        };
        let anchors = match mode {
            ViewMode::Unified => &col.disp_anchors_unified,
            ViewMode::Split => &col.disp_anchors_split,
        };
        // The file whose header row is the closest one at or above `row`.
        let path = anchors
            .iter()
            .filter(|(_, hdr)| *hdr <= row)
            .max_by_key(|(_, hdr)| *hdr)
            .map(|(p, _)| p.clone())?;
        let file_idx = files_full.iter().position(|f| f.path == path)?;
        let hunk_idx = match mode {
            ViewMode::Unified => {
                let r = col.disp_unified.get(row)?;
                let file = files_full.get(file_idx)?;
                match r.kind {
                    RowKind::Added => r.new_no.and_then(|n| n.checked_sub(1)).and_then(|idx| {
                        file.hunks
                            .iter()
                            .position(|h| h.new_row_range.contains(&idx))
                    }),
                    RowKind::Removed => r.old_no.and_then(|n| n.checked_sub(1)).and_then(|idx| {
                        file.hunks
                            .iter()
                            .position(|h| h.base_row_range.contains(&idx))
                    }),
                    _ => None,
                }
            }
            ViewMode::Split => None,
        };
        Some(DiffBodyScope { file_idx, hunk_idx })
    }

    /// US-003: serialize the scope (a single hunk when `want_hunk`, else the whole
    /// file) to the clipboard and flash a confirmation. Copying a hunk on a
    /// non-hunk scope is a no-op with a "No hunk here" flash.
    fn copy_scope(
        &mut self,
        col_idx: usize,
        scope: DiffBodyScope,
        want_hunk: bool,
        cx: &mut Context<Self>,
    ) {
        let result = {
            let Some(col) = self.columns.get(col_idx) else {
                return;
            };
            let ColumnState::Loaded { files_full, .. } = &col.state else {
                return;
            };
            let Some(file) = files_full.get(scope.file_idx) else {
                return;
            };
            if want_hunk {
                scope.hunk_idx.and_then(|h| file.hunks.get(h)).map(|hunk| {
                    (
                        super::extract::hunk_to_unified(file, hunk),
                        format!("Hunk copied ({})", super::extract::hunk_tag(file, hunk)),
                    )
                })
            } else {
                Some((
                    super::extract::file_to_unified(file),
                    format!("Copied {} diff", file.path),
                ))
            }
        };
        match result {
            Some((diff, msg)) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(diff));
                self.set_flash(msg.into(), cx);
            }
            None => self.set_flash("No hunk here".into(), cx),
        }
    }

    /// US-003 action handler (`Ctrl+Shift+C` in the `DiffView` context): copy the
    /// hunk under the last-known cursor position.
    fn copy_hovered_hunk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.effective_mode(window);
        let Some((col_idx, point)) = self.last_body_pos else {
            self.set_flash("No hunk here".into(), cx);
            return;
        };
        match self.resolve_body_scope(col_idx, point, mode) {
            Some(scope) => self.copy_scope(col_idx, scope, true, cx),
            None => self.set_flash("No hunk here".into(), cx),
        }
    }

    /// US-003: open the right-click body menu, pre-resolving the scope under the
    /// pointer. A right-click that resolves to nothing closes any open menu.
    fn open_body_menu(
        &mut self,
        col_idx: usize,
        point: Point<Pixels>,
        mode: ViewMode,
        cx: &mut Context<Self>,
    ) {
        self.select_column(col_idx, cx);
        self.body_menu = self
            .resolve_body_scope(col_idx, point, mode)
            .map(|scope| DiffBodyMenu {
                position: point,
                col_idx,
                scope,
            });
        cx.notify();
    }

    /// Resolve the changed line under a body point (unified mode only): its file
    /// path, 1-based line number, content, and whether it is a removed line.
    /// `None` on a context/header/gap row.
    fn resolve_clicked_line(
        &self,
        col_idx: usize,
        point: Point<Pixels>,
        mode: ViewMode,
    ) -> Option<ClickedLine> {
        if mode != ViewMode::Unified {
            return None;
        }
        let row = self.row_at_point(col_idx, point, mode)?;
        let col = self.columns.get(col_idx)?;
        let path = col
            .disp_anchors_unified
            .iter()
            .filter(|(_, hdr)| *hdr <= row)
            .max_by_key(|(_, hdr)| *hdr)
            .map(|(p, _)| p.clone())?;
        let r = col.disp_unified.get(row)?;
        let (lineno, removed) = match r.kind {
            RowKind::Added => (r.new_no?, false),
            RowKind::Removed => (r.old_no?, true),
            _ => return None,
        };
        Some(ClickedLine {
            path,
            lineno,
            content: r.text.to_string(),
            removed,
        })
    }

    /// The unified row under `point` IF it is a changed line (added/removed) —
    /// the hover-to-ask affordance. Left-clicking it sends the line to the review
    /// CLI, launching Claude Code first if no session is open, so changed lines
    /// are always clickable.
    fn actionable_row_at(
        &self,
        col_idx: usize,
        point: Point<Pixels>,
        mode: ViewMode,
    ) -> Option<usize> {
        if mode != ViewMode::Unified {
            return None;
        }
        let col = self.columns.get(col_idx)?;
        let row = self.row_at_point(col_idx, point, mode)?;
        let r = col.disp_unified.get(row)?;
        matches!(r.kind, RowKind::Added | RowKind::Removed).then_some(row)
    }

    /// Append `text` to the column's review CLI input WITHOUT Enter, then focus
    /// it, so the user types their question after. If NO session is open on the
    /// column, default to launching Claude Code and pre-fill `text` once it boots
    /// (prd-ai-in-diff-2026-Q3.md: left-click a line with no session running).
    fn send_to_review(
        &mut self,
        col_idx: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Existing session -> send immediately + focus.
        if let Some(col) = self.columns.get(col_idx)
            && let Some(rt) = col.review_terminals.first()
        {
            let term = rt.terminal.clone();
            term.read(cx).send_text(&text);
            term.read(cx).focus_handle(cx).focus(window, cx);
            return;
        }
        // No session -> launch Claude Code by default, pre-fill `text` after boot.
        let Some(col) = self.columns.get(col_idx) else {
            return;
        };
        let cwd = col.path.clone();
        let ws_id = col.workspace_id.unwrap_or(0);
        let cli = super::review::ReviewCli::ClaudeCode;
        let term = cx.new(|cx| crate::terminal::TerminalView::with_cwd(ws_id, Some(cwd), None, cx));
        term.read(cx).send_command(cli.launch_command());
        let prefill = text.clone();
        let term_weak = term.downgrade();
        cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
            smol::Timer::after(Duration::from_millis(REVIEW_PREFILL_DELAY_MS)).await;
            cx.update(|cx| {
                if let Some(t) = term_weak.upgrade() {
                    t.read(cx).send_text(&prefill);
                }
            });
        })
        .detach();
        term.read(cx).focus_handle(cx).focus(window, cx);
        if let Some(col) = self.columns.get_mut(col_idx) {
            col.review_terminals.push(ReviewTerminal {
                label: cli.label().into(),
                terminal: term,
            });
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        cx.notify();
    }

    /// Send a changed line (`path:line` + content) into the review CLI input so
    /// the user can ask about it.
    fn ask_review_about_line(
        &mut self,
        col_idx: usize,
        line: ClickedLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tag = if line.removed {
            format!("{}:{} (removed)", line.path, line.lineno)
        } else {
            format!("{}:{}", line.path, line.lineno)
        };
        let text = format!("`{tag}` `{}` — ", line.content.trim());
        self.send_to_review(col_idx, text, window, cx);
    }

    /// Send a hunk's unified diff into the review CLI input so the user can ask
    /// about it.
    fn ask_review_about_hunk(
        &mut self,
        col_idx: usize,
        scope: DiffBodyScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = {
            let Some(col) = self.columns.get(col_idx) else {
                return;
            };
            let ColumnState::Loaded { files_full, .. } = &col.state else {
                return;
            };
            let Some(file) = files_full.get(scope.file_idx) else {
                return;
            };
            let Some(hunk) = scope.hunk_idx.and_then(|h| file.hunks.get(h)) else {
                return;
            };
            format!(
                "About this change:\n{}\n",
                super::extract::hunk_to_unified(file, hunk)
            )
        };
        self.send_to_review(col_idx, text, window, cx);
    }

    /// US-003: show a transient confirmation pill, auto-cleared after a beat.
    fn set_flash(&mut self, msg: SharedString, cx: &mut Context<Self>) {
        self.flash = Some(msg);
        cx.notify();
        cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_millis(1600)).await;
            let _ = this.update(cx, |this, cx| {
                this.flash = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// US-003: the deferred right-click menu, window-anchored at the click point.
    fn render_body_menu(
        &self,
        menu: &DiffBodyMenu,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let has_hunk = menu.scope.hunk_idx.is_some();
        let col_idx = menu.col_idx;
        let scope = menu.scope;
        let panel = div()
            .id("diff-body-context-menu")
            .occlude()
            .w(px(230.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .rounded(px(8.))
            .shadow_lg()
            .flex()
            .flex_col()
            .p(px(4.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.body_menu = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            // Send the hunk into the embedded review CLI's input so the user can
            // ask about it (a changed LINE is sent by left-clicking it directly).
            .when(has_hunk, |panel| {
                panel.child(
                    div()
                        .id("diff-menu-ask-hunk")
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .text_size(px(12.))
                        .text_color(ui.text)
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.body_menu = None;
                            this.ask_review_about_hunk(col_idx, scope, window, cx);
                            cx.stop_propagation();
                        }))
                        .child("Ask the CLI about this hunk"),
                )
            })
            .child(
                div()
                    .id("diff-menu-copy-hunk")
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .text_color(if has_hunk { ui.text } else { ui.muted })
                    .when(has_hunk, |d| {
                        d.cursor_pointer()
                            .hover(|s| {
                                let ui = crate::theme::ui_colors();
                                s.bg(ui.subtle)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                this.body_menu = None;
                                this.copy_scope(col_idx, scope, true, cx);
                                cx.stop_propagation();
                            }))
                    })
                    .child("Copy hunk"),
            )
            .child(
                div()
                    .id("diff-menu-copy-file")
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .cursor_pointer()
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle)
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.body_menu = None;
                        this.copy_scope(col_idx, scope, false, cx);
                        cx.stop_propagation();
                    }))
                    .child("Copy file diff"),
            );
        deferred(
            anchored()
                .position(menu.position)
                .snap_to_window()
                .child(panel),
        )
        .priority(3)
        .into_any_element()
    }

    /// US-003: the transient "copied" pill, centered near the bottom of the view.
    fn render_flash(&self, msg: SharedString, ui: crate::theme::UiColors) -> AnyElement {
        deferred(
            div()
                .absolute()
                .bottom(px(16.))
                .left_0()
                .right_0()
                .flex()
                .flex_row()
                .justify_center()
                .child(
                    div()
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(6.))
                        .bg(ui.overlay)
                        .border_1()
                        .border_color(ui.border)
                        .shadow_lg()
                        .text_size(px(11.))
                        .text_color(ui.text)
                        .child(msg),
                ),
        )
        .priority(4)
        .into_any_element()
    }

    /// A branch column has something to review when it's loaded with > 0 files.
    fn column_has_changes(col: &Column) -> bool {
        matches!(&col.state, ColumnState::Loaded { file_count, .. } if *file_count > 0)
    }

    /// Open/close a column's Review CLI multi-select. On open, sync the pick
    /// toggles to the CLI list (default all-on). Clicking the same column's
    /// Review button again (or a different one) toggles / re-targets the popover.
    fn toggle_review_menu(&mut self, col_idx: usize, cx: &mut Context<Self>) {
        if self.review_menu_open == Some(col_idx) {
            self.review_menu_open = None;
        } else {
            self.review_menu_open = Some(col_idx);
            let n = super::review::ReviewCli::all().len();
            if self.review_picks.len() != n {
                self.review_picks = vec![true; n];
            }
        }
        cx.notify();
    }

    /// Toggle one CLI's inclusion in the next review.
    fn toggle_review_pick(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(p) = self.review_picks.get_mut(i) {
            *p = !*p;
            cx.notify();
        }
    }

    /// Launch the selected CLIs to review column `col_idx`'s branch: one real
    /// terminal per CLI, embedded UNDER the column's diff (in the Diff interface,
    /// not the CLI mode), cwd-pinned to the worktree, with a compact review prompt
    /// pre-filled (the human submits). Human-in-the-loop — no headless session.
    fn launch_review(&mut self, col_idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.review_menu_open = None;
        let clis = super::review::ReviewCli::all();
        let selected: Vec<usize> = (0..clis.len())
            .filter(|i| self.review_picks.get(*i).copied().unwrap_or(true))
            .collect();
        if selected.is_empty() {
            self.set_flash("Select at least one CLI".into(), cx);
            return;
        }
        let Some(col) = self.columns.get(col_idx) else {
            return;
        };
        let cwd = col.path.clone();
        let branch = col.branch.clone();
        let ws_id = col.workspace_id.unwrap_or(0);
        let base = col
            .base_override
            .clone()
            .unwrap_or_else(|| self.base_ref.clone());

        // One terminal per selected CLI; the 2nd+ get the adversarial framing so a
        // multi-CLI panel is a real second opinion, not an echo.
        let mut created: Vec<ReviewTerminal> = Vec::new();
        let mut first_prompt: Option<String> = None;
        let mut focus_target: Option<Entity<crate::terminal::TerminalView>> = None;
        for (rank, &i) in selected.iter().enumerate() {
            let cli = clis[i];
            let prompt = super::review::build_cli_review_prompt(&branch, &base, rank > 0);
            let term = cx.new(|cx| {
                crate::terminal::TerminalView::with_cwd(ws_id, Some(cwd.clone()), None, cx)
            });
            // Launch the CLI in the embedded terminal's shell.
            term.read(cx).send_command(cli.launch_command());
            // Pre-fill the prompt once the CLI has booted (tmux send-keys style):
            // a delayed write with NO Enter — the human reviews + submits. The
            // clipboard fallback (below) covers a missed timing window.
            let prefill = prompt.clone();
            let term_weak = term.downgrade();
            cx.spawn(async move |_, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(REVIEW_PREFILL_DELAY_MS)).await;
                cx.update(|cx| {
                    if let Some(t) = term_weak.upgrade() {
                        t.read(cx).send_text(&prefill);
                    }
                });
            })
            .detach();
            let label = if rank > 0 {
                format!("{} · 2nd opinion", cli.label())
            } else {
                cli.label().to_string()
            };
            if focus_target.is_none() {
                focus_target = Some(term.clone());
            }
            if first_prompt.is_none() {
                first_prompt = Some(prompt);
            }
            created.push(ReviewTerminal {
                label: label.into(),
                terminal: term,
            });
        }

        if let Some(col) = self.columns.get_mut(col_idx) {
            col.review_terminals = created; // replace any prior run (drops old PTYs)
        }
        if let Some(t) = focus_target {
            t.read(cx).focus_handle(cx).focus(window, cx);
        }
        if let Some(p) = first_prompt {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(p));
        }
        cx.notify();
    }

    /// Close one embedded terminal (drops it → PTY shutdown).
    fn close_review_terminal(&mut self, col_idx: usize, term_idx: usize, cx: &mut Context<Self>) {
        let Some(col) = self.columns.get_mut(col_idx) else {
            return;
        };
        if term_idx < col.review_terminals.len() {
            col.review_terminals.remove(term_idx);
            cx.notify();
        }
    }

    /// Terminal button on a column header: open a plain shell terminal in the
    /// branch's worktree, embedded under the diff. Just a terminal — no CLI
    /// launch, no prefill (distinct from Review).
    fn open_terminal_for_column(
        &mut self,
        col_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(col) = self.columns.get(col_idx) else {
            return;
        };
        let cwd = col.path.clone();
        let ws_id = col.workspace_id.unwrap_or(0);
        let term = cx.new(|cx| crate::terminal::TerminalView::with_cwd(ws_id, Some(cwd), None, cx));
        term.read(cx).focus_handle(cx).focus(window, cx);
        if let Some(col) = self.columns.get_mut(col_idx) {
            col.review_terminals.push(ReviewTerminal {
                label: "Terminal".into(),
                terminal: term,
            });
        }
        cx.notify();
    }

    /// Render the embedded review terminals under a column's diff body (one card
    /// per CLI, side by side). `None` when the column has no review running.
    fn render_review_terminals(
        &self,
        col_idx: usize,
        col: &Column,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if col.review_terminals.is_empty() {
            return None;
        }
        let terminals = div().flex_1().min_h_0().flex().flex_row().children(
            col.review_terminals.iter().enumerate().map(|(ti, rt)| {
                let header = div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .px(px(8.))
                    .py(px(3.))
                    .bg(ui.surface)
                    .border_b_1()
                    .border_color(ui.border)
                    .child(
                        gpui::svg()
                            .size(px(11.))
                            .flex_none()
                            .path("icons/terminal.svg")
                            .text_color(ui.accent),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui.text)
                            .child(rt.label.clone()),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "diff-review-term-close-{col_idx}-{ti}"
                            )))
                            .flex_none()
                            .px(px(4.))
                            .text_size(px(12.))
                            .text_color(ui.muted)
                            .cursor_pointer()
                            .hover(|s| {
                                let ui = crate::theme::ui_colors();
                                s.text_color(ui.text)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                                this.close_review_terminal(col_idx, ti, cx);
                            }))
                            .child("×"),
                    );
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(ti > 0, |d| d.border_l_1().border_color(ui.border))
                    .child(header)
                    .child(div().flex_1().min_h_0().child(rt.terminal.clone()))
            }),
        );
        // Drag handle (top edge): drag up/down to resize the review region.
        let divider = div()
            .id(SharedString::from(format!("diff-review-resize-{col_idx}")))
            .flex_none()
            .h(px(6.))
            .cursor(CursorStyle::ResizeUpDown)
            .bg(ui.border)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.accent)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _w, cx| {
                    let start_h = this
                        .columns
                        .get(col_idx)
                        .map(|c| c.review_height)
                        .unwrap_or(REVIEW_DEFAULT_HEIGHT);
                    this.review_resizing = Some((col_idx, f32::from(ev.position.y), start_h));
                    cx.stop_propagation();
                }),
            );
        let region = div()
            .flex_none()
            .h(px(col.review_height))
            .flex()
            .flex_col()
            .child(divider)
            .child(terminals);
        Some(region.into_any_element())
    }

    /// The Review chip's CLI multi-select popover. Lists the CLIs as toggles and
    /// a Review button that opens one terminal pane per checked CLI under the
    /// branch's worktree.
    fn render_review_menu(
        &self,
        col_idx: usize,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let clis = super::review::ReviewCli::all();
        let mut menu = div()
            .id("diff-review-menu")
            .occlude()
            .absolute()
            // Anchored just below this branch's header (header ≈ 26px tall).
            .top(px(30.))
            .right(px(6.))
            .w(px(256.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .rounded(px(8.))
            .shadow_lg()
            .flex()
            .flex_col()
            .p(px(6.))
            .gap(px(2.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.review_menu_open = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .child("Launch a CLI to review this branch"),
            );
        for (i, cli) in clis.iter().enumerate() {
            let checked = self.review_picks.get(i).copied().unwrap_or(true);
            let label = cli.label();
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("diff-review-pick-{i}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle)
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        this.toggle_review_pick(i, cx);
                    }))
                    .child(
                        div()
                            .flex_none()
                            .size(px(14.))
                            .rounded(px(3.))
                            .border_1()
                            .border_color(ui.border)
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(checked, |d| {
                                d.bg(ui.accent.opacity(0.18)).child(
                                    gpui::svg()
                                        .size(px(10.))
                                        .path("icons/check.svg")
                                        .text_color(ui.accent),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(ui.text)
                            .child(label),
                    ),
            );
        }
        menu = menu.child(
            div()
                .id("diff-review-run")
                .mt(px(2.))
                .flex()
                .items_center()
                .justify_center()
                .py(px(5.))
                .rounded(px(5.))
                .bg(ui.accent.opacity(0.15))
                .text_size(px(12.))
                .text_color(ui.accent)
                .cursor_pointer()
                .hover(|s| {
                    let ui = crate::theme::ui_colors();
                    s.bg(ui.accent.opacity(0.25))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.launch_review(col_idx, window, cx);
                }))
                .child("Review"),
        );
        deferred(menu).priority(8).into_any_element()
    }

    /// Toolbar: collapse every file in every visible column, or expand all.
    fn toggle_collapse_all(&mut self, cx: &mut Context<Self>) {
        self.all_collapsed = !self.all_collapsed;
        let collapse = self.all_collapsed;
        for col in &mut self.columns {
            if !col.visible {
                continue;
            }
            col.collapsed.clear();
            if collapse {
                let paths: Vec<String> = match &col.state {
                    ColumnState::Loaded {
                        anchors_unified, ..
                    } => anchors_unified.iter().map(|(p, _)| p.clone()).collect(),
                    _ => Vec::new(),
                };
                col.collapsed.extend(paths);
            }
            col.recompute_display();
        }
        cx.notify();
    }
}

impl DiffView {
    /// US-009 (prd-git-diff-mode-2026-Q3.md): the hunk palette is sourced
    /// entirely from the curated `vc_*` theme slots (US-007) — zero hardcoded
    /// hex. Because `render_column` rebuilds this from `ui_colors()` every
    /// render, a theme switch re-colors the diff live. `element.rs` only
    /// consumes the `RowPalette` it is handed, so the single color source is
    /// here.
    fn palette(ui: crate::theme::UiColors) -> RowPalette {
        RowPalette {
            text: ui.text,
            muted: ui.muted,
            header_bg: ui.surface,
            // Slightly elevated, opaque surface for the pinned sticky header so
            // it floats above the body (`ui.base`) instead of blending into it.
            sticky_header_bg: ui.overlay,
            border: ui.border,
            add_bg: ui.vc_added_background,
            del_bg: ui.vc_deleted_background,
            add_fg: ui.vc_added,
            del_fg: ui.vc_deleted,
            // Gutter numbers for changed lines: the status hue softened toward
            // the gutter's muted baseline so they tint without shouting over the
            // line wash they sit on.
            gutter_add: ui.muted.blend(ui.vc_added.opacity(0.75)),
            gutter_del: ui.muted.blend(ui.vc_deleted.opacity(0.75)),
            mod_fg: ui.vc_modified,
            // Zed paints the gutter hunk strip as `editor_background.blend(version_control_*)`
            // so it reads solid; pre-blend against the diff body surface (`ui.base`,
            // what context lines sit on) so the bar is opaque, not faint at the wash alpha.
            add_bar: ui.base.blend(ui.vc_added),
            del_bar: ui.base.blend(ui.vc_deleted),
            // Neutral alignment-row fill, derived from `muted` so it tracks the
            // theme instead of a hardcoded slate hex.
            phantom_bg: ui.muted.opacity(0.12),
            add_word_bg: ui.vc_word_added,
            del_word_bg: ui.vc_word_deleted,
        }
    }

    /// Cross-column scroll sync, FILE-ANCHORED. Always sources from the explicit
    /// `scroll_driver` (the last column the pointer scrolled), never from a
    /// follower — a short column whose offset got clamped to its own end never
    /// drags the others back, so the sync is drift-free across columns of
    /// differing height. Rather than copy the raw pixel offset (which drifts
    /// mid-file when the same file has different line counts across branches),
    /// it finds the file at the driver's viewport top + the intra-file delta and
    /// re-anchors each follower on THAT file's header, so "same file, two
    /// branches" stays truly lockstep. Falls back to the raw offset for a
    /// follower that doesn't contain the driver's top file.
    fn broadcast_scroll(&self, mode: ViewMode) {
        if !self.sync_scroll {
            return;
        }
        let driver = if self
            .columns
            .get(self.scroll_driver)
            .map(|c| c.visible)
            .unwrap_or(false)
        {
            self.scroll_driver
        } else {
            match self.columns.iter().position(|c| c.visible) {
                Some(i) => i,
                None => return,
            }
        };
        let Some(driver_col) = self.columns.get(driver) else {
            return;
        };
        let driver_y = f32::from(-driver_col.el_scroll.offset().y).max(0.0);
        let (top_file, intra) = self.file_at_offset(driver_col, mode, driver_y);
        for (i, col) in self.columns.iter().enumerate() {
            if i == driver || !col.visible {
                continue;
            }
            let target_y = match &top_file {
                // Align on the same file's header across branches; the intra-file
                // delta keeps the relative position within the file.
                Some(path) => self
                    .file_top_offset(col, mode, path)
                    .map(|fy| fy + intra)
                    .unwrap_or(driver_y),
                None => driver_y,
            };
            let cur = col.el_scroll.offset();
            if f32::from(-cur.y) != target_y {
                col.el_scroll.set_offset(point(cur.x, px(-target_y)));
            }
        }
    }

    /// The file (header anchor path) at scrolled offset `y` in `col`, plus the
    /// intra-file delta (`y` minus that file header's top). Walks the displayed
    /// rows accumulating their variable heights, tracking the most recent file
    /// header, stopping once the accumulated height passes `y`. `(None, y)` when
    /// the column has no file header at/above `y` (empty / pre-first-header).
    fn file_at_offset(&self, col: &Column, mode: ViewMode, y: f32) -> (Option<String>, f32) {
        let mut acc = 0.0f32;
        let mut current: Option<(String, f32)> = None;
        match mode {
            ViewMode::Unified => {
                let anchors = &col.disp_anchors_unified;
                let mut ai = 0usize;
                for (ri, row) in col.disp_unified.iter().enumerate() {
                    while ai < anchors.len() && anchors[ai].1 == ri {
                        current = Some((anchors[ai].0.clone(), acc));
                        ai += 1;
                    }
                    let h = display_row_height(row);
                    if acc + h > y {
                        break;
                    }
                    acc += h;
                }
            }
            ViewMode::Split => {
                let anchors = &col.disp_anchors_split;
                let mut ai = 0usize;
                for (ri, row) in col.disp_split.iter().enumerate() {
                    while ai < anchors.len() && anchors[ai].1 == ri {
                        current = Some((anchors[ai].0.clone(), acc));
                        ai += 1;
                    }
                    let h = split_row_height(row);
                    if acc + h > y {
                        break;
                    }
                    acc += h;
                }
            }
        }
        match current {
            Some((path, top)) => (Some(path), (y - top).max(0.0)),
            None => (None, y),
        }
    }

    /// Cumulative top offset (px) of `path`'s file header in `col`, or `None` if
    /// that column doesn't contain the file. Mirrors `jump_to_file`'s sum.
    fn file_top_offset(&self, col: &Column, mode: ViewMode, path: &str) -> Option<f32> {
        match mode {
            ViewMode::Unified => {
                let idx = col
                    .disp_anchors_unified
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, i)| *i)?;
                Some(
                    col.disp_unified
                        .get(..idx)?
                        .iter()
                        .map(display_row_height)
                        .sum(),
                )
            }
            ViewMode::Split => {
                let idx = col
                    .disp_anchors_split
                    .iter()
                    .find(|(p, _)| p == path)
                    .map(|(_, i)| *i)?;
                Some(
                    col.disp_split
                        .get(..idx)?
                        .iter()
                        .map(split_row_height)
                        .sum(),
                )
            }
        }
    }

    fn render_column(
        &self,
        idx: usize,
        col: &Column,
        mode: ViewMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let palette = Self::palette(ui);

        // Review is offered per branch: live only when this column has changes,
        // highlighted while its own CLI-picker popover is open.
        let col_has_changes = Self::column_has_changes(col);
        let review_open = self.review_menu_open == Some(idx);

        let summary = match &col.state {
            ColumnState::Loading => "loading…".to_string(),
            ColumnState::Failed(_) => "error".to_string(),
            ColumnState::Loaded { file_count, .. } => match file_count {
                0 => "no changes".to_string(),
                1 => "1 file".to_string(),
                n => format!("{n} files"),
            },
        };

        // Selected column drives the sidebar file list + jump-to-file. Only
        // visually distinguished when there is more than one column.
        let selected = self.selected_column == idx && self.visible_count() > 1;
        // Per-column base toggle chip: shows what this column diffs against (the
        // shared base, or `HEAD~1` when overridden) and flips between the two on
        // click — one branch can show just its latest-commit delta while siblings
        // keep the whole-branch-vs-base view.
        let overridden = col.base_override.is_some();
        let eff_base = col
            .base_override
            .clone()
            .unwrap_or_else(|| self.base_ref.clone());
        let has_base = !eff_base.is_empty();
        let base_short: String = if eff_base.chars().count() > 12 {
            let s: String = eff_base.chars().take(11).collect();
            format!("{s}…")
        } else {
            eff_base
        };
        let base_chip = div()
            .id(SharedString::from(format!("diff-col-base-{idx}")))
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.))
            .px(px(5.))
            .py(px(1.))
            .rounded(px(4.))
            .when(overridden, |d| d.bg(ui.accent.opacity(0.18)))
            .cursor_pointer()
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.toggle_column_base(idx, cx);
            }))
            .child(
                gpui::svg()
                    .size(px(10.))
                    .flex_none()
                    .path("icons/git-pull-request.svg")
                    .text_color(if overridden { ui.accent } else { ui.muted }),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(if overridden { ui.accent } else { ui.muted })
                    .child(base_short),
            );
        // Grab handle for drag-to-rearrange (inc 5): the branch name is the drag
        // payload's ghost label. Click still selects (GPUI distinguishes click
        // from drag by a move threshold).
        let branch_drag = SharedString::from(col.branch.clone());
        let header = div()
            .id(SharedString::from(format!("diff-col-head-{idx}")))
            // Positioned ancestor for the Review CLI-picker popover below.
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(4.))
            .bg(if selected { ui.subtle } else { ui.surface })
            .border_b_1()
            .border_color(if selected { ui.accent } else { ui.border })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.select_column(idx, cx);
            }))
            .on_drag(
                DiffColumnDrag { source_idx: idx },
                move |_drag, _offset, _window, cx| {
                    cx.new(|_| TabDragPreview {
                        title: branch_drag.clone(),
                        icon: "icons/git-branch.svg".into(),
                    })
                },
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if selected { ui.accent } else { ui.text })
                    .child(col.branch.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .child(summary),
            )
            .when(has_base, move |d| d.child(base_chip))
            // Review this branch: launch one or more CLIs against its diff. Sits
            // beside the terminal button (prd-ai-in-diff-2026-Q3.md); live only
            // when the column has changes.
            .when(col_has_changes, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("diff-col-review-{idx}")))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(18.))
                        .rounded(px(4.))
                        // Visible neutral wash — ui.subtle (0x2a2a2a) on the dark
                        // header (0x212121) is ~invisible. The open popover keeps
                        // it lit.
                        .when(review_open, |d| d.bg(ui.text.opacity(0.12)))
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.text.opacity(0.12))
                        })
                        .tooltip(|_w, cx| {
                            cx.new(|_| DiffHeaderTooltip {
                                label: "Review".into(),
                            })
                            .into()
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.toggle_review_menu(idx, cx);
                        }))
                        .child(
                            gpui::svg()
                                .size(px(12.))
                                .flex_none()
                                .path("icons/eye.svg")
                                .text_color(if review_open { ui.text } else { ui.muted }),
                        ),
                )
            })
            // Open a plain terminal in this branch's worktree, embedded under the
            // diff (prd-ai-in-diff-2026-Q3.md).
            .child(
                div()
                    .id(SharedString::from(format!("diff-col-term-{idx}")))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(18.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.text.opacity(0.12))
                    })
                    .tooltip(|_w, cx| {
                        cx.new(|_| DiffHeaderTooltip {
                            label: "Open terminal".into(),
                        })
                        .into()
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.open_terminal_for_column(idx, window, cx);
                    }))
                    .child(
                        gpui::svg()
                            .size(px(12.))
                            .flex_none()
                            .path("icons/terminal.svg")
                            .text_color(ui.muted),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("diff-col-hide-{idx}")))
                    .flex_none()
                    .px(px(4.))
                    .text_size(px(12.))
                    .text_color(ui.muted)
                    .cursor_pointer()
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.text_color(ui.text)
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                        // Worktree scope: deselect the branch from the scope (the
                        // host drops it + rebuilds) so it's strictly shown-or-not.
                        // Other scopes keep the in-place hide.
                        if this.close_removes {
                            if let Some(col) = this.columns.get(idx) {
                                cx.emit(DiffViewEvent::CloseColumn {
                                    path: col.path.clone(),
                                });
                            }
                        } else {
                            this.hide_column(idx, cx);
                        }
                    }))
                    .child("×"),
            )
            // Per-branch Review CLI-picker popover, anchored under this header.
            .when(review_open, |d| {
                d.child(self.render_review_menu(idx, ui, cx))
            });

        let body: AnyElement = match &col.state {
            ColumnState::Loading => centered(ui.muted, "Computing diff…".into()),
            ColumnState::Failed(e) => centered(ui.muted, e.clone()),
            ColumnState::Loaded { file_count, .. } if *file_count == 0 => {
                let b = col.base_override.as_deref().unwrap_or(&self.base_ref);
                centered(ui.muted, format!("No changes vs {b}"))
            }
            ColumnState::Loaded { .. } => {
                // Custom direct-paint element hosted in an overflow-scroll div:
                // the element reports full content height; the div clips/scrolls
                // and supplies the viewport clip the element culls against. Renders
                // the collapse-filtered views (`disp_*`). The scroll-wheel listener
                // marks this column the sync driver; the click listener maps the
                // click Y to a row and toggles that file's collapse if it landed
                // on a file header.
                let body = match mode {
                    ViewMode::Split => DiffBody::Split(col.disp_split.clone()),
                    ViewMode::Unified => DiffBody::Unified(col.disp_unified.clone()),
                };
                // Hover-to-ask: the changed line under the cursor (this column)
                // while a review CLI runs, highlighted + cursor-pointer + clickable.
                let hover_row = self.hover_line.filter(|(c, _)| *c == idx).map(|(_, r)| r);
                div()
                    .id(SharedString::from(format!("diff-col-{idx}")))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&col.el_scroll)
                    .on_scroll_wheel(cx.listener(
                        move |this, _: &gpui::ScrollWheelEvent, _w, _cx| {
                            this.scroll_driver = idx;
                        },
                    ))
                    .when(hover_row.is_some(), |d| d.cursor(CursorStyle::PointingHand))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        this.handle_body_click(idx, ev, window, cx);
                    }))
                    // Track the pointer for `Ctrl+Shift+C` (hunk under cursor) AND
                    // for the hover-to-ask highlight (changed line under cursor while
                    // a review CLI runs). Only re-renders on a hover-row transition.
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                        this.last_body_pos = Some((idx, ev.position));
                        let mode = this.effective_mode(window);
                        let new_hover = this
                            .actionable_row_at(idx, ev.position, mode)
                            .map(|r| (idx, r));
                        if this.hover_line != new_hover {
                            this.hover_line = new_hover;
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            let mode = this.effective_mode(window);
                            this.open_body_menu(idx, ev.position, mode, cx);
                        }),
                    )
                    .child(DiffElement::new(body, palette).hover_row(hover_row))
                    .into_any_element()
            }
        };

        div()
            .flex_1()
            // `h_full` + `min_h_0`: pin the column to the (definite) height of the
            // horizontally-scrolling columns row. Without a definite height the
            // body's `uniform_list` is laid out at content height (item_h ×
            // 10001) and renders EVERY row instead of virtualizing — the scroll
            // lag. With it, only the ~viewport rows paint.
            .h_full()
            .min_h_0()
            // Panes shrink to share the split evenly (inc 5); the DiffElement
            // clips long lines per-pane, so a narrow pane shows fewer columns of
            // code rather than overflowing. Borders are drawn by the arrangement
            // walk between siblings, so the column itself draws none.
            .min_w_0()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            // Embedded review CLIs render UNDER the diff body, in the Diff
            // interface (prd-ai-in-diff-2026-Q3.md, terminal-launch revision).
            .children(self.render_review_terminals(idx, col, ui, cx))
    }

    /// Toolbar: base selector + diffstat on the left, collapse / sync / view-mode
    /// on the right. JetBrains-flat — borderless controls that hover-fill, an
    /// active (open / engaged) state as a subtle wash, a thin divider before the
    /// view-mode toggle, and small monochrome icons.
    fn render_toolbar(&self, effective: ViewMode, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let hidden = self.columns.len() - self.visible_count();

        // US-009: diffstat for the column you're actually looking at (the
        // selected one, else first-visible; Zed-style `DiffStat`, curated `vc_*`
        // tokens). The CROSS-branch total lives in the aggregate strip, where it
        // is explicitly labeled "N branches vs <base>" — the toolbar shows only
        // the focused branch on purpose, so a +1000 branch never reads as the
        // +3000 sum of every visible branch. In single-column scopes the two
        // coincide (one column ⇒ its stats == the aggregate).
        let (total_added, total_removed) = self
            .selected_or_first_visible()
            .and_then(|i| self.columns.get(i))
            .map(|c| match &c.state {
                ColumnState::Loaded { files, .. } => files
                    .iter()
                    .fold((0u32, 0u32), |(a, r), f| (a + f.added, r + f.removed)),
                _ => (0, 0),
            })
            .unwrap_or((0, 0));

        // Hunk-nav state for the selected column: (total hunks, current index by
        // scroll position). `None` / total 0 hides the control. Stateless — read
        // from the live scroll offset so it tracks manual scrolling.
        let hunk_nav = self
            .selected_or_first_visible()
            .and_then(|i| self.columns.get(i))
            .map(|col| {
                let tops = Self::hunk_tops(col, effective);
                let cur_y = f32::from(-col.el_scroll.offset().y).max(0.0);
                let current = tops.iter().filter(|&&t| t <= cur_y + 4.0).count();
                (tops.len(), current)
            })
            .filter(|(total, _)| *total > 0);

        // Pill control (icon + label). `active` paints the resting highlight
        // (open popover / toggle on).
        let control = |id: &'static str, active: bool| {
            div()
                .id(id)
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(6.))
                .when(active, |d| d.bg(ui.subtle))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(ui.text)
                .hover(|s| {
                    let ui = crate::theme::ui_colors();
                    s.bg(ui.subtle)
                })
        };
        let icon = |path: &'static str| {
            gpui::svg()
                .size(px(13.))
                .flex_none()
                .path(path)
                .text_color(ui.muted)
        };

        // One segment of the Unified|Split control. Monochrome translucent
        // language (matches the CLI/Diff/Agents toggle) so it adapts to any
        // theme; the active segment is filled. Captures only `ui` (not `cx`) so
        // it can't tangle with the `cx` borrows elsewhere in the chain — the
        // click is attached by the caller for the inactive segment only.
        let seg =
            |id: &'static str, label: &'static str, icon_path: &'static str, is_active: bool| {
                let mut s = div()
                    .id(id)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.))
                    .h(px(20.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .text_size(px(12.));
                if is_active {
                    s = s
                        .bg(ui.text.opacity(0.10))
                        .text_color(ui.text)
                        .font_weight(FontWeight::SEMIBOLD);
                } else {
                    s = s
                        .text_color(ui.text.opacity(0.5))
                        .cursor_pointer()
                        .hover(|st| {
                            let ui = crate::theme::ui_colors();
                            st.text_color(ui.text)
                        });
                }
                s.child(
                    gpui::svg()
                        .size(px(12.))
                        .flex_none()
                        .path(icon_path)
                        .text_color(if is_active {
                            ui.text
                        } else {
                            ui.text.opacity(0.5)
                        }),
                )
                .child(label)
            };

        div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .h(px(34.))
            .flex_none()
            .px(px(8.))
            .bg(ui.surface)
            .border_b_1()
            .border_color(ui.border)
            // --- left: base branch + diffstat ---
            .child(
                control("diff-base-chip", self.base_picker_open)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_base_picker(window, cx);
                    }))
                    .child(icon("icons/git-branch.svg"))
                    .child(
                        div()
                            .text_color(if self.base_ref.is_empty() {
                                ui.muted
                            } else {
                                ui.text
                            })
                            .child(if self.base_ref.is_empty() {
                                "pick a branch".to_string()
                            } else {
                                self.base_ref.clone()
                            }),
                    )
                    .child(icon("icons/chevron-down.svg")),
            )
            .when(self.base_picker_open, |d| {
                d.child(deferred(self.render_base_popover(cx)).with_priority(10))
            })
            .when(total_added > 0 || total_removed > 0, |d| {
                // GitHub-style proportion bar: green:red filled in proportion to
                // added:removed, so the diff's shape reads at a glance before the
                // exact counts. Rounded + clipped; `subtle` shows through only
                // when a side is zero-width.
                let bar_w = 48.0_f32;
                let total = (total_added + total_removed).max(1) as f32;
                let gw = (bar_w * total_added as f32 / total).round();
                let rw = bar_w - gw;
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .ml(px(4.))
                        .text_size(px(11.))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .flex_row()
                                .h(px(6.))
                                .w(px(bar_w))
                                .rounded(px(3.))
                                .overflow_hidden()
                                .bg(ui.subtle)
                                .child(div().h_full().w(px(gw)).bg(ui.vc_added))
                                .child(div().h_full().w(px(rw)).bg(ui.vc_deleted)),
                        )
                        .when(total_added > 0, |d| {
                            d.child(
                                div()
                                    .text_color(ui.vc_added)
                                    .child(format!("+{total_added}")),
                            )
                        })
                        .when(total_removed > 0, |d| {
                            d.child(
                                div()
                                    .text_color(ui.vc_deleted)
                                    .child(format!("-{total_removed}")),
                            )
                        }),
                )
            })
            // --- hunk navigation: prev / counter / next ---
            .when_some(hunk_nav, |d, (total, current)| {
                let shown = current.clamp(1, total);
                let nav_btn = |id: &'static str, icon_path: &'static str| {
                    div()
                        .id(id)
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle)
                        })
                        .child(
                            gpui::svg()
                                .size(px(12.))
                                .flex_none()
                                .path(icon_path)
                                .text_color(ui.muted),
                        )
                };
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(1.))
                        .ml(px(4.))
                        .child(nav_btn("diff-hunk-prev", "icons/chevron_up.svg").on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.goto_hunk(false, window, cx);
                            }),
                        ))
                        .child(
                            div()
                                .flex_none()
                                .px(px(3.))
                                .text_size(px(11.))
                                .text_color(ui.muted)
                                .child(format!("{shown}/{total}")),
                        )
                        .child(
                            nav_btn("diff-hunk-next", "icons/chevron-down.svg").on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.goto_hunk(true, window, cx);
                                }),
                            ),
                        ),
                )
            })
            // --- spacer ---
            .child(div().flex_1())
            // --- right: list actions (AI Review now lives per-branch in the
            // column header, beside the terminal button) ---
            .child(
                control("diff-collapse-all", false)
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.toggle_collapse_all(cx);
                    }))
                    .text_color(ui.muted)
                    .child(icon(if self.all_collapsed {
                        "icons/chevron-down.svg"
                    } else {
                        "icons/chevron_up.svg"
                    }))
                    .child(if self.all_collapsed {
                        "Expand all"
                    } else {
                        "Collapse all"
                    }),
            )
            .when(hidden > 0, |d| {
                d.child(
                    control("diff-show-hidden", false)
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                            this.show_all_columns(cx);
                        }))
                        .text_color(ui.muted)
                        .child(format!("{hidden} hidden")),
                )
            })
            .when(self.visible_count() > 1, |d| {
                d.child(
                    control("diff-sync-toggle", self.sync_scroll)
                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| this.toggle_sync(cx)))
                        .child(icon("icons/link.svg"))
                        .child(if self.sync_scroll {
                            "Linked"
                        } else {
                            "Independent"
                        }),
                )
            })
            // --- right: view-mode segmented control ---
            .child(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h(px(16.))
                    .mx(px(2.))
                    .bg(ui.border),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2.))
                    .p(px(2.))
                    .rounded(px(6.))
                    .bg(ui.text.opacity(0.05))
                    .child(
                        seg(
                            "diff-mode-unified",
                            "Unified",
                            "icons/list.svg",
                            effective == ViewMode::Unified,
                        )
                        .when(effective != ViewMode::Unified, |d| {
                            d.on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.mode = ViewMode::Unified;
                                cx.notify();
                            }))
                        }),
                    )
                    .child(
                        seg(
                            "diff-mode-split",
                            "Split",
                            "icons/split_vertical.svg",
                            effective == ViewMode::Split,
                        )
                        .when(effective != ViewMode::Split, |d| {
                            d.on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                this.mode = ViewMode::Split;
                                cx.notify();
                            }))
                        }),
                    ),
            )
    }

    /// Floating, searchable base-branch popover anchored under the toolbar chip
    /// (US-013). Replaces the old wrapping chip-row: it floats above the diff
    /// body (no reflow), filters live as you type, marks the active base with a
    /// check, and supports keyboard (Esc to close, Enter to pick the top match).
    fn render_base_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();

        let filter = self.base_filter.read(cx).value().to_lowercase();
        let matches: Vec<(usize, &String)> = self
            .branches
            .iter()
            .enumerate()
            .filter(|(_, b)| filter.is_empty() || b.to_lowercase().contains(&filter))
            .collect();

        // Header: a search field. The leading glyph + the real cursor-aware
        // `TextInput` make the popover a command-palette-style picker.
        let search = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(7.))
            .px(px(10.))
            .py(px(7.))
            .border_b_1()
            .border_color(ui.border)
            .child(
                gpui::svg()
                    .size(px(13.))
                    .flex_none()
                    .path("icons/tool_search.svg")
                    .text_color(ui.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(ui.text)
                    .child(self.base_filter.clone()),
            )
            .when(!self.branches.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(ui.muted)
                        .child(format!("{}", matches.len())),
                )
            });

        // Scrollable result list with a bounded height so a 100-branch repo
        // doesn't grow the popover off-screen.
        let mut list = div()
            .id("diff-base-list")
            .flex()
            .flex_col()
            .gap(px(1.))
            .max_h(px(280.))
            .overflow_y_scroll()
            .p(px(4.));

        if self.branches.is_empty() {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(ui.muted)
                    .child("No local branches found"),
            );
        } else if matches.is_empty() {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(ui.muted)
                    .child("No branch matches your filter"),
            );
        } else {
            for (bi, branch) in matches {
                let is_current = *branch == self.base_ref;
                let branch_owned = branch.clone();
                list = list.child(
                    div()
                        .id(SharedString::from(format!("diff-base-opt-{bi}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(5.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(is_current, |d| d.bg(ui.subtle))
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.set_base(branch_owned.clone(), cx);
                            window.focus(&this.focus_handle, cx);
                        }))
                        .child(
                            gpui::svg()
                                .size(px(13.))
                                .flex_none()
                                .path("icons/git-branch.svg")
                                .text_color(if is_current { ui.accent } else { ui.muted }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(12.))
                                .text_color(ui.text)
                                .child(branch.clone()),
                        )
                        .when(is_current, |d| {
                            d.child(
                                gpui::svg()
                                    .size(px(13.))
                                    .flex_none()
                                    .path("icons/check.svg")
                                    .text_color(ui.accent),
                            )
                        }),
                );
            }
        }

        div()
            .id("diff-base-popover")
            .occlude()
            .absolute()
            .left(px(8.))
            .top(px(31.))
            .w(px(288.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                this.close_base_picker(window, cx);
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => {
                        this.close_base_picker(window, cx);
                        cx.stop_propagation();
                    }
                    "enter" => {
                        let raw = this.base_filter.read(cx).value().to_string();
                        let filter = raw.to_lowercase();
                        if let Some(branch) = this
                            .branches
                            .iter()
                            .find(|b| filter.is_empty() || b.to_lowercase().contains(&filter))
                            .cloned()
                        {
                            this.set_base(branch, cx);
                            window.focus(&this.focus_handle, cx);
                        } else if !raw.trim().is_empty() {
                            // No listed branch/tag matches — try the typed text as
                            // an arbitrary ref / SHA (validated off-thread).
                            this.resolve_and_set_base(raw, cx);
                            window.focus(&this.focus_handle, cx);
                        }
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(search)
            .child(list)
            .into_any_element()
    }
}

/// Whether a watcher event should trigger a re-diff. Filters out build and VCS
/// churn — `target/`, the git object store / logs, `node_modules/`, and lock
/// files — so an active build doesn't cause a refresh storm (US-015/US-016).
/// Genuine working-tree edits, `HEAD`/`index` moves, and ref updates pass.
fn watch_event_relevant(res: &notify::Result<Event>) -> bool {
    let Ok(event) = res else {
        return false;
    };
    // Ignore reads / atime / permission churn. Computing the diff `fs::read`s
    // every changed file, which fires Access (and atime Metadata) events — those
    // would self-trigger a reload loop (throttled only by the cooldown). Only
    // content / structural changes (Create / Remove / Modify(Data|Name)) refresh.
    match event.kind {
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_)) => return false,
        _ => {}
    }
    event.paths.iter().any(|p| {
        let s = p.to_string_lossy();
        // Skipped files are never diffed (git.rs SKIP_FILENAMES / MAX_FILE_BYTES),
        // so a `cargo add` rewriting Cargo.lock or a `git fetch` touching
        // FETCH_HEAD must not trigger a (no-op) re-diff storm.
        !(s.contains("/target/")
            || s.contains("/.git/objects")
            || s.contains("/.git/logs")
            || s.contains("/.git/index.lock")
            || s.contains("/node_modules/")
            || s.ends_with("/Cargo.lock")
            || s.ends_with("/package-lock.json")
            || s.ends_with("/bun.lockb")
            || s.ends_with("/yarn.lock")
            || s.ends_with("/pnpm-lock.yaml")
            || s.ends_with("/.git/FETCH_HEAD")
            || s.ends_with("/.git/ORIG_HEAD")
            || s.ends_with("/.git/COMMIT_EDITMSG")
            || s.ends_with("/.git/MERGE_HEAD"))
    })
}

/// Top-level directory names never watched: build artifacts and VCS metadata
/// that churn constantly (especially `target/`, tens of thousands of files
/// rewritten by every `cargo`/rust-analyzer build). Watching these recursively
/// floods the event loop and is pure noise — none of it changes the diff.
const WATCH_IGNORE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".jj",
    ".hg",
    ".svn",
    "dist",
    "build",
    ".next",
    ".cache",
    ".venv",
    "venv",
    "vendor",
];

/// Build a filesystem watcher over the working trees PLUS a *narrow* slice of
/// the shared `.git` (refs only). Forwards raw events to `tx`. The registration
/// walk runs on the calling thread — invoke via `smol::unblock`, never on the
/// GPUI thread.
///
/// The earlier infinite re-diff loop came from watching `.git/objects`,
/// `.git/logs`, and `.git/index` — the diff's own ~75 git subprocesses per cycle
/// (`merge-base`/`diff`/`show`) churn the object store and the index stat-cache,
/// so watching those self-triggers. This watches only `refs/heads` +
/// `packed-refs` + `HEAD` of the common git dir, which a read-only diff never
/// writes — so it catches the one signal the working-tree watch misses: the
/// **base branch advancing** (e.g. `develop` gets new commits → the merge-base
/// moves → the diff is stale). `watch_event_relevant` still drops
/// `objects`/`logs`/`index.lock` as belt-and-suspenders. Working-tree edits
/// (and the file churn of a branch switch / rebase / reset) come from the source
/// subtrees, excluding [`WATCH_IGNORE_DIRS`].
fn build_watcher(
    tx: mpsc::UnboundedSender<notify::Result<Event>>,
    worktrees: Vec<PathBuf>,
    repo_root: PathBuf,
) -> Option<RecommendedWatcher> {
    let mut watcher = match RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            let _ = tx.unbounded_send(res);
        },
        Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("diff watcher: failed to create: {e}");
            return None;
        }
    };

    // Collect the watch set first (filesystem reads), then register + count.
    let mut targets: Vec<(PathBuf, RecursiveMode)> = Vec::new();
    // Working tree only: top-level files, then each source subdir recursively,
    // skipping build/VCS dirs so neither `target/` nor `.git/` is registered.
    for wt in &worktrees {
        targets.push((wt.clone(), RecursiveMode::NonRecursive));
        let Ok(entries) = std::fs::read_dir(wt) else {
            continue;
        };
        for entry in entries.flatten() {
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_dir {
                continue;
            }
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !WATCH_IGNORE_DIRS.contains(&name) {
                targets.push((path, RecursiveMode::Recursive));
            }
        }
    }

    // Narrow `.git` watch on the shared (common) git dir: refs only, never the
    // object store / logs / index that the diff itself churns. Catches the base
    // branch advancing + commits/branch moves on any worktree (refs are shared).
    // `refs/heads` is recursive so slashed branch names (`feature/x`) are seen.
    let git_common = repo_root.join(".git");
    if git_common.is_dir() {
        targets.push((
            git_common.join("refs").join("heads"),
            RecursiveMode::Recursive,
        ));
        targets.push((git_common.join("packed-refs"), RecursiveMode::NonRecursive));
        targets.push((git_common.join("HEAD"), RecursiveMode::NonRecursive));
    }

    let mut ok = 0usize;
    for (p, mode) in &targets {
        match watcher.watch(p, *mode) {
            Ok(()) => ok += 1,
            // A missing dir (e.g. no `refs/heads` yet) is benign — debug.
            Err(e) => log::debug!("diff watcher: skip {}: {e}", p.display()),
        }
    }
    log::debug!(
        "diff: watcher registered {ok}/{} paths across {} worktrees",
        targets.len(),
        worktrees.len()
    );

    Some(watcher)
}

/// Build the collapse-filtered unified rows + their header anchors: each
/// collapsed file keeps only its header (prefixed `▸`); expanded files pass
/// through unchanged. Anchors are re-indexed to the filtered row positions so
/// body clicks + `jump_to_file` target what is actually shown. Panic-safe
/// (bounds-checked) per the no-panic lint.
fn apply_collapse_unified(
    rows: &[DisplayRow],
    anchors: &[(String, usize)],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<DisplayRow>, Vec<(String, usize)>) {
    let mut out: Vec<DisplayRow> = Vec::with_capacity(rows.len());
    let mut out_anchors: Vec<(String, usize)> = Vec::with_capacity(anchors.len());
    for (k, (path, start)) in anchors.iter().enumerate() {
        let Some(header) = rows.get(*start) else {
            continue;
        };
        let end = anchors
            .get(k + 1)
            .map(|(_, s)| *s)
            .unwrap_or(rows.len())
            .min(rows.len());
        out_anchors.push((path.clone(), out.len()));
        if collapsed.contains(path) {
            // Header only — the element paints the ▸/▾ chevron by peeking the
            // next row, so the text is left untouched.
            out.push(header.clone());
        } else if let Some(seg) = rows.get(*start..end) {
            out.extend_from_slice(seg);
        }
    }
    (out, out_anchors)
}

/// Side-by-side analog of [`apply_collapse_unified`].
fn apply_collapse_split(
    rows: &[SplitRow],
    anchors: &[(String, usize)],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<SplitRow>, Vec<(String, usize)>) {
    let mut out: Vec<SplitRow> = Vec::with_capacity(rows.len());
    let mut out_anchors: Vec<(String, usize)> = Vec::with_capacity(anchors.len());
    for (k, (path, start)) in anchors.iter().enumerate() {
        let end = anchors
            .get(k + 1)
            .map(|(_, s)| *s)
            .unwrap_or(rows.len())
            .min(rows.len());
        out_anchors.push((path.clone(), out.len()));
        if collapsed.contains(path) {
            match rows.get(*start) {
                Some(row @ SplitRow::Header(_)) => out.push(row.clone()),
                _ => {
                    out_anchors.pop();
                    continue;
                }
            }
        } else if let Some(seg) = rows.get(*start..end) {
            out.extend_from_slice(seg);
        }
    }
    (out, out_anchors)
}

/// Normalize a worktree path for dedup (canonicalize, lowercase on
/// case-insensitive filesystems) so the same worktree seeded from an open
/// workspace and discovered via `git worktree list` collapses to one column.
fn norm_key(p: &std::path::Path) -> String {
    let resolved = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = resolved.to_string_lossy().into_owned();
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        s.to_lowercase()
    } else {
        s
    }
}

/// Centered single-message body used for loading / error / empty states.
fn centered(color: Hsla, message: String) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(color)
        .child(message)
        .into_any_element()
}

// ── Inc 5: splittable diff-pane arrangement + drag-and-drop ──────────────────
impl DiffView {
    /// Render the arrangement tree: leaves become branch panes, splits become
    /// nested flex rows/columns with a hairline between siblings. Recursive;
    /// every node fills its slot so equal splits divide the space.
    fn render_arrange(&self, node: &Arrange, mode: ViewMode, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Arrange::Leaf(i) => self.render_pane(*i, mode, cx),
            Arrange::Split { axis, children } => {
                let ui = crate::theme::ui_colors();
                let row = *axis == Axis::Row;
                let mut container = div().size_full().flex().min_h_0().min_w_0();
                container = if row {
                    container.flex_row()
                } else {
                    container.flex_col()
                };
                for (k, child) in children.iter().enumerate() {
                    let mut cell = div().flex_1().min_h_0().min_w_0().flex();
                    if k > 0 {
                        cell = if row {
                            cell.border_l_1().border_color(ui.border)
                        } else {
                            cell.border_t_1().border_color(ui.border)
                        };
                    }
                    container = container.child(cell.child(self.render_arrange(child, mode, cx)));
                }
                container.into_any_element()
            }
        }
    }

    /// One branch pane (a leaf): the column's header + body, plus the
    /// drop-to-split machinery — a per-pane drag group, an edge probe on the
    /// content, and a relative-sized preview overlay shown while a sibling pane
    /// is dragged over it.
    fn render_pane(&self, idx: usize, mode: ViewMode, cx: &mut Context<Self>) -> AnyElement {
        let Some(col) = self.columns.get(idx) else {
            return div().into_any_element();
        };
        let ui = crate::theme::ui_colors();
        let group_name = SharedString::from(format!("{}-pane-{idx}", self.element_id));

        // Which half to highlight: the resolved edge when THIS pane is the drag
        // target (else full / nothing — the overlay is invisible unless a drag
        // is over its group, so the fallback is never seen).
        let region = self
            .drag_target
            .and_then(|(t, e)| if t == idx { e } else { None });
        let mut overlay = div().absolute().size_full().flex();
        let (bw, bh) = match region {
            None => (relative(1.), relative(1.)),
            Some(DropEdge::Left) => {
                overlay = overlay.flex_row().justify_start();
                (relative(0.5), relative(1.))
            }
            Some(DropEdge::Right) => {
                overlay = overlay.flex_row().justify_end();
                (relative(0.5), relative(1.))
            }
            Some(DropEdge::Up) => {
                overlay = overlay.flex_col().justify_start();
                (relative(1.), relative(0.5))
            }
            Some(DropEdge::Down) => {
                overlay = overlay.flex_col().justify_end();
                (relative(1.), relative(0.5))
            }
        };
        let highlight = div()
            .w(bw)
            .h(bh)
            .bg(ui.accent.opacity(0.22))
            .border_2()
            .border_color(gpui::rgba(0x0ea5e9bf))
            .rounded(px(6.));
        let overlay = overlay
            .invisible()
            .group_drag_over::<DiffColumnDrag>(group_name.clone(), |s| s.visible())
            .on_drop(cx.listener(move |this, drag: &DiffColumnDrag, _w, cx| {
                this.arrange_drop(drag.source_idx, idx, cx);
            }))
            .child(highlight);

        div()
            .id(SharedString::from(format!(
                "{}-panec-{idx}",
                self.element_id
            )))
            .group(group_name)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_drag_move::<DiffColumnDrag>(cx.listener(
                move |this, e: &DragMoveEvent<DiffColumnDrag>, _w, cx| {
                    this.apply_drag_edge(idx, e.bounds, e.event.position, cx);
                },
            ))
            .child(self.render_column(idx, col, mode, cx))
            .child(overlay)
            .into_any_element()
    }

    /// Edge probe during a pane drag: map the cursor (relative to the hovered
    /// pane's content bounds) to a [`DropEdge`] and stash it as the drag target.
    /// Only notifies on change, so it doesn't repaint every pointer tick.
    fn apply_drag_edge(
        &mut self,
        idx: usize,
        bounds: Bounds<Pixels>,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let w = f32::from(bounds.size.width);
        let h = f32::from(bounds.size.height);
        let x = f32::from(pos.x - bounds.origin.x);
        let y = f32::from(pos.y - bounds.origin.y);
        let edge = compute_drop_edge(w, h, x, y, SPLIT_EDGE_BAND);
        let next = Some((idx, edge));
        if self.drag_target != next {
            self.drag_target = next;
            cx.notify();
        }
    }

    /// Commit a pane drop: restructure the arrangement so `source` splits
    /// `target` toward the previewed edge (center → moved beside it). A
    /// drop onto itself is a no-op. The moved pane becomes selected.
    fn arrange_drop(&mut self, source: usize, target: usize, cx: &mut Context<Self>) {
        let edge = self.drag_target.take().and_then(|(_, e)| e);
        if source == target {
            cx.notify();
            return;
        }
        let (axis, before) = match edge {
            Some(DropEdge::Left) => (Axis::Row, true),
            Some(DropEdge::Right) => (Axis::Row, false),
            Some(DropEdge::Up) => (Axis::Col, true),
            Some(DropEdge::Down) => (Axis::Col, false),
            None => (Axis::Row, true), // center: move beside the target
        };
        self.arrange.remove(source);
        self.arrange.split(target, axis, source, before);
        self.selected_column = source;
        cx.notify();
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DiffViewEvent> for DiffView {}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let root = div()
            .id(self.element_id.clone())
            .key_context("DiffView")
            .track_focus(&self.focus_handle)
            // US-003: Ctrl+Shift+C copies the hunk under the cursor (scoped to the
            // DiffView context so it does not collide with the markdown / terminal
            // copy bindings).
            .on_action(cx.listener(|this, _: &crate::CopyDiffHunk, window, cx| {
                this.copy_hovered_hunk(window, cx);
            }))
            // Review-region resize: while a divider drag is active, the pointer is
            // tracked at the root so the drag survives leaving the 6px handle.
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _w, cx| {
                if let Some((col_idx, start_y, start_h)) = this.review_resizing
                    && let Some(col) = this.columns.get_mut(col_idx)
                {
                    let dy = start_y - f32::from(ev.position.y); // drag up = taller
                    col.review_height = (start_h + dy).clamp(REVIEW_MIN_HEIGHT, REVIEW_MAX_HEIGHT);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _w, cx| {
                    if this.review_resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(ui.base)
            .text_color(ui.text);

        if self.columns.is_empty() {
            return root.child(centered(ui.muted, "No sibling worktrees to diff".into()));
        }

        let mode = self.effective_mode(window);
        // Cross-column scroll sync: re-anchor the driver column's top file across
        // the others before building them, so all render at the synced offset
        // this frame (each clamps to its own content height).
        self.broadcast_scroll(mode);
        // Inc 5: reconcile the arrangement tree with the live (visible) columns,
        // then render it — panes split beside/under one another, rearrangeable
        // by drag. The `Vec<Column>` and its index-based logic are untouched.
        let visible: Vec<bool> = self.columns.iter().map(|c| c.visible).collect();
        self.arrange.reconcile(&visible);
        let arrange = self.arrange.clone();
        let body = self.render_arrange(&arrange, mode, cx);

        let root = root.child(self.render_toolbar(mode, cx));
        let mut root = root.child(div().flex_1().min_h_0().flex().child(body));
        // US-003 overlays: the right-click copy menu and the transient flash.
        if let Some(menu) = &self.body_menu {
            root = root.child(self.render_body_menu(menu, ui, cx));
        }
        if let Some(flash) = &self.flash {
            root = root.child(self.render_flash(flash.clone(), ui));
        }
        root
    }
}
