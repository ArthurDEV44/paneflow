//! Construct a `markdown::MarkdownStyle` from Paneflow's `UiColors`
//! without bootstrapping Zed's full `theme` / `theme_settings` stack.
//!
//! Zed's canonical entry point — `MarkdownStyle::themed(font, window,
//! cx)` — reaches into `cx.theme()` (provided by the `theme` crate's
//! `ActiveTheme` extension) and `ThemeSettings::get_global(cx)` to pull
//! its font sizes, palette and syntax theme. Paneflow does not run that
//! stack: we already maintain our own theme model
//! ([`crate::theme::TerminalTheme`] + [`crate::theme::UiColors`]) and
//! settings live in `paneflow-config`. So we build a `MarkdownStyle`
//! manually, defaulting every field and overriding only what we need
//! for visual parity with Zed's agent panel.

use gpui::{
    AbsoluteLength, DefiniteLength, EdgesRefinement, FontWeight, Hsla, StyleRefinement, TextStyle,
    TextStyleRefinement, px, rems,
};
use markdown::{HeadingLevelStyles, MarkdownStyle};

/// Build a `MarkdownStyle` keyed to Paneflow's active UI palette.
///
/// `body_size_px` lets the caller match the surrounding chrome (chat
/// messages render at 14 px today; tool labels use 13 px; the composer
/// preview might use a smaller size).
///
/// Equivalent to calling [`paneflow_markdown_style_with_line_height`]
/// with the compact `1.5` multiplier — appropriate for single-line
/// chunks (tool labels, user bubble previews). Multi-line assistant
/// paragraphs should instead use the airy `1.75` variant; see
/// [`render_assistant_body_md`](crate::agents::message_render::render_assistant_body_md).
pub(crate) fn paneflow_markdown_style(
    ui: crate::theme::UiColors,
    base_text_color: Hsla,
    body_size_px: f32,
) -> MarkdownStyle {
    paneflow_markdown_style_with_line_height(ui, base_text_color, body_size_px, 1.5)
}

