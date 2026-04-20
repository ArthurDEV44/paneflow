//! Sidebar rendering for `PaneFlowApp`: workspace cards, action buttons,
//! notification dropdown, and the context-menu row helpers (in the
//! [`context_menu`] submodule).
//!
//! Extracted from `main.rs` per US-025 of the src-app refactor PRD — pure
//! code-motion, behaviour unchanged. Toast utilities and sidebar-adjacent
//! types (`WorkspaceContextMenu`, `WorkspaceDrag`, `WorkspaceDragPreview`,
//! `Notification`) remain in `main.rs` because they cross module boundaries.

mod context_menu;

use gpui::{
    App, AppContext, ClickEvent, Context, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, SharedString, Styled, Window, deferred, div, prelude::*, px, rgb,
    svg,
};

use crate::{
    CLAUDE_SPINNER_FRAMES, CODEX_SPINNER_FRAMES, PaneFlowApp, SIDEBAR_WIDTH, WorkspaceContextMenu,
    WorkspaceDrag, WorkspaceDragPreview, ai_types,
};

impl PaneFlowApp {
    fn sidebar_action_btn(
        &self,
        id: &'static str,
        icon_path: &'static str,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_color(ui.muted)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle).text_color(ui.text)
            })
            .on_click(move |e, w, cx| on_click(e, w, cx))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path(icon_path)
                    .text_color(ui.muted),
            )
    }

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

        // ── Action buttons row ──
        sidebar = sidebar.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(10.))
                .py(px(6.))
                .child(
                    // Left side — section label
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.text)
                        .child("WORKSPACES"),
                )
                .child(
                    // Right side — action buttons
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.))
                        .child({
                            let has_unread = self.notifications.iter().any(|n| !n.read);
                            div()
                                .relative()
                                .child(self.sidebar_action_btn(
                                    "sidebar-bell",
                                    "icons/bell.svg",
                                    cx.listener(|this, _: &ClickEvent, _w, cx| {
                                        this.title_bar_menu_open = None;
                                        this.notif_menu_open = !this.notif_menu_open;
                                        cx.notify();
                                    }),
                                ))
                                .when(has_unread, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(2.))
                                            .right(px(2.))
                                            .w(px(6.))
                                            .h(px(6.))
                                            .rounded_full()
                                            .bg(rgb(0xf38ba8)),
                                    )
                                })
                        })
                        .child(self.sidebar_action_btn(
                            "sidebar-settings",
                            "icons/settings.svg",
                            cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_settings_window(window, cx);
                            }),
                        ))
                        .child(self.sidebar_action_btn(
                            "sidebar-new-ws",
                            "icons/plus.svg",
                            cx.listener(|this, _: &ClickEvent, w, cx| {
                                this.create_workspace_with_picker(w, cx);
                            }),
                        ))
                        .when(!self.workspaces.is_empty(), |d| {
                            d.child(self.sidebar_action_btn(
                                "sidebar-clear-all",
                                "icons/trash.svg",
                                cx.listener(|this, _: &ClickEvent, _w, cx| {
                                    this.close_all_workspaces(cx);
                                }),
                            ))
                        }),
                ),
        );

        // ── Notification dropdown menu ──
        if self.notif_menu_open {
            let mut menu = div()
                .id("notif-menu")
                .occlude()
                .absolute()
                .top(px(64.))
                .left(px(6.))
                .w(px(SIDEBAR_WIDTH - 12.))
                .max_h(px(300.))
                .overflow_y_scroll()
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .rounded(px(6.))
                .shadow_lg()
                .flex()
                .flex_col()
                .p(px(4.));

            if self.notifications.is_empty() {
                menu = menu.child(
                    div()
                        .px(px(10.))
                        .py(px(12.))
                        .text_xs()
                        .text_color(ui.muted)
                        .child("No notifications"),
                );
            } else {
                // Newest first
                for (ni, notif) in self.notifications.iter().enumerate().rev() {
                    let ws_id = notif.workspace_id;
                    let is_unread = !notif.read;
                    let notif_idx = ni;
                    menu = menu.child(
                        div()
                            .id(SharedString::from(format!("notif-{ni}")))
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .when(is_unread, |d| {
                                let ui = crate::theme::ui_colors();
                                d.bg(ui.subtle)
                            })
                            .hover(|s| {
                                let ui = crate::theme::ui_colors();
                                s.bg(ui.surface)
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                // Find workspace by stable ID
                                if let Some(idx) =
                                    this.workspaces.iter().position(|ws| ws.id == ws_id)
                                {
                                    this.select_workspace(idx, window, cx);
                                }
                                if notif_idx < this.notifications.len() {
                                    this.notifications[notif_idx].read = true;
                                }
                                this.notif_menu_open = false;
                                cx.notify();
                            }))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if is_unread { ui.text } else { ui.muted })
                                    .child(notif.workspace_title.clone()),
                            )
                            .child({
                                let msg_color = match notif.kind {
                                    ai_types::AiToolState::WaitingForInput(_) => {
                                        Hsla::from(rgb(0xf9e2af))
                                    }
                                    ai_types::AiToolState::Finished(_) => Hsla::from(rgb(0xa6e3a1)),
                                    _ => ui.muted,
                                };
                                div()
                                    .text_xs()
                                    .text_color(msg_color)
                                    .child(notif.message.clone())
                            }),
                    );
                }
            }

            sidebar = sidebar.child(deferred(menu));
        }

        // Workspace list — scrollable area
        let mut list = div()
            .id("workspace-list")
            .flex_1()
            .overflow_y_scroll()
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
                    .gap(px(10.))
                    .px(px(16.))
                    .child(
                        svg()
                            .size(px(32.))
                            .flex_none()
                            .path("icons/folder_open.svg")
                            .text_color(ui.muted),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(ui.muted)
                            .child("No workspaces yet"),
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
            let pane_count = ws.pane_count();
            let pane_label = format!(
                "{pane_count} pane{}",
                if pane_count != 1 { "s" } else { "" }
            );

            let idx = i;
            let ws_id = ws.id;
            let ws_title: SharedString = ws.title.clone().into();

            let mut card = div()
                .id(SharedString::from(format!("ws-{i}")))
                .mx(px(6.))
                .px(px(10.))
                .py(px(8.))
                .when(is_active, |d| d.bg(ui.surface))
                .rounded(px(6.))
                .cursor_pointer()
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
                    this.title_bar_menu_open = None;
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
                        this.title_bar_menu_open = None;
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
                .gap_1();

            // ── Row 1: Title + action menu ──
            let title_el = if self.renaming_idx == Some(i) {
                div()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .bg(ui.overlay)
                    .px_1()
                    .rounded_sm()
                    .child(format!("{}|", self.rename_text))
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(title)
            };
            let title_row = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .min_w_0()
                .child(title_el);

            card = card.child(title_row);

            // ── Row 2: Git branch ──
            if !ws.git_branch.is_empty() {
                card = card.child(
                    div()
                        .text_color(rgb(0x89b4fa)) // Catppuccin Blue
                        .text_xs()
                        .truncate()
                        .child(format!(" {}", ws.git_branch)),
                );
            }

            // ── Row 3: Subtitle — pane count as status ──
            card = card.child(div().text_color(ui.muted).text_xs().child(pane_label));

            // ── Row 3: Git diff stats ──
            if !ws.git_stats.is_empty() {
                let ins = ws.git_stats.insertions;
                let del = ws.git_stats.deletions;
                card = card.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            svg()
                                .size(px(14.))
                                .flex_none()
                                .path("icons/git_commit.svg")
                                .text_color(ui.muted),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.))
                                .text_xs()
                                .child(
                                    div()
                                        .text_color(rgb(0xa6e3a1)) // Catppuccin Green
                                        .child(format!("+{ins}")),
                                )
                                .child(
                                    div()
                                        .text_color(rgb(0xf38ba8)) // Catppuccin Red
                                        .child(format!("-{del}")),
                                ),
                        ),
                );
            } else if ws.is_git_repo {
                card = card.child(
                    div()
                        .text_color(ui.muted)
                        .text_xs()
                        .child("No changes detected"),
                );
            }

            // ── Row 4: Active ports — clickable URL badges ──
            if !ws.active_ports.is_empty() {
                let mut ports_row = div().flex().flex_row().flex_wrap().gap(px(4.));

                for (pi, port) in ws.active_ports.iter().take(4).enumerate() {
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
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(ui.subtle)
                                .text_size(px(11.))
                                .text_color(ui.accent)
                                .cursor_pointer()
                                .hover(|s| s.text_color(rgb(0xa0e8ff)))
                                .on_click(move |_, _, _| {
                                    let _ =
                                        std::process::Command::new("xdg-open").arg(&url).spawn();
                                })
                                .child(label),
                        );
                    } else {
                        ports_row =
                            ports_row.child(div().text_xs().text_color(ui.muted).child(label));
                    }
                }

                if ws.active_ports.len() > 4 {
                    ports_row = ports_row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(0xffffff))
                            .child(format!("+{} more", ws.active_ports.len() - 4)),
                    );
                }

                card = card.child(ports_row);
            }

            // ── Row: AI tool status (Claude Code / Codex) ──
            match ws.ai_state {
                ai_types::AiToolState::Thinking(tool) => {
                    let (frames, color): (&[char], u32) = match tool {
                        ai_types::AiTool::Claude => (&CLAUDE_SPINNER_FRAMES, 0xd97757),
                        ai_types::AiTool::Codex => (&CODEX_SPINNER_FRAMES, 0x10a37f),
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
                            .child(
                                div()
                                    .w(px(14.))
                                    .h(px(14.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(rgb(color))
                                    .text_xs()
                                    .child(format!("{spinner}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(color))
                                    .child(format!("{} thinking…", tool.label())),
                            ),
                    );
                }
                ai_types::AiToolState::WaitingForInput(tool) => {
                    card = card.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                svg()
                                    .size(px(14.))
                                    .flex_none()
                                    .path("icons/bell.svg")
                                    .text_color(rgb(0xf9e2af)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0xf9e2af))
                                    .child(format!("{} needs input", tool.label())),
                            ),
                    );
                }
                ai_types::AiToolState::Finished(tool) => {
                    card = card.child(
                        div().flex().flex_row().items_center().gap(px(6.)).child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(0xa6e3a1))
                                .child(format!("✓ {} done", tool.label())),
                        ),
                    );
                }
                ai_types::AiToolState::Inactive => {}
            }

            // ── Row 5: Working directory (monospace-style) ──
            card = card.child(div().text_color(ui.muted).text_xs().child(cwd_display));

            list = list.child(card);
        }

        sidebar = sidebar.child(list);

        sidebar
    }
}
