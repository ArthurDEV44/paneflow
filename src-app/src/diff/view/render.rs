//! Render-oriented helpers shared by the DiffView host and its column model.

use super::*;

pub(super) fn apply_collapse_unified(
    rows: &[DisplayRow],
    anchors: &[(String, usize)],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<DisplayRow>, Vec<(String, usize)>) {
    let mut out = Vec::with_capacity(rows.len());
    let mut out_anchors = Vec::with_capacity(anchors.len());
    for (index, (path, start)) in anchors.iter().enumerate() {
        let Some(header) = rows.get(*start) else {
            continue;
        };
        let end = anchors
            .get(index + 1)
            .map(|(_, next_start)| *next_start)
            .unwrap_or(rows.len())
            .min(rows.len());
        out_anchors.push((path.clone(), out.len()));
        if collapsed.contains(path) {
            out.push(header.clone());
        } else if let Some(segment) = rows.get(*start..end) {
            out.extend_from_slice(segment);
        }
    }
    (out, out_anchors)
}

pub(super) fn apply_collapse_split(
    rows: &[SplitRow],
    anchors: &[(String, usize)],
    collapsed: &std::collections::HashSet<String>,
) -> (Vec<SplitRow>, Vec<(String, usize)>) {
    let mut out = Vec::with_capacity(rows.len());
    let mut out_anchors = Vec::with_capacity(anchors.len());
    for (index, (path, start)) in anchors.iter().enumerate() {
        let end = anchors
            .get(index + 1)
            .map(|(_, next_start)| *next_start)
            .unwrap_or(rows.len())
            .min(rows.len());
        out_anchors.push((path.clone(), out.len()));
        if collapsed.contains(path) {
            match rows.get(*start) {
                Some(row @ SplitRow::Header(_)) => out.push(row.clone()),
                _ => {
                    out_anchors.pop();
                    continue;
                }
            }
        } else if let Some(segment) = rows.get(*start..end) {
            out.extend_from_slice(segment);
        }
    }
    (out, out_anchors)
}

pub(super) fn centered(color: Hsla, message: String) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(color)
        .child(message)
        .into_any_element()
}

impl DiffView {
    fn render_arrange(&self, node: &Arrange, mode: ViewMode, cx: &mut Context<Self>) -> AnyElement {
        match node {
            Arrange::Leaf(index) => self.render_pane(*index, mode, cx),
            Arrange::Split { axis, children } => {
                let ui = crate::theme::ui_colors();
                let row = *axis == Axis::Row;
                let mut container = div().size_full().flex().min_h_0().min_w_0();
                container = if row {
                    container.flex_row()
                } else {
                    container.flex_col()
                };
                for (index, child) in children.iter().enumerate() {
                    let mut cell = div().flex_1().min_h_0().min_w_0().flex();
                    if index > 0 {
                        cell = if row {
                            cell.border_l_1().border_color(ui.border)
                        } else {
                            cell.border_t_1().border_color(ui.border)
                        };
                    }
                    container = container.child(cell.child(self.render_arrange(child, mode, cx)));
                }
                container.into_any_element()
            }
        }
    }

