//! Codex-style git diff side panel for the Agents view.
//!
//! A right-docked panel (toggled by the `layout-sidebar-right` button in the
//! environment toolbar) that shows what the agent changed in the current
//! thread's working directory. It renders the working-tree diff against `HEAD`
//! (staged + unstaged tracked changes) plus untracked files, parsed from
//! `git`'s own unified-diff output and painted as Codex-like hunks with line
//! numbers and red/green washes.
//!
//! Self-contained on purpose: the full-screen [`crate::diff`] mode resolves a
//! base branch and diffs `merge-base(HEAD, base)..working-tree` across N
//! worktree columns, which is the wrong semantic (and far too heavy) for a
//! compact "what did the agent just touch" dock. Here we only shell out to
//! `git diff` and parse the patch, so the panel stays a leaf feature with a
//! tiny blast radius. All git calls run off the GPUI main thread via
//! `smol::unblock`, bounded by [`GIT_DEADLINE`] / [`GIT_OUTPUT_CAP`].
//!
//! Rendering is virtualized via [`gpui::list`]: files + hunks + lines are
//! flattened (respecting the per-file collapse set) into one row list with
//! per-kind heights (tall file headers over compact code lines, the way Codex
//! reads as an airy file list), so only the ~visible rows build elements each
//! frame. This keeps the panel smooth when the surrounding terminal repaints
//! constantly, even on a multi-thousand-line diff. The `list` scroll/measure
//! state lives on [`crate::AgentsViewState`] and is rebuilt only when the
//! flattened row set changes. Line text and line numbers are precomputed into
//! `SharedString`s at parse time so the per-frame row build is allocation-free.

use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Entity, FontWeight, Hsla, InteractiveElement,
    IntoElement, ListAlignment, ListState, MouseButton, ParentElement, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Window, div, list, px, svg,
};

use crate::PaneFlowApp;
use crate::settings::components::with_alpha;

/// Fixed width of the docked panel. Wide enough to read code without constant
/// wrapping, narrow enough to leave the terminal column usable beside it.
pub(crate) const AGENTS_DIFF_PANEL_WIDTH: f32 = 540.0;

/// Per-kind row heights for the variable-height `list`. Tall file headers sit
/// over compact hunk headers and code lines; `list` measures each row, so the
/// two tiers can coexist (unlike `uniform_list`'s single shared height).
const FILE_ROW_HEIGHT: f32 = 44.0;
const HUNK_ROW_HEIGHT: f32 = 26.0;
const LINE_ROW_HEIGHT: f32 = 22.0;
const NOTE_ROW_HEIGHT: f32 = 26.0;

/// Wall-clock deadline for every git call behind the panel (mirrors the diff
/// viewer's own bound). A healthy repo answers in well under a second; this
/// keeps a dead mount or a hung credential helper from pinning a blocking task.
const GIT_DEADLINE: Duration = Duration::from_secs(30);

/// stdout cap for the panel's git calls. Comfortably above any real `git diff`
/// while bounding a runaway / hijacked git that streams unbounded output.
const GIT_OUTPUT_CAP: u64 = 16 * 1024 * 1024;

/// Per-untracked-file read cap. Untracked files are synthesised into all-added
/// hunks by reading them directly (no extra subprocess), so bound each read.
const MAX_UNTRACKED_BYTES: u64 = 256 * 1024;

/// Stop enumerating untracked files past this many, so a fresh repo with a huge
/// `node_modules`-style tree (not yet ignored) can't blow up the panel.
const MAX_UNTRACKED_FILES: usize = 400;

/// Cap on diff rows kept per file. Beyond this the file shows a "N more lines"
/// note instead of carrying tens of thousands of rows for one pathological file.
const MAX_LINES_PER_FILE: usize = 4_000;

const CHANGE_BAR_WIDTH: f32 = 3.0;

#[derive(Clone, Copy)]
struct AgentsDiffPalette {
    added: Hsla,
    deleted: Hsla,
    added_background: Hsla,
    deleted_background: Hsla,
    added_gutter_background: Hsla,
    deleted_gutter_background: Hsla,
}

fn agents_diff_palette(ui: crate::theme::UiColors) -> AgentsDiffPalette {
    if ui.base.l > 0.5 {
        return AgentsDiffPalette {
            added: ui.vc_added,
            deleted: ui.vc_deleted,
            added_background: ui.vc_added_background,
            deleted_background: ui.vc_deleted_background,
            added_gutter_background: ui.vc_added_background,
            deleted_gutter_background: ui.vc_deleted_background,
        };
    }

    AgentsDiffPalette {
        // Sampled from Codex App's dark diff panel.
        added: color_from_hex(0x40c977),
        deleted: color_from_hex(0xfa423e),
        added_background: color_from_hex(0x1f3124),
        deleted_background: color_from_hex(0x3b1f1a),
        added_gutter_background: color_from_hex(0x1c291f),
        deleted_gutter_background: color_from_hex(0x311c18),
    }
}

fn color_from_hex(hex: u32) -> Hsla {
    let r = ((hex >> 16) & 0xff) as f32 / 255.0;
    let g = ((hex >> 8) & 0xff) as f32 / 255.0;
    let b = (hex & 0xff) as f32 / 255.0;
    Hsla::from(Rgba { r, g, b, a: 1.0 })
}

/// How a file changed, mapped to the file-icon tint via [`status_color`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineKind {
    Context,
    Added,
    Removed,
}

/// One physical diff line, render-ready. `old_no` / `new_no` are the 1-based
/// line numbers on each side (empty where the line doesn't exist there) and
/// `text` is already tab-expanded, all as `SharedString` so the per-frame row
/// build is a handful of cheap `Arc` clones rather than string allocations.
#[derive(Clone)]
struct DiffLine {
    kind: LineKind,
    old_no: SharedString,
    new_no: SharedString,
    text: SharedString,
}

/// A contiguous run of lines under one `@@ -a,b +c,d @@` header.
#[derive(Clone)]
struct DiffHunk {
    header: SharedString,
    lines: Vec<DiffLine>,
}

