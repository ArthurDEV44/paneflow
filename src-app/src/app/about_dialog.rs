//! About PaneFlow — modal shown from the profile menu's "About PaneFlow"
//! action. Displays the app name, tagline, version and build credits.
//!
//! Behaviour mirrors `custom_buttons_modal`: backdrop overlay rendered via
//! `deferred().with_priority(10)`, click-outside to dismiss, and a close
//! button + primary OK action inside the card.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, ObjectFit,
    ParentElement, SharedString, Styled, deferred, div, hsla, img, prelude::*, px, svg,
};

use crate::PaneFlowApp;

const REPO_URL: &str = "https://github.com/ArthurDEV44/paneflow";

impl PaneFlowApp {
    pub(crate) fn render_about_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let version = env!("CARGO_PKG_VERSION");

        // ── Header row: spacer + close X ──
        // The card is visually centered; the close button sits on the top-right
        // so users have a familiar dismiss affordance alongside the backdrop
        // click and the primary "Close" button below.
        let close_x = div()
            .id("about-close-x")
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.))
            .h(px(22.))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle)
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_about_dialog = false;
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                svg()
                    .size(px(11.))
                    .flex_none()
                    .path("icons/close.svg")
                    .text_color(ui.muted),
            );

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .w_full()
            .child(close_x);

        // ── Real PaneFlow icon (PNG embedded via rust-embed). ──
        // No tile / background: the PNG already has its own shape + rounded
        // edges, so wrapping it would double up the visual frame.
        let logo_mark = img("icons/paneflow.png")
            .w(px(64.))
            .h(px(64.))
            .object_fit(ObjectFit::Contain);

        // ── Identity block: name + version chip + tagline ──
        let identity = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_color(ui.text)
                            .text_size(px(20.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("PaneFlow"),
                    )
                    .child(
                        div()
                            .px(px(7.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(ui.border)
                            .bg(ui.subtle)
                            .text_size(px(10.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(ui.muted)
                            .child(format!("v{version}")),
                    ),
            )
            .child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(12.))
                    .child("Cross-platform terminal multiplexer"),
            );

        // ── Meta rows (two-column: label muted, value text) ──
        let meta_row = |label: &str, value: SharedString| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .w_full()
                .py(px(4.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(ui.muted)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(ui.text)
                        .child(value),
                )
        };

        let meta = div()
            .flex()
            .flex_col()
            .w_full()
            .px(px(12.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(ui.border)
            .bg(ui.surface)
            .child(meta_row("Built with", SharedString::from("Rust + GPUI")))
            .child(div().h(px(1.)).w_full().bg(ui.border))
            .child(meta_row("License", SharedString::from("MIT")))
            .child(div().h(px(1.)).w_full().bg(ui.border))
            .child(meta_row("Author", SharedString::from("Arthur Jean")));

        // ── Repo link (ghost button, icon + label) ──
        let repo_btn = div()
            .id("about-repo")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .border_1()
            .border_color(ui.border)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle).border_color(ui.muted)
            })
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(ui.text)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Err(err) = open::that(REPO_URL) {
                    log::warn!("about: open repo URL failed: {err}");
                    this.show_toast(format!("Could not open URL: {err}"), cx);
                }
                cx.stop_propagation();
            }))
            .child(
                svg()
                    .size(px(12.))
                    .flex_none()
                    .path("icons/brand-git.svg")
                    .text_color(ui.muted),
            )
            .child("View on GitHub");

        // ── Primary action ──
        let close_btn = div()
            .id("about-dialog-close")
            .px(px(22.))
            .py(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .bg(ui.text)
            .text_color(ui.base)
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .hover(|s| s.opacity(0.85))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_about_dialog = false;
                cx.notify();
                cx.stop_propagation();
            }))
            .child("Close");

        let actions = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .pt(px(4.))
            .child(repo_btn)
            .child(close_btn);

        // ── Assembled card ──
        let card = div()
            .id("about-dialog")
            .occlude()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .w(px(340.))
            .px(px(22.))
            .pt(px(12.))
            .pb(px(20.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .rounded(px(12.))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(header)
            .child(logo_mark)
            .child(identity)
            .child(meta)
            .child(actions);

        deferred(
            div()
                .id("about-dialog-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(hsla(0., 0., 0., 0.5))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.show_about_dialog = false;
                    cx.notify();
                }))
                .child(card),
        )
        .with_priority(10)
        .into_any_element()
    }
}
