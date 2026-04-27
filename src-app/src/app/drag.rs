//! Drag-and-drop payloads for sidebar workspace reordering.
//!
//! Extracted from `main.rs` per US-002. `WorkspaceDrag` is the payload
//! used as the drag value; `WorkspaceDragPreview` is a small floating
//! GPUI entity rendered under the cursor during the drag.

use gpui::{
    Context, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px,
};

/// Drag payload used when reordering workspace cards in the sidebar.
#[derive(Clone)]
pub(crate) struct WorkspaceDrag {
    pub(crate) id: u64,
    pub(crate) title: SharedString,
}

/// Floating preview entity rendered under the cursor during a workspace drag.
pub(crate) struct WorkspaceDragPreview {
    pub(crate) title: SharedString,
}

impl Render for WorkspaceDragPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        div()
            .px(px(10.))
            .py(px(6.))
            .rounded(px(6.))
            .bg(ui.overlay)
            .border_1()
            .border_color(ui.border)
            .shadow_lg()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ui.text)
            .child(self.title.clone())
    }
}
