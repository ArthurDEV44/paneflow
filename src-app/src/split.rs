//! N-ary tree layout for terminal panes.
//!
//! Leaf nodes hold terminal pane entities. Container nodes hold 2+ children
//! with a direction (horizontal/vertical) and per-child flex ratios.

use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Entity, Focusable, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Styled, Window, canvas, div, px, rgb,
};

use paneflow_config::schema::{LayoutNode, SurfaceDefinition};

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

impl LayoutChild {
    /// Maximum pixels this child can yield if shrunk along `dir`.
    /// Uses `computed_size` (the child's actual pixel size from layout).
    pub fn resize_check(&self, dir: SplitDirection) -> f32 {
        match &self.node {
            LayoutTree::Leaf(_) => (self.computed_size.get() - MIN_PANE_SIZE).max(0.0),
            LayoutTree::Container { direction, .. } => {
                if *direction == dir {
                    // Same direction: children can shrink independently
                    if let LayoutTree::Container { children, .. } = &self.node {
                        children.iter().map(|c| c.resize_check(dir)).sum()
                    } else {
                        0.0
                    }
                } else {
                    // Cross direction: limited by tightest child
                    if let LayoutTree::Container { children, .. } = &self.node {
                        children
                            .iter()
                            .map(|c| c.resize_check(dir))
                            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    }
                }
            }
        }
    }
}

pub enum LayoutTree {
    Leaf(Entity<Pane>),
    Container {
        direction: SplitDirection,
        children: Vec<LayoutChild>,
        drag: Rc<Cell<Option<DragState>>>,
        /// Actual main-axis pixel size of this container, captured each frame
        /// via canvas() prepaint. Used for pixel-accurate drag-to-resize.
        container_size: Rc<Cell<f32>>,
    },
}

const DIVIDER_PX: f32 = 4.0;
/// Minimum pane size in pixels. No pane may be resized below this.
const MIN_PANE_SIZE: f32 = 80.0;

/// Re-normalize ratios so they sum to 1.0 (proportional scaling).
fn normalize_ratios(children: &[LayoutChild]) {
    let sum: f32 = children.iter().map(|c| c.ratio.get()).sum();
    if sum > 0.0 && (sum - 1.0).abs() > f32::EPSILON {
        for child in children {
            child.ratio.set(child.ratio.get() / sum);
        }
    }
}

