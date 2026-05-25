//! Agents-style UI primitives shared across every settings tab.
//!
//! Visual recipes mirror `agents_view::view` + `app::agents_sidebar`:
//! - **section_header** — lowercase eyebrow (12px, NORMAL, `ui.muted`), no
//!   border below. Matches `threads_section_header` in the agents sidebar.
//! - **section_header_with_action** — same eyebrow with a right-aligned
//!   secondary button (used by Shortcuts/Appearance "Reset to defaults").
//! - **setting_card** — `ui.surface` panel with `ui.border` 1px border and
//!   8px radius. Wraps row groups so each section reads as a card the way
//!   Agents content cards do.
//! - **hairline** — 1px row separator (border at ~50% alpha), used inside
//!   cards to split rows without competing with the card border.
//! - **toggle_pill** — 32x20 pill switch with a 12px sliding thumb.
//! - **secondary_button** — filled, agents cancel-button style
//!   (`ui.subtle` bg, no border).
//!
//! All helpers return `impl IntoElement` or `Div`. They take no listeners
//! beyond the explicit on_click on `secondary_button` — parent rows wire
//! their own `.id()` and `.on_click()`.

use gpui::{
    ClickEvent, CursorStyle, Div, Hsla, InteractiveElement, IntoElement, ParentElement, Styled,
    div, prelude::*, px,
};

/// Apply an alpha override to an `Hsla` color. GPUI's `Hsla` has no
/// dedicated builder method for alpha, so we update the field manually.
pub fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..color }
}

/// Lowercase eyebrow section label (12px, NORMAL, muted). No border below.
/// Mirrors `app::agents_sidebar::threads_section_header`.
pub fn section_header(ui: crate::theme::UiColors, label: &'static str) -> impl IntoElement {
    div().pb(px(8.)).child(
        div()
            .text_size(px(12.))
            .font_weight(gpui::FontWeight::NORMAL)
            .text_color(ui.muted)
            .child(label),
    )
}

/// Eyebrow header with a right-aligned action element (typically a
/// `secondary_button`). Same typography as `section_header`.
pub fn section_header_with_action(
    ui: crate::theme::UiColors,
    label: &'static str,
    action: impl IntoElement,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .pb(px(8.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::NORMAL)
                .text_color(ui.muted)
                .child(label),
        )
        .child(action)
}

/// Card container for grouped setting rows. `ui.surface` bg + 1px
/// `ui.border` outline + 8px radius. Mirrors the agents content cards
/// (`agents_view::view::render_agent_card`).
///
/// Returns a `Div` so callers can chain `.child()` to add rows.
pub fn setting_card(ui: crate::theme::UiColors) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(ui.surface)
        .border_1()
        .border_color(ui.border)
        .rounded(px(8.))
        .overflow_hidden()
}

/// 1px hairline divider used between rows inside a `setting_card`.
/// Half-alpha border so it reads as a separator without competing with
/// the card outline.
pub fn hairline(ui: crate::theme::UiColors) -> impl IntoElement {
    div().h(px(1.)).w_full().bg(with_alpha(ui.border, 0.5))
}

/// Pure-visual pill toggle. The parent row owns the `id` + `on_click`;
/// this just renders the 32x20 track with the 12px thumb in the right
/// place for the `on` state.
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

/// Standard "title + description" left column used by every setting row.
/// Grows (`flex_1`) and shrinks below content width (`min_w_0`) so the
/// description text wraps inside the row's bounds.
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

/// Filled secondary button (agents cancel-button style): `ui.subtle` bg,
/// no border, hover lifts to `ui.border`. Used for "Reset to defaults"
/// and similar inline actions inside section headers.
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
        .bg(ui.subtle)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(ui.text)
        .hover(|s| s.bg(ui.border))
        .child(label)
        .on_click(on_click)
}
