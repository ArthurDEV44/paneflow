//! Binary tree split layout for terminal panes.
//!
//! Leaf nodes hold terminal pane entities. Branch nodes represent splits
//! with a direction (horizontal/vertical) and two children.

use gpui::{div, prelude::*, px, AnyElement, App, Entity, Focusable, IntoElement, Styled, Window};

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
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Render the split tree recursively as nested GPUI flex divs.
    #[allow(clippy::only_used_in_recursion)]
    pub fn render(&self, window: &Window, cx: &App) -> AnyElement {
        match self {
            SplitNode::Leaf(terminal) => {
                div().size_full().child(terminal.clone()).into_any_element()
            }

            SplitNode::Split {
                direction,
                first,
                second,
            } => {
                let first_elem = first.render(window, cx);
                let second_elem = second.render(window, cx);

                let container = div().flex().size_full().overflow_hidden();
                let container = match direction {
                    SplitDirection::Horizontal => container.flex_col(),
                    SplitDirection::Vertical => container.flex_row(),
                };

                container
                    .child(
                        div()
                            .flex_1()
                            .size_full()
                            .min_w(px(80.))
                            .min_h(px(80.))
                            .overflow_hidden()
                            .child(first_elem),
                    )
                    .child(
                        div()
                            .flex_1()
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
    /// Returns true if a split was performed.
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
                    // Placeholder is immediately overwritten — Entity clone is cheap (Arc)
                    let old = std::mem::replace(self, SplitNode::Leaf(new_terminal.clone()));
                    *self = SplitNode::Split {
                        direction,
                        first: Box::new(old),
                        second: Box::new(SplitNode::Leaf(new_terminal)),
                    };
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

    /// Close the focused pane. Consumes self and returns the surviving tree (if any).
    pub fn close_focused(self, window: &Window, cx: &App) -> (Option<SplitNode>, bool) {
        match self {
            SplitNode::Leaf(terminal) => {
                if terminal.read(cx).focus_handle(cx).is_focused(window) {
                    (None, true) // This leaf closes
                } else {
                    (Some(SplitNode::Leaf(terminal)), false) // Not focused, keep
                }
            }
            SplitNode::Split {
                direction,
                first,
                second,
            } => {
                // Try closing in first child
                let (new_first, closed) = first.close_focused(window, cx);
                if closed {
                    return match new_first {
                        None => (Some(*second), true), // First was leaf, promote second
                        Some(f) => (
                            // First was modified internally
                            Some(SplitNode::Split {
                                direction,
                                first: Box::new(f),
                                second,
                            }),
                            true,
                        ),
                    };
                }
                let first = Box::new(new_first.unwrap()); // Not found in first, so it's intact

                // Try closing in second child
                let (new_second, closed) = second.close_focused(window, cx);
                if closed {
                    return match new_second {
                        None => (Some(*first), true), // Second was leaf, promote first
                        Some(s) => (
                            Some(SplitNode::Split {
                                direction,
                                first,
                                second: Box::new(s),
                            }),
                            true,
                        ),
                    };
                }
                let second = Box::new(new_second.unwrap());

                // Not found in either child
                (
                    Some(SplitNode::Split {
                        direction,
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

    /// Focus the first leaf in the tree.
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
}