/// One side of a split-view row: a gutter line number, the line text, and the
/// change kind that drives its wash. `None` in a [`SplitRow`] means that side
/// has no line here (the change block was longer on the other side).
#[derive(Clone)]
struct SplitCell {
    no: SharedString,
    text: SharedString,
    kind: LineKind,
}

/// One row of the side-by-side view: old text on the left, new on the right.
/// Context rows fill both sides; a pure add or delete fills only one.
#[derive(Clone)]
struct SplitRow {
    left: Option<SplitCell>,
    right: Option<SplitCell>,
}

/// One file's complete diff payload.
#[derive(Clone)]
pub(crate) struct AgentsDiffFile {
    path: String,
    old_path: Option<String>,
    status: DiffStatus,
    hunks: Vec<DiffHunk>,
    added: u32,
    removed: u32,
    is_binary: bool,
}

/// Render-ready snapshot of the panel's data. Cheap to clone every frame: the
/// file vector is shared behind an `Rc` (single-threaded GPUI state).
#[derive(Clone)]
pub(crate) struct AgentsDiffData {
    /// The working directory this diff was computed for. Used to ignore a
    /// stale async result after the user switches threads or closes the panel.
    pub(crate) cwd: String,
    loading: bool,
    error: Option<String>,
    files: Rc<Vec<AgentsDiffFile>>,
    added: u32,
    removed: u32,
}

impl AgentsDiffData {
    fn loading(cwd: String) -> Self {
        Self {
            cwd,
            loading: true,
            error: None,
            files: Rc::new(Vec::new()),
            added: 0,
            removed: 0,
        }
    }

    fn loaded(cwd: String, files: Vec<AgentsDiffFile>, added: u32, removed: u32) -> Self {
        Self {
            cwd,
            loading: false,
            error: None,
            files: Rc::new(files),
            added,
            removed,
        }
    }

    fn message(cwd: String, error: String) -> Self {
        Self {
            cwd,
            loading: false,
            error: Some(error),
            files: Rc::new(Vec::new()),
            added: 0,
            removed: 0,
        }
    }

    /// The file paths in this snapshot, used to drive "collapse all".
    fn paths(&self) -> Vec<String> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }
}

impl PaneFlowApp {
    /// Toggle the Codex-style diff dock. Opening (re)computes the diff for the
    /// current thread's cwd off-thread; closing just hides it (the cached data
    /// is dropped on the next open so it never goes stale silently).
    pub(crate) fn toggle_agents_diff_panel(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agents_view.agents_diff_open {
            self.agents_view.agents_diff_open = false;
            cx.notify();
            return;
        }
        self.agents_view.agents_diff_open = true;
        let cwd = self
            .current_thread_view_target()
            .and_then(|target| self.thread_for_target(target))
            .map(|thread| thread.cwd.clone())
            .unwrap_or_default();
        self.refresh_agents_diff(cwd, cx);
    }