/// Redistribute a removed child's ratio equally among remaining children.
/// Each sibling gets `removed_ratio / num_remaining` added to its current ratio.
fn redistribute_equal(children: &[LayoutChild], removed_ratio: f32) {
    if children.is_empty() {
        return;
    }
    let share = removed_ratio / children.len() as f32;
    for child in children {
        child.ratio.set(child.ratio.get() + share);
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
    debug_assert!(old_ratio.is_finite(), "insert_sibling: ratio is NaN/inf");
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
            container_size: Rc::new(Cell::new(0.0)),
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
                let child_constraints: Vec<f32> =
                    children.iter().map(|c| c.resize_check(dir)).collect();

                let mut container = div().flex().size_full().overflow_hidden().on_mouse_move(
                    move |e, _window, _cx| {
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

                            // Constraint clamping: use the pre-computed resize_check of the
                            // child being shrunk to prevent nested subtree minimum violations.
                            let pair_sum = ds.start_ratio_before + ds.start_ratio_after;
                            let min_r = MIN_PANE_SIZE / csize;

                            // Max shrinkable ratio for each side, from recursive constraints
                            let max_shrink_before = child_constraints
                                .get(ds.divider_idx)
                                .copied()
                                .unwrap_or(0.0)
                                / csize;
                            let max_shrink_after = child_constraints
                                .get(ds.divider_idx + 1)
                                .copied()
                                .unwrap_or(0.0)
                                / csize;

                            // Clamp: child[i] can't shrink more than its constraint,
                            // child[i+1] can't shrink more than its constraint.
                            let lower = (ds.start_ratio_before - max_shrink_before).max(min_r);
                            let upper =
                                (ds.start_ratio_before + max_shrink_after).min(pair_sum - min_r);
                            // Guarantee lower <= upper (can invert with tiny pair_sum)
                            let upper = upper.max(lower);
                            let new_before =
                                (ds.start_ratio_before + ratio_delta).clamp(lower, upper);
                            let new_after = pair_sum - new_before;

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

    /// Close the focused pane. Consumes self and returns:
    /// - `tree`: the surviving layout (None if the last pane was closed)
    /// - `closed`: whether a pane was actually closed
    /// - `focus_neighbor`: the pane that should receive focus (previous sibling,
    ///    or next sibling if the closed pane was first)
    pub fn close_focused(
        self,
        window: &Window,
        cx: &App,
    ) -> (Option<LayoutTree>, bool, Option<Entity<Pane>>) {
        match self {
            LayoutTree::Leaf(pane) => {
                if pane.read(cx).focus_handle(cx).is_focused(window) {
                    (None, true, None)
                } else {
                    (Some(LayoutTree::Leaf(pane)), false, None)
                }
            }
            LayoutTree::Container {
                direction,
                children,
                drag,
                container_size,
            } => {
                let mut new_children = Vec::with_capacity(children.len());
                let mut closed = false;
                let mut closed_idx: Option<usize> = None;
                let mut removed_ratio = 0.0_f32;
                let mut focus_neighbor: Option<Entity<Pane>> = None;

                for (i, child) in children.into_iter().enumerate() {
                    if closed {
                        new_children.push(child);
                        continue;
                    }
                    let (new_node, was_closed, nested_focus) = child.node.close_focused(window, cx);
                    if was_closed {
                        closed = true;
                        closed_idx = Some(i);
                        // Propagate focus neighbor from deeper levels
                        focus_neighbor = nested_focus;
                        if let Some(node) = new_node {
                            new_children.push(LayoutChild {
                                node,
                                ratio: child.ratio,
                                computed_size: child.computed_size,
                            });
                        } else {
                            // Direct child leaf was removed — record its ratio
                            removed_ratio = child.ratio.get();
                        }
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
                            container_size,
                        }),
                        false,
                        None,
                    );
                }

                // Cancel any in-progress drag before structural changes
                drag.set(None);

                // Determine focus neighbor when a direct child was removed
                // (only if no nested focus was already determined)
                if focus_neighbor.is_none() {
                    if let Some(idx) = closed_idx {
                        // Prefer previous sibling, fall back to next
                        if idx > 0 {
                            focus_neighbor =
                                new_children.get(idx - 1).and_then(|c| c.node.last_leaf());
                        } else {
                            focus_neighbor = new_children.first().and_then(|c| c.node.first_leaf());
                        }
                    }
                }

                match new_children.len() {
                    0 => (None, true, None),
                    1 => {
                        // Collapse single-child container
                        let child = new_children.into_iter().next().unwrap();
                        (Some(child.node), true, focus_neighbor)
                    }
                    _ => {
                        // Redistribute removed child's ratio equally
                        if removed_ratio > 0.0 {
                            redistribute_equal(&new_children, removed_ratio);
                        } else {
                            normalize_ratios(&new_children);
                        }
                        (
                            Some(LayoutTree::Container {
                                direction,
                                children: new_children,
                                drag,
                                container_size,
                            }),
                            true,
                            focus_neighbor,
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
                container_size,
            } => {
                let mut new_children = Vec::with_capacity(children.len());
                let mut removed_ratio = 0.0_f32;
                for child in children {
                    if let Some(node) = child.node.remove_pane(target) {
                        new_children.push(LayoutChild {
                            node,
                            ratio: child.ratio,
                            computed_size: child.computed_size,
                        });
                    } else {
                        removed_ratio += child.ratio.get();
                    }
                }

                // Cancel any in-progress drag before structural changes
                drag.set(None);

                match new_children.len() {
                    0 => None,
                    1 => Some(new_children.into_iter().next().unwrap().node),
                    _ => {
                        if removed_ratio > 0.0 {
                            redistribute_equal(&new_children, removed_ratio);
                        } else {
                            normalize_ratios(&new_children);
                        }
                        Some(LayoutTree::Container {
                            direction,
                            children: new_children,
                            drag,
                            container_size,
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

    /// Build a flat container with all panes at equal ratios in the given direction.
    /// Returns `None` for empty, `Leaf` for single pane, `Container` for 2+.
    pub fn from_panes_equal(direction: SplitDirection, panes: Vec<Entity<Pane>>) -> Option<Self> {
        match panes.len() {
            0 => None,
            1 => Some(LayoutTree::Leaf(panes.into_iter().next().unwrap())),
            n => {
                let ratio = 1.0 / n as f32;
                let children = panes
                    .into_iter()
                    .map(|pane| LayoutChild {
                        node: LayoutTree::Leaf(pane),
                        ratio: Rc::new(Cell::new(ratio)),
                        computed_size: Rc::new(Cell::new(0.0)),
                    })
                    .collect();
                Some(LayoutTree::Container {
                    direction,
                    children,
                    drag: Rc::new(Cell::new(None)),
                    container_size: Rc::new(Cell::new(0.0)),
                })
            }
        }
    }

    /// Build a "main-vertical" layout: one main pane at 60% width on the left,
    /// remaining panes stacked vertically on the right at 40%.
    /// `main_pane` is placed first. Returns `None` for empty, `Leaf` for single.
    pub fn main_vertical(main_pane: Entity<Pane>, others: Vec<Entity<Pane>>) -> Option<Self> {
        if others.is_empty() {
            return Some(LayoutTree::Leaf(main_pane));
        }

        // Right side: stack remaining panes with equal ratios (Horizontal = top/bottom)
        let right = LayoutTree::from_panes_equal(SplitDirection::Horizontal, others)
            .expect("others is non-empty");

        // Outer: Vertical (side by side) — main 60%, right panel 40%
        Some(LayoutTree::Container {
            direction: SplitDirection::Vertical,
            children: vec![
                LayoutChild {
                    node: LayoutTree::Leaf(main_pane),
                    ratio: Rc::new(Cell::new(0.6)),
                    computed_size: Rc::new(Cell::new(0.0)),
                },
                LayoutChild {
                    node: right,
                    ratio: Rc::new(Cell::new(0.4)),
                    computed_size: Rc::new(Cell::new(0.0)),
                },
            ],
            drag: Rc::new(Cell::new(None)),
            container_size: Rc::new(Cell::new(0.0)),
        })
    }

    /// Build a tiled grid layout. Uses tmux's algorithm: increment rows and
    /// columns alternately until `rows * cols >= N`. Each row is a Vertical
    /// container; rows are stacked in a Horizontal container.
    /// Returns `None` for empty, `Leaf` for single.
    pub fn tiled(panes: Vec<Entity<Pane>>) -> Option<Self> {
        match panes.len() {
            0 => return None,
            1 => return Some(LayoutTree::Leaf(panes.into_iter().next().unwrap())),
            _ => {}
        }

        let n = panes.len();
        // tmux algorithm: increment rows and cols alternately until rows*cols >= n
        let mut rows = 1usize;
        let mut cols = 1usize;
        while rows * cols < n {
            if cols <= rows {
                cols += 1;
            } else {
                rows += 1;
            }
        }

        // Distribute panes across rows
        let row_ratio = 1.0 / rows as f32;
        let mut pane_iter = panes.into_iter();
        let mut row_children: Vec<LayoutChild> = Vec::with_capacity(rows);

        for r in 0..rows {
            // Last row may have fewer panes
            let panes_in_row = if r < rows - 1 {
                cols
            } else {
                n - cols * (rows - 1)
            };

            let row_panes: Vec<Entity<Pane>> = pane_iter.by_ref().take(panes_in_row).collect();
            let row_tree = LayoutTree::from_panes_equal(SplitDirection::Vertical, row_panes)
                .expect("row is non-empty");

            row_children.push(LayoutChild {
                node: row_tree,
                ratio: Rc::new(Cell::new(row_ratio)),
                computed_size: Rc::new(Cell::new(0.0)),
            });
        }

        Some(LayoutTree::Container {
            direction: SplitDirection::Horizontal,
            children: row_children,
            drag: Rc::new(Cell::new(None)),
            container_size: Rc::new(Cell::new(0.0)),
        })
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

    /// Compute the maximum pixels this subtree can yield if shrunk along `dir`.
    ///
    /// For accurate leaf results, use `LayoutChild::resize_check()` instead —
    /// it has access to the leaf's `computed_size`. This method returns 0.0
    /// for bare Leaf nodes (they lack size context at the LayoutTree level).
    ///
    /// - **Same-direction container:** sum of children's resize_check (all shrink independently)
    /// - **Cross-direction container:** min of children's resize_check (tightest constraint)
    pub fn resize_check(&self, dir: SplitDirection) -> f32 {
        match self {
            LayoutTree::Leaf(_) => 0.0,
            LayoutTree::Container {
                direction,
                children,
                ..
            } => {
                if *direction == dir {
                    // Same direction: all children can yield space independently
                    children.iter().map(|c| c.resize_check(dir)).sum()
                } else {
                    // Cross direction: limited by the tightest child
                    children
                        .iter()
                        .map(|c| c.resize_check(dir))
                        .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                        .unwrap_or(0.0)
                }
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

    /// Serialize the layout tree to a `LayoutNode` (config schema type).
    ///
    /// Each leaf produces a `LayoutNode::Pane` with a single `SurfaceDefinition`
    /// containing the terminal's OSC title as `name`. Each container produces a
    /// `LayoutNode::Split` with per-child `ratios` and recursive children.
    pub fn serialize(&self, cx: &App) -> LayoutNode {
        match self {
            LayoutTree::Leaf(pane) => {
                let pane_ref = pane.read(cx);
                let title = if pane_ref.tabs.is_empty() {
                    None
                } else {
                    let t = pane_ref.active_terminal().read(cx).terminal.title.clone();
                    if t.is_empty() { None } else { Some(t) }
                };
                LayoutNode::Pane {
                    surfaces: vec![SurfaceDefinition {
                        surface_type: Some("terminal".to_string()),
                        name: title,
                        command: None,
                        cwd: None,
                        env: None,
                        focus: None,
                    }],
                }
            }
            LayoutTree::Container {
                direction,
                children,
                ..
            } => {
                let dir_str = match direction {
                    SplitDirection::Horizontal => "horizontal",
                    SplitDirection::Vertical => "vertical",
                };
                let ratios: Vec<f64> = children.iter().map(|c| c.ratio.get() as f64).collect();
                let child_nodes: Vec<LayoutNode> =
                    children.iter().map(|c| c.node.serialize(cx)).collect();
                LayoutNode::Split {
                    direction: dir_str.to_string(),
                    ratio: None,
                    ratios: Some(ratios),
                    children: child_nodes,
                }
            }
        }
    }

    /// Rebuild a `LayoutTree` from a `LayoutNode` (config schema).
    ///
    /// Panes are consumed from `panes` in left-to-right order for each leaf.
    /// When `panes` is exhausted, `spawn` is called to create new panes.
    pub fn from_layout_node(
        node: &LayoutNode,
        panes: &mut VecDeque<Entity<Pane>>,
        spawn: &mut impl FnMut() -> Entity<Pane>,
    ) -> Self {
        match node {
            LayoutNode::Pane { .. } => {
                let pane = panes.pop_front().unwrap_or_else(|| spawn());
                LayoutTree::Leaf(pane)
            }
            LayoutNode::Split {
                direction,
                children,
                ..
            } => {
                let dir = match direction.as_str() {
                    "vertical" => SplitDirection::Vertical,
                    _ => SplitDirection::Horizontal,
                };
                let resolved = node.resolved_ratios();
                let child_trees: Vec<LayoutChild> = children
                    .iter()
                    .enumerate()
                    .map(|(i, child_node)| {
                        let ratio = resolved
                            .get(i)
                            .copied()
                            .unwrap_or(1.0 / children.len() as f64);
                        LayoutChild {
                            node: LayoutTree::from_layout_node(child_node, panes, spawn),
                            ratio: Rc::new(Cell::new(ratio as f32)),
                            computed_size: Rc::new(Cell::new(0.0)),
                        }
                    })
                    .collect();
                LayoutTree::Container {
                    direction: dir,
                    children: child_trees,
                    drag: Rc::new(Cell::new(None)),
                    container_size: Rc::new(Cell::new(0.0)),
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
