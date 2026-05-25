//! Inline "Thinking" row, rendered as a chunk inside an
//! [`AssistantMessage`]. Mirrors Zed's `render_thinking_block`
//! (`agent_ui/src/conversation_view/thread_view.rs:5933`): a single
//! header line (lightbulb icon + muted "Thinking" label + chevron),
//! with a collapsible body indented under a left border.
//!
//! Expanded by default (Zed parity -- `auto_expand_streaming_thought`
//! at `crates/agent_ui/src/conversation_view/thread_view.rs:5831`
//! keeps the body visible while a thought streams in, and the
//! `AlwaysExpanded` display mode keeps it visible after). The
//! disclosure chevron is muted and shows on hover; clicking anywhere
//! on the header toggles collapse.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, ClickEvent, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, linear_color_stop, linear_gradient, px,
};
use markdown::{Markdown, MarkdownElement};
use theme::ActiveTheme;
use ui::{Color, Disclosure, Icon, IconName, IconSize, prelude::*};

use crate::theme::UiColors;

/// Render a single inline `Thought` chunk. `key` is `(entry_ix,
/// chunk_ix)` — used only to seed unique GPUI element ids.
///
/// Ported from Zed `thread_view.rs:5933-6061` `render_thinking_block`
/// (US-009 in `prd-agent-ui-visual-parity-2026-Q3.md`). `is_constrained`
/// matches Zed's `ThinkingBlockDisplay::Preview` branch: caps body at
/// `max_h(px(256.))` (Zed's `max_h_64`) and overlays a top-down panel-bg
/// fade gradient. Zed's `block_mouse_except_scroll` does not exist in
/// Paneflow's GPUI pin — the gradient is purely visual and does not
/// intercept clicks (acceptable divergence, documented per AC #5).
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_inline_thinking(
    key: (usize, usize),
    markdown: Entity<Markdown>,
    expanded: bool,
    is_constrained: bool,
    ui: UiColors,
    _window: &Window,
    cx: &gpui::App,
    on_toggle: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + Clone + 'static,
) -> AnyElement {
    let header_id: SharedString = format!("thinking-block-header-{}-{}", key.0, key.1).into();
    let card_header_id: SharedString = format!("thinking-card-header-{}-{}", key.0, key.1).into();
    let body_id: SharedString = format!("thinking-content-{}-{}", key.0, key.1).into();
    let on_toggle_for_disclosure = on_toggle.clone();

    let panel_bg = cx.theme().colors().panel_background;

    let body_color = gpui::rgb(0xc4c4c4).into();
    // 1.75x line-height matches Zed `render_thinking_block` at
    // `crates/agent_ui/src/conversation_view/thread_view.rs:6038-6042`
    // which passes `MarkdownStyle::themed(MarkdownFont::Agent)` -- and
    // mirrors what `message_render::render_assistant_body_md` now uses
    // for assistant paragraphs. The body sits inside the indented
    // `border_l_1` rail, so the airy multi-line spacing reads as a
    // proper thought stream rather than a compact tool label.
    let style =
        super::markdown_style::paneflow_markdown_style_with_line_height(ui, body_color, 14.0, 1.75);

    // Outer wrapper: `py(10.)` matches the per-row breathing room of
    // the inline tool call (`inline_tool_call::render_inline_tool_call`)
    // so a thinking header sits on the same vertical rhythm as the
    // tool calls it usually appears next to. Padding (not margin)
    // because the parent GPUI `list` widget ignores margins -- see
    // the comment in `inline_tool_call.rs` for the long form.
    v_flex()
        .py(px(5.))
        .gap_1()
        .child(
            h_flex()
                .id(header_id)
                .group(card_header_id.clone())
                .relative()
                .w_full()
                .pr_1()
                .justify_between()
                .child(
                    h_flex()
                        .min_h(px(26.))
                        .items_center()
                        .gap_1p5()
                        .overflow_hidden()
                        .child(
                            Icon::new(IconName::ToolThink)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            div()
                                // Mirror Zed's `tool_name_font_size()` —
                                // 12 px keeps the "Thinking" label flush
                                // with the inline tool-call label sizes.
                                .text_size(px(12.))
                                .text_color(cx.theme().colors().text_muted)
                                .child("Thinking"),
                        ),
                )
                .child(
                    Disclosure::new(("expand-thinking", key.0 * 1000 + key.1), expanded)
                        .opened_icon(IconName::ChevronUp)
                        .closed_icon(IconName::ChevronDown)
                        .visible_on_hover(card_header_id.clone())
                        .on_click(move |ev, w, cx| {
                            on_toggle_for_disclosure(ev, w, cx);
                        }),
                )
                .on_click(on_toggle),
        )
        .when(expanded, |this| {
            let body = div()
                .id(body_id)
                .ml_1p5()
                .pl_3p5()
                .border_l_1()
                .border_color(cx.theme().colors().border.opacity(0.8))
                .when(is_constrained, |this| this.max_h(px(256.)))
                .overflow_hidden()
                .child(MarkdownElement::new(markdown, style));
            this.child(
                div()
                    .when(is_constrained, |this| this.relative())
                    .child(body)
                    .when(is_constrained, |this| {
                        this.child(div().absolute().inset_0().size_full().bg(linear_gradient(
                            180.,
                            linear_color_stop(panel_bg.opacity(0.8), 0.),
                            linear_color_stop(panel_bg.opacity(0.), 0.1),
                        )))
                    }),
            )
        })
        .into_any_element()
}
