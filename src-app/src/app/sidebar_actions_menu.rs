//! Bottom-of-sidebar "Settings" footer + popover. Both the CLI and
//! the Agents sidebars host all of their top-level affordances inside
//! the same trigger: a single `Settings` button at the bottom of the
//! sidebar opens an upward popover with the relevant action list.
//!
//! The popover state (`PaneFlowApp::sidebar_actions_menu_open`) is
//! shared because only one sidebar is rendered at a time (mode toggle
//! swaps the whole sidebar tree).

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*, px, svg,
};

use crate::PaneFlowApp;

impl PaneFlowApp {
    /// Render the bottom Settings trigger + the popover overlay that
    /// opens upward when `sidebar_actions_menu_open` is true. Wrap the
    /// result inside a `relative()` container at the bottom of the
    /// sidebar's flex column.
    pub(crate) fn render_sidebar_settings_footer(
        &self,
        items: Vec<SidebarMenuItem>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let theme = crate::theme::active_theme();
        let is_open = self.sidebar_actions_menu_open;

        let trigger = div()
            .id("sidebar-settings-trigger")
            .mx(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .when(is_open, |d| d.bg(ui.subtle))
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.sidebar_actions_menu_open = !this.sidebar_actions_menu_open;
                cx.notify();
            }))
            .child(
                svg()
                    .size(px(14.))
                    .flex_none()
                    .path("icons/settings.svg")
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
                    .child("Settings"),
            );

        let popover: Option<AnyElement> = if is_open {
            // Vertical menu opening upward from the trigger. Each row
            // matches the sidebar's row language (icon + label, ghost
            // hover). Background uses the panel surface tint so the
            // popover separates from the sidebar's `title_bar_background`.
            let mut menu = div()
                .id("sidebar-settings-popover")
                .absolute()
                .left(px(6.))
                .right(px(6.))
                // 38px = trigger total height (px8/py6 + 14px icon) plus
                // a 6px gap. Adjust if the trigger padding changes.
                .bottom(px(38.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .p(px(4.))
                .rounded(px(8.))
                .bg(theme.title_bar_background)
                .border_1()
                .border_color(ui.border)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(0.25),
                    offset: gpui::point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                }])
                // Click anywhere outside the popover (or its trigger)
                // dismisses it. Same pattern as `profile_menu.rs:128`.
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.sidebar_actions_menu_open {
                        this.sidebar_actions_menu_open = false;
                        cx.notify();
                    }
                }));
            for item in items {
                menu = menu.child(render_menu_item(item, ui, cx));
            }
            Some(menu.into_any_element())
        } else {
            None
        };

        let mut footer = div().relative().flex_none().py(px(6.));
        if let Some(popover) = popover {
            footer = footer.child(popover);
        }
        footer.child(trigger).into_any_element()
    }
}

pub(crate) type SidebarMenuAction =
    Box<dyn Fn(&mut PaneFlowApp, &mut gpui::Window, &mut Context<PaneFlowApp>) + 'static>;

/// A single action row in the Settings popover.
pub(crate) struct SidebarMenuItem {
    pub id: SharedString,
    pub icon: &'static str,
    pub label: SharedString,
    pub on_click: SidebarMenuAction,
}

fn render_menu_item(
    item: SidebarMenuItem,
    ui: crate::theme::UiColors,
    cx: &mut Context<PaneFlowApp>,
) -> AnyElement {
    let handler = item.on_click;
    div()
        .id(item.id)
        .px(px(8.))
        .py(px(6.))
        .rounded(px(5.))
        .cursor_pointer()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .hover(|s| {
            let ui = crate::theme::ui_colors();
            s.bg(ui.subtle)
        })
        .on_click(cx.listener(move |this, _: &ClickEvent, w, cx| {
            handler(this, w, cx);
            this.sidebar_actions_menu_open = false;
            cx.notify();
        }))
        .child(
            svg()
                .size(px(14.))
                .flex_none()
                .path(item.icon)
                .text_color(ui.muted),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(12.))
                .font_weight(FontWeight::NORMAL)
                .text_color(ui.text)
                .truncate()
                .child(item.label),
        )
        .into_any_element()
}
