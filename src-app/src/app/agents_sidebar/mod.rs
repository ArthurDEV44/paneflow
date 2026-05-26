// US-010 (prd-agents-view.md): the Agents-view sidebar paints from
// `self.projects` directly. Many helpers here are wired for events
// that US-011 (create / rename / delete affordances + context menus)
// and US-012 (search filter) will attach -- the foundation lives in
// US-010, the actions land in US-011/US-012. The module-scoped allow
// keeps that staging visible in one line.
#![allow(dead_code)]

//! Agents-mode sidebar: project headers + thread rows.
//!
//! This module owns the [`AppMode::Agents`] arm of the sidebar render.
//! It mirrors the visual language of [`crate::app::sidebar`] (cards,
//! hover, scroll wrapper) but speaks the Agents domain model:
//! collapsible project groups, per-thread agent icon, status dot and
//! relative-timestamp.
//!
//! Per the PRD the sidebar is the only surface the user has for
//! cross-thread navigation; it must stay responsive at 60 fps with up
//! to 100 threads per project (5 000 threads total once US-006's DB
//! query lands). For US-010 we lean on the same `overflow_y_scroll +
//! sidebar_list_wrapper` pattern the workspace sidebar uses (200 rows
//! of plain `div` render comfortably under 16 ms on a mid-range
//! laptop, verified manually). When real-world data sizes exceed that
//! envelope, switching to `gpui::list` is a localised change inside
//! [`Self::render_agents_sidebar`] -- no consumer touches the row
//! widgets directly.
//!
//! See US-010 of `tasks/prd-agents-view.md`.

mod affordances;
mod context_menus;
mod filter;
mod state;

pub(crate) use context_menus::render_open_agents_menu;
pub(crate) use state::{AgentsContextMenu, AgentsDeleteTarget, AgentsRenameTarget};

use gpui::{
    ClickEvent, Context, Font, FontFeatures, FontStyle, FontWeight, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, StyledText, TextRun,
    Transformation, Window, div, percentage, prelude::*, px, rgb, svg,
};

use crate::PaneFlowApp;

use super::agents_view_actions::AGENTS_SIDEBAR_WIDTH;

