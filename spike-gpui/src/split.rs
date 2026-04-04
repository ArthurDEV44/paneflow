//! Binary tree split layout for terminal panes.
//!
//! Leaf nodes hold terminal pane entities. Branch nodes represent splits
//! with a direction (horizontal/vertical), a drag-adjustable ratio, and two children.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, rgb, AnyElement, App, Entity, Focusable, InteractiveElement, IntoElement,
    MouseButton, Styled, Window,
};

use crate::terminal::TerminalView;

// ---------------------------------------------------------------------------
// Split direction
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum SplitDirection {
    /// Horizontal divider — panes stacked top/bottom (flex_col)
    Horizontal,
    /// Vertical divider — panes side by side (flex_row)
    Vertical,
}

// ---------------------------------------------------------------------------
// Split node — binary tree
// ---------------------------------------------------------------------------

pub enum SplitNode {
    Leaf(Entity<TerminalView>),
    Split {
        direction: SplitDirection,
        /// Ratio for first child (0.0–1.0). Second child gets 1.0 - ratio.
        ratio: Rc<Cell<f32>>,
        /// Mouse position along split axis when drag started (None = not dragging).
        drag_start: Rc<Cell<Option<f32>>>,
        /// Ratio at drag start — used to compute absolute position.
        drag_start_ratio: Rc<Cell<f32>>,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

/// Minimum ratio to prevent panes from collapsing below 80px.
const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 0.9;
const DIVIDER_PX: f32 = 4.0;

impl SplitNode {
    /// Create a new split from two nodes with default 50/50 ratio.
    pub fn new_split(direction: SplitDirection, first: SplitNode, second: SplitNode) -> Self {
        SplitNode::Split {
            direction,
            ratio: Rc::new(Cell::new(0.5)),
            drag_start: Rc::new(Cell::new(None)),
            drag_start_ratio: Rc::new(Cell::new(0.5)),
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Render the split tree recursively as nested GPUI flex divs.
    #[allow(clippy::only_used_in_recursion)]
    pub fn render(&self, window: &Window, cx: &App) -> AnyElement {
        match self {
            SplitNode::Leaf(terminal) => {
                let focused = terminal.read(cx).focus_handle(cx).is_focused(window);
                let border_color = if focused {
                    rgb(0x89b4fa) // Catppuccin Blue — accent for focused pane
                } else {
                    rgb(0x1e1e2e) // Match background — invisible border
                };
                div()
                    .size_full()
                    .border_2()
                    .border_color(border_color)
                    .child(terminal.clone())
                    .into_any_element()
            }

            SplitNode::Split {
                direction,
                ratio,
                drag_start,
                drag_start_ratio,
                first,
                second,
            } => {
                let r = ratio.get();
                let first_elem = first.render(window, cx);
                let second_elem = second.render(window, cx);
                let dir = *direction;

                // Build divider with cursor style and drag handlers
                let drag_start_clone = drag_start.clone();
                let ratio_for_start = ratio.clone();
                let drag_start_ratio_clone = drag_start_ratio.clone();

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

                let divider = divider.on_mouse_down(MouseButton::Left, move |e, _window, _cx| {
                    let pos = match dir {
                        SplitDirection::Horizontal => e.position.y.as_f32(),
                        SplitDirection::Vertical => e.position.x.as_f32(),
                    };
                    drag_start_clone.set(Some(pos));
                    drag_start_ratio_clone.set(ratio_for_start.get());
                });

                // Build container with mouse_move/up for drag tracking
                let drag_start_move = drag_start.clone();
                let ratio_move = ratio.clone();
                let drag_ratio_move = drag_start_ratio.clone();

                let drag_start_up = drag_start.clone();

                let container = div()
                    .flex()
                    .size_full()
                    .overflow_hidden()
                    .on_mouse_move(move |e, _window, _cx| {
                        if let Some(start_pos) = drag_start_move.get() {
                            let current_pos = match dir {
                                SplitDirection::Horizontal => e.position.y.as_f32(),
                                SplitDirection::Vertical => e.position.x.as_f32(),
                            };
                            // Estimate container size from mouse travel potential
                            // For more accuracy, this would need actual bounds
                            let delta = current_pos - start_pos;
                            let start_r = drag_ratio_move.get();
                            // Approximate container as ~800px (window minus sidebar)
                            // The ratio change is proportional to pixel delta / container
                            // Approximate container size — for precise drag, a custom Element
                            // with real bounds would be needed. CSS min_w/min_h is the true guard.
                            // TODO: use actual element bounds for precise drag
                            let container_estimate = 800.0_f32;
                            let new_ratio =
                                (start_r + delta / container_estimate).clamp(MIN_RATIO, MAX_RATIO);
                            ratio_move.set(new_ratio);
                        }
                    })
                    .on_mouse_up(MouseButton::Left, {
                        let ds = drag_start_up.clone();
                        move |_e, _window, _cx| {
                            ds.set(None);
                        }
                    })
                    .on_mouse_up_out(MouseButton::Left, move |_e, _window, _cx| {
                        drag_start_up.set(None);
                    });

                let container = match dir {
                    SplitDirection::Horizontal => container.flex_col(),
                    SplitDirection::Vertical => container.flex_row(),
                };

                container
                    .child(
                        div()
                            .flex_basis(gpui::relative(r))
                            .flex_grow()
                            .flex_shrink()
                            .size_full()
                            .min_w(px(80.))
                            .min_h(px(80.))
                            .overflow_hidden()
                            .child(first_elem),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex_basis(gpui::relative(1.0 - r))
                            .flex_grow()
                            .flex_shrink()
                            .size_full()
                            .min_w(px(80.))
                            .min_h(px(80.))
                            .overflow_hidden()
                            .child(second_elem),
                    )
                    .into_any_element()
            }
        }
    }

    /// Split the focused pane in the given direction.
    pub fn split_at_focused(
        &mut self,
        direction: SplitDirection,
        new_terminal: Entity<TerminalView>,
        window: &Window,
        cx: &App,
    ) -> bool {
        match self {
            SplitNode::Leaf(terminal) => {
                if terminal.read(cx).focus_handle(cx).is_focused(window) {
                    let old = std::mem::replace(self, SplitNode::Leaf(new_terminal.clone()));
                    *self = SplitNode::new_split(direction, old, SplitNode::Leaf(new_terminal));
                    true
                } else {
                    false
                }
            }
            SplitNode::Split { first, second, .. } => {
                if first.split_at_focused(direction, new_terminal.clone(), window, cx) {
                    return true;
                }
                second.split_at_focused(direction, new_terminal, window, cx)
            }
        }
    }

    /// Split the first (leftmost/topmost) leaf in the given direction.
    /// Used by the IPC handler where no Window/focus context is available.
    pub fn split_first_leaf(
        &mut self,
        direction: SplitDirection,
        new_terminal: Entity<TerminalView>,
    ) {
        match self {
            SplitNode::Leaf(_) => {
                let old = std::mem::replace(self, SplitNode::Leaf(new_terminal.clone()));
                *self = SplitNode::new_split(direction, old, SplitNode::Leaf(new_terminal));
            }
            SplitNode::Split { first, .. } => {
                first.split_first_leaf(direction, new_terminal);
            }
        }
    }

    /// Close the focused pane. Consumes self and returns the surviving tree (if any).
    pub fn close_focused(self, window: &Window, cx: &App) -> (Option<SplitNode>, bool) {
        match self {
            SplitNode::Leaf(terminal) => {
                if terminal.read(cx).focus_handle(cx).is_focused(window) {
                    (None, true)
                } else {
                    (Some(SplitNode::Leaf(terminal)), false)
                }
            }
            SplitNode::Split {
                direction,
                ratio,
                drag_start,
                drag_start_ratio,
                first,
                second,
            } => {
                let (new_first, closed) = first.close_focused(window, cx);
                if closed {
                    return match new_first {
                        None => (Some(*second), true),
                        Some(f) => (
                            Some(SplitNode::Split {
                                direction,
                                ratio,
                                drag_start,
                                drag_start_ratio,
                                first: Box::new(f),
                                second,
                            }),
                            true,
                        ),
                    };
                }
                let first = Box::new(new_first.unwrap());

                let (new_second, closed) = second.close_focused(window, cx);
                if closed {
                    return match new_second {
                        None => (Some(*first), true),
                        Some(s) => (
                            Some(SplitNode::Split {
                                direction,
                                ratio,
                                drag_start,
                                drag_start_ratio,
                                first,
                                second: Box::new(s),
                            }),
                            true,
                        ),
                    };
                }
                let second = Box::new(new_second.unwrap());

                (
                    Some(SplitNode::Split {
                        direction,
                        ratio,
                        drag_start,
                        drag_start_ratio,
                        first,
                        second,
                    }),
                    false,
                )
            }
        }
    }

    /// Count the number of leaf (terminal) panes in the tree.
    pub fn leaf_count(&self) -> usize {
        match self {
            SplitNode::Leaf(_) => 1,
            SplitNode::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// Focus the first (leftmost/topmost) leaf in the tree.
    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        match self {
            SplitNode::Leaf(terminal) => {
                terminal.read(cx).focus_handle(cx).focus(window, cx);
            }
            SplitNode::Split { first, .. } => {
                first.focus_first(window, cx);
            }
        }
    }

    /// Focus the last (rightmost/bottommost) leaf in the tree.
    pub fn focus_last(&self, window: &mut Window, cx: &mut App) {
        match self {
            SplitNode::Leaf(terminal) => {
                terminal.read(cx).focus_handle(cx).focus(window, cx);
            }
            SplitNode::Split { second, .. } => {
                second.focus_last(window, cx);
            }
        }
    }

    /// Move focus in the given direction. Returns true if focus was moved.
    ///
    /// Algorithm: recurse to find the focused leaf. At each ancestor Split,
    /// check if the direction is compatible and the focused leaf is on the
    /// correct side. If so, focus the nearest-edge leaf of the opposite child.
    pub fn focus_in_direction(
        &self,
        dir: FocusDirection,
        window: &mut Window,
        cx: &mut App,
    ) -> FocusNav {
        match self {
            SplitNode::Leaf(terminal) => {
                if terminal.read(cx).focus_handle(cx).is_focused(window) {
                    FocusNav::FocusedHere
                } else {
                    FocusNav::NotHere
                }
            }
            SplitNode::Split {
                direction,
                first,
                second,
                ..
            } => {
                // Try first child
                match first.focus_in_direction(dir, window, cx) {
                    FocusNav::Moved => return FocusNav::Moved,
                    FocusNav::FocusedHere => {
                        // can_move_from_first only matches Right/Down
                        if can_move_from_first(*direction, dir) {
                            second.focus_first(window, cx);
                            return FocusNav::Moved;
                        }
                        return FocusNav::FocusedHere;
                    }
                    FocusNav::NotHere => {}
                }

                // Try second child
                match second.focus_in_direction(dir, window, cx) {
                    FocusNav::Moved => FocusNav::Moved,
                    FocusNav::FocusedHere => {
                        // can_move_from_second only matches Left/Up
                        if can_move_from_second(*direction, dir) {
                            first.focus_last(window, cx);
                            FocusNav::Moved
                        } else {
                            FocusNav::FocusedHere
                        }
                    }
                    FocusNav::NotHere => FocusNav::NotHere,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Focus navigation types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum FocusDirection {
    Left,
    Right,
    Up,
    Down,
}

pub enum FocusNav {
    NotHere,
    FocusedHere,
    Moved,
}

/// Can we move from the first child in the given direction at this split?
fn can_move_from_first(split_dir: SplitDirection, focus_dir: FocusDirection) -> bool {
    matches!(
        (split_dir, focus_dir),
        (SplitDirection::Vertical, FocusDirection::Right)
            | (SplitDirection::Horizontal, FocusDirection::Down)
    )
}

/// Can we move from the second child in the given direction at this split?
fn can_move_from_second(split_dir: SplitDirection, focus_dir: FocusDirection) -> bool {
    matches!(
        (split_dir, focus_dir),
        (SplitDirection::Vertical, FocusDirection::Left)
            | (SplitDirection::Horizontal, FocusDirection::Up)
    )
}
