//! Sidebar rendering for `PaneFlowApp`: workspace cards, action buttons,
//! notification dropdown, and the context-menu row helpers (in the
//! [`context_menu`] submodule).
//!
//! Extracted from `main.rs` per US-025 of the src-app refactor PRD — pure
//! code-motion, behaviour unchanged. Toast utilities and sidebar-adjacent
//! types (`WorkspaceContextMenu`, `WorkspaceDrag`, `WorkspaceDragPreview`)
//! remain in `main.rs` because they cross module boundaries.

mod context_menu;

use gpui::{
    AnyElement, AppContext, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Styled, Window, deferred, div,
    prelude::*, px, rgb, svg,
};

use crate::{
    CLAUDE_SPINNER_FRAMES, CODEX_SPINNER_FRAMES, PaneFlowApp, SIDEBAR_WIDTH, WorkspaceContextMenu,
    WorkspaceDrag, WorkspaceDragPreview, ai_types,
};

impl PaneFlowApp {
    pub(crate) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();
        let mut sidebar = div()
            .relative()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .bg(theme.title_bar_background)
            .border_r_1()
            .border_color(ui.border)
            .flex()
            .flex_col();

        // All top-of-sidebar affordances (New workspace, Clear all,
        // Open Settings) moved into the bottom-of-sidebar Settings
        // popover. The top of the CLI sidebar is now empty -- see
        // `cli_menu_items` for the popover contents.
        let _ = theme;

