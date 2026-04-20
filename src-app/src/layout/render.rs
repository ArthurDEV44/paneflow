//! `LayoutTree::render` — recursive GPUI flex rendering with drag-to-resize
//! divider handlers and per-frame container-size capture.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Window,
    canvas, div, px, rgb,
};

use super::tree::{DIVIDER_PX, DragState, LayoutTree, MIN_PANE_SIZE, SplitDirection};

impl LayoutTree {
    /// Render the layout tree recursively as nested GPUI flex divs.
    #[allow(clippy::only_used_in_recursion)]
    pub fn render(&self, window: &Window, cx: &App) -> AnyElement {
        match self {
            LayoutTree::Leaf(pane) => div().size_full().child(pane.clone()).into_any_element(),

            LayoutTree::Container {
                direction,
                children,
                drag,
                container_size,
            } => {
                let dir = *direction;

                // Build container with drag tracking.
                // Pre-compute per-child constraints (max yieldable pixels) so the
                // drag closure can clamp based on nested subtree minimums.
                let drag_move = drag.clone();
                let size_for_drag = container_size.clone();
                let child_ratios: Vec<Rc<Cell<f32>>> =
                    children.iter().map(|c| c.ratio.clone()).collect();

                let mut container = div().flex().size_full().overflow_hidden().on_mouse_move(
                    move |e, window, _cx| {
                        if let Some(ds) = drag_move.get() {
                            let csize = size_for_drag.get();
                            if csize <= 0.0 {
                                return;
                            }
                            let current_pos = match dir {
                                SplitDirection::Horizontal => e.position.y.as_f32(),
                                SplitDirection::Vertical => e.position.x.as_f32(),
                            };
                            let delta = current_pos - ds.start_pos;
                            let ratio_delta = delta / csize;

                            // Clamp so neither child goes below MIN_PANE_SIZE pixels.
                            // min_r is the minimum ratio a child can occupy given the
                            // current container size.
                            let pair_sum = ds.start_ratio_before + ds.start_ratio_after;
                            let min_r = MIN_PANE_SIZE / csize;
                            let lower = min_r;
                            let upper = (pair_sum - min_r).max(lower);
                            let new_before =
                                (ds.start_ratio_before + ratio_delta).clamp(lower, upper);
                            let new_after = pair_sum - new_before;

                            if let Some(r) = child_ratios.get(ds.divider_idx) {
                                r.set(new_before);
                            }
                            if let Some(r) = child_ratios.get(ds.divider_idx + 1) {
                                r.set(new_after);
                            }

                            // Request a repaint so the new ratios take effect immediately.
                            // GPUI only auto-refreshes on mouse_move when cx.has_active_drag()
                            // (i.e., GPUI-managed drags). Our Cell-based drag needs an explicit
                            // refresh to avoid waiting for the next terminal poll cycle.
                            window.refresh();
                        }
                    },
                );

                let drag_up = drag.clone();
                container = container
                    .on_mouse_up(MouseButton::Left, {
                        let d = drag_up.clone();
                        move |_e, _window, _cx| {
                            d.set(None);
                        }
                    })
                    .on_mouse_up_out(MouseButton::Left, move |_e, _window, _cx| {
                        drag_up.set(None);
                    });

                container = match dir {
                    SplitDirection::Horizontal => container.flex_col(),
                    SplitDirection::Vertical => container.flex_row(),
                };

                // Capture actual container bounds each frame via canvas prepaint.
                // The canvas fills the container (absolute + size_full) so it
                // receives the parent's bounds without affecting flex layout.
                let size_capture = container_size.clone();
                let drag_cancel = drag.clone();
                container = container.child(
                    canvas(
                        move |bounds, _window, _cx| {
                            let main_axis: f32 = match dir {
                                SplitDirection::Horizontal => bounds.size.height.into(),
                                SplitDirection::Vertical => bounds.size.width.into(),
                            };
                            let prev = size_capture.get();
                            size_capture.set(main_axis);
                            // Cancel drag if container was resized (window resize)
                            if prev > 0.0 && (prev - main_axis).abs() > 1.0 {
                                drag_cancel.set(None);
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                );

                // Render children with dividers between adjacent pairs
                for (i, child) in children.iter().enumerate() {
                    if i > 0 {
                        // Divider between children[i-1] and children[i]
                        let drag_for_div = drag.clone();
                        let divider_idx = i - 1;
                        let ratio_before = children[divider_idx].ratio.clone();
                        let ratio_after = child.ratio.clone();

                        let divider = match dir {
                            SplitDirection::Horizontal => div()
                                .h(px(DIVIDER_PX))
                                .w_full()
                                .flex_shrink_0()
                                .cursor_row_resize()
                                .bg(rgb(0x313244)),
                            SplitDirection::Vertical => div()
                                .w(px(DIVIDER_PX))
                                .h_full()
                                .flex_shrink_0()
                                .cursor_col_resize()
                                .bg(rgb(0x313244)),
                        };

                        let divider =
                            divider.on_mouse_down(MouseButton::Left, move |e, _window, _cx| {
                                let pos = match dir {
                                    SplitDirection::Horizontal => e.position.y.as_f32(),
                                    SplitDirection::Vertical => e.position.x.as_f32(),
                                };
                                drag_for_div.set(Some(DragState {
                                    divider_idx,
                                    start_pos: pos,
                                    start_ratio_before: ratio_before.get(),
                                    start_ratio_after: ratio_after.get(),
                                }));
                            });

                        container = container.child(divider);
                    }

                    let elem = child.node.render(window, cx);
                    container = container.child(
                        div()
                            .flex_basis(gpui::relative(child.ratio.get()))
                            .flex_grow()
                            .flex_shrink()
                            .size_full()
                            .min_w(px(80.))
                            .min_h(px(80.))
                            .overflow_hidden()
                            .child(elem),
                    );
                }

                container.into_any_element()
            }
        }
    }
}
