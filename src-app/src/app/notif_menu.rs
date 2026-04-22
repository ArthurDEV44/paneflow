//! Notification dropdown — opened by clicking the bell icon in the sidebar
//! action row. Mirrors the `profile_menu` / `title_bar_menu` shape: state is
//! an `Option<Point<Pixels>>` on `PaneFlowApp`, rendering goes through
//! `deferred()` so the dropdown overlays the full window, and the viewport
//! is clamped with flip-if-overflow so the menu never spills off-screen.

use gpui::{
    deferred, div, prelude::*, px, svg, AnyElement, ClickEvent, Context, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, SharedString,
    Styled, Window,
};

use crate::{ai_types, PaneFlowApp};

const NOTIF_MENU_WIDTH: Pixels = px(280.);
const NOTIF_MENU_MAX_HEIGHT: Pixels = px(360.);

impl PaneFlowApp {
    pub(crate) fn render_notif_menu(
        &self,
        anchor: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let win_size = window.window_bounds().get_bounds().size;

        // Anchor the menu so its top-left sits just below the bell. Clamp to
        // the viewport on both axes (same flip-if-overflow pattern used by the
        // profile menu / workspace context menu).
        let desired_left = anchor.x;
        let max_left = (win_size.width - NOTIF_MENU_WIDTH - px(4.)).max(px(4.));
        let left = desired_left.clamp(px(4.), max_left);

        let desired_top = anchor.y + px(6.);
        let top = if desired_top + NOTIF_MENU_MAX_HEIGHT > win_size.height {
            (desired_top - NOTIF_MENU_MAX_HEIGHT).max(px(4.))
        } else {
            desired_top
        };

        // ── Header: title + unread count + "Mark all read" action ──
        let unread_count = self.notifications.iter().filter(|n| !n.read).count();
        let has_unread = unread_count > 0;

        let title_area = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child("Notifications"),
            )
            .when(has_unread, |d| {
                d.child(
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(ui.text)
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui.base)
                        .child(format!("{unread_count}")),
                )
            });

        let mark_all = div()
            .id("notif-mark-all")
            .px(px(6.))
            .py(px(2.))
            .rounded(px(4.))
            .text_size(px(10.))
            .text_color(if has_unread { ui.muted } else { ui.border })
            .when(has_unread, |d| {
                d.cursor_pointer().hover(|s| {
                    let ui = crate::theme::ui_colors();
                    s.bg(ui.subtle).text_color(ui.text)
                })
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                for n in &mut this.notifications {
                    n.read = true;
                }
                cx.notify();
                cx.stop_propagation();
            }))
            .child("Mark all read");

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(10.))
            .py(px(8.))
            .border_b_1()
            .border_color(ui.border)
            .child(title_area)
            .child(mark_all);

        // ── Body: scrollable list or empty state ──
        let body: AnyElement = if self.notifications.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .px(px(12.))
                .py(px(28.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(36.))
                        .h(px(36.))
                        .rounded(px(8.))
                        .bg(ui.subtle)
                        .child(
                            svg()
                                .size(px(16.))
                                .flex_none()
                                .path("icons/bell.svg")
                                .text_color(ui.muted),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(ui.muted)
                        .child("No notifications"),
                )
                .into_any_element()
        } else {
            let now = std::time::Instant::now();
            let mut list = div()
                .id("notif-menu-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .flex()
                .flex_col();

            // Newest first
            for (ni, notif) in self.notifications.iter().enumerate().rev() {
                let ws_id = notif.workspace_id;
                let is_unread = !notif.read;
                let is_last = ni == 0; // after .rev(), index 0 is last visually
                let notif_idx = ni;

                // Icon + text weight convey the kind without hue.
                //   WaitingForInput → bell   + MEDIUM emphasis (needs attention)
                //   Finished        → checks + muted (resolved)
                //   Other           → bell   + muted
                let (icon_path, text_color, weight) = match notif.kind {
                    ai_types::AiToolState::WaitingForInput(_) => {
                        ("icons/bell.svg", ui.text, FontWeight::MEDIUM)
                    }
                    ai_types::AiToolState::Finished(_) => {
                        ("icons/checks.svg", ui.muted, FontWeight::NORMAL)
                    }
                    _ => ("icons/bell.svg", ui.muted, FontWeight::NORMAL),
                };

                let elapsed = now.duration_since(notif.timestamp);
                let stamp = format_relative(elapsed);

                let row = div()
                    .id(SharedString::from(format!("notif-{ni}")))
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(10.))
                    .px(px(10.))
                    .py(px(8.))
                    .cursor_pointer()
                    .when(is_unread, |d| {
                        let ui = crate::theme::ui_colors();
                        d.bg(ui.subtle)
                    })
                    .hover(|s| {
                        let ui = crate::theme::ui_colors();
                        s.bg(ui.surface)
                    })
                    .when(!is_last, |d| d.border_b_1().border_color(ui.border))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if let Some(idx) = this.workspaces.iter().position(|ws| ws.id == ws_id) {
                            this.select_workspace(idx, window, cx);
                        }
                        if notif_idx < this.notifications.len() {
                            this.notifications[notif_idx].read = true;
                        }
                        this.notif_menu_open = None;
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    // Leading icon tile
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(22.))
                            .h(px(22.))
                            .flex_none()
                            .mt(px(1.))
                            .rounded(px(5.))
                            .bg(ui.overlay)
                            .border_1()
                            .border_color(ui.border)
                            .child(
                                svg()
                                    .size(px(12.))
                                    .flex_none()
                                    .path(icon_path)
                                    .text_color(text_color),
                            ),
                    )
                    // Body column: title row + message
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(if is_unread { ui.text } else { ui.muted })
                                            .truncate()
                                            .child(notif.workspace_title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(px(10.))
                                            .text_color(ui.muted)
                                            .child(stamp),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(weight)
                                    .text_color(text_color)
                                    .child(notif.message.clone()),
                            ),
                    );
                list = list.child(row);
            }

            list.into_any_element()
        };

        deferred(
            div()
                .id("notif-menu")
                .occlude()
                .absolute()
                .left(left)
                .top(top)
                .w(NOTIF_MENU_WIDTH)
                .max_h(NOTIF_MENU_MAX_HEIGHT)
                .flex()
                .flex_col()
                .bg(ui.overlay)
                .border_1()
                .border_color(ui.border)
                .rounded(px(8.))
                .shadow_lg()
                .overflow_hidden()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.notif_menu_open = None;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(header)
                .child(body),
        )
        .with_priority(4)
        .into_any_element()
    }
}

/// Compact relative timestamp ("just now", "3m", "2h", "4d"). Kept local —
/// small enough that adding a `humantime`-ish dependency wouldn't be worth it.
fn format_relative(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 10 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}