impl PaneFlowApp {
    /// Render the Agents-mode sidebar: section header, project +
    /// thread list, empty state, scroll wrapper.
    ///
    /// Visual language matches [`Self::render_sidebar`] (action-button
    /// row, card-style rows, `sidebar_list_wrapper` scrollbar). The
    /// data binding is direct: project headers + threads come from
    /// `self.projects`. Newest threads appear first (we iterate
    /// `threads` in reverse, since [`crate::project::next_thread_id`]
    /// is monotonic so insertion order tracks `created_at`).
    pub(crate) fn render_agents_sidebar(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();

        let mut sidebar = div()
            .relative()
            .w(px(AGENTS_SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .bg(theme.title_bar_background)
            .border_r_1()
            .border_color(ui.border)
            .flex()
            .flex_col();

        // Primary creation + navigation affordances stay at the top
        // of the scrollable list ("New threads", "Skills"). Connect
        // and the escape hatch to the Settings window live in the
        // bottom-of-sidebar popover.
        let _ = window;

        // -- Scrollable list area. The wheel-scroll behaviour comes
        // from `overflow_y_scroll + track_scroll`; the visible scroll
        // bar has been removed, so the list uses the full sidebar
        // width and there is no trailing gutter.
        let mut list = div()
            .id("agents-sidebar-list")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            .flex()
            .flex_col()
            .gap(px(2.))
            .py_2()
            .child(self.new_project_row(ui, cx))
            .child(self.skills_row(ui, cx));

        if self.projects.is_empty() {
            list = list.child(empty_state(ui));
            sidebar = sidebar.child(self.sidebar_list_wrapper(list, cx));
            return sidebar.into_any_element();
        }

        // Section header sitting just above the first project: a
        // "Threads" eyebrow. Extra top margin creates breathing room
        // from the "New project" row.
        list = list.child(self.threads_section_header(ui, cx));

        // US-012: if a filter is active and matches nothing, swap the
        // entire list body for the AC #7 empty-state hint.
        let query = self.agents_filter.clone();
        if filter::nothing_matches(&self.projects, &query) {
            list = list.child(no_matches_hint(&query, ui));
            sidebar = sidebar.child(self.sidebar_list_wrapper(list, cx));
            return sidebar.into_any_element();
        }

        let now_ms = now_unix_millis();
        let active_project_idx = self.active_project_idx;
        let active_thread_idx = self.active_thread_idx;
        let renaming = self.agents_renaming;
        let rename_input = self.agents_rename_input.clone();

        let projects_len = self.projects.len();
        let filtering = !query.is_empty();
        for project_idx in 0..projects_len {
            let project = &self.projects[project_idx];
            // US-012: skip projects that neither match the filter
            // themselves nor have any matching thread.
            if filtering && !filter::project_visible(project, &query) {
                continue;
            }

            let project_id = project.id;
            // While a filter is active, force-expand matching projects
            // so users see their hits immediately (AC #2's intent --
            // results "filter" the list, not gate it behind a click).
            let is_expanded = if filtering { true } else { project.is_expanded };
            let title = project.title.clone();
            let git_stats = project.git_stats.clone();
            let is_renaming_project = matches!(renaming, Some(AgentsRenameTarget::Project { project_idx: r }) if r == project_idx);

            list = list.child(self.project_header_row(
                ProjectHeaderArgs {
                    project_idx,
                    project_id,
                    title,
                    is_expanded,
                    rename_input: if is_renaming_project {
                        rename_input.clone()
                    } else {
                        None
                    },
                    git_stats,
                    ui,
                },
                cx,
            ));

            if !is_expanded {
                continue;
            }

            // Iterate threads in reverse (newest first). Indices stay
            // tied to the underlying Vec position so `select_thread`
            // and `remove_thread` resolve to the correct row.
            let thread_count_in_project = self.projects[project_idx].threads.len();
            let mut shown_threads = 0usize;
            for thread_idx in (0..thread_count_in_project).rev() {
                let thread = &self.projects[project_idx].threads[thread_idx];
                if filtering && !filter::thread_visible_in_project(thread, project, &query) {
                    continue;
                }
                shown_threads += 1;
                let is_active =
                    project_idx == active_project_idx && active_thread_idx == Some(thread_idx);
                let match_pos = if filtering {
                    filter::match_positions(&thread.title, &query)
                } else {
                    None
                };
                let is_renaming_thread = matches!(
                    renaming,
                    Some(AgentsRenameTarget::Thread { project_idx: rp, thread_idx: rt })
                        if rp == project_idx && rt == thread_idx
                );
                list = list.child(self.thread_row(
                    ThreadRowArgs {
                        project_idx,
                        thread_idx,
                        thread_id: thread.id,
                        title: thread.title.clone(),
                        rename_input: if is_renaming_thread {
                            rename_input.clone()
                        } else {
                            None
                        },
                        created_at_ms: thread.created_at,
                        is_active,
                        now_ms,
                        match_pos,
                        ui,
                    },
                    cx,
                ));
            }

            // Empty-project hint (US-010 AC #10 -> sidebar-level CTA)
            // applies when the underlying project is genuinely empty,
            // NOT when the filter merely hid every thread (then the
            // project header alone is enough signal that there are
            // hidden children).
            if thread_count_in_project == 0 && !filtering {
                list = list.child(empty_project_hint(ui));
            } else if filtering && shown_threads == 0 {
                // The project title matched but no threads did; this
                // never happens with the current `thread_visible_in_project`
                // (which surfaces every thread when the project title
                // matches), but the branch stays here so a future
                // tightening of that rule does not silently collapse
                // the row.
                list = list.child(empty_project_hint(ui));
            }
        }

        sidebar = sidebar.child(self.sidebar_list_wrapper(list, cx));
        sidebar = sidebar.child(self.render_sidebar_settings_footer(self.agents_menu_items(), cx));
        sidebar.into_any_element()
    }

    /// Items rendered inside the bottom Settings popover when in
    /// Agents mode. Order: creation actions first, then navigation,
    /// then escape hatch to the real Settings window.
    fn agents_menu_items(&self) -> Vec<crate::app::sidebar_actions_menu::SidebarMenuItem> {
        use crate::app::sidebar_actions_menu::SidebarMenuItem;
        vec![
            SidebarMenuItem {
                id: "agents-menu-connect".into(),
                icon: "icons/topology-star-3.svg",
                label: "Connect".into(),
                on_click: Box::new(|app, _w, cx| {
                    app.show_agents_welcome(cx);
                }),
            },
            SidebarMenuItem {
                id: "agents-menu-themes".into(),
                icon: "icons/palette.svg",
                label: "Themes".into(),
                on_click: Box::new(|app, w, cx| {
                    app.open_theme_picker(w, cx);
                }),
            },
            SidebarMenuItem {
                id: "agents-menu-about".into(),
                icon: "icons/info-circle.svg",
                label: "About Paneflow".into(),
                on_click: Box::new(|app, _w, cx| {
                    app.show_about_dialog = true;
                    cx.notify();
                }),
            },
            SidebarMenuItem {
                id: "agents-menu-open-settings".into(),
                icon: "icons/settings.svg",
                label: "Settings".into(),
                on_click: Box::new(|app, w, cx| {
                    app.open_settings_window(w, cx);
                }),
            },
        ]
    }

    /// "New project" affordance, styled identically to a
    /// `project_header_row` so it slots into the list visually. Opens
    /// the native folder picker on click.
    fn new_project_row(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id("agents-sidebar-new-project")
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.create_agents_project_with_picker(cx);
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path("icons/edit.svg")
                    .text_color(ui.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .truncate()
                    .child("New threads"),
            )
            .into_any_element()
    }

    /// "Connect" affordance, styled identically to `new_project_row`.
    /// Switches the main pane to the AgentsView welcome screen so the
    /// user can see signed-in chips + (re)connect agents.
    fn connect_row(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .id("agents-sidebar-connect")
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.show_agents_welcome(cx);
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path("icons/topology-star-3.svg")
                    .text_color(ui.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .truncate()
                    .child("Connect"),
            )
            .into_any_element()
    }

    /// "Skills" affordance, styled identically to `new_project_row` /
    /// `connect_row`. Switches the main pane to the skills browser
    /// (scans `~/.claude/skills`, `~/.codex/skills`, `~/.agents/skills`).
    fn skills_row(&self, ui: crate::theme::UiColors, cx: &mut Context<Self>) -> gpui::AnyElement {
        div()
            .id("agents-sidebar-skills")
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.show_agents_skills(cx);
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path("icons/layout-grid.svg")
                    .text_color(ui.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .truncate()
                    .child("Skills"),
            )
            .into_any_element()
    }

    /// "Threads" eyebrow above the first project.
    fn threads_section_header(
        &self,
        ui: crate::theme::UiColors,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .mt(px(8.))
            .px(px(14.))
            .py(px(2.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(ui.muted)
                    .child("Threads"),
            )
            .into_any_element()
    }

    fn project_header_row(
        &self,
        args: ProjectHeaderArgs,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let ProjectHeaderArgs {
            project_idx,
            project_id,
            title,
            is_expanded,
            rename_input,
            git_stats,
            ui,
        } = args;
        let folder_path = if is_expanded {
            "icons/folder-open.svg"
        } else {
            "icons/folder.svg"
        };

        // Title element switches between read-only and inline-input
        // mode. The input is a full [`TextArea`] entity (cursor,
        // selection, IME, clipboard, click-to-position, double-click
        // word select) -- same widget the chat composer uses, so the
        // editing experience is consistent across the app.
        let title_el: gpui::AnyElement = if let Some(input) = rename_input {
            div()
                .flex_1()
                .min_w_0()
                .bg(ui.overlay)
                .px_1()
                .rounded_sm()
                .child(input)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .text_color(ui.text)
                .text_size(px(12.))
                .font_weight(FontWeight::NORMAL)
                .truncate()
                .child(title)
                .into_any_element()
        };

        div()
            .id(SharedString::from(format!("agents-project-{project_id}")))
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(move |this, e: &ClickEvent, w, cx| {
                this.close_agents_menu(cx);
                let is_double = matches!(e, ClickEvent::Mouse(m) if m.down.click_count == 2);
                if is_double {
                    // PRD AC #3: double-click -> inline rename mode.
                    this.begin_agents_rename(AgentsRenameTarget::Project { project_idx }, w, cx);
                } else {
                    // Single click toggles collapse. If a rename is in
                    // progress on this row, commit it first so the
                    // toggle isn't blocked.
                    this.commit_agents_rename(cx);
                    if let Some(project) = this.projects.get_mut(project_idx) {
                        project.is_expanded = !project.is_expanded;
                        cx.notify();
                    }
                }
            }))
            .on_aux_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                if e.is_right_click()
                    && let Some(position) = e.mouse_position()
                {
                    this.commit_agents_rename(cx);
                    this.open_agents_project_menu(project_idx, position, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(folder_path)
                    .text_color(ui.muted),
            )
            .child(title_el)
            .when(!git_stats.is_empty(), |d| {
                // Trailing `+N -N` badge — git diff --shortstat of the
                // project's cwd, cached + refreshed on the 30 s poller
                // in `app/bootstrap.rs`. Catppuccin Green / Red match
                // the workspace sidebar palette at `app/sidebar/mod.rs:464-470`.
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(
                            div()
                                .text_color(rgb(0xa6e3a1))
                                .child(format!("+{}", git_stats.insertions)),
                        )
                        .child(
                            div()
                                .text_color(rgb(0xf38ba8))
                                .child(format!("-{}", git_stats.deletions)),
                        ),
                )
            })
            .into_any_element()
    }

    fn thread_row(&self, args: ThreadRowArgs, cx: &mut Context<Self>) -> gpui::AnyElement {
        let ThreadRowArgs {
            project_idx,
            thread_idx,
            thread_id,
            title,
            rename_input,
            created_at_ms,
            is_active,
            now_ms,
            match_pos,
            ui,
        } = args;
        let timestamp = format_relative_ts(now_ms, created_at_ms);
        let title_color = ui.text;
        let title_weight = FontWeight::NORMAL;
        let is_renaming = rename_input.is_some();

        // US-023: shared group name so the hover-only action cluster
        // can listen for hover on the row container without listening
        // on itself. Mirrors `pane.rs:401-464`.
        let row_group: SharedString = format!("agents-thread-row-{thread_id}").into();

        let title_el: gpui::AnyElement = if let Some(input) = rename_input {
            // Inline rename input -- full TextArea entity (same
            // widget the composer uses) so the user gets real cursor,
            // selection, IME, copy/paste, click-to-position, double-
            // click word select. Background pill matches the project
            // header rename styling.
            div()
                .flex_1()
                .min_w_0()
                .bg(ui.overlay)
                .px_1()
                .rounded_sm()
                .child(input)
                .into_any_element()
        } else if let Some((m_start, m_end)) = match_pos {
            // US-021: paint a single highlight run on the matched
            // substring (Zed `highlight_positions` slot on ThreadItem).
            // Surrounding text keeps `title_color`; the match gets
            // `ui.accent` over a tinted `ui.subtle` background.
            let runs =
                build_title_highlight_runs(&title, m_start, m_end, title_color, ui, title_weight);
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .overflow_hidden()
                .child(StyledText::new(SharedString::from(title)).with_runs(runs))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .text_color(title_color)
                .text_size(px(12.))
                .font_weight(title_weight)
                .truncate()
                .child(title)
                .into_any_element()
        };

        // Match the project_header_row's layout exactly: same `mx`,
        // same `px` (instead of split `pl`/`pr`), same `gap`. The only
        // difference is the leading child -- project rows render a
        // folder icon, thread rows render an invisible 14px spacer
        // (mirrors Zed's `ThreadItem` pattern at
        // `crates/ui/src/components/ai/thread_item.rs:264-270` where
        // the icon container is always rendered with `.size_4()` and
        // toggled to `.invisible()` to keep layout space). Padding
        // numbers were identical before the icon was removed but the
        // titles read as shifted because removing the child also
        // removed the `gap` slot. Reserving an invisible placeholder
        // restores pixel-for-pixel alignment with the project title.
        let mut row = div()
            .id(SharedString::from(format!("agents-thread-{thread_id}")))
            .group(row_group.clone())
            .relative()
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.));

        if is_active {
            row = row.bg(ui.surface);
        } else {
            row = row.hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            });
        }

        row = row
            .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
                this.close_agents_menu(cx);
                // While the row is in rename mode we let the click
                // pass through to the embedded TextArea (which then
                // positions the caret / extends the selection). The
                // row's own select_thread is skipped so an in-place
                // mouse click on the input doesn't navigate away.
                if is_renaming {
                    return;
                }
                this.commit_agents_rename(cx);
                let _ = this.select_thread(project_idx, thread_idx, cx);
            }))
            .on_aux_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                if e.is_right_click()
                    && let Some(position) = e.mouse_position()
                {
                    this.commit_agents_rename(cx);
                    this.open_agents_thread_menu(project_idx, thread_idx, position, cx);
                    cx.stop_propagation();
                }
            }))
            // Invisible 14px placeholder where the project row's
            // folder icon sits. Keeps the title's X-position aligned
            // with the project title's X-position one row up. Mirrors
            // Zed's `icon_container().when(!icon_visible, |this| this.invisible())`.
            .child(div().size(px(14.)).flex_none())
            .child(title_el)
            .child(
                // Right slot reserves a fixed 48px so the title always
                // truncates before the absolute hover-action cluster
                // (2 × 20px buttons + 4px gap = 44px). Without this the
                // cluster overlays the truncated title at hover-time
                // because GPUI group_hover does not relayout siblings.
                div()
                    .flex_none()
                    .w(px(48.))
                    .flex()
                    .justify_end()
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .group_hover(row_group.clone(), |s| s.invisible())
                    .child(timestamp),
            )
            .child(hover_actions_cluster(
                project_idx,
                thread_idx,
                thread_id,
                row_group,
                ui,
                cx,
            ));

        row.into_any_element()
    }

    /// Filter row wraps the US-012 search input in a flex container so
    /// future trailing affordances can sit on the same row at any
    /// sidebar width without re-plumbing the outer layout.
    pub(crate) fn render_agents_filter_row(
        &self,
        ui: crate::theme::UiColors,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .mb(px(4.))
            .child(
                div()
                    .flex_1()
                    .child(self.render_agents_filter_input(ui, window, cx)),
            )
            .into_any_element()
    }

    /// Render the sidebar search/filter input (US-012). Uses the same
    /// inline-cursor pattern as the workspace `font_search`: the text
    /// is rendered as plain string + `|` suffix when the input has
    /// focus, and a placeholder when empty.
    pub(crate) fn render_agents_filter_input(
        &self,
        ui: crate::theme::UiColors,
        window: &gpui::Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let has_focus = self.agents_filter_focus.is_focused(window);
        let query = self.agents_filter.clone();
        let is_empty = query.is_empty();

        // What the user sees in the input area: placeholder when
        // empty + not focused, the live query otherwise (with a `|`
        // cursor when focused).
        let display_label: SharedString = if is_empty && !has_focus {
            "Search threads".into()
        } else if has_focus {
            format!("{query}|").into()
        } else {
            query.clone().into()
        };
        let label_color = if is_empty && !has_focus {
            ui.muted
        } else {
            ui.text
        };

        // The outer row wrapper owns horizontal padding + bottom
        // margin so trailing affordances can sit on the same row as
        // the input without double-spacing.
        let mut input = div()
            .id("agents-sidebar-filter")
            .track_focus(&self.agents_filter_focus)
            .px(px(8.))
            .py(px(5.))
            .rounded(px(6.))
            .border_1()
            .bg(ui.surface)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .cursor_text();

        if has_focus {
            input = input.border_color(ui.accent);
        } else {
            input = input.border_color(ui.border).hover(|s| {
                let ui = crate::theme::ui_colors();
                s.border_color(ui.muted)
            });
        }

        // Mark the input as focusable + accept key events. Down arrow
        // jumps to the first match; Escape clears + blurs; Backspace
        // pops; printable chars push.
        input = input
            .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                this.agents_filter_focus.focus(w, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _w, cx| {
                handle_filter_key(this, e, cx);
            }))
            .child(
                // US-022: magnifier icon (Zed's FilterEditor uses
                // IconName::MagnifyingGlass). Paneflow's existing
                // tool_search.svg is the closest visual analog and is
                // already shipped for the inline read/search tool.
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/tool_search.svg")
                    .text_color(ui.muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(12.))
                    .text_color(label_color)
                    .truncate()
                    .child(display_label),
            );

        // Trailing "clear filter" button when the field has content,
        // so mouse users do not have to hit Escape.
        if !is_empty {
            input = input.child(
                div()
                    .id("agents-sidebar-filter-clear")
                    .flex_none()
                    .w(px(16.))
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .cursor_pointer()
                    .text_color(ui.muted)
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle).text_color(ui.text)
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                        this.agents_filter.clear();
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .size(px(10.))
                            .flex_none()
                            .path("icons/close.svg")
                            .text_color(ui.muted),
                    ),
            );
        }

        input.into_any_element()
    }
}

