//! Read-only traversal: focused-pane lookup, leaf counting, leaf extraction,
//! equalize-ratios mutator (mutates interior `Rc<Cell>` state, not tree shape).

use gpui::{App, Entity, Focusable, Window};

use crate::pane::Pane;

use super::tree::LayoutTree;

impl LayoutTree {
    /// Find the focused pane entity in the tree.
    pub fn focused_pane(&self, window: &Window, cx: &App) -> Option<Entity<Pane>> {
        match self {
            LayoutTree::Leaf(pane) => {
                if pane.read(cx).focus_handle(cx).is_focused(window) {
                    Some(pane.clone())
                } else {
                    None
                }
            }
            LayoutTree::Container { children, .. } => {
                for child in children {
                    if let Some(pane) = child.node.focused_pane(window, cx) {
                        return Some(pane);
                    }
                }
                None
            }
        }
    }

    /// Count the number of leaf (terminal) panes in the tree.
    pub fn leaf_count(&self) -> usize {
        match self {
            LayoutTree::Leaf(_) => 1,
            LayoutTree::Container { children, .. } => {
                children.iter().map(|c| c.node.leaf_count()).sum()
            }
        }
    }

    /// Collect all leaf pane entities in left-to-right (top-to-bottom) order.
    pub fn collect_leaves(&self) -> Vec<Entity<Pane>> {
        match self {
            LayoutTree::Leaf(pane) => vec![pane.clone()],
            LayoutTree::Container { children, .. } => children
                .iter()
                .flat_map(|c| c.node.collect_leaves())
                .collect(),
        }
    }

    /// Set all split ratios to equal values at every level of the tree.
    /// Each container's children get `1.0 / n` where `n` is the child count.
    /// The last child absorbs floating-point remainder to ensure exact sum of 1.0.
    /// Leaf nodes are unchanged. No-op on a single-pane or zoomed workspace.
    /// Mutates interior state via `Rc<Cell<f32>>` ratios.
    pub fn equalize_ratios(&self) {
        if let LayoutTree::Container { children, .. } = self {
            let n = children.len();
            let equal = 1.0 / n as f32;
            for (i, child) in children.iter().enumerate() {
                if i == n - 1 {
                    // Last child absorbs rounding error
                    child.ratio.set(1.0 - equal * (n - 1) as f32);
                } else {
                    child.ratio.set(equal);
                }
                child.node.equalize_ratios();
            }
        }
    }

    /// Return the first (leftmost/topmost) leaf entity without focusing it.
    pub fn first_leaf(&self) -> Option<Entity<Pane>> {
        match self {
            LayoutTree::Leaf(pane) => Some(pane.clone()),
            LayoutTree::Container { children, .. } => {
                children.first().and_then(|c| c.node.first_leaf())
            }
        }
    }

    /// Return the last (rightmost/bottommost) leaf entity without focusing it.
    pub fn last_leaf(&self) -> Option<Entity<Pane>> {
        match self {
            LayoutTree::Leaf(pane) => Some(pane.clone()),
            LayoutTree::Container { children, .. } => {
                children.last().and_then(|c| c.node.last_leaf())
            }
        }
    }
}
