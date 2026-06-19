//! Context-menu row helpers shared between the sidebar workspace menu and the
//! title-bar burger menu. Includes the `shortcut_for_description` lookup used
//! to render the keyboard-shortcut label next to each action.
//!
//! Part of the US-025 sidebar decomposition.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, Window, deferred, div, prelude::*, px,
};

use crate::pane::Pane;
use crate::settings::components::{menu_divider_color, select_item, select_menu, with_alpha};
use crate::{PaneFlowApp, TabContextMenu, WorkspaceContextMenu};

impl PaneFlowApp {
    pub(crate) fn shortcut_for_description(&self, description: &str) -> Option<&str> {
        self.effective_shortcuts
            .iter()
            .find(|entry| entry.description == description)
            .map(|entry| entry.key.as_str())
    }

    pub(crate) fn render_context_menu_item(
        &self,
        id: SharedString,
        label: &str,
        shortcut: Option<SharedString>,
        ui: crate::theme::UiColors,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(4.))
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(ui.text)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(on_click)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label.to_string()),
            )
            .when_some(shortcut, |d, shortcut| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(ui.muted)
                        .child(shortcut),
                )
            })
    }

    /// One workspace-menu row in the shared Settings "Shell" select look
    /// (`components::select_item`): 28px tall, 7px radius, 12px label flex-filled
    /// with the optional shortcut pinned right, and the whisper hover highlight
    /// (`text @ 0.05`) instead of the older flat `ui.subtle`. Keeps every app
    /// menu reading as one consistent menu language.
    pub(crate) fn render_select_menu_item(
        &self,
        id: SharedString,
        label: &str,
        shortcut: Option<SharedString>,
        ui: crate::theme::UiColors,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        select_item(id, false, ui)
            .on_click(on_click)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(ui.text)
                    .child(label.to_string()),
            )
            .when_some(shortcut, |d, shortcut| {
                d.child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(ui.muted)
                        .child(shortcut),
                )
            })
    }

    /// Build the deferred element that paints the right-click workspace
    /// context menu. Caller is responsible for the
    /// `if let Some(menu) = self.workspace_menu_open && menu.idx < self.workspaces.len()`
    /// guard. Extracted from `main.rs` per US-002.
    pub(crate) fn render_workspace_context_menu(
        &self,
        menu: WorkspaceContextMenu,
        ui: crate::theme::UiColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let idx = menu.idx;
        let can_delete = !self.workspaces.is_empty();

        // Data-driven editor entries: (id, label, command, shortcut_description)
        let editors: &[(&str, &str, &str, &str)] = &[
            ("zed", "Open in Zed", "zed", "Open in Zed"),
            ("cursor", "Open in Cursor", "cursor", "Open in Cursor"),
            ("vscode", "Open in VS Code", "code", "Open in VS Code"),
            (
                "windsurf",
                "Open in Windsurf",
                "windsurf",
                "Open in Windsurf",
            ),
        ];

        // Estimated menu height: 8 items × 28px + 2 separators × 9px + 8px padding
        let menu_height = px(250.);
        let win_h = window.window_bounds().get_bounds().size.height;
        // Flip: if not enough space below the click, show the menu above it
        let menu_y = if menu.position.y + menu_height > win_h {
            (menu.position.y - menu_height).max(px(0.))
        } else {
            menu.position.y
        };

        let mut context_menu = select_menu("workspace-context-menu", ui)
            .occlude()
            .absolute()
            .left(menu.position.x)
            .top(menu_y)
            .w(px(248.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.workspace_menu_open = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());

        for &(id, label, command, shortcut_desc) in editors {
            let shortcut = self
                .shortcut_for_description(shortcut_desc)
                .map(|s| SharedString::from(s.to_string()));
            let command = command.to_string();
            let label_owned = label.to_string();
            context_menu = context_menu.child(self.render_select_menu_item(
                SharedString::from(format!("workspace-context-{id}")),
                label,
                shortcut,
                ui,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.open_workspace_in_editor(idx, &command, &label_owned, cx);
                    cx.stop_propagation();
                }),
            ));
        }

        // ── Separator ──
        context_menu = context_menu.child(
            div()
                .mx(px(6.))
                .my(px(4.))
                .h(px(1.))
                .bg(menu_divider_color(ui)),
        );

        // Reveal in file manager
        let reveal_shortcut = self
            .shortcut_for_description("Reveal in file manager")
            .map(|s| SharedString::from(s.to_string()));
        context_menu = context_menu.child(self.render_select_menu_item(
            "workspace-context-reveal".into(),
            "Reveal in File Manager",
            reveal_shortcut,
            ui,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.reveal_workspace_in_file_manager(idx, cx);
                cx.stop_propagation();
            }),
        ));

        // Copy path
        let copy_shortcut = self
            .shortcut_for_description("Copy path")
            .map(|s| SharedString::from(s.to_string()));
        context_menu = context_menu.child(self.render_select_menu_item(
            "workspace-context-copy".into(),
            "Copy Path",
            copy_shortcut,
            ui,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.copy_workspace_path(idx, cx);
                cx.stop_propagation();
            }),
        ));

        // Manage Custom Buttons - opens the per-workspace button editor modal.
        context_menu = context_menu.child(self.render_select_menu_item(
            "workspace-context-custom-buttons".into(),
            "Manage Custom Buttons…",
            None,
            ui,
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.open_custom_buttons_modal(idx, window, cx);
                cx.stop_propagation();
            }),
        ));

        // ── Separator ──
        context_menu = context_menu.child(
            div()
                .mx(px(6.))
                .my(px(4.))
                .h(px(1.))
                .bg(menu_divider_color(ui)),
        );

        // Delete workspace (conditionally disabled)
        let close_shortcut = self
            .shortcut_for_description("Close workspace")
            .map(|s| SharedString::from(s.to_string()));
        context_menu = context_menu.child(
            div()
                .id("workspace-context-delete")
                .h(px(28.))
                .px(px(8.))
                .rounded(px(7.))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(ui.muted)
                .when(can_delete, |d| d.text_color(ui.text))
                .when(can_delete, |d| {
                    d.cursor_pointer()
                        .hover(move |s| s.bg(with_alpha(ui.text, 0.05)))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    if can_delete {
                        this.close_workspace_at(idx, window, cx);
                    } else {
                        this.workspace_menu_open = None;
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_x_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child("Delete"),
                )
                .when_some(close_shortcut, |d, shortcut| {
                    d.child(
                        div()
                            .flex_none()
                            .text_size(px(10.))
                            .text_color(ui.muted)
                            .child(shortcut),
                    )
                }),
        );

        deferred(context_menu).priority(3).into_any_element()
    }

    /// Build the deferred "Move to pane…" tab context menu (EP-002 US-006, the
    /// WCAG 2.5.7 non-drag alternative to a cross-pane drag). Lists every other
    /// pane in the source pane's workspace; selecting one moves the tab there
    /// through the same [`crate::pane_drag::move_tab_into`] path the drag uses,
    /// so the PTY is preserved and an emptied source pane is reflowed away. When
    /// the source pane is the workspace's only pane, the menu shows a disabled
    /// note instead of move targets.
    pub(crate) fn render_tab_context_menu(
        &self,
        menu: TabContextMenu,
        ui: crate::theme::UiColors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let source = menu.source_pane.clone();
        let source_idx = menu.tab_idx;

        // Enumerate the panes of the workspace that owns the source pane, in
        // tree order, dropping the source itself.
        let others: Vec<(usize, Entity<Pane>)> = self
            .workspaces
            .iter()
            .find_map(|ws| {
                ws.root
                    .as_ref()
                    .filter(|r| r.contains_leaf(&source))
                    .map(|r| r.collect_leaves())
            })
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .filter(|(_, p)| p != &source)
            .collect();

        let rows = others.len().max(1);
        let menu_height = px(16. + rows as f32 * 27.);
        let win_h = window.window_bounds().get_bounds().size.height;
        let menu_y = if menu.position.y + menu_height > win_h {
            (menu.position.y - menu_height).max(px(0.))
        } else {
            menu.position.y
        };

        let mut context_menu = div()
            .id("tab-context-menu")
            .occlude()
            .absolute()
            .left(menu.position.x)
            .top(menu_y)
            .w(px(248.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .rounded(px(8.))
            .flex()
            .flex_col()
            .p(px(4.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.tab_menu_open = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());

        if others.is_empty() {
            // AC US-006: with a single pane there is nowhere to move to.
            context_menu = context_menu.child(
                div()
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .text_size(px(11.))
                    .text_color(ui.muted)
                    .child("No other panes"),
            );
        } else {
            for (orig_idx, dest) in others {
                let label = format!(
                    "Move to Pane {} - {}",
                    orig_idx + 1,
                    dest.read(cx).active_tab_label(cx)
                );
                let dest_for_click = dest.clone();
                let source_for_click = source.clone();
                context_menu = context_menu.child(self.render_context_menu_item(
                    SharedString::from(format!("tab-move-{orig_idx}")),
                    &label,
                    None,
                    ui,
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.tab_menu_open = None;
                        cx.stop_propagation();
                        // Both panes are held alive by strong refs, but either
                        // could have been removed from the tree while the menu
                        // was open (e.g. a background shell exited and emptied
                        // its pane). Moving into/out of an off-tree pane would
                        // be a confusing no-op, so verify both are still live
                        // leaves of the same workspace before committing.
                        let both_live = this.workspaces.iter().any(|ws| {
                            ws.root.as_ref().is_some_and(|r| {
                                let leaves = r.collect_leaves();
                                leaves.contains(&source_for_click)
                                    && leaves.contains(&dest_for_click)
                            })
                        });
                        if !both_live {
                            cx.notify();
                            return;
                        }
                        dest_for_click.update(cx, |dest_pane, dest_cx| {
                            let dest_idx = dest_pane.tabs.len();
                            crate::pane_drag::move_tab_into(
                                dest_pane,
                                dest_cx,
                                &source_for_click,
                                source_idx,
                                dest_idx,
                                window,
                            );
                        });
                        this.save_session(cx);
                        cx.notify();
                    }),
                ));
            }
        }

        // EP-001 US-003 (cli-cockpit): cancel this tab's queued prompt -
        // the non-Composer cancel path. Only shown when a buffer exists.
        let pending_sid = source
            .read(cx)
            .tabs
            .get(source_idx)
            .and_then(|t| t.as_terminal())
            .map(|t| t.entity_id().as_u64())
            .filter(|sid| self.broadcast.pending.contains_key(sid));
        if let Some(sid) = pending_sid {
            context_menu = context_menu.child(self.render_context_menu_item(
                SharedString::from("tab-cancel-queued"),
                "Cancel queued prompt",
                None,
                ui,
                cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.tab_menu_open = None;
                    this.cancel_pending_for(sid, cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            ));
        }

        deferred(context_menu).priority(3).into_any_element()
    }
}