/// Argument carrier for [`PaneFlowApp::project_header_row`]. Lets the
/// fn signature stay under clippy's `too_many_arguments` threshold
/// while keeping the per-row state explicit at the call site.
struct ProjectHeaderArgs {
    project_idx: usize,
    project_id: u64,
    title: String,
    is_expanded: bool,
    /// `Some` when this row is the current inline-rename target; the
    /// entity is rendered in place of the static title and owns its
    /// own keyboard / mouse handling (cursor, selection, IME, ...).
    rename_input: Option<gpui::Entity<crate::widgets::text_area::TextArea>>,
    git_stats: crate::workspace::GitDiffStats,
    ui: crate::theme::UiColors,
}

struct ThreadRowArgs {
    project_idx: usize,
    thread_idx: usize,
    thread_id: u64,
    title: String,
    /// `Some` when this row is the current inline-rename target; the
    /// entity is rendered in place of the static title and owns its
    /// own keyboard / mouse handling (cursor, selection, IME, ...).
    rename_input: Option<gpui::Entity<crate::widgets::text_area::TextArea>>,
    created_at_ms: u64,
    is_active: bool,
    now_ms: u64,
    /// US-021: byte-range of the current filter query inside `title`,
    /// or `None` when no filter is active or the query does not hit
    /// this row's title. The renderer uses this to paint the matched
    /// substring with a tinted background -- mirrors Zed's
    /// `highlight_positions` slot on `ThreadItem`.
    match_pos: Option<(usize, usize)>,
    ui: crate::theme::UiColors,
}