    /// Recompute the diff for `cwd`, parking a loading state first. Shared by the
    /// open path and the panel's refresh button. The async result is dropped if
    /// the panel has since rebound to a different cwd (thread switch / close).
    pub(crate) fn refresh_agents_diff(&mut self, cwd: String, cx: &mut Context<Self>) {
        let cwd = cwd.trim().to_string();
        if cwd.is_empty() {
            self.agents_view.agents_diff = Some(AgentsDiffData::message(
                cwd,
                "No folder is linked to this thread.".to_string(),
            ));
            self.bump_agents_diff_rev();
            cx.notify();
            return;
        }
        self.agents_view.agents_diff = Some(AgentsDiffData::loading(cwd.clone()));
        self.bump_agents_diff_rev();
        cx.notify();

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let result = smol::unblock({
                    let cwd = cwd.clone();
                    move || compute_agents_diff(&cwd)
                })
                .await;
                let _ = cx.update(|cx| {
                    this.update(cx, |app, cx| {
                        // Apply only if the panel is still bound to this cwd.
                        let still_current = app
                            .agents_view
                            .agents_diff
                            .as_ref()
                            .is_some_and(|data| data.cwd == cwd);
                        if !still_current {
                            return;
                        }
                        app.agents_view.agents_diff = Some(match result {
                            Ok((files, added, removed)) => {
                                AgentsDiffData::loaded(cwd.clone(), files, added, removed)
                            }
                            Err(err) => AgentsDiffData::message(cwd.clone(), err),
                        });
                        app.bump_agents_diff_rev();
                        cx.notify();
                    })
                });
            },
        )
        .detach();
    }

    /// Fold / unfold a single file in the diff dock (click on its header row).
    pub(crate) fn toggle_diff_file_collapsed(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.agents_view.agents_diff_collapsed.remove(&path) {
            self.agents_view.agents_diff_collapsed.insert(path);
        }
        self.bump_agents_diff_rev();
        cx.notify();
    }

    /// "Collapse all" / "expand all" for the diff dock. `collapse == true` folds
    /// every file in `paths`; `false` clears the whole collapse set.
    pub(crate) fn set_all_diff_collapsed(
        &mut self,
        paths: &[String],
        collapse: bool,
        cx: &mut Context<Self>,
    ) {
        if collapse {
            self.agents_view
                .agents_diff_collapsed
                .extend(paths.iter().cloned());
        } else {
            self.agents_view.agents_diff_collapsed.clear();
        }
        self.bump_agents_diff_rev();
        cx.notify();
    }

    /// Bump the diff dock's row-set revision so the next render rebuilds the
    /// variable-height [`ListState`]. Cheap: only the render path compares it.
    fn bump_agents_diff_rev(&mut self) {
        self.agents_view.agents_diff_rev = self.agents_view.agents_diff_rev.wrapping_add(1);
    }

    /// Switch the diff dock between unified and split views. No-op when already in
    /// the requested mode; otherwise bumps the revision so the row set (and its
    /// `ListState`) is rebuilt for the new layout.
    pub(crate) fn set_agents_diff_split(&mut self, split: bool, cx: &mut Context<Self>) {
        if self.agents_view.agents_diff_split == split {
            return;
        }
        self.agents_view.agents_diff_split = split;
        self.bump_agents_diff_rev();
        cx.notify();
    }

    /// The docked diff panel: a header over the body. Reads the live snapshot
    /// from state (cloned cheaply) so the caller keeps its `self` borrow short.
    pub(crate) fn render_agents_diff_panel(
        &mut self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let data = self.agents_view.agents_diff.clone();
        let cwd = data.as_ref().map(|d| d.cwd.clone()).unwrap_or_default();
        let folder = Path::new(&cwd)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let split = self.agents_view.agents_diff_split;
        let header = render_diff_panel_header(&data, &folder, cwd, split, ui, cx);
        let body = self.render_agents_diff_body(&data, ui, cx);

        div()
            .w(px(AGENTS_DIFF_PANEL_WIDTH))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(ui.base)
            .border_l_1()
            .border_color(ui.border)
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The diff body: a thin files toolbar over the virtualized `list`. Empty,
    /// loading and error states render a centered placeholder instead. Owns the
    /// [`ListState`] lifecycle: it rebuilds only when `agents_diff_rev` moves, so
    /// scroll and measurements survive ordinary repaints.
    fn render_agents_diff_body(
        &mut self,
        data: &Option<AgentsDiffData>,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(data) = data else {
            return diff_panel_centered(
                "icons/file-text.svg",
                "Open the panel to see changes.",
                ui,
            );
        };
        if data.loading {
            return diff_panel_centered("icons/loader-circle.svg", "Loading changes…", ui);
        }
        if let Some(error) = &data.error {
            return diff_panel_centered("icons/triangle-alert.svg", error, ui);
        }
        if data.files.is_empty() {
            return diff_panel_centered("icons/check.svg", "No uncommitted changes.", ui);
        }

        let entity = cx.entity();
        let collapsed = self.agents_view.agents_diff_collapsed.clone();
        let split = self.agents_view.agents_diff_split;
        let toolbar = render_diff_files_toolbar(data, &collapsed, ui, &entity);

        let files = data.files.clone();
        let rows = Rc::new(flatten_rows(&files, &collapsed, split));

        // `uniform_list` can't host two row heights, so the panel uses `list`.
        // Its scroll/measure state persists on the view and is reset only when
        // the flattened row set actually changes (refresh / collapse toggle).
        let rev = self.agents_view.agents_diff_rev;
        if self.agents_view.agents_diff_list.is_none()
            || self.agents_view.agents_diff_list_rev != rev
        {
            let count = rows.len();
            if let Some(state) = self.agents_view.agents_diff_list.as_ref() {
                state.reset(count);
            } else {
                self.agents_view.agents_diff_list =
                    Some(ListState::new(count, ListAlignment::Top, px(400.)));
            }
            self.agents_view.agents_diff_list_rev = rev;
        }
        let Some(state) = self.agents_view.agents_diff_list.clone() else {
            return diff_panel_centered("icons/triangle-alert.svg", "Diff unavailable.", ui);
        };

        let mono: SharedString = crate::terminal::element::resolve_font_family(None).into();
        let list_rows = rows.clone();
        let list_files = files.clone();
        let list_entity = entity.clone();
        let body_list = list(state, move |i, _window, _cx| {
            render_flat_row(&list_rows[i], &list_files, &mono, ui, &list_entity)
        })
        .flex_1()
        .min_h(px(0.))
        .w_full();

        div()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(body_list)
            .into_any_element()
    }
}

/// Toolbar button (sibling to the environment-panel toggle) that opens the diff
/// dock. Visually identical to [`super::agents_view_actions`]'s list toggle: a
/// bare glyph at rest, a whisper fill on hover or while the dock is open.
pub(crate) fn render_agents_diff_toggle_button(
    open: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let fill = with_alpha(ui.text, if open { 0.08 } else { 0.0 });
    let hover = with_alpha(ui.text, 0.08);
    div()
        .id("agents-env-toolbar-diff")
        .flex_none()
        .h(px(28.))
        .w(px(30.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .cursor(CursorStyle::PointingHand)
        .bg(fill)
        .hover(move |d| d.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            this.toggle_agents_diff_panel(event, window, cx);
        }))
        .child(
            svg()
                .size(px(16.))
                .flex_none()
                .path("icons/layout-sidebar-right.svg")
                .text_color(with_alpha(ui.text, 0.7)),
        )
        .into_any_element()
}

fn render_diff_panel_header(
    data: &Option<AgentsDiffData>,
    folder: &str,
    cwd: String,
    split: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let loaded = data
        .as_ref()
        .is_some_and(|d| !d.loading && d.error.is_none());
    let (added, removed) = data
        .as_ref()
        .map(|d| (d.added, d.removed))
        .unwrap_or((0, 0));
    let diff = agents_diff_palette(ui);

    let mut title_row = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ui.text)
                .child("Changes"),
        );
    if !folder.is_empty() {
        title_row = title_row.child(
            div()
                .min_w_0()
                .overflow_x_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(ui.muted)
                .child(SharedString::from(folder.to_string())),
        );
    }

    let mut right = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.));
    if loaded {
        right = right.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .mr(px(4.))
                .text_size(px(12.))
                .child(div().text_color(diff.added).child(format!("+{added}")))
                .child(div().text_color(diff.deleted).child(format!("-{removed}"))),
        );
        right = right.child(render_diff_split_button(split, ui, cx));
    }
    right = right
        .child(render_diff_header_icon_button(
            "agents-diff-refresh",
            "icons/refresh.svg",
            ui,
            cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.refresh_agents_diff(cwd.clone(), cx);
            }),
            ui.muted,
        ))
        .child(render_diff_header_icon_button(
            "agents-diff-close",
            "icons/close.svg",
            ui,
            cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.agents_view.agents_diff_open = false;
                cx.notify();
            }),
            ui.muted,
        ));

    div()
        .h(px(48.))
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .px(px(14.))
        .border_b_1()
        .border_color(ui.border)
        .child(title_row)
        .child(right)
        .into_any_element()
}