        // Workspace list — scrollable area. Wheel-scroll comes from
        // `overflow_y_scroll + track_scroll`; the visible scroll bar
        // is gone, so the list uses the full sidebar width without a
        // trailing gutter.
        let mut list = div()
            .id("workspace-list")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.sidebar_scroll)
            .flex()
            .flex_col()
            .gap(px(6.))
            .py_2();

        if self.workspaces.is_empty() {
            list = list.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(44.))
                            .h(px(44.))
                            .rounded(px(10.))
                            .bg(ui.subtle)
                            .child(
                                svg()
                                    .size(px(20.))
                                    .flex_none()
                                    .path("icons/folder_open.svg")
                                    .text_color(ui.muted),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ui.text)
                                    .child("No workspaces yet"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(ui.muted)
                                    .child("Create one to get started"),
                            ),
                    )
                    .child(
                        div()
                            .id("empty-new-ws")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .px(px(12.))
                            .py(px(6.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .bg(ui.text)
                            .text_color(ui.base)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .hover(|s| s.opacity(0.85))
                            .on_click(cx.listener(|this, _: &ClickEvent, w, cx| {
                                this.create_workspace_with_picker(w, cx);
                            }))
                            .child(
                                svg()
                                    .size(px(12.))
                                    .flex_none()
                                    .path("icons/plus.svg")
                                    .text_color(ui.base),
                            )
                            .child("New Workspace"),
                    ),
            );
            sidebar = sidebar.child(list);
            return sidebar;
        }

        for (i, ws) in self.workspaces.iter().enumerate() {
            let is_active = i == self.active_idx;

            let title = ws.title.clone();
            // Format cwd as ~/... (collapse home dir)
            let cwd_display = {
                if !self.home_dir.is_empty() && ws.cwd.starts_with(&self.home_dir) {
                    format!("~{}", &ws.cwd[self.home_dir.len()..])
                } else {
                    ws.cwd.clone()
                }
            };

            let idx = i;
            let ws_id = ws.id;
            let ws_title: SharedString = ws.title.clone().into();

            let mut card = div()
                .id(SharedString::from(format!("ws-{i}")))
                .mx(px(8.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(is_active, |d| {
                    let ui = crate::theme::ui_colors();
                    d.bg(ui.surface)
                })
                .when(!is_active, |d| {
                    d.hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle)
                    })
                })
                .on_drag(
                    WorkspaceDrag {
                        id: ws_id,
                        title: ws_title.clone(),
                    },
                    |drag, _offset, _window, cx| {
                        cx.new(|_| WorkspaceDragPreview {
                            title: drag.title.clone(),
                        })
                    },
                )
                .on_drop(cx.listener(move |this, drag: &WorkspaceDrag, _window, cx| {
                    this.reorder_workspace(drag.id, idx, cx);
                }))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.workspace_menu_open = None;
                    this.profile_menu_open = None;
                    let is_double = matches!(e, ClickEvent::Mouse(m) if m.down.click_count == 2);
                    if is_double {
                        this.commit_rename(cx); // commit any previous rename
                        this.rename_text = this.workspaces[idx].title.clone();
                        this.renaming_idx = Some(idx);
                    } else {
                        this.commit_rename(cx);
                        this.select_workspace(idx, window, cx);
                    }
                    cx.notify();
                }))
                .on_aux_click(cx.listener(move |this, e: &ClickEvent, _window, cx| {
                    if e.is_right_click()
                        && let Some(position) = e.mouse_position()
                    {
                        this.commit_rename(cx);
                        this.profile_menu_open = None;
                        this.workspace_menu_open = Some(WorkspaceContextMenu { idx, position });
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _window, cx| {
                    if this.renaming_idx != Some(idx) {
                        return;
                    }
                    let key = e.keystroke.key.as_str();
                    match key {
                        "enter" => {
                            this.commit_rename(cx);
                            cx.notify();
                        }
                        "escape" => {
                            this.renaming_idx = None;
                            this.rename_text.clear();
                            cx.notify();
                        }
                        "backspace" => {
                            this.rename_text.pop();
                            cx.notify();
                        }
                        _ => {
                            if let Some(ch) = &e.keystroke.key_char
                                && !ch.is_empty()
                                && !e.keystroke.modifiers.control
                                && !e.keystroke.modifiers.platform
                            {
                                this.rename_text.push_str(ch);
                                cx.notify();
                            }
                        }
                    }
                }))
                .flex()
                .flex_col()
                .gap(px(5.));

            // ── Row 1: Title ──
            let title_el = if self.renaming_idx == Some(i) {
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(FontWeight::NORMAL)
                    .bg(ui.overlay)
                    .px_1()
                    .rounded_sm()
                    .child(format!("{}|", self.rename_text))
            } else {
                // Display title keeps its natural width so the
                // session pastille that follows sits flush against
                // the workspace name. No `flex_1 / min_w_0 / truncate`
                // here -- that combo causes GPUI to collapse the
                // label to "…" even with plenty of room (same bug
                // we hit on the branch label). Unusually long titles
                // will push the row width past the card; the user
                // can rename them.
                div()
                    .flex_none()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(FontWeight::NORMAL)
                    .child(title)
            };

            // Active AI session pastille — one blue dot to the right
            // of the title as soon as the workspace holds at least
            // one live `claude` or `codex` process. Source of truth is
            // `Workspace::detected_agents`, populated by walking the
            // PTY descendants (`detect_ai_processes`) so the signal
            // works even when Claude Code is launched without the
            // Paneflow IPC shim that would otherwise register the PID.
            let active_agents: Vec<&str> = {
                let mut keys: Vec<&str> = ws.detected_agents.iter().map(|s| s.as_str()).collect();
                keys.sort_unstable();
                keys
            };
            let has_active_session = !active_agents.is_empty();
            let session_tooltip: SharedString = match active_agents.as_slice() {
                [] => SharedString::default(),
                [one] => format!("{} session active", capitalize_agent(one)).into(),
                [a, b] => format!(
                    "{} + {} sessions active",
                    capitalize_agent(a),
                    capitalize_agent(b)
                )
                .into(),
                many => format!("{} active sessions", many.len()).into(),
            };

            let title_row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .child(title_el)
                .when(has_active_session, |d| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("ws-{i}-session-dot")))
                            .flex_none()
                            .w(px(7.))
                            .h(px(7.))
                            .rounded_full()
                            .bg(rgb(0x3B82F6)) // Tailwind blue-500
                            .tooltip({
                                let label = session_tooltip.clone();
                                move |_w, cx| {
                                    cx.new(|_| SidebarTooltip {
                                        label: label.clone(),
                                    })
                                    .into()
                                }
                            }),
                    )
                });

            card = card.child(title_row);

            // ── Row 2: Meta line — branch (icon + name) and diff stats on
            // the same row, separated by a muted dot. Skipped entirely
            // when the workspace is not a git repo and has no diff,
            // which keeps non-git folders to a single-row card.
            if !ws.git_branch.is_empty() || !ws.git_stats.is_empty() {
                // `flex_wrap()` lets the diff stats drop to a second
                // line if the branch name is unusually long, so the
                // branch itself is never truncated or clipped — losing
                // the branch is more confusing than gaining a row.
                let mut meta_row = div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .text_xs()
                    .text_color(ui.muted);

                if !ws.git_branch.is_empty() {
                    // Branch label keeps its natural width. GPUI's
                    // truncate + flex_shrink + min_w_0 combo rabbits
                    // the label down to "…" even when there's plenty of
                    // room, so we let the row overflow horizontally
                    // for the rare long branch name instead.
                    meta_row = meta_row
                        .child(
                            svg()
                                .size(px(11.))
                                .flex_none()
                                .path("icons/git-branch.svg")
                                .text_color(ui.muted),
                        )
                        .child(div().flex_none().child(ws.git_branch.clone()));
                }

                if !ws.git_branch.is_empty() && !ws.git_stats.is_empty() {
                    meta_row = meta_row.child(div().flex_none().text_color(ui.muted).child("·"));
                }

                if !ws.git_stats.is_empty() {
                    let ins = ws.git_stats.insertions;
                    let del = ws.git_stats.deletions;
                    meta_row = meta_row
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(0xa6e3a1)) // Catppuccin Green
                                .child(format!("+{ins}")),
                        )
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(0xf38ba8)) // Catppuccin Red
                                .child(format!("-{del}")),
                        );
                }

                card = card.child(meta_row);
            }

            // ── Row 3: Active ports. Frontend ports are clickable
            // tinted chips; non-frontend ports render as plain muted
            // `:port` text so the chip ink stays meaningful (= clickable).
            // Capped at 3 visible to keep the card height predictable;
            // overflow is condensed to a `+N` muted counter.
            if !ws.active_ports.is_empty() {
                const PORTS_VISIBLE: usize = 3;
                let mut ports_row = div().flex().flex_row().flex_wrap().gap(px(4.));

                for (pi, port) in ws.active_ports.iter().take(PORTS_VISIBLE).enumerate() {
                    let info = ws.service_labels.get(port);
                    let is_frontend = info.is_some_and(|i| i.is_frontend);
                    let label = if let Some(i) = info
                        && let Some(ref l) = i.label
                    {
                        format!("{l} :{port}")
                    } else {
                        format!(":{port}")
                    };

                    if is_frontend {
                        let url = info
                            .and_then(|i| i.url.clone())
                            .unwrap_or_else(|| format!("http://localhost:{port}"));
                        ports_row = ports_row.child(
                            div()
                                .id(SharedString::from(format!("port-{idx}-{pi}")))
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(ui.subtle)
                                .text_size(px(10.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(ui.text)
                                .cursor_pointer()
                                .hover(|s| {
                                    let ui = crate::theme::ui_colors();
                                    s.bg(ui.surface)
                                })
                                // US-011 AC4/5/6 + AC7: delegate to the
                                // `open` crate which already dispatches
                                // per-OS — `xdg-open` subprocess on
                                // Linux, `open` subprocess on macOS,
                                // and `ShellExecuteW` (Win32 API call,
                                // NOT a `cmd /C start ""` subprocess)
                                // on Windows.
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, _w, cx| {
                                        if let Err(err) = open::that(&url) {
                                            let msg = if err.kind()
                                                == std::io::ErrorKind::NotFound
                                            {
                                                "Could not open URL — install xdg-utils (Linux), or check your default browser".to_string()
                                            } else {
                                                format!("Could not open URL: {err}")
                                            };
                                            log::warn!("sidebar: open URL failed: {err}");
                                            this.show_toast(msg, cx);
                                        }
                                    },
                                ))
                                .child(label),
                        );
                    } else {
                        ports_row = ports_row
                            .child(div().text_size(px(10.)).text_color(ui.muted).child(label));
                    }
                }

                if ws.active_ports.len() > PORTS_VISIBLE {
                    ports_row = ports_row.child(
                        div()
                            .px(px(4.))
                            .text_size(px(10.))
                            .text_color(ui.muted)
                            .child(format!("+{}", ws.active_ports.len() - PORTS_VISIBLE)),
                    );
                }

                card = card.child(ports_row);
            }

            // ── Row 4: AI tool status (one row per active tool). Aggregate
            // the per-PID sessions stored on the workspace into one
            // ToolAggregate per AiTool (Claude > Codex), pick the most
            // salient state per tool (Waiting > Thinking > Finished),
            // and render the matching badge. The "+N" suffix appears
            // whenever a tool has more than one concurrent session.
            let rows = ai_types::aggregate_by_tool(ws.agent_sessions.values());
            for agg in rows {
                let extra = agg.extra_suffix();
                match agg.dominant {
                    ai_types::AgentState::Thinking => {
                        let frames: &[char] = match agg.tool {
                            ai_types::AiTool::Claude => &CLAUDE_SPINNER_FRAMES,
                            ai_types::AiTool::Codex => &CODEX_SPINNER_FRAMES,
                        };
                        let thinking_color = match agg.tool {
                            ai_types::AiTool::Claude => rgb(0xE89271), // Anthropic salmon
                            ai_types::AiTool::Codex => rgb(0x5B6CFF),  // Codex indigo
                        };
                        let angle = ws.loader_angle.get();
                        let idx = ((angle / std::f32::consts::TAU) * frames.len() as f32) as usize
                            % frames.len();
                        let spinner = frames[idx];
                        card = card.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .text_xs()
                                .text_color(thinking_color)
                                .child(
                                    div()
                                        .w(px(11.))
                                        .h(px(11.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(format!("{spinner}")),
                                )
                                .child(div().child(format!(
                                    "{} thinking…{}",
                                    agg.tool.label(),
                                    extra
                                ))),
                        );
                    }
                    ai_types::AgentState::WaitingForInput => {
                        card = card.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .self_start()
                                .px(px(6.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(rgb(0xFBBF24)) // amber warning
                                .child(
                                    svg()
                                        .size(px(11.))
                                        .flex_none()
                                        .path("icons/bell.svg")
                                        .text_color(ui.base),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ui.base)
                                        .child(format!(
                                            "{} needs input{}",
                                            agg.tool.label(),
                                            extra
                                        )),
                                ),
                        );
                    }
                    ai_types::AgentState::Finished => {
                        let done_color = rgb(0x00E08A); // neon mint
                        card = card.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(6.))
                                .text_xs()
                                .text_color(done_color)
                                .child(
                                    svg()
                                        .size(px(11.))
                                        .flex_none()
                                        .path("icons/check.svg")
                                        .text_color(done_color),
                                )
                                .child(div().child(format!("{} done{}", agg.tool.label(), extra))),
                        );
                    }
                }
            }

            // Working directory moves into a hover tooltip so the card
            // height stays tight. Users who need the path still get it
            // on hover; the visible layout no longer carries a row
            // that duplicates information already implied by the title.
            let cwd_tooltip = SharedString::from(cwd_display);
            card = card.tooltip(move |_w, cx| {
                cx.new(|_| WorkspaceCwdTooltip {
                    path: cwd_tooltip.clone(),
                })
                .into()
            });

            list = list.child(card);
        }

        // Wrap list + scrollbar in a relative flex_1 container so the
        // overlay can absolutely-position itself over the list. Mouse
        // handlers stay on the wrapper to track drag even if the cursor
        // leaves the 6px-wide thumb.
        sidebar = sidebar.child(self.sidebar_list_wrapper(list, cx));
        sidebar = sidebar.child(self.render_sidebar_settings_footer(self.cli_menu_items(), cx));
        sidebar = sidebar.child(self.render_mode_toggle(cx));
        sidebar
    }

    /// Items rendered inside the bottom Settings popover when in CLI
    /// mode. Order: creation actions first, destructive last, escape
    /// hatch to the Settings window.
    fn cli_menu_items(&self) -> Vec<crate::app::sidebar_actions_menu::SidebarMenuItem> {
        use crate::app::sidebar_actions_menu::SidebarMenuItem;
        let mut items = vec![SidebarMenuItem {
            id: "cli-menu-new-ws".into(),
            icon: "icons/plus.svg",
            label: "New workspace".into(),
            on_click: Box::new(|app, w, cx| {
                app.create_workspace_with_picker(w, cx);
            }),
        }];
        if !self.workspaces.is_empty() {
            items.push(SidebarMenuItem {
                id: "cli-menu-clear-all".into(),
                icon: "icons/trash.svg",
                label: "Close all workspaces".into(),
                on_click: Box::new(|app, _w, cx| {
                    app.close_all_workspaces(cx);
                }),
            });
        }
        items.push(SidebarMenuItem {
            id: "cli-menu-themes".into(),
            icon: "icons/palette.svg",
            label: "Themes".into(),
            on_click: Box::new(|app, w, cx| {
                app.open_theme_picker(w, cx);
            }),
        });
        items.push(SidebarMenuItem {
            id: "cli-menu-about".into(),
            icon: "icons/info-circle.svg",
            label: "About Paneflow".into(),
            on_click: Box::new(|app, _w, cx| {
                app.show_about_dialog = true;
                cx.notify();
            }),
        });
        items.push(SidebarMenuItem {
            id: "cli-menu-open-settings".into(),
            icon: "icons/settings.svg",
            label: "Settings".into(),
            on_click: Box::new(|app, w, cx| {
                app.open_settings_window(w, cx);
            }),
        });
        items
    }

    pub(crate) fn sidebar_list_wrapper(
        &self,
        list: gpui::Stateful<gpui::Div>,
        _cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        // The visible scroll bar was removed; wheel-scroll on the
        // inner `list` (driven by `overflow_y_scroll + track_scroll`)
        // is the only scrolling surface now. The wrapper still
        // exists so callers keep a stable insertion point if a
        // trailing affordance lands here later.
        div()
            .id("sidebar-list-wrapper")
            .relative()
            .flex_1()
            .flex()
            .flex_col()
            .min_h_0()
            .child(list)
    }

    /// Modal confirmation for "Close all workspaces". Same visual
    /// language as `render_agents_confirm_delete_dialog` -- backdrop
    /// dim, centred card, Cancel (subtle) + danger (red) buttons --
    /// so the two destructive guards feel like one product. Rendered
    /// from the top-level `Render` impl when
    /// `confirm_close_all_workspaces` is set.
    pub(crate) fn render_close_all_confirm_dialog(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = self.workspaces.len();
        let body = if count == 1 {
            "Close the open workspace? Unsaved terminal state will be lost.".to_string()
        } else {
            format!("Close all {count} workspaces? Unsaved terminal state will be lost.")
        };

        deferred(
            div()
                .id("close-all-confirm-backdrop")
                .occlude()
                .absolute()
                .top(px(0.))
                .left(px(0.))
                .size_full()
                .bg(gpui::black().opacity(0.45))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_close_all_workspaces(cx);
                    }),
                )
                .child(
                    div()
                        .id("close-all-confirm-dialog")
                        .occlude()
                        .w(px(360.))
                        .bg(ui.overlay)
                        .border_1()
                        .border_color(ui.border)
                        .rounded(px(10.))
                        .shadow_lg()
                        .p(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ui.text)
                                .child("Close all workspaces"),
                        )
                        .child(div().text_size(px(12.)).text_color(ui.muted).child(body))
                        .child(
                            div()
                                .mt(px(6.))
                                .flex()
                                .flex_row()
                                .justify_end()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .id("close-all-confirm-cancel")
                                        .px(px(14.))
                                        .py(px(7.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .bg(ui.subtle)
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(ui.text)
                                        .hover(|s| {
                                            let ui = crate::theme::ui_colors();
                                            s.bg(ui.surface)
                                        })
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.cancel_close_all_workspaces(cx);
                                        }))
                                        .child("Cancel"),
                                )
                                .child(
                                    div()
                                        .id("close-all-confirm-go")
                                        .px(px(14.))
                                        .py(px(7.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .bg(rgb(0xf38ba8))
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ui.base)
                                        .hover(|s| s.opacity(0.88))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                                            this.execute_close_all_workspaces(cx);
                                        }))
                                        .child("Close all"),
                                ),
                        ),
                ),
        )
        .priority(4)
        .into_any_element()
    }
}

/// Pretty-print the agent key stored in `Workspace::agent_pids`
/// (always lower-case: `"claude"`, `"codex"`) for human display in
/// tooltips. Anything unknown is rendered verbatim so a future agent
/// kind shows up readable even if we forget to add a branch here.
fn capitalize_agent(key: &str) -> &'static str {
    match key {
        "claude" => "Claude",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        _ => "AI",
    }
}

/// Lightweight tooltip body reused by sidebar affordances that just
/// need to show one short label. Mirrors the `WorkspaceCwdTooltip`
/// style minus the monospace font so prose reads naturally.
struct SidebarTooltip {
    label: SharedString,
}

impl Render for SidebarTooltip {
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

/// Tooltip body for a workspace card. Surfaces the full cwd path so
/// it can stay off-screen on the card itself (the title is enough
/// signal at idle; the path is only relevant when the user needs to
/// distinguish two workspaces with similar titles or open a shell at
/// that exact location).
struct WorkspaceCwdTooltip {
    path: SharedString,
}

impl Render for WorkspaceCwdTooltip {
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
            .font_family("monospace")
            .child(self.path.clone())
    }
}
