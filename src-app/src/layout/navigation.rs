//! Focus navigation: directional movement through the tree, first/last leaf
//! focus helpers, and the forward/backward axis-match predicates that decide
//! when a direction crosses a container boundary.

use gpui::{App, Focusable, Window};

use super::tree::{LayoutTree, SplitDirection};

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

/// Does the focus direction move forward (Right/Down) through the container axis?
fn is_forward(split_dir: SplitDirection, focus_dir: FocusDirection) -> bool {
    matches!(
        (split_dir, focus_dir),
        (SplitDirection::Vertical, FocusDirection::Right)
            | (SplitDirection::Horizontal, FocusDirection::Down)
    )
}

/// Does the focus direction move backward (Left/Up) through the container axis?
fn is_backward(split_dir: SplitDirection, focus_dir: FocusDirection) -> bool {
    matches!(
        (split_dir, focus_dir),
        (SplitDirection::Vertical, FocusDirection::Left)
            | (SplitDirection::Horizontal, FocusDirection::Up)
    )
}

impl LayoutTree {
    /// Focus the first (leftmost/topmost) leaf in the tree.
    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        match self {
            LayoutTree::Leaf(pane) => {
                pane.read(cx).focus_handle(cx).focus(window, cx);
            }
            LayoutTree::Container { children, .. } => {
                if let Some(first) = children.first() {
                    first.node.focus_first(window, cx);
                }
            }
        }
    }

    /// Focus the last (rightmost/bottommost) leaf in the tree.
    pub fn focus_last(&self, window: &mut Window, cx: &mut App) {
        match self {
            LayoutTree::Leaf(pane) => {
                pane.read(cx).focus_handle(cx).focus(window, cx);
            }
            LayoutTree::Container { children, .. } => {
                if let Some(last) = children.last() {
                    last.node.focus_last(window, cx);
                }
            }
        }
    }

    /// Move focus in the given direction. Returns the navigation result.
    ///
    /// Algorithm: iterate children to find which contains the focused leaf.
    /// If the direction matches the container axis and there is a next/prev
    /// sibling, move focus there. Otherwise propagate up.
    pub fn focus_in_direction(
        &self,
        dir: FocusDirection,
        window: &mut Window,
        cx: &mut App,
    ) -> FocusNav {
        match self {
            LayoutTree::Leaf(pane) => {
                if pane.read(cx).focus_handle(cx).is_focused(window) {
                    FocusNav::FocusedHere
                } else {
                    FocusNav::NotHere
                }
            }
            LayoutTree::Container {
                direction,
                children,
                ..
            } => {
                for (i, child) in children.iter().enumerate() {
                    match child.node.focus_in_direction(dir, window, cx) {
                        FocusNav::Moved => return FocusNav::Moved,
                        FocusNav::FocusedHere => {
                            if is_forward(*direction, dir) && i + 1 < children.len() {
                                children[i + 1].node.focus_first(window, cx);
                                return FocusNav::Moved;
                            }
                            if is_backward(*direction, dir) && i > 0 {
                                children[i - 1].node.focus_last(window, cx);
                                return FocusNav::Moved;
                            }
                            return FocusNav::FocusedHere;
                        }
                        FocusNav::NotHere => continue,
                    }
                }
                FocusNav::NotHere
            }
        }
    }
}