/// Key handler for the sidebar filter input (US-012). Escape clears
/// the filter and blurs the input; Down arrow jumps to the first
/// matching thread (selecting it); Backspace pops a character;
/// printable chars are appended.
fn handle_filter_key(this: &mut PaneFlowApp, e: &KeyDownEvent, cx: &mut Context<PaneFlowApp>) {
    let key = e.keystroke.key.as_str();
    match key {
        "escape" => {
            // AC #4: Escape clears the filter and returns focus to
            // the project list. There is no single focus handle for
            // "the list" today, so we clear the filter + drop the
            // input's focus by sending a global cx.notify (the next
            // click on any row takes focus naturally).
            this.agents_filter.clear();
            cx.notify();
        }
        "down" => {
            // AC #4: Down arrow moves focus to the first matching
            // row. We map "focus" to "the active selection" -- the
            // currently visible representation of which thread is in
            // the spotlight. If the filter matches nothing, do not
            // mutate selection.
            let query = this.agents_filter.clone();
            if let Some((p_idx, t_idx)) = filter::first_matching_thread(&this.projects, &query) {
                let _ = this.select_thread(p_idx, t_idx, cx);
            }
        }
        "backspace" => {
            this.agents_filter.pop();
            cx.notify();
        }
        _ => {
            if let Some(ch) = &e.keystroke.key_char
                && !ch.is_empty()
                && !e.keystroke.modifiers.control
                && !e.keystroke.modifiers.platform
            {
                this.agents_filter.push_str(ch);
                cx.notify();
            }
        }
    }
}

