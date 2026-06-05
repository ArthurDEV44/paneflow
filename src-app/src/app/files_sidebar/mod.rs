//! Docked Files right sidebar (PRD `prd-files-tree-sidebar-2026-Q3`, EP-001).
//!
//! Mirrors the agent-sessions sidebar (`sessions_sidebar.rs`): a
//! `flex_shrink_0` child of the root `flex_row`, toggled by the tab-bar Files
//! button via `PaneEvent::ToggleFilesSidebar`, mutually exclusive with the
//! sessions sidebar (one right column). Renders a lazily-expanded,
//! folders-first tree of the active workspace's `cwd`. Markdown rows are
//! full-color + click-to-open into the active pane (the WCAG 2.5.7
//! single-pointer alternative to the EP-003 drag); every other file is greyed
//! and inert; gitignored/hidden entries are dimmed further.
//!
//! This module holds the state mutations (open/close, re-root, expand/collapse,
//! open-markdown) + the container render; the header/body/row rendering lives
//! in `view.rs`, and the pure tree model + fs helpers in `files_tree.rs`.

mod context_menu;
mod row;
mod view;
mod watch;

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Pixels, Styled, div,
    prelude::*, px,
};

use crate::PaneFlowApp;
use crate::app::files_tree::{self, FilesTreeState};

/// Fixed sidebar width — matches the sessions sidebar (a resizable width is
/// deferred per the PRD non-goals).
pub(super) const SIDEBAR_WIDTH: Pixels = px(300.);
pub(super) const ROW_HEIGHT: Pixels = px(28.);
/// Per-depth indentation added to the row's left padding.
pub(super) const INDENT_STEP: f32 = 12.;
/// Extra opacity knock-down for gitignored / hidden rows (US-004 second tier).
pub(super) const DIMMED_OPACITY: f32 = 0.55;

impl PaneFlowApp {
    /// Toggle the Files sidebar. Opening resolves the active workspace's `cwd`
    /// to the tree root, reads + auto-expands it, and closes the sessions
    /// sidebar (mutual exclusion). Re-clicking closes and releases the tree.
    pub(crate) fn toggle_files_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.files_sidebar_open {
            self.close_files_sidebar(cx);
            return;
        }
        let Some(ws) = self.workspaces.get(self.active_idx) else {
            return;
        };
        let root = PathBuf::from(&ws.cwd);
        // US-007: restore this workspace's expansion (held on the Workspace,
        // so it survives a previous close within the session and a restart).
        let persisted = ws.files_expanded.clone();

        // Mutual exclusion: only one right column is ever visible.
        if self.agent_sessions.sessions_sidebar_open {
            self.close_sessions_sidebar(cx);
        }
        // Floating dropdowns would paint over the docked panel.
        self.workspace_menu_open = None;
        self.profile_menu_open = None;

        self.files_sidebar_open = true;
        self.files_tree_scroll = gpui::ScrollHandle::new();
        // US-018: hydrate the tree + install the recursive watcher OFF the
        // render thread — a recursive `notify` walk over a repo carrying a
        // `target/` (~23k dirs) otherwise froze Wayland. A root shell paints
        // this frame; `sync_files_expansion` runs (and reconciles stale
        // persisted paths back into `session.json`) once hydration lands.
        self.spawn_files_hydration(root, persisted, cx);
        cx.notify();
    }

    /// Close the sidebar and release the per-open tree cache + watcher. The
    /// per-workspace expansion lives on the `Workspace`, so it is NOT reset
    /// here (US-007) — reopening restores it.
    pub(crate) fn close_files_sidebar(&mut self, cx: &mut Context<Self>) {
        self.files_sidebar_open = false;
        self.files_tree = FilesTreeState::default();
        // US-005: drop the recursive watch + its channel while closed.
        self.files_watcher = None;
        self.files_event_rx = None;
        // Close any open row context menu so it can't outlive the tree.
        self.files_menu_open = None;
        cx.notify();
    }

    /// Re-root the tree on the active workspace's `cwd` when it changed while
    /// the sidebar is open (US-002 workspace-switch). No-op when closed or when
    /// the root is unchanged. Restores the new workspace's expansion (US-007)
    /// and re-targets the watcher (US-005).
    pub(crate) fn reroot_files_tree(&mut self, cx: &mut Context<Self>) {
        if !self.files_sidebar_open {
            return;
        }
        let Some(ws) = self.workspaces.get(self.active_idx) else {
            return;
        };
        let root = PathBuf::from(&ws.cwd);
        if self.files_tree.root == root {
            return;
        }
        let persisted = ws.files_expanded.clone();
        // US-018: re-root off the render thread (the recursive watch walk).
        self.spawn_files_hydration(root, persisted, cx);
    }

    /// Expand or collapse a directory. First expand reads its listing (lazy,
    /// cached thereafter); when the live watcher is unavailable (US-006), every
    /// expand re-reads so manual navigation stays current without push updates.
    /// Reads are synchronous on the interaction (not the render path) per the
    /// PRD's "start synchronous" decision. Mirrors the expansion into the
    /// workspace + persists it (US-007).
    fn toggle_dir(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.files_tree.expanded.contains(path) {
            self.files_tree.expanded.remove(path);
        } else {
            self.files_tree.expanded.insert(path.to_path_buf());
            let stale =
                self.files_watcher.is_none() || !self.files_tree.children.contains_key(path);
            if stale {
                let listing = files_tree::read_dir_sorted(&self.files_tree.root, path);
                self.files_tree.children.insert(path.to_path_buf(), listing);
            }
        }
        self.sync_files_expansion();
        self.save_session(cx);
        cx.notify();
    }

    /// Open a markdown file in the active pane — the focused pane of the active
    /// workspace, falling back to its first leaf. Reuses `MarkdownView::open` +
    /// `Pane::add_markdown_tab` unchanged; the sidebar stays open.
    fn open_markdown_in_active_pane(
        &mut self,
        path: PathBuf,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self
            .workspaces
            .get(self.active_idx)
            .and_then(|ws| ws.root.as_ref())
        else {
            return;
        };
        let Some(target) = root
            .focused_pane(window, cx)
            .or_else(|| root.collect_leaves().into_iter().next())
        else {
            return;
        };
        let markdown = cx.new(|cx| crate::markdown::MarkdownView::open(path, cx));
        target.update(cx, |pane, cx| {
            pane.add_markdown_tab(markdown, cx);
            cx.notify();
        });
        self.save_session(cx);
        cx.notify();
    }

    /// Render the docked Files sidebar. Only called when `files_sidebar_open`.
    pub(crate) fn render_files_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        div()
            .id("files-sidebar")
            .flex()
            .flex_col()
            .w(SIDEBAR_WIDTH)
            .flex_shrink_0()
            .h_full()
            .bg(ui.surface)
            .border_l_1()
            .border_color(ui.border)
            .child(self.files_sidebar_header(ui, cx))
            .child(self.files_sidebar_body(ui, cx))
            .into_any_element()
    }
}