fn render_diff_header_icon_button(
    id: &'static str,
    icon: &'static str,
    ui: crate::theme::UiColors,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    color: Hsla,
) -> AnyElement {
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor(CursorStyle::PointingHand)
        .hover(move |d| d.bg(with_alpha(ui.text, 0.08)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(on_click)
        .child(svg().size(px(15.)).flex_none().path(icon).text_color(color))
        .into_any_element()
}

/// Single toggle for the split (side-by-side) view, shown in the header once a
/// diff is loaded. The glyph is a fixed red/green two-pane image, rendered via
/// `img` because `svg` would flatten it to a monochrome mask. While split is on
/// the button wears the hover wash as a resting fill; clicking flips the mode.
fn render_diff_split_button(
    split: bool,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let rest = with_alpha(ui.text, if split { 0.08 } else { 0.0 });
    let hover = with_alpha(ui.text, 0.08);
    div()
        .id("agents-diff-view-split")
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor(CursorStyle::PointingHand)
        .bg(rest)
        .hover(move |d| d.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
            this.set_agents_diff_split(!split, cx);
        }))
        .child(gpui::img("icons/diff-split.svg").size(px(16.)).flex_none())
        .into_any_element()
}

/// The top "files toolbar": a muted file count on the left, a collapse-all /
/// expand-all toggle on the right. Its label + glyph flip based on whether
/// every file is already folded.
fn render_diff_files_toolbar(
    data: &AgentsDiffData,
    collapsed: &HashSet<String>,
    ui: crate::theme::UiColors,
    entity: &Entity<PaneFlowApp>,
) -> AnyElement {
    let count = data.files.len();
    let all_collapsed = data.files.iter().all(|f| collapsed.contains(&f.path));
    let (label, icon, next_collapse) = if all_collapsed {
        ("Expand all", "icons/chevron_down.svg", false)
    } else {
        ("Collapse all", "icons/chevron_up.svg", true)
    };
    let paths = data.paths();
    let entity = entity.clone();

    div()
        .flex_none()
        .h(px(34.))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .border_b_1()
        .border_color(ui.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.))
                .text_color(ui.muted)
                .child(format!("{count} file{}", if count == 1 { "" } else { "s" })),
        )
        .child(
            div()
                .id("agents-diff-collapse-all")
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(6.))
                .cursor(CursorStyle::PointingHand)
                .hover(move |d| d.bg(with_alpha(ui.text, 0.08)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(
                    move |_e: &ClickEvent, _w: &mut Window, cx: &mut gpui::App| {
                        let paths = paths.clone();
                        entity.update(cx, |this, cx| {
                            this.set_all_diff_collapsed(&paths, next_collapse, cx);
                        });
                    },
                )
                .child(
                    svg()
                        .size(px(13.))
                        .flex_none()
                        .path(icon)
                        .text_color(ui.muted),
                )
                .child(div().text_size(px(11.5)).text_color(ui.muted).child(label)),
        )
        .into_any_element()
}

fn diff_panel_centered(
    icon: &'static str,
    label: impl Into<String>,
    ui: crate::theme::UiColors,
) -> AnyElement {
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .px(px(24.))
        .child(
            svg()
                .size(px(22.))
                .flex_none()
                .path(icon)
                .text_color(ui.muted),
        )
        .child(
            div()
                .max_w(px(360.))
                .text_size(px(12.))
                .text_color(ui.muted)
                .text_center()
                .child(label.into()),
        )
        .into_any_element()
}

// -- flattened virtualized row model --------------------------------------

/// One row in the flattened, collapse-resolved list fed to `list`.
enum FlatRow {
    /// A file header; `idx` indexes the shared `files` vector, `collapsed` drives
    /// the chevron direction.
    File {
        idx: usize,
        collapsed: bool,
    },
    Hunk(SharedString),
    Line(DiffLine),
    /// A side-by-side row: old text on the left, new on the right.
    SplitLine(SplitRow),
    /// A muted note row (binary file / truncation).
    Note(SharedString),
}