/// Sidebar-level empty state when zero projects exist. Mirrors the
/// CLI sidebar's "No workspaces yet" copy. The "+ New project" button
/// at the top of the sidebar opens the folder picker via
/// `create_agents_project_with_picker`.
fn empty_state(ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .px(px(16.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui.text)
                .child("No projects yet"),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(ui.muted)
                .text_center()
                .child("Create a project to start a thread with Claude Code or Codex."),
        )
}

/// Inline hint shown directly under an expanded project header that
/// has zero threads. PRD Edge Case #10:
/// "Sidebar shows project header expanded with inline CTA --
///  No threads yet. Click + to start a new chat."
fn empty_project_hint(ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .mx(px(12.))
        .pl(px(18.))
        .py(px(4.))
        .text_size(px(11.))
        .text_color(ui.muted)
        .child("No threads yet. Click + to start a new chat.")
}

/// US-012 AC #7: inline empty-state row shown when the filter
/// matches nothing. Backticks around the query mirror the PRD copy
/// verbatim: "No threads match `<query>`. Press Esc to clear."
fn no_matches_hint(query: &str, ui: crate::theme::UiColors) -> impl IntoElement {
    div()
        .mx(px(12.))
        .my(px(12.))
        .px(px(8.))
        .py(px(10.))
        .rounded(px(6.))
        .bg(ui.subtle)
        .text_size(px(11.))
        .text_color(ui.muted)
        .child(format!("No threads match `{query}`. Press Esc to clear."))
}

