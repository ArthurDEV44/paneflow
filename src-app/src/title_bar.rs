use gpui::{div, prelude::*, px, rgb, IntoElement, Styled, Window};

/// Maximum workspace name display length before truncation.
const MAX_WORKSPACE_NAME_LEN: usize = 40;

/// Render the PaneFlow title bar.
///
/// Returns an `h_flex` row sized to Zed's title bar height formula:
/// `(1.75 * rem_size).max(34px)`.
pub fn render(workspace_name: Option<&str>, window: &Window) -> impl IntoElement {
    let height = (1.75 * window.rem_size()).max(px(34.));

    let mut title = div().flex().flex_row().items_center().gap(px(8.)).child(
        div()
            .text_color(rgb(0xcdd6f4))
            .text_sm()
            .font_weight(gpui::FontWeight::BOLD)
            .child("PaneFlow"),
    );

    if let Some(name) = workspace_name {
        let display_name = if name.chars().count() > MAX_WORKSPACE_NAME_LEN {
            let truncated: String = name.chars().take(MAX_WORKSPACE_NAME_LEN).collect();
            format!("{truncated}...")
        } else {
            name.to_string()
        };

        title = title
            .child(div().text_color(rgb(0x6c7086)).text_sm().child("\u{2014}"))
            .child(
                div()
                    .text_color(rgb(0xa6adc8))
                    .text_sm()
                    .child(display_name),
            );
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .h(height)
        .bg(rgb(0x181825))
        .border_b_1()
        .border_color(rgb(0x313244))
        .px(px(12.))
        .child(title)
}
