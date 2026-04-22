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
    App, AppContext, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, SharedString, Styled, Window, div, prelude::*, px, rgb, svg,
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
                    // Left side — section eyebrow
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.muted)
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
                                    cx.listener(|this, e: &ClickEvent, _w, cx| {
                                        this.title_bar_menu_open = None;
                                        this.profile_menu_open = None;
                                        this.notif_menu_open = if this.notif_menu_open.is_some() {
                                            None
                                        } else {
                                            Some(e.position())
                                        };
                                        cx.notify();
                                    }),
                                ))
                                .when(has_unread, |d| {
                                    let ui = crate::theme::ui_colors();
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(2.))
                                            .right(px(2.))
                                            .w(px(6.))
                                            .h(px(6.))
                                            .rounded_full()
                                            .bg(ui.text)
                                            .border_1()
                                            .border_color(theme.title_bar_background),
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

        // Notification dropdown is rendered at the app level from `main.rs`
        // (same pattern as profile / title-bar menus) so it can overlay the
        // full window and use the click-anchored deferred layer.

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
            let terminal_count = ws.terminal_count(cx);

            let idx = i;
            let ws_id = ws.id;
            let ws_title: SharedString = ws.title.clone().into();

            let mut card = div()
                .id(SharedString::from(format!("ws-{i}")))
                .mx(px(6.))
                .px(px(10.))
                .py(px(9.))
                .when(is_active, |d| {
                    let ui = crate::theme::ui_colors();
                    d.bg(ui.surface)
                })
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
                        this.title_bar_menu_open = None;
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
                .gap_1();

            // ── Row 1: Title + pane-count chip ──
            let title_el = if self.renaming_idx == Some(i) {
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(ui.text)
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
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
            let terminal_chip = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.))
                .flex_none()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(4.))
                .bg(ui.subtle)
                .text_size(px(10.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(ui.muted)
                .child(
                    svg()
                        .size(px(10.))
                        .flex_none()
                        .path("icons/terminal.svg")
                        .text_color(ui.muted),
                )
                .child(format!("{terminal_count}"))
                .when(terminal_count == 0, |d| d.invisible());
            let title_row = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .min_w_0()
                .child(title_el)
                .child(terminal_chip);

            card = card.child(title_row);

            // ── Row 2: Git branch + diff stats (compact, monochrome) ──
            if !ws.git_branch.is_empty() || !ws.git_stats.is_empty() {
                let mut git_row = div().flex().flex_row().items_center().gap(px(8.)).min_w_0();

                if !ws.git_branch.is_empty() {
                    git_row = git_row.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.))
                            .flex_1()
                            .min_w_0()
                            .child(
                                svg()
                                    .size(px(11.))
                                    .flex_none()
                                    .path("icons/git-branch.svg")
                                    .text_color(ui.muted),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(ui.muted)
                                    .text_xs()
                                    .truncate()
                                    .child(ws.git_branch.clone()),
                            ),
                    );
                }

                if !ws.git_stats.is_empty() {
                    let ins = ws.git_stats.insertions;
                    let del = ws.git_stats.deletions;
                    git_row = git_row.child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_none()
                            .gap(px(6.))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
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
                    );
                }

                card = card.child(git_row);
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
                                // on Windows. For `https://` URLs the
                                // Windows path is functionally
                                // equivalent to `cmd /C start`, but
                                // the mechanism is distinct —
                                // clarified here because the prior
                                // "byte-identical" phrasing misled a
                                // v0.2.0 audit (US-007). On failure
                                // show a toast instead of the previous
                                // silent `let _ = ...` swallow.
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
                        ports_row =
                            ports_row.child(div().text_xs().text_color(ui.muted).child(label));
                    }
                }

                if ws.active_ports.len() > 4 {
                    ports_row = ports_row.child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .text_size(px(11.))
                            .text_color(ui.muted)
                            .child(format!("+{} more", ws.active_ports.len() - 4)),
                    );
                }

                card = card.child(ports_row);
            }

            // ── Row: AI tool status (Claude Code / Codex) — monochrome ──
            // Hierarchy is conveyed via weight + container, not hue:
            //   Thinking        → spinner + text in `ui.text` (live)
            //   WaitingForInput → badge-style with ui.text bg (claims attention)
            //   Finished        → muted check + text (subdued, done)
            match ws.ai_state {
                ai_types::AiToolState::Thinking(tool) => {
                    let frames: &[char] = match tool {
                        ai_types::AiTool::Claude => &CLAUDE_SPINNER_FRAMES,
                        ai_types::AiTool::Codex => &CODEX_SPINNER_FRAMES,
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
                                    .text_color(ui.text)
                                    .text_xs()
                                    .child(format!("{spinner}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(ui.text)
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
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(ui.text)
                            .child(
                                svg()
                                    .size(px(12.))
                                    .flex_none()
                                    .path("icons/bell.svg")
                                    .text_color(ui.base),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ui.base)
                                    .child(format!("{} needs input", tool.label())),
                            ),
                    );
                }
                ai_types::AiToolState::Finished(tool) => {
                    card = card.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                svg()
                                    .size(px(12.))
                                    .flex_none()
                                    .path("icons/checks.svg")
                                    .text_color(ui.muted),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(ui.muted)
                                    .child(format!("{} done", tool.label())),
                            ),
                    );
                }
                ai_types::AiToolState::Inactive => {}
            }

            // ── Row 5: Working directory (monospace for path affordance) ──
            card = card.child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(10.))
                    .font_family("monospace")
                    .truncate()
                    .child(cwd_display),
            );

            list = list.child(card);
        }

        sidebar = sidebar.child(list);

        sidebar
    }
}
