//! Context-menu row helpers shared between the sidebar workspace menu and the
//! title-bar burger menu. Includes the `shortcut_for_description` lookup used
//! to render the keyboard-shortcut label next to each action.
//!
//! Part of the US-025 sidebar decomposition.

use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled, Window, deferred, div, prelude::*, px,
};

use crate::{PaneFlowApp, WorkspaceContextMenu};

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

        // Estimated menu height: 8 items × 25px + 2 separators × 7px + 8px padding
        let menu_height = px(228.);
        let win_h = window.window_bounds().get_bounds().size.height;
        // Flip: if not enough space below the click, show the menu above it
        let menu_y = if menu.position.y + menu_height > win_h {
            (menu.position.y - menu_height).max(px(0.))
        } else {
            menu.position.y
        };

        let mut context_menu = div()
            .id("workspace-context-menu")
            .occlude()
            .absolute()
            .left(menu.position.x)
            .top(menu_y)
            .w(px(248.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .rounded(px(8.))
            .shadow_lg()
            .flex()
            .flex_col()
            .p(px(4.))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.workspace_menu_open = None;
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());

        for &(id, label, command, shortcut_desc) in editors {
            let shortcut = self
                .shortcut_for_description(shortcut_desc)
                .map(|s| SharedString::from(s.to_string()));
            let command = command.to_string();
            let label_owned = label.to_string();
            context_menu = context_menu.child(self.render_context_menu_item(
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
        context_menu = context_menu.child(div().mx(px(-4.)).my(px(3.)).h(px(1.)).bg(ui.border));

        // Reveal in file manager
        let reveal_shortcut = self
            .shortcut_for_description("Reveal in file manager")
            .map(|s| SharedString::from(s.to_string()));
        context_menu = context_menu.child(self.render_context_menu_item(
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
        context_menu = context_menu.child(self.render_context_menu_item(
            "workspace-context-copy".into(),
            "Copy Path",
            copy_shortcut,
            ui,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.copy_workspace_path(idx, cx);
                cx.stop_propagation();
            }),
        ));

        // Manage Custom Buttons — opens the per-workspace button editor modal.
        context_menu = context_menu.child(self.render_context_menu_item(
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
        context_menu = context_menu.child(div().mx(px(-4.)).my(px(3.)).h(px(1.)).bg(ui.border));

        // Delete workspace (conditionally disabled)
        let close_shortcut = self
            .shortcut_for_description("Close workspace")
            .map(|s| SharedString::from(s.to_string()));
        context_menu = context_menu.child(
            div()
                .id("workspace-context-delete")
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .px(px(8.))
                .py(px(5.))
                .rounded(px(4.))
                .when(can_delete, |d| d.cursor_pointer())
                .text_size(px(11.))
                .text_color(ui.muted)
                .when(can_delete, |d| d.text_color(ui.text))
                .when(can_delete, |d| {
                    d.hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.subtle)
                    })
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
}