/// Pair a hunk's sequential lines into side-by-side rows. Removed lines stack on
/// the left, added on the right; a context line flushes the pending change block
/// (aligning removed[i] with added[i]) then sits on both sides. Leftover removed
/// or added lines in an unbalanced block get an empty cell opposite them.
fn split_hunk_rows(lines: &[DiffLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut removed: Vec<SplitCell> = Vec::new();
    let mut added: Vec<SplitCell> = Vec::new();
    for line in lines {
        match line.kind {
            LineKind::Removed => removed.push(SplitCell {
                no: line.old_no.clone(),
                text: line.text.clone(),
                kind: LineKind::Removed,
            }),
            LineKind::Added => added.push(SplitCell {
                no: line.new_no.clone(),
                text: line.text.clone(),
                kind: LineKind::Added,
            }),
            LineKind::Context => {
                flush_change_block(&mut rows, &mut removed, &mut added);
                rows.push(SplitRow {
                    left: Some(SplitCell {
                        no: line.old_no.clone(),
                        text: line.text.clone(),
                        kind: LineKind::Context,
                    }),
                    right: Some(SplitCell {
                        no: line.new_no.clone(),
                        text: line.text.clone(),
                        kind: LineKind::Context,
                    }),
                });
            }
        }
    }
    flush_change_block(&mut rows, &mut removed, &mut added);
    rows
}

/// Drain the pending removed/added cells into paired [`SplitRow`]s.
fn flush_change_block(
    rows: &mut Vec<SplitRow>,
    removed: &mut Vec<SplitCell>,
    added: &mut Vec<SplitCell>,
) {
    let pairs = removed.len().max(added.len());
    for i in 0..pairs {
        rows.push(SplitRow {
            left: removed.get(i).cloned(),
            right: added.get(i).cloned(),
        });
    }
    removed.clear();
    added.clear();
}

/// Flatten files → rows, honoring the per-file collapse set. A collapsed file
/// contributes only its header row; an expanded file adds its hunk headers and
/// lines (capped per file). Cheap: pushes lightweight enums with `Arc`-backed
/// strings, no element construction.
fn flatten_rows(
    files: &[AgentsDiffFile],
    collapsed: &HashSet<String>,
    split: bool,
) -> Vec<FlatRow> {
    let mut rows = Vec::new();
    for (idx, file) in files.iter().enumerate() {
        let is_collapsed = collapsed.contains(&file.path);
        rows.push(FlatRow::File {
            idx,
            collapsed: is_collapsed,
        });
        if is_collapsed {
            continue;
        }
        if file.is_binary {
            rows.push(FlatRow::Note(SharedString::from(
                "Binary file: diff not shown",
            )));
            continue;
        }
        let mut rendered = 0usize;
        let mut hidden = 0usize;
        for hunk in &file.hunks {
            let split_rows = if split {
                Some(split_hunk_rows(&hunk.lines))
            } else {
                None
            };
            let unit_count = split_rows.as_ref().map_or(hunk.lines.len(), |s| s.len());
            if rendered >= MAX_LINES_PER_FILE {
                hidden += unit_count;
                continue;
            }
            rows.push(FlatRow::Hunk(hunk.header.clone()));
            if let Some(split_rows) = split_rows {
                for srow in split_rows {
                    if rendered >= MAX_LINES_PER_FILE {
                        hidden += 1;
                    } else {
                        rows.push(FlatRow::SplitLine(srow));
                        rendered += 1;
                    }
                }
            } else {
                for line in &hunk.lines {
                    if rendered >= MAX_LINES_PER_FILE {
                        hidden += 1;
                    } else {
                        rows.push(FlatRow::Line(line.clone()));
                        rendered += 1;
                    }
                }
            }
        }
        if hidden > 0 {
            rows.push(FlatRow::Note(SharedString::from(format!(
                "… {hidden} more line{} hidden",
                if hidden == 1 { "" } else { "s" }
            ))));
        }
    }
    rows
}

fn render_flat_row(
    row: &FlatRow,
    files: &[AgentsDiffFile],
    mono: &SharedString,
    ui: crate::theme::UiColors,
    entity: &Entity<PaneFlowApp>,
) -> AnyElement {
    match row {
        FlatRow::File { idx, collapsed } => files
            .get(*idx)
            .map(|file| render_flat_file_header(file, *collapsed, ui, entity))
            .unwrap_or_else(|| div().h(px(FILE_ROW_HEIGHT)).into_any_element()),
        FlatRow::Hunk(text) => render_flat_hunk(text.clone(), mono.clone(), ui),
        FlatRow::Line(line) => render_flat_line(line, mono.clone(), ui),
        FlatRow::SplitLine(row) => render_flat_split_line(row, mono.clone(), ui),
        FlatRow::Note(text) => render_flat_note(text.clone(), ui),
    }
}

fn render_flat_file_header(
    file: &AgentsDiffFile,
    collapsed: bool,
    ui: crate::theme::UiColors,
    entity: &Entity<PaneFlowApp>,
) -> AnyElement {
    let diff = agents_diff_palette(ui);
    let icon_color = status_color(file.status, ui, diff);
    let (dir, name) = split_path(&file.path);
    let chevron = if collapsed {
        "icons/chevron-right.svg"
    } else {
        "icons/chevron-down.svg"
    };
    let path = file.path.clone();
    let entity = entity.clone();

    let mut path_row = div().min_w_0().flex().flex_row().items_center();
    if let Some(old) = &file.old_path {
        let (_, old_name) = split_path(old);
        path_row = path_row.child(
            div()
                .flex_none()
                .text_size(px(13.))
                .text_color(ui.muted)
                .child(format!("{old_name} → ")),
        );
    }
    if !dir.is_empty() {
        path_row = path_row.child(
            div()
                .min_w_0()
                .overflow_x_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .text_color(ui.muted)
                .child(SharedString::from(dir.to_string())),
        );
    }
    path_row = path_row.child(
        div()
            .flex_none()
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ui.text)
            .child(SharedString::from(name.to_string())),
    );

    // Codex shows both counts on every row; a zero side reads dimmer.
    let counts = div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .text_size(px(12.))
        .child(
            div()
                .text_color(count_color(file.added, diff.added, ui))
                .child(format!("+{}", file.added)),
        )
        .child(
            div()
                .text_color(count_color(file.removed, diff.deleted, ui))
                .child(format!("-{}", file.removed)),
        );

    div()
        .id(SharedString::from(format!("agents-diff-file-{path}")))
        .h(px(FILE_ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .px(px(14.))
        .border_b_1()
        .border_color(ui.base)
        .bg(with_alpha(ui.text, 0.05))
        .cursor(CursorStyle::PointingHand)
        .hover(move |d| d.bg(with_alpha(ui.text, 0.08)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(
            move |_e: &ClickEvent, _w: &mut Window, cx: &mut gpui::App| {
                let path = path.clone();
                entity.update(cx, |this, cx| {
                    this.toggle_diff_file_collapsed(path, cx);
                });
            },
        )
        .child(
            svg()
                .size(px(16.))
                .flex_none()
                .path(file_icon(&file.path))
                .text_color(icon_color),
        )
        .child(path_row)
        .child(div().flex_1())
        .child(counts)
        .child(
            svg()
                .size(px(14.))
                .flex_none()
                .path(chevron)
                .text_color(with_alpha(ui.muted, 0.7)),
        )
        .into_any_element()
}

fn render_flat_hunk(
    header: SharedString,
    mono: SharedString,
    ui: crate::theme::UiColors,
) -> AnyElement {
    div()
        .h(px(HUNK_ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .px(px(12.))
        .bg(with_alpha(ui.text, 0.03))
        .font_family(mono)
        .text_size(px(11.))
        .text_color(with_alpha(ui.muted, 0.9))
        .whitespace_nowrap()
        .overflow_x_hidden()
        .text_ellipsis()
        .child(header)
        .into_any_element()
}

fn render_flat_line(line: &DiffLine, mono: SharedString, ui: crate::theme::UiColors) -> AnyElement {
    let diff = agents_diff_palette(ui);
    let transparent = with_alpha(ui.text, 0.0);
    let (sign, sign_color, gutter_color, text_color, row_bg, gutter_bg, bar_color) = match line.kind
    {
        LineKind::Added => (
            "+",
            diff.added,
            diff.added,
            ui.text,
            diff.added_background,
            diff.added_gutter_background,
            diff.added,
        ),
        LineKind::Removed => (
            "-",
            diff.deleted,
            diff.deleted,
            with_alpha(ui.text, 0.82),
            diff.deleted_background,
            diff.deleted_gutter_background,
            diff.deleted,
        ),
        LineKind::Context => (
            " ",
            ui.muted,
            with_alpha(ui.muted, 0.7),
            with_alpha(ui.text, 0.9),
            transparent,
            transparent,
            transparent,
        ),
    };

    div()
        .h(px(LINE_ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .bg(row_bg)
        .child(
            div()
                .flex_none()
                .w(px(CHANGE_BAR_WIDTH))
                .h_full()
                .bg(bar_color),
        )
        .child(
            div()
                .flex_none()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .bg(gutter_bg)
                .child(render_gutter(
                    line.old_no.clone(),
                    mono.clone(),
                    gutter_color,
                ))
                .child(render_gutter(
                    line.new_no.clone(),
                    mono.clone(),
                    gutter_color,
                ))
                .child(
                    div()
                        .flex_none()
                        .w(px(16.))
                        .flex()
                        .justify_center()
                        .font_family(mono.clone())
                        .text_size(px(12.))
                        .text_color(sign_color)
                        .child(sign),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pr(px(12.))
                .overflow_x_hidden()
                .whitespace_nowrap()
                .font_family(mono)
                .text_size(px(12.))
                .text_color(text_color)
                .child(line.text.clone()),
        )
        .into_any_element()
}

/// A side-by-side row: two equal halves (old left, new right) split by a thin
/// panel-coloured groove. Each half carries its own change bar, line-number
/// gutter and wash; a `None` cell renders as a faint filler.
fn render_flat_split_line(
    row: &SplitRow,
    mono: SharedString,
    ui: crate::theme::UiColors,
) -> AnyElement {
    div()
        .h(px(LINE_ROW_HEIGHT))
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .child(render_split_half(row.left.as_ref(), mono.clone(), ui))
        .child(div().flex_none().w(px(1.)).h_full().bg(ui.base))
        .child(render_split_half(row.right.as_ref(), mono, ui))
        .into_any_element()
}

fn render_split_half(
    cell: Option<&SplitCell>,
    mono: SharedString,
    ui: crate::theme::UiColors,
) -> AnyElement {
    let diff = agents_diff_palette(ui);
    let transparent = with_alpha(ui.text, 0.0);
    let Some(cell) = cell else {
        return div()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(with_alpha(ui.text, 0.02))
            .into_any_element();
    };
    let (bar_color, row_bg, gutter_bg, gutter_color, text_color) = match cell.kind {
        LineKind::Added => (
            diff.added,
            diff.added_background,
            diff.added_gutter_background,
            diff.added,
            ui.text,
        ),
        LineKind::Removed => (
            diff.deleted,
            diff.deleted_background,
            diff.deleted_gutter_background,
            diff.deleted,
            with_alpha(ui.text, 0.82),
        ),
        LineKind::Context => (
            transparent,
            transparent,
            transparent,
            with_alpha(ui.muted, 0.7),
            with_alpha(ui.text, 0.9),
        ),
    };

    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_row()
        .items_center()
        .bg(row_bg)
        .child(
            div()
                .flex_none()
                .w(px(CHANGE_BAR_WIDTH))
                .h_full()
                .bg(bar_color),
        )
        .child(
            div()
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                .bg(gutter_bg)
                .child(render_gutter(cell.no.clone(), mono.clone(), gutter_color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px(px(8.))
                .overflow_x_hidden()
                .whitespace_nowrap()
                .font_family(mono)
                .text_size(px(12.))
                .text_color(text_color)
                .child(cell.text.clone()),
        )
        .into_any_element()
}

fn render_flat_note(text: SharedString, ui: crate::theme::UiColors) -> AnyElement {
    div()
        .h(px(NOTE_ROW_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .px(px(14.))
        .text_size(px(11.))
        .text_color(ui.muted)
        .whitespace_nowrap()
        .overflow_x_hidden()
        .text_ellipsis()
        .child(text)
        .into_any_element()
}

fn render_gutter(number: SharedString, mono: SharedString, color: Hsla) -> AnyElement {
    div()
        .flex_none()
        .w(px(36.))
        .flex()
        .justify_end()
        .pr(px(8.))
        .font_family(mono)
        .text_size(px(11.))
        .text_color(color)
        .child(number)
        .into_any_element()
}

/// Status → file-icon tint. The glyph itself comes from [`file_icon`] (by
/// extension); its color carries the git status the way Codex's panel does, so
/// the old single-letter A/M/D/R badge is gone.
fn status_color(status: DiffStatus, ui: crate::theme::UiColors, diff: AgentsDiffPalette) -> Hsla {
    match status {
        DiffStatus::Added => diff.added,
        DiffStatus::Modified => ui.vc_modified,
        DiffStatus::Deleted => diff.deleted,
        DiffStatus::Renamed => ui.vc_modified,
    }
}

/// Pick a file-type glyph from the path's extension, Codex-style. Extension-less
/// or unknown files fall back to a plain document icon.
fn file_icon(path: &str) -> &'static str {
    let (_, name) = split_path(path);
    let ext = name.rsplit('.').next().filter(|e| *e != name).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "go" | "c" | "h" | "cc"
        | "cpp" | "cxx" | "hpp" | "java" | "kt" | "kts" | "rb" | "php" | "swift" | "scala"
        | "sh" | "bash" | "zsh" | "fish" | "lua" | "dart" | "ex" | "exs" | "vue" | "svelte"
        | "html" | "htm" | "xml" | "json" | "toml" | "yaml" | "yml" => "icons/code.svg",
        "css" | "scss" | "sass" | "less" | "svg" | "png" | "jpg" | "jpeg" | "gif" | "webp"
        | "ico" | "avif" => "icons/palette.svg",
        "sql" | "db" | "sqlite" | "csv" | "parquet" => "icons/database.svg",
        _ => "icons/file-text.svg",
    }
}

/// Per-file count tint. Codex prints both `+N` and `-N` on every row; a zero
/// side is dimmed rather than hidden so the columns stay aligned and calm.
fn count_color(value: u32, color: Hsla, ui: crate::theme::UiColors) -> Hsla {
    if value == 0 {
        with_alpha(ui.muted, 0.55)
    } else {
        color
    }
}

/// Split a path into `(dir_with_trailing_slash, file_name)`.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    }
}

// -- git plumbing ----------------------------------------------------------

/// Compute the panel's diff: tracked changes vs `HEAD` (staged + unstaged),
/// parsed from `git diff`, plus untracked files synthesised as all-added.
/// Returns `(files, total_added, total_removed)`. Runs off the main thread.
fn compute_agents_diff(cwd: &str) -> Result<(Vec<AgentsDiffFile>, u32, u32), String> {
    // `-M` makes git surface renames as `rename from/to` instead of delete+add.
    // On a repo with no commits `HEAD` doesn't resolve, so fall back to the
    // index diff (still useful) rather than failing the whole panel.
    let patch = run_git(cwd, &["diff", "--no-color", "-M", "HEAD"])
        .or_else(|_| run_git(cwd, &["diff", "--no-color", "-M"]))
        .unwrap_or_default();
    let mut files = parse_unified_diff(&patch);

    if let Ok(listing) = run_git(cwd, &["ls-files", "--others", "--exclude-standard", "-z"]) {
        for rel in listing
            .split('\0')
            .filter(|s| !s.is_empty())
            .take(MAX_UNTRACKED_FILES)
        {
            if let Some(file) = read_untracked_file(cwd, rel) {
                files.push(file);
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let added = files.iter().map(|f| f.added).sum();
    let removed = files.iter().map(|f| f.removed).sum();
    Ok((files, added, removed))
}

fn run_git(cwd: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0");
    let output = paneflow_process::run_with_timeout(cmd, GIT_DEADLINE, GIT_OUTPUT_CAP)
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let trimmed = stderr.trim();
        return Err(if trimmed.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            trimmed.lines().next().unwrap_or(trimmed).to_string()
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Read an untracked file (capped) and synthesise an all-added [`AgentsDiffFile`]
/// without spawning another git process. Binary files (NUL byte) are listed
/// without a body.
fn read_untracked_file(cwd: &str, rel: &str) -> Option<AgentsDiffFile> {
    use std::io::Read;
    let full = Path::new(cwd).join(rel);
    let file = std::fs::File::open(&full).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_UNTRACKED_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;

    if bytes.contains(&0) {
        return Some(AgentsDiffFile {
            path: rel.to_string(),
            old_path: None,
            status: DiffStatus::Added,
            hunks: Vec::new(),
            added: 0,
            removed: 0,
            is_binary: true,
        });
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut lines = Vec::new();
    for (i, content) in text.lines().enumerate() {
        lines.push(DiffLine {
            kind: LineKind::Added,
            old_no: SharedString::default(),
            new_no: num(i as u32 + 1),
            text: expand_tabs(content),
        });
    }
    let added = lines.len() as u32;
    let header = SharedString::from(format!("@@ -0,0 +1,{added} @@"));
    Some(AgentsDiffFile {
        path: rel.to_string(),
        old_path: None,
        status: DiffStatus::Added,
        hunks: vec![DiffHunk { header, lines }],
        added,
        removed: 0,
        is_binary: false,
    })
}

/// Parse `git diff` unified output into per-file payloads. Paths are taken from
/// the unambiguous `--- a/` / `+++ b/` and `rename` lines (the `diff --git`
/// header is only a section delimiter, since paths with spaces make it
/// ambiguous).
fn parse_unified_diff(patch: &str) -> Vec<AgentsDiffFile> {
    let mut files: Vec<AgentsDiffFile> = Vec::new();
    let mut current: Option<AgentsDiffFile> = None;
    let mut old_no = 0u32;
    let mut new_no = 0u32;

    for raw in patch.split('\n') {
        if raw.starts_with("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(AgentsDiffFile {
                path: String::new(),
                old_path: None,
                status: DiffStatus::Modified,
                hunks: Vec::new(),
                added: 0,
                removed: 0,
                is_binary: false,
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };

        if raw.starts_with("new file mode") {
            file.status = DiffStatus::Added;
        } else if raw.starts_with("deleted file mode") {
            file.status = DiffStatus::Deleted;
        } else if let Some(rest) = raw.strip_prefix("rename from ") {
            file.old_path = Some(rest.trim().to_string());
            file.status = DiffStatus::Renamed;
        } else if let Some(rest) = raw.strip_prefix("rename to ") {
            file.path = rest.trim().to_string();
            file.status = DiffStatus::Renamed;
        } else if raw.starts_with("Binary files") {
            file.is_binary = true;
        } else if let Some(rest) = raw.strip_prefix("--- ") {
            if file.status == DiffStatus::Deleted
                && let Some(path) = strip_diff_path(rest)
            {
                file.path = path;
            }
        } else if let Some(rest) = raw.strip_prefix("+++ ") {
            if file.status != DiffStatus::Deleted
                && let Some(path) = strip_diff_path(rest)
            {
                file.path = path;
            }
        } else if raw.starts_with("@@") {
            let (o_start, n_start) = parse_hunk_header(raw);
            old_no = o_start;
            new_no = n_start;
            file.hunks.push(DiffHunk {
                header: SharedString::from(raw.to_string()),
                lines: Vec::new(),
            });
        } else if let Some(hunk) = file.hunks.last_mut() {
            if let Some(text) = raw.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: LineKind::Added,
                    old_no: SharedString::default(),
                    new_no: num(new_no),
                    text: expand_tabs(text),
                });
                new_no += 1;
                file.added += 1;
            } else if let Some(text) = raw.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: LineKind::Removed,
                    old_no: num(old_no),
                    new_no: SharedString::default(),
                    text: expand_tabs(text),
                });
                old_no += 1;
                file.removed += 1;
            } else if let Some(text) = raw.strip_prefix(' ') {
                hunk.lines.push(DiffLine {
                    kind: LineKind::Context,
                    old_no: num(old_no),
                    new_no: num(new_no),
                    text: expand_tabs(text),
                });
                old_no += 1;
                new_no += 1;
            }
            // "\ No newline at end of file" and stray blanks are ignored.
        }
    }
    if let Some(file) = current.take() {
        files.push(file);
    }
    // Drop pure-metadata entries that produced no path (defensive).
    files.retain(|f| !f.path.is_empty());
    files
}

/// Format a line number as a shared string (cached once, cloned per frame).
fn num(n: u32) -> SharedString {
    SharedString::from(n.to_string())
}

/// Expand tabs to four spaces (tabs don't advance to a stop in a plain div) and
/// hand back a render-ready shared string.
fn expand_tabs(text: &str) -> SharedString {
    if text.contains('\t') {
        SharedString::from(text.replace('\t', "    "))
    } else {
        SharedString::from(text.to_string())
    }
}

/// Strip the `a/` / `b/` prefix (and surrounding quotes / trailing tab) from a
/// `---` / `+++` path line. Returns `None` for `/dev/null`.
fn strip_diff_path(raw: &str) -> Option<String> {
    let raw = raw.trim_end_matches('\t').trim();
    if raw == "/dev/null" {
        return None;
    }
    let unprefixed = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    Some(unprefixed.trim_matches('"').to_string())
}

/// Parse the start line numbers out of `@@ -oldStart,oldCount +newStart,newCount @@`.
fn parse_hunk_header(raw: &str) -> (u32, u32) {
    let mut old_start = 0u32;
    let mut new_start = 0u32;
    if let Some(rest) = raw.strip_prefix("@@ ")
        && let Some(end) = rest.find(" @@")
    {
        for token in rest[..end].split(' ') {
            if let Some(value) = token.strip_prefix('-') {
                old_start = value
                    .split(',')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
            } else if let Some(value) = token.strip_prefix('+') {
                new_start = value
                    .split(',')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
            }
        }
    }
    (old_start, new_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modified_file_with_added_and_removed() {
        // NB: a single-line literal with explicit `\n`; a `\`-continuation
        // would eat the leading space of the " keep" context line.
        let patch = "diff --git a/src/foo.rs b/src/foo.rs\nindex 111..222 100644\n--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,3 +1,3 @@ fn main()\n keep\n-old line\n+new line\n";
        let files = parse_unified_diff(patch);
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "src/foo.rs");
        assert_eq!(f.status, DiffStatus::Modified);
        assert_eq!(f.added, 1);
        assert_eq!(f.removed, 1);
        let lines = &f.hunks[0].lines;
        assert_eq!(lines[0].kind, LineKind::Context);
        assert_eq!(lines[0].old_no.as_ref(), "1");
        assert_eq!(lines[0].new_no.as_ref(), "1");
        assert_eq!(lines[1].kind, LineKind::Removed);
        assert_eq!(lines[1].old_no.as_ref(), "2");
        assert_eq!(lines[1].text.as_ref(), "old line");
        assert_eq!(lines[2].kind, LineKind::Added);
        assert_eq!(lines[2].new_no.as_ref(), "2");
        assert_eq!(lines[2].text.as_ref(), "new line");
    }

    #[test]
    fn parses_new_and_deleted_files() {
        let patch = "diff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,1 @@\n+hello\ndiff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n--- a/gone.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\n";
        let files = parse_unified_diff(patch);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].status, DiffStatus::Added);
        assert_eq!(files[1].path, "gone.txt");
        assert_eq!(files[1].status, DiffStatus::Deleted);
    }

    #[test]
    fn parses_rename_with_old_path() {
        let patch = "diff --git a/old/name.rs b/new/name.rs\nsimilarity index 95%\nrename from old/name.rs\nrename to new/name.rs\n";
        let files = parse_unified_diff(patch);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, DiffStatus::Renamed);
        assert_eq!(files[0].path, "new/name.rs");
        assert_eq!(files[0].old_path.as_deref(), Some("old/name.rs"));
    }

    #[test]
    fn hunk_header_line_numbers() {
        assert_eq!(parse_hunk_header("@@ -12,7 +15,9 @@ context"), (12, 15));
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), (1, 1));
    }

    #[test]
    fn split_path_separates_dir_and_name() {
        assert_eq!(split_path("src/app/foo.rs"), ("src/app/", "foo.rs"));
        assert_eq!(split_path("foo.rs"), ("", "foo.rs"));
    }

    #[test]
    fn flatten_collapses_file_body() {
        let patch = "diff --git a/a.rs b/a.rs\nindex 1..2 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        let files = parse_unified_diff(patch);
        let mut collapsed = HashSet::new();
        // Expanded: header + hunk header + 2 lines = 4 rows.
        assert_eq!(flatten_rows(&files, &collapsed, false).len(), 4);
        // Collapsed: just the header row.
        collapsed.insert("a.rs".to_string());
        assert_eq!(flatten_rows(&files, &collapsed, false).len(), 1);
    }

    #[test]
    fn split_pairs_modifications_and_unbalanced_blocks() {
        let lines = vec![
            DiffLine {
                kind: LineKind::Context,
                old_no: num(1),
                new_no: num(1),
                text: "ctx".into(),
            },
            DiffLine {
                kind: LineKind::Removed,
                old_no: num(2),
                new_no: SharedString::default(),
                text: "old a".into(),
            },
            DiffLine {
                kind: LineKind::Removed,
                old_no: num(3),
                new_no: SharedString::default(),
                text: "old b".into(),
            },
            DiffLine {
                kind: LineKind::Added,
                old_no: SharedString::default(),
                new_no: num(2),
                text: "new a".into(),
            },
        ];
        let rows = split_hunk_rows(&lines);
        // context row, then the 2-removed / 1-added block paired into 2 rows.
        assert_eq!(rows.len(), 3);
        assert!(rows[0].left.is_some() && rows[0].right.is_some());
        assert_eq!(rows[1].left.as_ref().unwrap().text.as_ref(), "old a");
        assert_eq!(rows[1].right.as_ref().unwrap().text.as_ref(), "new a");
        assert_eq!(rows[2].left.as_ref().unwrap().text.as_ref(), "old b");
        assert!(rows[2].right.is_none());
    }
}
