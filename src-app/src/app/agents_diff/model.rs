//! The Agents dock's render-ready data snapshot + its layout constants.

use std::collections::HashSet;
use std::rc::Rc;

use gpui::Hsla;

use super::git::{AgentsDiffBuilt, AgentsDiffRows};
use crate::diff::{
    DisplayRow, FileSpan, SplitRow, apply_collapse_split, apply_collapse_unified, split_file_spans,
    split_max_line_no, split_offsets, unified_file_spans, unified_max_line_no, unified_offsets,
};

/// Default width of the docked panel. Wide enough to read code without constant
/// wrapping, narrow enough to leave the terminal column usable beside it. The
/// panel is user-resizable by dragging its left edge; the live width lives on
/// [`crate::AgentsViewState::agents_diff_width`], clamped to the bounds below.
pub(crate) const AGENTS_DIFF_PANEL_WIDTH: f32 = 540.0;

/// Resize clamp for the diff dock's user-dragged width. The floor keeps the
/// gutters plus a readable code column; the ceiling stops the dock from
/// swallowing the whole main area on a wide window.
pub(super) const AGENTS_DIFF_PANEL_MIN_WIDTH: f32 = 360.0;
pub(super) const AGENTS_DIFF_PANEL_MAX_WIDTH: f32 = 1100.0;

/// Added/deleted *text* colors for diff counters rendered outside the diff
/// panel (e.g. the Environment card's "Changes" row), kept in lockstep with the
/// panel's own palette so the +/- counts match the washes on every theme. The
/// canonical diff palette lives on [`crate::theme::UiColors::diff_colors`] so
/// the Agents dock, the Diff/Review view, and the diff sidebar share one source.
pub(crate) fn agents_diff_count_colors(ui: crate::theme::UiColors) -> (Hsla, Hsla) {
    let diff = ui.diff_colors();
    (diff.added, diff.deleted)
}

/// Render-ready snapshot of the panel's data. Cheap to clone every frame: every
/// row vector is shared behind an `Rc` (single-threaded GPUI state). Mirrors a
/// single [`crate::diff`] `Column`: the full rows are kept so a collapse toggle
/// re-derives the filtered `disp_*` views without re-shelling git.
#[derive(Clone)]
pub(crate) struct AgentsDiffData {
    /// The working directory this diff was computed for. Used to ignore a
    /// stale async result after the user switches threads or closes the panel.
    pub(crate) cwd: String,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) unified_loaded: bool,
    pub(super) split_loaded: bool,
    // Full (uncollapsed) rows + path→header-index anchors from the shared
    // pipeline, retained so a collapse toggle re-derives the filtered views.
    pub(super) unified: Rc<Vec<DisplayRow>>,
    pub(super) split: Rc<Vec<SplitRow>>,
    pub(super) anchors_unified: Rc<Vec<(String, usize)>>,
    pub(super) anchors_split: Rc<Vec<(String, usize)>>,
    // Collapse-filtered display rows + cached layout inputs (lockstep), consumed
    // by `DiffElement` each frame.
    pub(super) disp_unified: Rc<Vec<DisplayRow>>,
    pub(super) disp_split: Rc<Vec<SplitRow>>,
    pub(super) disp_anchors_unified: Rc<Vec<(String, usize)>>,
    pub(super) disp_anchors_split: Rc<Vec<(String, usize)>>,
    pub(super) disp_unified_offsets: Rc<Vec<f32>>,
    pub(super) disp_split_offsets: Rc<Vec<f32>>,
    pub(super) disp_unified_max_no: u32,
    pub(super) disp_split_max_no: u32,
    /// Per-file horizontal-scroll spans (widest code line per file), kept in
    /// lockstep with the display rows so `DiffElement` bounds each file's
    /// horizontal offset without re-measuring rows per frame.
    pub(super) disp_unified_spans: Rc<Vec<FileSpan>>,
    pub(super) disp_split_spans: Rc<Vec<FileSpan>>,
    pub(super) paths: Vec<String>,
    pub(super) file_count: usize,
    pub(super) added: u32,
    pub(super) removed: u32,
}

