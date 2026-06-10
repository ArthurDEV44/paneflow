//! About Paneflow — modal shown from the Settings popover's "About" action.
//!
//! Codex-minimal redesign: one quiet card with the identity block (logo,
//! name, tagline), a single muted meta line, and a quiet GitHub link. No
//! inner boxes or borders — hierarchy comes from typography and spacing
//! (the OpenAI "Space" principle). Dismissal is the top-right ✕ or the
//! backdrop click; no redundant primary "Close" button.
//!
//! Behaviour mirrors `custom_buttons_modal`: backdrop overlay rendered via
//! `deferred().with_priority(10)`, click-outside to dismiss.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, ObjectFit,
    ParentElement, Styled, deferred, div, hsla, img, prelude::*, px, svg,
};

use crate::PaneFlowApp;

const REPO_URL: &str = "https://github.com/ArthurDEV44/paneflow";

impl PaneFlowApp {
    pub(crate) fn render_about_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let ui = crate::theme::ui_colors();
        let version = env!("CARGO_PKG_VERSION");

        // ── Close ✕, floating in the card's top-right corner ──
        let close_x = div()
            .id("about-close-x")
            .absolute()
            .top(px(10.))
            .right(px(10.))
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.))
            .h(px(24.))
            .rounded(px(5.))
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

        // ── Identity: logo, name + version inline, tagline ──
        // The PNG carries its own shape + rounded edges — no tile around it.
        let logo_mark = img("icons/paneflow.png")
            .w(px(56.))
            .h(px(56.))
            .object_fit(ObjectFit::Contain);

        let identity = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .gap(px(7.))
                    .child(
                        div()
                            .text_color(ui.text)
                            .text_size(px(18.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Paneflow"),
                    )
                    .child(
                        // Quiet inline version — no chip, no border (the old
                        // bordered badge fought the name for attention).
                        div()
                            .text_size(px(12.))
                            .text_color(ui.muted)
                            .child(format!("v{version}")),
                    ),
            )
            .child(
                div()
                    .text_color(ui.muted)
                    .text_size(px(12.))
                    .child("Run coding agents in parallel"),
            );

        // ── Single muted meta line — replaces the old bordered 3-row table.
        // Pure metadata: it must read in one glance and never compete with
        // the identity block.
        let meta = div()
            .text_size(px(11.))
            .text_color(ui.muted.opacity(0.8))
            .child("Rust + GPUI · GPL-3.0-or-later · Arthur Jean");

        // ── Quiet GitHub link (icon + label, hover fill — no border) ──
        let repo_btn = div()
            .id("about-repo")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(5.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(ui.muted)
            .hover(|s| {
                let ui = crate::theme::ui_colors();
                s.bg(ui.subtle).text_color(ui.text)
            })
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

        // ── Assembled card — generous breathing room, no inner boxes ──
        let card = div()
            .id("about-dialog")
            .occlude()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(16.))
            .w(px(320.))
            .px(px(24.))
            .pt(px(36.))
            .pb(px(24.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .rounded(px(12.))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(close_x)
            .child(logo_mark)
            .child(identity)
            .child(meta)
            .child(repo_btn);

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
