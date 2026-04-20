//! Tree-growing mutations: split, swap.

use gpui::{App, Entity, Focusable, Window};

use crate::pane::Pane;

use super::tree::{LayoutTree, SplitDirection, insert_sibling};

impl LayoutTree {
    /// Split the focused pane in the given direction.
    ///
    /// If the parent container has the same direction, the new pane is added
    /// as a sibling (N-ary insertion). Otherwise a new nested container is created.
    pub fn split_at_focused(
        &mut self,
        direction: SplitDirection,
        new_pane: Entity<Pane>,
        window: &Window,
        cx: &App,
    ) -> bool {
        match self {
            LayoutTree::Leaf(pane) => {
                // Cross-direction case: wrap in a new 2-child container
                if pane.read(cx).focus_handle(cx).is_focused(window) {
                    let old = std::mem::replace(self, LayoutTree::Leaf(new_pane.clone()));
                    *self = LayoutTree::new_split(direction, old, LayoutTree::Leaf(new_pane));
                    true
                } else {
                    false
                }
            }
            LayoutTree::Container {
                direction: dir,
                children,
                ..
            } => {
                // Same-direction: check if any direct child leaf is the target
                if *dir == direction {
                    for i in 0..children.len() {
                        if let LayoutTree::Leaf(pane) = &children[i].node
                            && pane.read(cx).focus_handle(cx).is_focused(window)
                        {
                            insert_sibling(children, i, new_pane);
                            return true;
                        }
                    }
                }
                // Recurse into children (handles cross-direction + deeper matches)
                for child in children.iter_mut() {
                    if child
                        .node
                        .split_at_focused(direction, new_pane.clone(), window, cx)
                    {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Split the first (leftmost/topmost) leaf in the given direction.
    /// Used by the IPC handler where no Window/focus context is available.
    ///
    /// Same-direction splits insert as a sibling in the parent container.
    pub fn split_first_leaf(&mut self, direction: SplitDirection, new_pane: Entity<Pane>) {
        match self {
            LayoutTree::Leaf(_) => {
                let old = std::mem::replace(self, LayoutTree::Leaf(new_pane.clone()));
                *self = LayoutTree::new_split(direction, old, LayoutTree::Leaf(new_pane));
            }
            LayoutTree::Container {
                direction: dir,
                children,
                ..
            } => {
                // Same direction + first child is a leaf → sibling insert
                if *dir == direction
                    && matches!(children.first(), Some(c) if matches!(c.node, LayoutTree::Leaf(_)))
                {
                    insert_sibling(children, 0, new_pane);
                    return;
                }
                // Otherwise recurse into first child
                if let Some(first) = children.first_mut() {
                    first.node.split_first_leaf(direction, new_pane);
                }
            }
        }
    }

    /// Split at a specific pane entity (identified by Entity identity, not focus).
    /// Used when the split request comes from a button on the pane itself.
    pub fn split_at_pane(
        &mut self,
        target: &Entity<Pane>,
        direction: SplitDirection,
        new_pane: Entity<Pane>,
    ) -> bool {
        match self {
            LayoutTree::Leaf(pane) => {
                // Cross-direction case: wrap in a new 2-child container
                if pane == target {
                    let old = std::mem::replace(self, LayoutTree::Leaf(new_pane.clone()));
                    *self = LayoutTree::new_split(direction, old, LayoutTree::Leaf(new_pane));
                    true
                } else {
                    false
                }
            }
            LayoutTree::Container {
                direction: dir,
                children,
                ..
            } => {
                // Same-direction: check if any direct child leaf is the target
                if *dir == direction {
                    for i in 0..children.len() {
                        if let LayoutTree::Leaf(pane) = &children[i].node
                            && pane == target
                        {
                            insert_sibling(children, i, new_pane);
                            return true;
                        }
                    }
                }
                // Recurse into children
                for child in children.iter_mut() {
                    if child
                        .node
                        .split_at_pane(target, direction, new_pane.clone())
                    {
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Swap two pane entities in the tree. Walks recursively, replacing
    /// every `Leaf(a)` with `Leaf(b)` and vice versa. Ratios and tree shape
    /// are preserved — only the pane references move.
    pub fn swap_panes(&mut self, a: &Entity<Pane>, b: &Entity<Pane>) {
        match self {
            LayoutTree::Leaf(pane) => {
                if pane == a {
                    *pane = b.clone();
                } else if pane == b {
                    *pane = a.clone();
                }
            }
            LayoutTree::Container { children, .. } => {
                for child in children {
                    child.node.swap_panes(a, b);
                }
            }
        }
    }
}
