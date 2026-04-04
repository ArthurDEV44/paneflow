//! N-ary tree layout for terminal panes.
//!
//! Leaf nodes hold terminal pane entities. Container nodes hold 2+ children
//! with a direction (horizontal/vertical) and per-child flex ratios.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, rgb, AnyElement, App, Entity, Focusable, InteractiveElement, IntoElement,
    MouseButton, Styled, Window,
};

use crate::pane::Pane;

// ---------------------------------------------------------------------------
// Split direction
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum SplitDirection {
    /// Horizontal divider — panes stacked top/bottom (flex_col)
    Horizontal,
    /// Vertical divider — panes side by side (flex_row)
    Vertical,
}

// ---------------------------------------------------------------------------
// Drag state — tracks an in-progress divider drag
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) struct DragState {
    divider_idx: usize,
    start_pos: f32,
    start_ratio_before: f32,
    start_ratio_after: f32,
}

// ---------------------------------------------------------------------------
// Layout tree — N-ary tree
// ---------------------------------------------------------------------------

pub struct LayoutChild {
    pub node: LayoutTree,
    pub ratio: Rc<Cell<f32>>,
    pub computed_size: Rc<Cell<f32>>,
}

pub enum LayoutTree {
    Leaf(Entity<Pane>),
    Container {
        direction: SplitDirection,
        children: Vec<LayoutChild>,
        drag: Rc<Cell<Option<DragState>>>,
    },
}

const MIN_RATIO: f32 = 0.1;
const MAX_RATIO: f32 = 0.9;
const DIVIDER_PX: f32 = 4.0;

/// Re-normalize ratios so they sum to 1.0.
fn normalize_ratios(children: &[LayoutChild]) {
    let sum: f32 = children.iter().map(|c| c.ratio.get()).sum();
    if sum > 0.0 && (sum - 1.0).abs() > f32::EPSILON {
        for child in children {
            child.ratio.set(child.ratio.get() / sum);
        }
    }
}

/// Insert a new pane as a sibling after `children[idx]`.
/// The new child steals half of the target child's ratio.
///
/// # Panics
/// Panics in debug builds if `idx >= children.len()`.
fn insert_sibling(children: &mut Vec<LayoutChild>, idx: usize, new_pane: Entity<Pane>) {
    debug_assert!(idx < children.len(), "insert_sibling: idx out of bounds");
    let old_ratio = children[idx].ratio.get();
    let half = old_ratio / 2.0;
    children[idx].ratio.set(half);
    children.insert(
        idx + 1,
        LayoutChild {
            node: LayoutTree::Leaf(new_pane),
            ratio: Rc::new(Cell::new(half)),
            computed_size: Rc::new(Cell::new(0.0)),
        },
    );
}

impl LayoutTree {
    /// Create a new 2-child container with 50/50 ratios.
    pub fn new_split(direction: SplitDirection, first: LayoutTree, second: LayoutTree) -> Self {
        LayoutTree::Container {
            direction,
            children: vec![
                LayoutChild {
                    node: first,
                    ratio: Rc::new(Cell::new(0.5)),
                    computed_size: Rc::new(Cell::new(0.0)),
                },
                LayoutChild {
                    node: second,
                    ratio: Rc::new(Cell::new(0.5)),
                    computed_size: Rc::new(Cell::new(0.0)),
                },
            ],
            drag: Rc::new(Cell::new(None)),
        }
    }

