//! Core layout tree types, constants, and ratio-manipulation helpers.
//!
//! Part of the US-029 `split.rs` decomposition. The `LayoutTree` enum + its
//! `LayoutChild`/`DragState` supporting types live here; rendering, mutation,
//! navigation, preset, and serialization logic is split into sibling modules.

use std::cell::Cell;
use std::rc::Rc;

use gpui::Entity;

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
    pub(crate) divider_idx: usize,
    pub(crate) start_pos: f32,
    pub(crate) start_ratio_before: f32,
    pub(crate) start_ratio_after: f32,
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
        /// Actual main-axis pixel size of this container, captured each frame
        /// via canvas() prepaint. Used for pixel-accurate drag-to-resize.
        container_size: Rc<Cell<f32>>,
    },
}

pub(super) const DIVIDER_PX: f32 = 4.0;
/// Minimum pane size in pixels. No pane may be resized below this.
pub(super) const MIN_PANE_SIZE: f32 = 80.0;

/// Re-normalize ratios so they sum to 1.0 (proportional scaling).
pub(super) fn normalize_ratios(children: &[LayoutChild]) {
    let sum: f32 = children.iter().map(|c| c.ratio.get()).sum();
    if sum > 0.0 && (sum - 1.0).abs() > f32::EPSILON {
        for child in children {
            child.ratio.set(child.ratio.get() / sum);
        }
    }
}

/// Redistribute a removed child's ratio equally among remaining children.
/// Each sibling gets `removed_ratio / num_remaining` added to its current ratio.
pub(super) fn redistribute_equal(children: &[LayoutChild], removed_ratio: f32) {
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
pub(super) fn insert_sibling(children: &mut Vec<LayoutChild>, idx: usize, new_pane: Entity<Pane>) {
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
    pub(super) fn new_split(
        direction: SplitDirection,
        first: LayoutTree,
        second: LayoutTree,
    ) -> Self {
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
}