    fn render_pane(&self, index: usize, mode: ViewMode, cx: &mut Context<Self>) -> AnyElement {
        let Some(column) = self.columns.get(index) else {
            return div().into_any_element();
        };
        let ui = crate::theme::ui_colors();
        let group_name = SharedString::from(format!("{}-pane-{index}", self.element_id));
        let region = self
            .drag_target
            .and_then(|(target, edge)| if target == index { edge } else { None });
        let mut overlay = div().absolute().size_full().flex();
        let (width, height) = match region {
            None => (relative(1.), relative(1.)),
            Some(DropEdge::Left) => {
                overlay = overlay.flex_row().justify_start();
                (relative(0.5), relative(1.))
            }
            Some(DropEdge::Right) => {
                overlay = overlay.flex_row().justify_end();
                (relative(0.5), relative(1.))
            }
            Some(DropEdge::Up) => {
                overlay = overlay.flex_col().justify_start();
                (relative(1.), relative(0.5))
            }
            Some(DropEdge::Down) => {
                overlay = overlay.flex_col().justify_end();
                (relative(1.), relative(0.5))
            }
        };
        let highlight = div()
            .w(width)
            .h(height)
            .bg(ui.accent.opacity(0.22))
            .border_2()
            .border_color(gpui::rgba(0x0ea5e9bf))
            .rounded(px(6.));
        let overlay = overlay
            .invisible()
            .group_drag_over::<DiffColumnDrag>(group_name.clone(), |style| style.visible())
            .on_drop(
                cx.listener(move |this, drag: &DiffColumnDrag, _window, cx| {
                    this.arrange_drop(drag.source_idx, index, cx);
                }),
            )
            .child(highlight);

        div()
            .id(SharedString::from(format!(
                "{}-panec-{index}",
                self.element_id
            )))
            .group(group_name)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_drag_move::<DiffColumnDrag>(cx.listener(
                move |this, event: &DragMoveEvent<DiffColumnDrag>, _window, cx| {
                    this.apply_drag_edge(index, event.bounds, event.event.position, cx);
                },
            ))
            .child(self.render_column(index, column, mode, cx))
            .child(overlay)
            .into_any_element()
    }

    fn apply_drag_edge(
        &mut self,
        index: usize,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let x = f32::from(position.x - bounds.origin.x);
        let y = f32::from(position.y - bounds.origin.y);
        let edge = compute_drop_edge(width, height, x, y, SPLIT_EDGE_BAND);
        let next = Some((index, edge));
        if self.drag_target != next {
            self.drag_target = next;
            cx.notify();
        }
    }

    fn arrange_drop(&mut self, source: usize, target: usize, cx: &mut Context<Self>) {
        let edge = self.drag_target.take().and_then(|(_, edge)| edge);
        if source == target {
            cx.notify();
            return;
        }
        let (axis, before) = match edge {
            Some(DropEdge::Left) => (Axis::Row, true),
            Some(DropEdge::Right) => (Axis::Row, false),
            Some(DropEdge::Up) => (Axis::Col, true),
            Some(DropEdge::Down) => (Axis::Col, false),
            None => (Axis::Row, true),
        };
        self.arrange.remove(source);
        self.arrange.split(target, axis, source, before);
        self.selected_column = source;
        self.scroll_driver = source;
        cx.notify();
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DiffViewEvent> for DiffView {}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = crate::theme::ui_colors();
        let root = div()
            .id(self.element_id.clone())
            .key_context("DiffView")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &crate::CopyDiffHunk, window, cx| {
                this.copy_hovered_hunk(window, cx);
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _window, cx| {
                if let Some((column_index, start_y, start_height)) = this.review_resizing
                    && let Some(column) = this.columns.get_mut(column_index)
                {
                    let delta_y = start_y - f32::from(event.position.y);
                    column.review_height =
                        (start_height + delta_y).clamp(REVIEW_MIN_HEIGHT, REVIEW_MAX_HEIGHT);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _window, cx| {
                    if this.review_resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(ui.base)
            .text_color(ui.text);

        if self.columns.is_empty() {
            return root.child(centered(ui.muted, "No sibling worktrees to diff".into()));
        }

        let mode = self.effective_mode(window);
        self.broadcast_scroll(mode);
        let visible: Vec<bool> = self.columns.iter().map(|column| column.visible).collect();
        self.arrange.reconcile(&visible);
        let body = self.render_arrange(&self.arrange, mode, cx);

        let root = root.child(self.render_toolbar(mode, cx));
        let mut root = root.child(div().flex_1().min_h_0().flex().child(body));
        if let Some(menu) = &self.body_menu {
            root = root.child(self.render_body_menu(menu, ui, cx));
        }
        if let Some(flash) = &self.flash {
            root = root.child(self.render_flash(flash.clone(), ui));
        }
        root
    }
}
