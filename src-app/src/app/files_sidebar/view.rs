//! Files sidebar presentation: header + scrollable body. The per-row render
//! lives in `row.rs`; this file stays under the 250-line component budget.

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*, px,
};

use crate::PaneFlowApp;
use crate::app::files_tree;

impl PaneFlowApp {
    pub(super) fn files_sidebar_header(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Title = the workspace folder's final component (the tree root name).
        let title: SharedString = self
            .files_tree
            .root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.files_tree.root.to_string_lossy().into_owned())
            .into();
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            // Quiet header - no divider (Codex: separation by spacing, not
            // borders). 36px matches the unified chrome row height.
            .h(px(36.))
            .flex_none()
            .px(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_x_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui.text)
                    .child(title),
            )
            .child(
                div()
                    .id("files-sidebar-close")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .text_size(px(14.))
                    .text_color(ui.muted)
                    .hover(|s| {
                        s.bg(crate::app::constants::sidebar_tab_hover_background())
                            .text_color(ui.text)
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.close_files_sidebar(cx);
                        cx.stop_propagation();
                    }))
                    .child("×"),
            )
            .into_any_element()
    }

    pub(super) fn files_sidebar_body(
        &self,
        ui: crate::theme::UiColors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = files_tree::flatten_visible(
            &self.files_tree.root,
            &self.files_tree.expanded,
            &self.files_tree.children,
        );

        if rows.is_empty() {
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .p(px(14.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(ui.muted)
                        .child("This folder is empty."),
                )
                .into_any_element();
        }

        let mut body = div()
            .id("files-sidebar-body")
            .flex()
            .flex_col()
            .flex_1()
            .py(px(4.))
            // US-003: vertical scroll only - long names ellipsize, never scroll
            // horizontally.
            .overflow_x_hidden()
            .overflow_y_scroll()
            .track_scroll(&self.files_tree_scroll);
        for row in &rows {
            body = body.child(self.files_row(row, ui, cx));
        }
        body.into_any_element()
    }
}
