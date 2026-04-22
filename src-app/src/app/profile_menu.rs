//! Profile menu — opened by clicking the user avatar on the right of the
//! title bar. Mirrors Zed's user menu shape: a user-info header and an app
//! action list (Settings, Themes…, About). Sign Out will be added once auth
//! is wired.

use gpui::{
    deferred, div, px, AnyElement, ClickEvent, Context, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, SharedString, Styled, Window,
};

use crate::PaneFlowApp;

/// Approximate width of the profile menu — used to shift the menu left so
/// its right edge sits under the profile button (click anchor is inside the
/// button on the far right of the title bar).
const PROFILE_MENU_WIDTH: Pixels = px(220.);

impl PaneFlowApp {
    pub(crate) fn render_profile_menu(
        &self,
        anchor: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let win_size = window.window_bounds().get_bounds().size;

        // Anchor the menu's top-right corner just below-left of the cursor so
        // the menu opens toward the free area (down-and-left of the profile
        // button). Then clamp both axes so the menu never spills past the
        // window edges — same flip-if-overflow pattern as the workspace
        // context menu in `main.rs`.
        let desired_left = anchor.x - PROFILE_MENU_WIDTH;
        let max_left = (win_size.width - PROFILE_MENU_WIDTH - px(4.)).max(px(4.));
        let left = desired_left.clamp(px(4.), max_left);

        // Estimated menu height: header (26) + sep (7) + 3 items × 24 (72)
        //                      + p(4) ×2 + border = ~115.
        // Rounded up to leave slack for font-metric variance.
        let menu_height = px(140.);
        let desired_top = anchor.y + px(4.);
        let top = if desired_top + menu_height > win_size.height {
            (desired_top - menu_height).max(px(4.))
        } else {
            desired_top
        };

        // ── User info header (placeholder until auth lands) ──
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(10.))
            .py(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(ui.text)
                    .child("Guest"),
            )
            .child(
                div()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(ui.subtle)
                    .text_size(px(10.))
                    .text_color(ui.muted)
                    .child("Free"),
            );

        // ── App actions ──
        let settings_item = self.render_context_menu_item(
            SharedString::from("profile-menu-settings"),
            "Settings",
            None,
            ui,
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.profile_menu_open = None;
                this.open_settings_window(window, cx);
                cx.stop_propagation();
            }),
        );

        let themes_item = self.render_context_menu_item(
            SharedString::from("profile-menu-themes"),
            "Themes…",
            None,
            ui,
            cx.listener(|this, _: &ClickEvent, window, cx| {
                this.profile_menu_open = None;
                this.open_theme_picker(window, cx);
                cx.stop_propagation();
            }),
        );

        let about_item = self.render_context_menu_item(
            SharedString::from("profile-menu-about"),
            "About PaneFlow",
            None,
            ui,
            cx.listener(|this, _: &ClickEvent, _w, cx| {
                this.profile_menu_open = None;
                this.show_about_dialog = true;
                cx.notify();
                cx.stop_propagation();
            }),
        );

        deferred(
            div()
                .id("profile-menu")
                .occlude()
                .absolute()
                .left(left)
                .top(top)
                .w(PROFILE_MENU_WIDTH)
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .rounded(px(8.))
                .shadow_lg()
                .flex()
                .flex_col()
                .p(px(4.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.profile_menu_open = None;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(header)
                .child(div().mx(px(-4.)).my(px(3.)).h(px(1.)).bg(ui.border))
                .child(settings_item)
                .child(themes_item)
                .child(about_item),
        )
        .with_priority(4)
        .into_any_element()
    }
}
