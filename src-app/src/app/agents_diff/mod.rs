//! Codex-style git diff side panel for the Agents view.
//!
//! A right-docked panel (toggled by the `layout-sidebar-right` button in the
//! environment toolbar) that shows what the agent changed in the current
//! thread's working directory: the working-tree diff against `HEAD` (staged +
//! unstaged tracked changes) plus untracked files.
//!
//! EP-001 (prd-review-redesign-2026-Q3.md, US-001/US-002): the dock no longer
//! has its own diff renderer or unified-diff parser. It renders through the
//! exact same path as the full-screen Review view ([`crate::diff`]): the shared
//! git pipeline ([`crate::diff::compute_head_diff`]), the shared row model
//! ([`crate::diff::build_display_rows`] / `build_split_rows`) and the shared
//! direct-paint [`crate::diff::DiffElement`] hosted in an `overflow_y_scroll`
//! div. The dock keeps the cheap HEAD-relative semantic (the right "what did the
//! agent just touch" base, vs the Review view's `merge-base(HEAD, base)`), but
//! shares everything else — so a visual change to the diff body is made once.
//!
//! Split (US-004) into seams: [`git`] (off-thread build), [`model`]
//! ([`AgentsDiffData`] + layout constants) and [`render`] (chrome render
//! helpers). This module owns the [`PaneFlowApp`] panel orchestration: the
//! open/refresh/collapse lifecycle, the panel + body render, and the body click.

mod git;
mod model;
mod render;

pub(crate) use model::{AGENTS_DIFF_PANEL_WIDTH, AgentsDiffData, agents_diff_count_colors};
pub(crate) use render::render_agents_diff_toggle_button;