impl AgentsDiffData {
    pub(super) fn loading(cwd: String) -> Self {
        Self {
            cwd,
            loading: true,
            error: None,
            unified_loaded: false,
            split_loaded: false,
            unified: Rc::new(Vec::new()),
            split: Rc::new(Vec::new()),
            anchors_unified: Rc::new(Vec::new()),
            anchors_split: Rc::new(Vec::new()),
            disp_unified: Rc::new(Vec::new()),
            disp_split: Rc::new(Vec::new()),
            disp_anchors_unified: Rc::new(Vec::new()),
            disp_anchors_split: Rc::new(Vec::new()),
            disp_unified_offsets: Rc::new(vec![0.0]),
            disp_split_offsets: Rc::new(vec![0.0]),
            disp_unified_max_no: 0,
            disp_split_max_no: 0,
            disp_unified_spans: Rc::new(Vec::new()),
            disp_split_spans: Rc::new(Vec::new()),
            paths: Vec::new(),
            file_count: 0,
            added: 0,
            removed: 0,
        }
    }

    pub(super) fn message(cwd: String, error: String) -> Self {
        let mut data = Self::loading(cwd);
        data.loading = false;
        data.error = Some(error);
        data
    }

    /// Rebuild the collapse-filtered views from the full rows + `collapsed`.
    /// When nothing is collapsed the full rows are shared as-is (no allocation);
    /// otherwise collapsed files keep only their header. Mirrors
    /// [`crate::diff::DiffView`]'s `recompute_display`.
    pub(super) fn recompute(&mut self, collapsed: &HashSet<String>) {
        if self.unified_loaded {
            self.recompute_unified(collapsed);
        }
        if self.split_loaded {
            self.recompute_split(collapsed);
        }
    }

    pub(super) fn has_mode(&self, split: bool) -> bool {
        if split {
            self.split_loaded
        } else {
            self.unified_loaded
        }
    }

    pub(super) fn apply_built(&mut self, built: AgentsDiffBuilt, collapsed: &HashSet<String>) {
        self.loading = false;
        self.error = None;
        self.paths = built.paths;
        self.file_count = built.file_count;
        self.added = built.added;
        self.removed = built.removed;

        match built.rows {
            AgentsDiffRows::Unified { rows, anchors } => {
                self.unified = Rc::new(rows);
                self.anchors_unified = Rc::new(anchors);
                self.unified_loaded = true;
                self.recompute_unified(collapsed);
            }
            AgentsDiffRows::Split { rows, anchors } => {
                self.split = Rc::new(rows);
                self.anchors_split = Rc::new(anchors);
                self.split_loaded = true;
                self.recompute_split(collapsed);
            }
        }
    }

    fn recompute_unified(&mut self, collapsed: &HashSet<String>) {
        if collapsed.is_empty() {
            self.disp_unified = self.unified.clone();
            self.disp_anchors_unified = self.anchors_unified.clone();
        } else {
            let (du, au) = apply_collapse_unified(&self.unified, &self.anchors_unified, collapsed);
            self.disp_unified = Rc::new(du);
            self.disp_anchors_unified = Rc::new(au);
        }
        self.disp_unified_offsets = Rc::new(unified_offsets(&self.disp_unified));
        self.disp_unified_max_no = unified_max_line_no(&self.disp_unified);
        self.disp_unified_spans = Rc::new(unified_file_spans(&self.disp_unified));
    }

    fn recompute_split(&mut self, collapsed: &HashSet<String>) {
        if collapsed.is_empty() {
            self.disp_split = self.split.clone();
            self.disp_anchors_split = self.anchors_split.clone();
        } else {
            let (ds, as_) = apply_collapse_split(&self.split, &self.anchors_split, collapsed);
            self.disp_split = Rc::new(ds);
            self.disp_anchors_split = Rc::new(as_);
        }
        self.disp_split_offsets = Rc::new(split_offsets(&self.disp_split));
        self.disp_split_max_no = split_max_line_no(&self.disp_split);
        self.disp_split_spans = Rc::new(split_file_spans(&self.disp_split));
    }

    /// The file paths in this snapshot, used to drive "collapse all".
    pub(super) fn paths(&self) -> Vec<String> {
        self.paths.clone()
    }

    /// Whether every file is currently folded (drives the toolbar toggle label).
    pub(super) fn all_collapsed(&self, collapsed: &HashSet<String>) -> bool {
        !self.paths.is_empty() && self.paths.iter().all(|p| collapsed.contains(p))
    }
}