/// Like [`paneflow_markdown_style`] but lets the caller pick the
/// line-height multiplier. Use `1.75` for assistant body paragraphs
/// (matches Zed's `MarkdownStyle::themed(MarkdownFont::Agent)` where
/// `line_height = buffer_font_size * 1.75`); use `1.5` for compact
/// single-line chunks where 1.75x would leave the row too tall.
pub(crate) fn paneflow_markdown_style_with_line_height(
    ui: crate::theme::UiColors,
    base_text_color: Hsla,
    body_size_px: f32,
    line_height_multiplier: f32,
) -> MarkdownStyle {
    let mut style = MarkdownStyle::default();

    let body_size = px(body_size_px);
    let line_height = px(body_size_px * line_height_multiplier);

    // Use IBM Plex Sans (bundled in assets/fonts/) instead of inheriting
    // the system default (DejaVu Sans on Linux). Plex has a noticeably
    // smaller x-height than DejaVu, which is why Zed-style 16 px text
    // reads as more compact than a default-font 13 px equivalent —
    // perceived size is a font-family property, not just a px value.
    style.base_text_style = TextStyle {
        color: base_text_color,
        font_family: "IBM Plex Sans".into(),
        font_size: body_size.into(),
        line_height: line_height.into(),
        ..TextStyle::default()
    };

    style.rule_color = ui.border;
    style.block_quote_border_color = ui.border;
    // Accent-tinted selection background at ~30% opacity so the
    // selected text stays readable underneath (matches the native +
    // Zed convention — a fully-opaque accent on top of text occludes
    // the glyphs and makes the selection unreadable, see Image #7).
    style.selection_background_color = ui.accent.opacity(0.3);

    // List bullets render as plain `div().child("•")` / `div().child("1.")`
    // inside the markdown crate, with no per-bullet color override.
    // They inherit from the outer container's text style, while
    // paragraph text overrides via `base_text_style`. Matching the
    // bullet / ordered-number colour to the body (`base_text_color`)
    // keeps the list markers neutral; the earlier salmon `#cc7755`
    // accent fought the off-white body and read as a competing brand
    // tint inside long assistant turns.
    style.container_style = StyleRefinement::default();
    style.container_style.text = TextStyleRefinement {
        color: Some(base_text_color),
        ..TextStyleRefinement::default()
    };

    // Inline `code` — subtle background highlight, same font + size +
    // color as the surrounding body. Earlier versions used Lilex mono
    // at 0.95x and the salmon container colour bled through, giving
    // every inline-code token a noticeably warm tint and a tighter
    // glyph metric. Inheriting from the body restores typographic
    // continuity so the highlight reads as a tint, not a code span.
    // (The markdown crate's `inline_code` is `TextStyleRefinement`,
    // so a per-token border radius is not exposed -- the background
    // paints as a flat rectangle behind the glyphs. Rounding the
    // pill would require patching the Zed markdown crate.)
    style.inline_code = TextStyleRefinement {
        background_color: Some(ui.subtle),
        color: Some(base_text_color),
        ..TextStyleRefinement::default()
    };

    // Fenced code blocks — outlined panel with internal padding and
    // horizontal scroll. Lilex mono for the glyphs so code reads
    // distinctly from the Plex sans body. Syntax highlight colours
    // come from `style.syntax` (defaulted) when no LanguageRegistry
    // is wired.
    style.code_block = StyleRefinement {
        padding: EdgesRefinement {
            top: Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(8.)))),
            bottom: Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(8.)))),
            left: Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(10.)))),
            right: Some(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(10.)))),
        },
        ..StyleRefinement::default()
    };
    style.code_block.text = TextStyleRefinement {
        font_family: Some("Lilex".into()),
        font_size: Some(px(body_size_px * 0.95).into()),
        ..TextStyleRefinement::default()
    };
    style.code_block_overflow_x_scroll = true;

    // Tables: shrink-to-content columns instead of expanding to fill
    // the available width. Combined with the tighter body size, this
    // keeps data tables visually compact rather than stretching across
    // the whole thread surface.
    style.table_columns_min_size = true;

    // Block-quote text — slightly muted, italic.
    style.block_quote = TextStyleRefinement {
        color: Some(ui.muted),
        ..TextStyleRefinement::default()
    };

    // Links — accent color, no underline by default (the `markdown`
    // crate renders underlines on hover via its own paint pass).
    style.link = TextStyleRefinement {
        color: Some(ui.accent),
        ..TextStyleRefinement::default()
    };

    // Headings — bold weight, with per-level rem-based sizes so h1..h6
    // form a clear visual hierarchy (matches the proportions Zed uses
    // for its assistant-mode markdown rendering). Rem units scale with
    // the surrounding text size so the same style sheet works at 12 px,
    // 13 px or 14 px base sizes.
    style.heading = StyleRefinement::default();
    style.heading.text = TextStyleRefinement {
        font_weight: Some(FontWeight::BOLD),
        color: Some(base_text_color),
        ..TextStyleRefinement::default()
    };
    // Tight heading scale: h1 sits only ~20% above body, h6 below.
    // Hierarchy is still readable (bold weight does the heavy lifting)
    // without large vertical jumps that fight the compact body.
    style.heading_level_styles = Some(HeadingLevelStyles {
        h1: Some(TextStyleRefinement {
            font_size: Some(rems(1.2).into()),
            ..TextStyleRefinement::default()
        }),
        h2: Some(TextStyleRefinement {
            font_size: Some(rems(1.12).into()),
            ..TextStyleRefinement::default()
        }),
        h3: Some(TextStyleRefinement {
            font_size: Some(rems(1.06).into()),
            ..TextStyleRefinement::default()
        }),
        h4: Some(TextStyleRefinement {
            font_size: Some(rems(1.0).into()),
            ..TextStyleRefinement::default()
        }),
        h5: Some(TextStyleRefinement {
            font_size: Some(rems(0.95).into()),
            ..TextStyleRefinement::default()
        }),
        h6: Some(TextStyleRefinement {
            font_size: Some(rems(0.9).into()),
            ..TextStyleRefinement::default()
        }),
    });

    style
}