    /// Render the layout tree recursively as nested GPUI flex divs.
    pub fn render(&self, window: &Window, cx: &App) -> AnyElement {
        match self {
            LayoutTree::Leaf(pane) => div().size_full().child(pane.clone()).into_any_element(),

            LayoutTree::Container {
                direction,
                children,
                drag,
            } => {
                let dir = *direction;

                // Build container with drag tracking
                let drag_move = drag.clone();
                let child_ratios: Vec<Rc<Cell<f32>>> =
                    children.iter().map(|c| c.ratio.clone()).collect();

                let mut container = div().flex().size_full().overflow_hidden().on_mouse_move(
                    move |e, _window, _cx| {
                        if let Some(ds) = drag_move.get() {
                            let current_pos = match dir {
                                SplitDirection::Horizontal => e.position.y.as_f32(),
                                SplitDirection::Vertical => e.position.x.as_f32(),
                            };
                            let delta = current_pos - ds.start_pos;
                            // TODO(US-007): replace 800px estimate with actual container bounds
                            let container_estimate = 800.0_f32;
                            let ratio_delta = delta / container_estimate;
                            let new_before =
                                (ds.start_ratio_before + ratio_delta).clamp(MIN_RATIO, MAX_RATIO);
                            let new_after =
                                (ds.start_ratio_after - ratio_delta).clamp(MIN_RATIO, MAX_RATIO);
                            if let Some(r) = child_ratios.get(ds.divider_idx) {
                                r.set(new_before);
                            }
                            if let Some(r) = child_ratios.get(ds.divider_idx + 1) {
                                r.set(new_after);
                            }
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
                        if let LayoutTree::Leaf(pane) = &children[i].node {
                            if pane.read(cx).focus_handle(cx).is_focused(window) {
                                insert_sibling(children, i, new_pane);
                                return true;
                            }
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
                        if let LayoutTree::Leaf(pane) = &children[i].node {
                            if pane == target {
                                insert_sibling(children, i, new_pane);
                                return true;
                            }
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

    /// Close the focused pane. Consumes self and returns the surviving tree (if any).
    pub fn close_focused(self, window: &Window, cx: &App) -> (Option<LayoutTree>, bool) {
        match self {
            LayoutTree::Leaf(pane) => {
                if pane.read(cx).focus_handle(cx).is_focused(window) {
                    (None, true)
                } else {
                    (Some(LayoutTree::Leaf(pane)), false)
                }
            }
            LayoutTree::Container {
                direction,
                children,
                drag,
            } => {
                let mut new_children = Vec::with_capacity(children.len());
                let mut closed = false;

                for child in children {
                    if closed {
                        new_children.push(child);
                        continue;
                    }
                    let (new_node, was_closed) = child.node.close_focused(window, cx);
                    if was_closed {
                        closed = true;
                        if let Some(node) = new_node {
                            new_children.push(LayoutChild {
                                node,
                                ratio: child.ratio,
                                computed_size: child.computed_size,
                            });
                        }
                        // else: child was the focused leaf — removed entirely
                    } else {
                        new_children.push(LayoutChild {
                            node: new_node
                                .expect("close_focused: non-closed child must return Some"),
                            ratio: child.ratio,
                            computed_size: child.computed_size,
                        });
                    }
                }

                if !closed {
                    return (
                        Some(LayoutTree::Container {
                            direction,
                            children: new_children,
                            drag,
                        }),
                        false,
                    );
                }

                // Cancel any in-progress drag before structural changes
                drag.set(None);

                match new_children.len() {
                    0 => (None, true),
                    1 => {
                        // Collapse single-child container (AC-6)
                        let child = new_children.into_iter().next().unwrap();
                        (Some(child.node), true)
                    }
                    _ => {
                        normalize_ratios(&new_children);
                        (
                            Some(LayoutTree::Container {
                                direction,
                                children: new_children,
                                drag,
                            }),
                            true,
                        )
                    }
                }
            }
        }
    }

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

    /// Remove a specific pane entity from the tree. Consumes `self`, returns
    /// the surviving tree (None if the removed pane was the only leaf).
    pub fn remove_pane(self, target: &Entity<Pane>) -> Option<LayoutTree> {
        match self {
            LayoutTree::Leaf(ref pane) => {
                if pane == target {
                    None
                } else {
                    Some(self)
                }
            }
            LayoutTree::Container {
                direction,
                children,
                drag,
            } => {
                let mut new_children = Vec::with_capacity(children.len());
                for child in children {
                    if let Some(node) = child.node.remove_pane(target) {
                        new_children.push(LayoutChild {
                            node,
                            ratio: child.ratio,
                            computed_size: child.computed_size,
                        });
                    }
                }

                // Cancel any in-progress drag before structural changes
                drag.set(None);

                match new_children.len() {
                    0 => None,
                    1 => Some(new_children.into_iter().next().unwrap().node),
                    _ => {
                        normalize_ratios(&new_children);
                        Some(LayoutTree::Container {
                            direction,
                            children: new_children,
                            drag,
                        })
                    }
                }
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

    /// Maximum depth of the tree (leaf = 0, container with leaves = 1).
    pub fn depth(&self) -> usize {
        match self {
            LayoutTree::Leaf(_) => 0,
            LayoutTree::Container { children, .. } => {
                1 + children.iter().map(|c| c.node.depth()).max().unwrap_or(0)
            }
        }
    }

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
