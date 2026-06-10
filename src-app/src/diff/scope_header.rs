//! US-012 (prd-git-diff-mode-2026-Q3.md): the app-level scope-selector header
//! for Git Diff mode. A trigger shows the active scope and opens a popover
//! listing Project / Multi-project / Worktree (a check marks the active one);
//! choosing one rebuilds the mounted view. The base-ref picker deliberately
//! stays in the `DiffView` toolbar (it is single-repo state owned by the view),
//! so it is not duplicated here.
//!
//! This renders `PaneFlowApp` (app-level) state, so the `impl` lives on
//! `PaneFlowApp` even though the file sits in the `diff` module next to the
//! other scope types.

use crate::PaneFlowApp;
use crate::diff::DiffScope;
use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, deferred, div, prelude::*, px, svg,
};

impl PaneFlowApp {
    pub(crate) fn render_scope_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let active = self.diff_mode.diff_scope;
        let open = self.diff_mode.diff_scope_picker_open;

        let trigger = div()
            .id("diff-scope-trigger")
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .h(px(22.))
            .px(px(7.))
            .rounded(px(5.))
            .when(open, |d| d.bg(ui.subtle))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(ui.text)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.diff_mode.diff_scope_picker_open = !this.diff_mode.diff_scope_picker_open;
                this.diff_mode.diff_project_picker_open = false;
                this.diff_mode.diff_worktree_picker_open = false;
                cx.notify();
            }))
            .child(
                svg()
                    .size(px(13.))
                    .flex_none()
                    .path("icons/git-pull-request.svg")
                    .text_color(ui.muted),
            )
            .child(active.label())
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            );

        let popover: Option<AnyElement> = if open {
            let mut menu = div()
                .id("diff-scope-popover")
                .occlude()
                .absolute()
                .left(px(8.))
                .top(px(30.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .p(px(4.))
                .rounded(px(6.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.diff_mode.diff_scope_picker_open {
                        this.diff_mode.diff_scope_picker_open = false;
                        cx.notify();
                    }
                }));
            for scope in DiffScope::all() {
                let is_active = scope == active;
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("diff-scope-{}", scope.label())))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(if is_active { ui.text } else { ui.muted })
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.diff_mode.diff_scope_picker_open = false;
                            this.diff_mode.diff_project_picker_open = false;
                            this.diff_mode.diff_worktree_picker_open = false;
                            if this.diff_mode.diff_scope != scope {
                                this.diff_mode.diff_scope = scope;
                                this.rebuild_diff_view(cx);
                            } else {
                                cx.notify();
                            }
                        }))
                        .child(
                            div()
                                .flex_none()
                                .w(px(12.))
                                .text_color(ui.accent)
                                .child(if is_active { "✓" } else { "" }),
                        )
                        .child(scope.label()),
                );
            }
            // Paint in the top layer: as a plain `.absolute()` child the popover
            // is painted before — and thus UNDER — the diff body (the later
            // sibling in `render_diff_main`), so it was invisible. `deferred`
            // hoists it above everything (the pattern every other Paneflow menu
            // uses); `.occlude()` stops clicks falling through to the body.
            Some(deferred(menu).with_priority(4).into_any_element())
        } else {
            None
        };

        // Project selector — only for the single-repo scopes (Project /
        // Worktree). Multi-project has its own repo tab bar, so it owns repo
        // switching there. This lets the user pick *which* open workspace's repo
        // the diff follows from inside Diff mode (it routes through
        // `select_workspace`, the same path `Ctrl+1-9` uses), instead of being
        // stuck on whatever workspace happened to be active on entry.
        let show_project = active != DiffScope::MultiProject;
        let project_open = self.diff_mode.diff_project_picker_open;
        let project_label = self
            .workspaces
            .get(self.active_idx)
            .and_then(|ws| ws.repo_root.as_ref())
            .and_then(|r| r.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "No project".to_string());

        let project_trigger = div()
            .id("diff-project-trigger")
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .h(px(22.))
            .px(px(7.))
            .rounded(px(5.))
            .when(project_open, |d| d.bg(ui.subtle))
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(ui.text)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.diff_mode.diff_project_picker_open = !this.diff_mode.diff_project_picker_open;
                this.diff_mode.diff_scope_picker_open = false;
                this.diff_mode.diff_worktree_picker_open = false;
                cx.notify();
            }))
            .child(
                svg()
                    .size(px(13.))
                    .flex_none()
                    .path("icons/folder.svg")
                    .text_color(ui.muted),
            )
            .child(project_label)
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/chevron-down.svg")
                    .text_color(ui.muted),
            );

        let project_popover: Option<AnyElement> = if project_open {
            let mut menu = div()
                .id("diff-project-popover")
                .occlude()
                .absolute()
                .left(px(0.))
                .top(px(28.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .p(px(4.))
                .rounded(px(6.))
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.diff_mode.diff_project_picker_open {
                        this.diff_mode.diff_project_picker_open = false;
                        cx.notify();
                    }
                }));
            // Every open workspace that resolves to a git repo (worktrees of one
            // repo each list separately, disambiguated by their branch).
            let repo_workspaces: Vec<(usize, String, String)> = self
                .workspaces
                .iter()
                .enumerate()
                .filter_map(|(idx, ws)| {
                    let root = ws.repo_root.as_ref()?;
                    let name = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string());
                    Some((idx, name, ws.git_branch.clone()))
                })
                .collect();
            if repo_workspaces.is_empty() {
                menu = menu.child(
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("No git projects open"),
                );
            } else {
                for (idx, name, branch) in repo_workspaces {
                    let is_active = idx == self.active_idx;
                    menu = menu.child(
                        div()
                            .id(SharedString::from(format!("diff-project-{idx}")))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(4.))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .when(is_active, |d| d.bg(ui.subtle))
                            .hover(|s| {
                                let ui = crate::theme::ui_colors();
                                s.bg(ui.subtle)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.diff_mode.diff_project_picker_open = false;
                                // Routes through the standard workspace switch
                                // (re-roots files tree, saves session, rebuilds
                                // the diff via `reconcile_diff_after_workspace_change`).
                                this.select_workspace(idx, window, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(12.))
                                    .text_color(ui.accent)
                                    .child(if is_active { "✓" } else { "" }),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(if is_active { ui.text } else { ui.muted })
                                    .child(name),
                            )
                            .when(!branch.is_empty(), |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .text_color(ui.muted)
                                        .child(format!("· {branch}")),
                                )
                            }),
                    );
                }
            }
            Some(deferred(menu).with_priority(4).into_any_element())
        } else {
            None
        };

        // Branches multi-select — Worktree scope only. Lets the user CHOOSE which
        // of the repo's worktrees show as columns (default: all). Unchosen
        // branches are never diffed (the chosen set filters `rebuild_diff_view`).
        let repo_root = self
            .workspaces
            .get(self.active_idx)
            .and_then(|ws| ws.repo_root.clone());
        let show_branches = active == DiffScope::Worktree && repo_root.is_some();
        let branches_open = self.diff_mode.diff_worktree_picker_open;
        let (branches_trigger, branches_popover): (Option<AnyElement>, Option<AnyElement>) =
            match repo_root.clone().filter(|_| show_branches) {
                Some(root) => {
                    let label = match self.diff_mode.diff_chosen_worktrees.get(&root) {
                        Some(s) => format!("{} branches", s.len()),
                        None => "All branches".to_string(),
                    };
                    let trig_root = root.clone();
                    let trigger = div()
                        .id("diff-branches-trigger")
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.))
                        .h(px(22.))
                        .px(px(7.))
                        .rounded(px(5.))
                        .when(branches_open, |d| d.bg(ui.subtle))
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(ui.text)
                        .hover(|s| {
                            let ui = crate::theme::ui_colors();
                            s.bg(ui.subtle)
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                            this.diff_mode.diff_worktree_picker_open =
                                !this.diff_mode.diff_worktree_picker_open;
                            this.diff_mode.diff_scope_picker_open = false;
                            this.diff_mode.diff_project_picker_open = false;
                            if this.diff_mode.diff_worktree_picker_open {
                                this.refresh_diff_available_worktrees(trig_root.clone(), cx);
                            }
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .size(px(13.))
                                .flex_none()
                                .path("icons/git-branch.svg")
                                .text_color(ui.muted),
                        )
                        .child(label)
                        .child(
                            svg()
                                .size(px(12.))
                                .flex_none()
                                .path("icons/chevron-down.svg")
                                .text_color(ui.muted),
                        );

                    let popover: Option<AnyElement> = if branches_open {
                        let mut menu = div()
                            .id("diff-branches-popover")
                            .occlude()
                            .absolute()
                            .left(px(0.))
                            .top(px(28.))
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .p(px(4.))
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .rounded(px(6.))
                            .bg(ui.overlay)
                            .border_1()
                            .border_color(ui.border)
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                if this.diff_mode.diff_worktree_picker_open {
                                    this.diff_mode.diff_worktree_picker_open = false;
                                    cx.notify();
                                }
                            }));
                        if self.diff_mode.diff_available_worktrees.is_empty() {
                            menu = menu.child(
                                div()
                                    .px(px(8.))
                                    .py(px(4.))
                                    .text_size(px(12.))
                                    .text_color(ui.muted)
                                    .child("Loading worktrees…"),
                            );
                        } else {
                            for w in &self.diff_mode.diff_available_worktrees {
                                let path_str = w.path.to_string_lossy().into_owned();
                                let chosen = self.diff_worktree_is_chosen(&root, &path_str);
                                let row_root = root.clone();
                                let row_path = path_str.clone();
                                let dir_tail = w
                                    .path
                                    .parent()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_default();
                                menu = menu.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "diff-branch-opt-{path_str}"
                                        )))
                                        .px(px(8.))
                                        .py(px(4.))
                                        .rounded(px(4.))
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(7.))
                                        .cursor_pointer()
                                        .hover(|s| {
                                            let ui = crate::theme::ui_colors();
                                            s.bg(ui.subtle)
                                        })
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, _w, cx| {
                                                this.toggle_chosen_worktree(
                                                    row_root.clone(),
                                                    row_path.clone(),
                                                    cx,
                                                );
                                            },
                                        ))
                                        .child(
                                            div()
                                                .flex_none()
                                                .w(px(13.))
                                                .text_size(px(12.))
                                                .text_color(ui.accent)
                                                .child(if chosen { "✓" } else { "" }),
                                        )
                                        .child(
                                            div()
                                                .flex_none()
                                                .text_size(px(12.))
                                                .text_color(if chosen { ui.text } else { ui.muted })
                                                .child(w.branch.clone()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.))
                                                .text_color(ui.muted)
                                                .child(dir_tail),
                                        ),
                                );
                            }
                        }
                        Some(deferred(menu).with_priority(4).into_any_element())
                    } else {
                        None
                    };
                    (Some(trigger.into_any_element()), popover)
                }
                None => (None, None),
            };

        // Breadcrumb FRAGMENT — scope › project › branches. No bar of its own
        // (no height / bg / padding): it is INJECTED into the single unified
        // toolbar (DiffView toolbar in single-repo scopes, the repo-tab strip
        // in Multi-project) via the `scope_slot` push, so the whole Diff mode
        // has exactly one row of chrome.
        div()
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .flex_none()
            .child(trigger)
            .children(popover)
            .when(show_project, |d| {
                d.child(
                    svg()
                        .size(px(13.))
                        .flex_none()
                        .path("icons/chevron-right.svg")
                        .text_color(ui.muted),
                )
                .child(
                    // Relative wrapper so the popover anchors directly under the
                    // project trigger (its x depends on the scope chip width).
                    div()
                        .relative()
                        .child(project_trigger)
                        .children(project_popover),
                )
            })
            .when(show_branches, |d| {
                d.child(
                    svg()
                        .size(px(13.))
                        .flex_none()
                        .path("icons/chevron-right.svg")
                        .text_color(ui.muted),
                )
                .child(
                    // Relative wrapper so the branches popover anchors under its
                    // own trigger (scope › project › branches).
                    div()
                        .relative()
                        .children(branches_trigger)
                        .children(branches_popover),
                )
            })
            .into_any_element()
    }
}