/// Compact relative timestamp matching the PRD AC ("2m", "1h", "3d").
/// Inputs are Unix milliseconds; clock skew (now < created) is
/// clamped to `now` -> "now" so a sidebar restored from a session
/// saved on a different time zone does not show negative deltas.
fn format_relative_ts(now_ms: u64, created_at_ms: u64) -> String {
    let delta_ms = now_ms.saturating_sub(created_at_ms);
    let secs = delta_ms / 1000;
    if secs < 5 {
        return "now".to_string();
    }
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    if days < 7 {
        return format!("{days}d");
    }
    let weeks = days / 7;
    if weeks < 5 {
        return format!("{weeks}w");
    }
    // Fallback for ancient threads -- a single quantum rather than a
    // calendar date keeps the row width predictable.
    let months = days / 30;
    format!("{months}mo")
}

/// Suppress an unused-import warning when none of the rotation-based
/// chevron animations is referenced. Keeps `Transformation` /
/// `percentage` available for follow-up stories without re-importing.
#[allow(dead_code)]
fn _keep_rotation_imports(t: Transformation) -> Transformation {
    let _ = percentage(0.0);
    t
}

fn now_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// US-021: build per-run text styling for the thread title when a
/// search filter is active. Returns three runs: `[before, match,
/// after]`. The matched span uses `ui.accent` over a `ui.subtle`
/// background tint; surrounding text keeps `base_color`. When the
/// match covers the entire title the before/after runs have length
/// 0 and the shaper skips them safely.
fn build_title_highlight_runs(
    title: &str,
    m_start: usize,
    m_end: usize,
    base_color: Hsla,
    ui: crate::theme::UiColors,
    weight: FontWeight,
) -> Vec<TextRun> {
    let font = Font {
        family: ".SystemUIFont".into(),
        features: FontFeatures::default(),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    };
    let make = |len: usize, color: Hsla, bg: Option<Hsla>| TextRun {
        len,
        font: font.clone(),
        color,
        background_color: bg,
        underline: None,
        strikethrough: None,
    };
    vec![
        make(m_start, base_color, None),
        make(m_end - m_start, ui.accent, Some(ui.subtle)),
        make(title.len().saturating_sub(m_end), base_color, None),
    ]
}