use std::path::Path;

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use self::git::build_agents_diff;
use self::model::{AGENTS_DIFF_PANEL_MAX_WIDTH, AGENTS_DIFF_PANEL_MIN_WIDTH};
use self::render::{
    diff_panel_centered, render_diff_files_toolbar, render_diff_panel_header,
    render_diff_resize_handle,
};
use crate::PaneFlowApp;
use crate::diff::{DiffBody, DiffElement, palette, row_at_offset};

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
            cx.notify();
            return;
        }
        self.agents_view.agents_diff = Some(AgentsDiffData::loading(cwd.clone()));
        cx.notify();

        // Capture the theme on the main thread (the syntax pass needs it) and
        // move it into the worker, exactly as the Review view does.
        let theme = crate::theme::active_theme();
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let result = smol::unblock({
                    let cwd = cwd.clone();
                    move || build_agents_diff(&cwd, theme)
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
                        // Read the live collapse set (it may have changed during
                        // the async build) so the first paint honors it.
                        let collapsed = app.agents_view.agents_diff_collapsed.clone();
                        app.agents_view.agents_diff = Some(match result {
                            Ok(built) => AgentsDiffData::loaded(cwd.clone(), built, &collapsed),
                            Err(err) => AgentsDiffData::message(cwd.clone(), err),
                        });
                        cx.notify();
                    })
                });
            },
        )
        .detach();
    }

    /// Re-derive the cached collapse-filtered display rows after a collapse /
    /// split change (no git work — just re-filters the retained full rows).
    fn recompute_agents_diff_display(&mut self) {
        let collapsed = self.agents_view.agents_diff_collapsed.clone();
        if let Some(data) = self.agents_view.agents_diff.as_mut() {
            data.recompute(&collapsed);
        }
    }

    /// Fold / unfold a single file in the diff dock (click on its header row).
    pub(crate) fn toggle_diff_file_collapsed(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.agents_view.agents_diff_collapsed.remove(&path) {
            self.agents_view.agents_diff_collapsed.insert(path);
        }
        self.recompute_agents_diff_display();
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
        self.recompute_agents_diff_display();
        cx.notify();
    }

    /// Switch the diff dock between unified and split views. No-op when already in
    /// the requested mode.
    pub(crate) fn set_agents_diff_split(&mut self, split: bool, cx: &mut Context<Self>) {
        if self.agents_view.agents_diff_split == split {
            return;
        }
        self.agents_view.agents_diff_split = split;
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
            .relative()
            .w(px(self.agents_view.agents_diff_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(ui.base)
            .border_l_1()
            .border_color(ui.border)
            .child(render_diff_resize_handle(ui, cx))
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// Apply a live resize drag: set the dock width so its left edge tracks the
    /// cursor. Driven by the Agents main area's `on_mouse_move` (a full-height
    /// capture surface, so the drag survives the cursor leaving the dock for the
    /// terminal column beside it). No-op when no drag is in progress.
    pub(crate) fn drag_agents_diff_resize(&mut self, cursor_x: f32, cx: &mut Context<Self>) {
        if let Some((anchor_x, anchor_w)) = self.agents_view.agents_diff_resize {
            // The panel docks right and the handle is on its left edge, so
            // dragging left (cursor_x shrinks) widens the dock.
            let delta = anchor_x - cursor_x;
            self.agents_view.agents_diff_width =
                (anchor_w + delta).clamp(AGENTS_DIFF_PANEL_MIN_WIDTH, AGENTS_DIFF_PANEL_MAX_WIDTH);
            cx.notify();
        }
    }

    /// End a diff-dock resize drag (mouse up / button released mid-move). Returns
    /// whether a drag was actually in progress, so the caller can skip a
    /// redundant notify.
    pub(crate) fn end_agents_diff_resize(&mut self, cx: &mut Context<Self>) -> bool {
        if self.agents_view.agents_diff_resize.take().is_some() {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// The diff body: a thin files toolbar over the shared [`DiffElement`] in an
    /// `overflow_y_scroll` host (the same render path as the Review view). Empty,
    /// loading and error states render a centered placeholder instead.
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
        if data.file_count == 0 {
            return diff_panel_centered("icons/check.svg", "No uncommitted changes.", ui);
        }

        let entity = cx.entity();
        let collapsed = self.agents_view.agents_diff_collapsed.clone();
        let split = self.agents_view.agents_diff_split;
        let toolbar = render_diff_files_toolbar(data, &collapsed, ui, &entity);

        // Collapse-filtered rows + cached layout inputs (recomputed only on a
        // collapse / split change), handed to the direct-paint element.
        let body = if split {
            DiffBody::Split {
                rows: data.disp_split.clone(),
                offsets: data.disp_split_offsets.clone(),
                max_line_no: data.disp_split_max_no,
            }
        } else {
            DiffBody::Unified {
                rows: data.disp_unified.clone(),
                offsets: data.disp_unified_offsets.clone(),
                max_line_no: data.disp_unified_max_no,
            }
        };
        let pal = palette(ui);
        let scroll = self.agents_view.agents_diff_scroll.clone();

        // Custom direct-paint element hosted in an overflow-scroll div: the
        // element reports full content height; the div clips/scrolls and supplies
        // the viewport clip the element culls against. A body click maps its Y to
        // a row and toggles that file's collapse if it landed on a file header.
        let element = div()
            .id("agents-diff-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .on_click(cx.listener(|this, ev: &ClickEvent, _w, cx| {
                this.handle_agents_diff_body_click(ev, cx);
            }))
            .child(DiffElement::new(body, pal));

        div()
            .id("agents-diff-body")
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(element)
            .into_any_element()
    }

    /// Map a body click to a row and, if it landed on a file header, toggle that
    /// file's collapse. Mirrors the Review view's header-collapse path (the dock
    /// has no click-to-ask, so a non-header click is a no-op).
    fn handle_agents_diff_body_click(&mut self, ev: &ClickEvent, cx: &mut Context<Self>) {
        let split = self.agents_view.agents_diff_split;
        let path = {
            let Some(data) = self.agents_view.agents_diff.as_ref() else {
                return;
            };
            let bounds = self.agents_view.agents_diff_scroll.bounds();
            let y = ev.position().y;
            if y < bounds.top() || y > bounds.bottom() {
                return;
            }
            let target =
                f32::from(y - bounds.top() - self.agents_view.agents_diff_scroll.offset().y)
                    .max(0.0);
            let offsets = if split {
                &data.disp_split_offsets
            } else {
                &data.disp_unified_offsets
            };
            let Some(row) = row_at_offset(offsets, target) else {
                return; // click past the last row
            };
            let anchors = if split {
                &data.disp_anchors_split
            } else {
                &data.disp_anchors_unified
            };
            anchors
                .iter()
                .find(|(_, i)| *i == row)
                .map(|(p, _)| p.clone())
        };
        let Some(path) = path else {
            return; // not a file header — nothing to collapse
        };
        self.toggle_diff_file_collapsed(path, cx);
    }
}
