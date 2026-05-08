//! Zed-style UI primitives shared across every settings tab.
//!
//! These mirror the visual recipes from
//! `~/dev/zed/crates/settings_ui/src/settings_ui.rs` and
//! `~/dev/zed/crates/ui/src/components/toggle.rs`:
//! - **section_header** — small uppercase muted label + 1px border-faded divider.
//! - **hairline** — 1px row separator (border at ~50% alpha).
//! - **toggle_pill** — 32x20 pill switch with a 12px sliding thumb.
//! - **secondary_button** — small text button used for "Reset to defaults".
//!
//! All helpers return `impl IntoElement`. They take no listeners — the
//! parent row is responsible for wiring `.id()` and `.on_click()`. This
//! keeps the helpers reusable across very different click handlers
//! (boolean writes, recording-state captures, popover toggles, etc.).

use gpui::{
    ClickEvent, CursorStyle, Hsla, InteractiveElement, IntoElement, ParentElement, Styled, div,
    prelude::*, px,
};

/// Apply an alpha override to an `Hsla` color. GPUI's `Hsla` has no
/// dedicated builder method for alpha, so we update the field manually.
pub fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

/// Section label (uppercase, small, muted) + 1px faded divider below.
/// Mirrors Zed's `SettingsSectionHeader`.
pub fn section_header(ui: crate::theme::UiColors, label: &'static str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .mb(px(4.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(ui.muted)
                .child(label),
        )
        .child(div().h(px(1.)).w_full().bg(ui.border))
}

/// 1px hairline divider used between rows inside a section.
/// Half-alpha border so it reads as a separator without competing with
/// the heavier section_header divider.
pub fn hairline(ui: crate::theme::UiColors) -> impl IntoElement {
    div().h(px(1.)).w_full().bg(with_alpha(ui.border, 0.5))
}

/// Pure-visual pill toggle. The parent row owns the `id` + `on_click`;
/// this just renders the 32x20 track with the 12px thumb in the right
/// place for the `on` state.
///
/// Recipe (Zed `toggle.rs:451-498`):
/// - Track: 32x20, `rounded_full`, 1px border, 2px horizontal padding.
/// - Thumb: 12px circle, slid via flex `justify_end`/`justify_start`.
/// - On: accent at ~55% alpha, border at 70% alpha, thumb at full alpha.
/// - Off: muted at ~18% alpha, plain border, thumb at 55% alpha.
pub fn toggle_pill(on: bool, ui: crate::theme::UiColors) -> impl IntoElement {
    let track_bg = if on {
        with_alpha(ui.accent, 0.55)
    } else {
        with_alpha(ui.muted, 0.18)
    };
    let track_border = if on {
        with_alpha(ui.accent, 0.7)
    } else {
        ui.border
    };
    let thumb_alpha = if on { 1.0 } else { 0.55 };

    let track = div()
        .flex()
        .flex_row()
        .items_center()
        .w(px(32.))
        .h(px(20.))
        .rounded_full()
        .px(px(2.))
        .border_1()
        .border_color(track_border)
        .bg(track_bg)
        .when(on, |s| s.justify_end())
        .when(!on, |s| s.justify_start())
        .child(
            div()
                .w(px(12.))
                .h(px(12.))
                .rounded_full()
                .bg(with_alpha(ui.text, thumb_alpha)),
        );

    div().flex_shrink_0().child(track)
}

/// Standard "title + description" left column used by every Zed-style
/// setting row. Returns a flex column that **grows** (`flex_1`) and
/// **shrinks below content width** (`min_w_0`) so the description text
/// wraps inside the row's bounds instead of pushing the row taller
/// than its container.
///
/// Pair with `flex_shrink_0()` on the right-side control to lock the
/// row layout: control keeps its intrinsic size, text fills the rest.
pub fn setting_text(
    ui: crate::theme::UiColors,
    title: &'static str,
    description: &'static str,
) -> impl IntoElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(ui.text)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(ui.muted)
                .child(description),
        )
}

/// Compact text button used for "Reset to defaults" / "Edit in
/// settings.json" — small, outlined, hover-highlights. The caller wires
/// the click handler.
pub fn secondary_button(
    id: &'static str,
    label: &'static str,
    ui: crate::theme::UiColors,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px(px(10.))
        .py(px(4.))
        .rounded(px(6.))
        .cursor(CursorStyle::PointingHand)
        .border_1()
        .border_color(ui.border)
        .hover(|s| s.bg(ui.subtle).text_color(ui.text))
        .text_size(px(12.))
        .text_color(ui.muted)
        .child(label)
        .on_click(on_click)
}