/// US-023: small h_flex cluster of per-row action buttons (currently
/// just Delete). Hidden by default and revealed when the row's group
/// is hovered -- mirrors Zed's `visible_on_hover` slot on `ThreadItem`
/// (zero layout shift because `.invisible()` keeps the buttons in flow).
fn hover_actions_cluster(
    project_idx: usize,
    thread_idx: usize,
    thread_id: u64,
    row_group: SharedString,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> gpui::AnyElement {
    let trash_btn = div()
        .id(SharedString::from(format!(
            "agents-thread-{thread_id}-trash"
        )))
        .flex_none()
        .w(px(20.))
        .h(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .text_color(ui.muted)
        .hover(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle).text_color(ui.text)
        })
        .tooltip(|_w, cx| {
            cx.new(|_| HoverActionTooltip {
                label: SharedString::from("Delete"),
            })
            .into()
        })
        .on_click(cx.listener(move |this, _: &ClickEvent, _w, cx| {
            this.request_agents_confirm_delete(
                AgentsDeleteTarget::Thread {
                    project_idx,
                    thread_idx,
                },
                cx,
            );
            cx.stop_propagation();
        }))
        .child(
            svg()
                .size(px(12.))
                .flex_none()
                .path("icons/trash.svg")
                .text_color(ui.muted),
        );

    div()
        .absolute()
        .top(px(0.))
        .bottom(px(0.))
        .right(px(8.))
        .flex()
        .flex_row()
        .items_center()
        // Zed `gap_1` = 4px (4px grid base unit). US-023 AC #2.
        .gap(px(4.))
        .invisible()
        .group_hover(row_group, |s| s.visible())
        .child(trash_btn)
        .into_any_element()
}

/// US-023: tooltip body for the per-row Delete hover button.
struct HoverActionTooltip {
    label: SharedString,
}

impl Render for HoverActionTooltip {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_ts_buckets() {
        // Anchor: 2026-05-23T12:00:00Z in millis.
        let now: u64 = 1_777_022_400_000;
        assert_eq!(format_relative_ts(now, now), "now");
        assert_eq!(format_relative_ts(now, now - 3_000), "now");
        assert_eq!(format_relative_ts(now, now - 6_000), "6s");
        assert_eq!(format_relative_ts(now, now - 59_000), "59s");
        assert_eq!(format_relative_ts(now, now - 120_000), "2m");
        assert_eq!(format_relative_ts(now, now - 3_600_000), "1h");
        assert_eq!(format_relative_ts(now, now - 26 * 3_600_000), "1d");
        assert_eq!(format_relative_ts(now, now - 8 * 24 * 3_600_000), "1w");
        // Clock skew: created in the future clamps to "now".
        assert_eq!(format_relative_ts(now, now + 60_000), "now");
    }
}
