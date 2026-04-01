// US-003: Binary-tree split layout engine

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when manipulating a [`SplitTree`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SplitTreeError {
    #[error("pane not found: {0}")]
    PaneNotFound(Uuid),

    #[error("cannot close the last pane")]
    CannotCloseLastPane,
}

// ---------------------------------------------------------------------------
// Direction
// ---------------------------------------------------------------------------

/// Axis along which a split divides its area.
///
/// * `Horizontal` -- splits the **width** (left | right).
/// * `Vertical`   -- splits the **height** (top | bottom).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Horizontal,
    Vertical,
}

// ---------------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------------

/// An axis-aligned rectangle used for layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ---------------------------------------------------------------------------
// SplitTree
// ---------------------------------------------------------------------------

/// A binary tree that describes how panes are arranged inside a tab.
///
/// Every leaf holds a `pane_id`; every interior node holds a split direction
/// and a ratio that determines how the available space is divided between its
/// two children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SplitTree {
    Leaf {
        pane_id: Uuid,
    },
    Split {
        direction: Direction,
        ratio: f64,
        first: Box<SplitTree>,
        second: Box<SplitTree>,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Clamp a ratio to the allowed range `[0.1, 0.9]`.
fn clamp_ratio(r: f64) -> f64 {
    r.clamp(0.1, 0.9)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl SplitTree {
    /// Create a new tree containing a single pane.
    pub fn new(pane_id: Uuid) -> Self {
        SplitTree::Leaf { pane_id }
    }

    // -- query ---------------------------------------------------------------

    /// Returns `Some(true)` if the pane exists in the tree.
    /// Returns `None` if the pane is not found.
    pub fn find_pane(&self, target: Uuid) -> Option<bool> {
        match self {
            SplitTree::Leaf { pane_id } => {
                if *pane_id == target {
                    Some(true)
                } else {
                    None
                }
            }
            SplitTree::Split { first, second, .. } => {
                first.find_pane(target).or_else(|| second.find_pane(target))
            }
        }
    }

    /// Collect every leaf pane id in left-to-right / top-to-bottom order
    /// (i.e. an in-order traversal where "first" comes before "second").
    pub fn all_panes(&self) -> Vec<Uuid> {
        let mut out = Vec::new();
        self.collect_panes(&mut out);
        out
    }

    fn collect_panes(&self, out: &mut Vec<Uuid>) {
        match self {
            SplitTree::Leaf { pane_id } => out.push(*pane_id),
            SplitTree::Split { first, second, .. } => {
                first.collect_panes(out);
                second.collect_panes(out);
            }
        }
    }

    // -- mutation -------------------------------------------------------------

    /// Split the leaf identified by `pane_id` along `direction`.
    ///
    /// The existing leaf becomes the **first** child; a brand-new leaf (with a
    /// fresh UUID) becomes the **second** child.  The default ratio is `0.5`.
    ///
    /// Returns the UUID of the newly created pane.
    pub fn split(&mut self, pane_id: Uuid, direction: Direction) -> Result<Uuid, SplitTreeError> {
        let (found, new_id) = self.split_inner(pane_id, direction);
        if found {
            Ok(new_id.unwrap())
        } else {
            Err(SplitTreeError::PaneNotFound(pane_id))
        }
    }

    /// Close (remove) the leaf identified by `pane_id`.
    ///
    /// If the pane is the only leaf in the tree, returns
    /// [`SplitTreeError::CannotCloseLastPane`].  Otherwise the parent split
    /// node is replaced by the sibling subtree.
    pub fn close(&mut self, pane_id: Uuid) -> Result<(), SplitTreeError> {
        // Closing the root leaf is not allowed.
        if let SplitTree::Leaf { pane_id: id } = self {
            if *id == pane_id {
                return Err(SplitTreeError::CannotCloseLastPane);
            }
            return Err(SplitTreeError::PaneNotFound(pane_id));
        }

        if self.close_inner(pane_id) {
            Ok(())
        } else {
            Err(SplitTreeError::PaneNotFound(pane_id))
        }
    }

    /// Adjust the split ratio of the **parent** split that contains `pane_id`
    /// as a direct child.  The ratio is clamped to `[0.1, 0.9]`.
    pub fn resize(&mut self, pane_id: Uuid, new_ratio: f64) -> Result<(), SplitTreeError> {
        if self.resize_inner(pane_id, clamp_ratio(new_ratio)) {
            Ok(())
        } else {
            Err(SplitTreeError::PaneNotFound(pane_id))
        }
    }

    // -- layout ---------------------------------------------------------------

    /// Compute pixel-precise rectangles for every leaf in the tree.
    ///
    /// The result is ordered identically to [`all_panes`](Self::all_panes).
    pub fn layout(&self, width: f64, height: f64) -> Vec<(Uuid, Rect)> {
        let mut out = Vec::new();
        self.layout_inner(
            Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            &mut out,
        );
        out
    }

    fn layout_inner(&self, area: Rect, out: &mut Vec<(Uuid, Rect)>) {
        match self {
            SplitTree::Leaf { pane_id } => {
                out.push((*pane_id, area));
            }
            SplitTree::Split {
                direction,
                ratio,
                first,
                second,
            } => match direction {
                Direction::Horizontal => {
                    let first_w = area.width * ratio;
                    let second_w = area.width - first_w;
                    first.layout_inner(
                        Rect {
                            x: area.x,
                            y: area.y,
                            width: first_w,
                            height: area.height,
                        },
                        out,
                    );
                    second.layout_inner(
                        Rect {
                            x: area.x + first_w,
                            y: area.y,
                            width: second_w,
                            height: area.height,
                        },
                        out,
                    );
                }
                Direction::Vertical => {
                    let first_h = area.height * ratio;
                    let second_h = area.height - first_h;
                    first.layout_inner(
                        Rect {
                            x: area.x,
                            y: area.y,
                            width: area.width,
                            height: first_h,
                        },
                        out,
                    );
                    second.layout_inner(
                        Rect {
                            x: area.x,
                            y: area.y + first_h,
                            width: area.width,
                            height: second_h,
                        },
                        out,
                    );
                }
            },
        }
    }

    // -- private helpers ------------------------------------------------------

    /// Recursively find the leaf with `target` and replace it with a split.
    /// Returns `(found, new_pane_id)`.
    fn split_inner(&mut self, target: Uuid, direction: Direction) -> (bool, Option<Uuid>) {
        match self {
            SplitTree::Leaf { pane_id } if *pane_id == target => {
                let new_id = Uuid::new_v4();
                let old_leaf = SplitTree::Leaf { pane_id: *pane_id };
                let new_leaf = SplitTree::Leaf { pane_id: new_id };
                *self = SplitTree::Split {
                    direction,
                    ratio: 0.5,
                    first: Box::new(old_leaf),
                    second: Box::new(new_leaf),
                };
                (true, Some(new_id))
            }
            SplitTree::Leaf { .. } => (false, None),
            SplitTree::Split { first, second, .. } => {
                let res = first.split_inner(target, direction);
                if res.0 {
                    return res;
                }
                second.split_inner(target, direction)
            }
        }
    }

    /// Remove the leaf `target` and collapse its parent split.
    /// Returns `true` if the pane was found and removed.
    fn close_inner(&mut self, target: Uuid) -> bool {
        // We can only collapse when `self` is a Split whose direct child is
        // the target leaf.
        if let SplitTree::Split { first, second, .. } = self {
            // Check if first child is the target leaf.
            if let SplitTree::Leaf { pane_id } = first.as_ref() {
                if *pane_id == target {
                    // Replace self with the sibling (second).
                    *self = *second.clone();
                    return true;
                }
            }
            // Check if second child is the target leaf.
            if let SplitTree::Leaf { pane_id } = second.as_ref() {
                if *pane_id == target {
                    *self = *first.clone();
                    return true;
                }
            }
            // Recurse into children.
            if first.close_inner(target) {
                return true;
            }
            return second.close_inner(target);
        }
        false
    }

    /// Set the ratio of the parent split that has `target` as a direct child.
    /// Returns `true` if found.
    fn resize_inner(&mut self, target: Uuid, new_ratio: f64) -> bool {
        if let SplitTree::Split {
            first,
            second,
            ratio,
            ..
        } = self
        {
            // Is target a direct child?
            let first_is_target =
                matches!(first.as_ref(), SplitTree::Leaf { pane_id } if *pane_id == target);
            let second_is_target =
                matches!(second.as_ref(), SplitTree::Leaf { pane_id } if *pane_id == target);

            if first_is_target || second_is_target {
                *ratio = new_ratio;
                return true;
            }

            // Recurse.
            if first.resize_inner(target, new_ratio) {
                return true;
            }
            return second.resize_inner(target, new_ratio);
        }
        false
    }
}

impl SplitTree {
    /// Builder-friendly split that consumes `self` and returns `(Self, new_pane_id)`.
    /// Prefer the `&mut self` version ([`split`](Self::split)) in most code.
    pub fn split_new(
        mut self,
        pane_id: Uuid,
        direction: Direction,
    ) -> Result<(Self, Uuid), SplitTreeError> {
        let (found, new_id) = self.split_inner(pane_id, direction);
        if !found {
            return Err(SplitTreeError::PaneNotFound(pane_id));
        }
        Ok((self, new_id.unwrap()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- helpers -------------------------------------------------------------

    fn approx_eq(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn assert_rect_eq(actual: &Rect, expected: &Rect) {
        assert!(
            approx_eq(actual.x, expected.x)
                && approx_eq(actual.y, expected.y)
                && approx_eq(actual.width, expected.width)
                && approx_eq(actual.height, expected.height),
            "rects differ:\n  actual:   {actual:?}\n  expected: {expected:?}"
        );
    }

    // -- split ---------------------------------------------------------------

    #[test]
    fn split_creates_two_children() {
        let pane_a = Uuid::new_v4();
        let mut tree = SplitTree::new(pane_a);

        let pane_b = tree.split(pane_a, Direction::Horizontal).unwrap();

        assert!(tree.find_pane(pane_a).is_some());
        assert!(tree.find_pane(pane_b).is_some());
        assert_eq!(tree.all_panes(), vec![pane_a, pane_b]);
    }

    #[test]
    fn split_nonexistent_pane_returns_error() {
        let pane_a = Uuid::new_v4();
        let mut tree = SplitTree::new(pane_a);
        let bogus = Uuid::new_v4();

        let err = tree.split(bogus, Direction::Vertical).unwrap_err();
        assert_eq!(err, SplitTreeError::PaneNotFound(bogus));
    }

    #[test]
    fn split_preserves_order() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        let c = tree.split(b, Direction::Vertical).unwrap();

        assert_eq!(tree.all_panes(), vec![a, b, c]);
    }

    // -- close ---------------------------------------------------------------

    #[test]
    fn close_last_pane_returns_error() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        assert_eq!(
            tree.close(a).unwrap_err(),
            SplitTreeError::CannotCloseLastPane,
        );
    }

    #[test]
    fn close_nonexistent_pane_returns_error() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let bogus = Uuid::new_v4();
        assert_eq!(
            tree.close(bogus).unwrap_err(),
            SplitTreeError::PaneNotFound(bogus),
        );
    }

    #[test]
    fn close_collapses_to_sibling() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();

        tree.close(b).unwrap();

        assert_eq!(tree.all_panes(), vec![a]);
        assert_eq!(tree, SplitTree::Leaf { pane_id: a });
    }

    #[test]
    fn close_first_child_collapses_to_second() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();

        tree.close(a).unwrap();

        assert_eq!(tree.all_panes(), vec![b]);
        assert_eq!(tree, SplitTree::Leaf { pane_id: b });
    }

    #[test]
    fn close_in_nested_tree() {
        // Build: split a -> (a, b), then split b -> (b, c)
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        let c = tree.split(b, Direction::Vertical).unwrap();

        // Close b -- the inner split (b, c) collapses to c.
        tree.close(b).unwrap();

        assert_eq!(tree.all_panes(), vec![a, c]);
        assert!(tree.find_pane(b).is_none());
    }

    // -- resize --------------------------------------------------------------

    #[test]
    fn resize_clamps_ratio() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let _b = tree.split(a, Direction::Horizontal).unwrap();

        // Try to set ratio to 0.0 -- should clamp to 0.1
        tree.resize(a, 0.0).unwrap();

        if let SplitTree::Split { ratio, .. } = &tree {
            assert!(approx_eq(*ratio, 0.1));
        } else {
            panic!("expected Split");
        }

        // Try to set ratio to 1.0 -- should clamp to 0.9
        tree.resize(a, 1.0).unwrap();

        if let SplitTree::Split { ratio, .. } = &tree {
            assert!(approx_eq(*ratio, 0.9));
        } else {
            panic!("expected Split");
        }
    }

    #[test]
    fn resize_sets_exact_ratio() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let _b = tree.split(a, Direction::Horizontal).unwrap();

        tree.resize(a, 0.3).unwrap();

        if let SplitTree::Split { ratio, .. } = &tree {
            assert!(approx_eq(*ratio, 0.3));
        } else {
            panic!("expected Split");
        }
    }

    #[test]
    fn resize_nonexistent_pane_returns_error() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let bogus = Uuid::new_v4();
        assert_eq!(
            tree.resize(bogus, 0.5).unwrap_err(),
            SplitTreeError::PaneNotFound(bogus),
        );
    }

    // -- find_pane -----------------------------------------------------------

    #[test]
    fn find_pane_returns_some_for_existing() {
        let a = Uuid::new_v4();
        let tree = SplitTree::new(a);
        assert_eq!(tree.find_pane(a), Some(true));
    }

    #[test]
    fn find_pane_returns_none_for_missing() {
        let a = Uuid::new_v4();
        let tree = SplitTree::new(a);
        assert_eq!(tree.find_pane(Uuid::new_v4()), None);
    }

    // -- layout --------------------------------------------------------------

    #[test]
    fn layout_single_pane() {
        let a = Uuid::new_v4();
        let tree = SplitTree::new(a);
        let rects = tree.layout(1000.0, 600.0);

        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0].0, a);
        assert_rect_eq(
            &rects[0].1,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 600.0,
            },
        );
    }

    #[test]
    fn layout_horizontal_split_at_half() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();

        let rects = tree.layout(1000.0, 600.0);

        assert_eq!(rects.len(), 2);

        // First child (a): left half
        assert_eq!(rects[0].0, a);
        assert_rect_eq(
            &rects[0].1,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 500.0,
                height: 600.0,
            },
        );

        // Second child (b): right half
        assert_eq!(rects[1].0, b);
        assert_rect_eq(
            &rects[1].1,
            &Rect {
                x: 500.0,
                y: 0.0,
                width: 500.0,
                height: 600.0,
            },
        );
    }

    #[test]
    fn layout_vertical_split_at_half() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Vertical).unwrap();

        let rects = tree.layout(1000.0, 600.0);

        assert_eq!(rects.len(), 2);

        // First child (a): top half
        assert_eq!(rects[0].0, a);
        assert_rect_eq(
            &rects[0].1,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 300.0,
            },
        );

        // Second child (b): bottom half
        assert_eq!(rects[1].0, b);
        assert_rect_eq(
            &rects[1].1,
            &Rect {
                x: 0.0,
                y: 300.0,
                width: 1000.0,
                height: 300.0,
            },
        );
    }

    #[test]
    fn layout_deeply_nested_three_levels() {
        // Level 0: root split H (a | rest)
        // Level 1: rest split V  (b / bottom)
        // Level 2: bottom split H (c | d)
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);

        let b = tree.split(a, Direction::Horizontal).unwrap(); // a | b
        let c = tree.split(b, Direction::Vertical).unwrap(); // b / c
        let d = tree.split(c, Direction::Horizontal).unwrap(); // c | d

        let rects = tree.layout(1000.0, 600.0);
        assert_eq!(rects.len(), 4);

        let panes: Vec<Uuid> = rects.iter().map(|(id, _)| *id).collect();
        assert_eq!(panes, vec![a, b, c, d]);

        // a: left half of root (500 x 600)
        assert_rect_eq(
            &rects[0].1,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 500.0,
                height: 600.0,
            },
        );

        // b: top-right (500 x 300)
        assert_rect_eq(
            &rects[1].1,
            &Rect {
                x: 500.0,
                y: 0.0,
                width: 500.0,
                height: 300.0,
            },
        );

        // c: bottom-right-left (250 x 300)
        assert_rect_eq(
            &rects[2].1,
            &Rect {
                x: 500.0,
                y: 300.0,
                width: 250.0,
                height: 300.0,
            },
        );

        // d: bottom-right-right (250 x 300)
        assert_rect_eq(
            &rects[3].1,
            &Rect {
                x: 750.0,
                y: 300.0,
                width: 250.0,
                height: 300.0,
            },
        );
    }

    #[test]
    fn layout_respects_custom_ratio() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let _b = tree.split(a, Direction::Horizontal).unwrap();
        tree.resize(a, 0.3).unwrap();

        let rects = tree.layout(1000.0, 600.0);

        assert_rect_eq(
            &rects[0].1,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 600.0,
            },
        );
        assert_rect_eq(
            &rects[1].1,
            &Rect {
                x: 300.0,
                y: 0.0,
                width: 700.0,
                height: 600.0,
            },
        );
    }

    // -- serialization -------------------------------------------------------

    #[test]
    fn serde_roundtrip_leaf() {
        let a = Uuid::new_v4();
        let tree = SplitTree::new(a);

        let json = serde_json::to_string(&tree).unwrap();
        let restored: SplitTree = serde_json::from_str(&json).unwrap();

        assert_eq!(tree, restored);
    }

    #[test]
    fn serde_roundtrip_nested() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        let _c = tree.split(b, Direction::Vertical).unwrap();

        let json = serde_json::to_string_pretty(&tree).unwrap();
        let restored: SplitTree = serde_json::from_str(&json).unwrap();

        assert_eq!(tree, restored);
    }

    #[test]
    fn serde_direction_roundtrip() {
        let json = serde_json::to_string(&Direction::Horizontal).unwrap();
        assert_eq!(json, "\"Horizontal\"");
        let restored: Direction = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, Direction::Horizontal);
    }

    #[test]
    fn serde_rect_roundtrip() {
        let rect = Rect {
            x: 10.0,
            y: 20.0,
            width: 300.0,
            height: 400.0,
        };
        let json = serde_json::to_string(&rect).unwrap();
        let restored: Rect = serde_json::from_str(&json).unwrap();
        assert_eq!(rect, restored);
    }

    // -- edge cases ----------------------------------------------------------

    #[test]
    fn split_returns_new_pane_id() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        assert!(tree.find_pane(b).is_some());
        assert_ne!(a, b);
    }

    #[test]
    fn split_new_builder_works() {
        let a = Uuid::new_v4();
        let tree = SplitTree::new(a);
        let (tree, b) = tree.split_new(a, Direction::Vertical).unwrap();
        assert!(tree.find_pane(a).is_some());
        assert!(tree.find_pane(b).is_some());
    }

    #[test]
    fn close_after_multiple_splits() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        let c = tree.split(a, Direction::Vertical).unwrap();
        // Tree: Split(H, Split(V, a, c), b)

        // Close c -- inner split collapses back to a.
        tree.close(c).unwrap();
        assert_eq!(tree.all_panes(), vec![a, b]);

        // Close a -- collapses to b.
        tree.close(a).unwrap();
        assert_eq!(tree.all_panes(), vec![b]);
        assert_eq!(tree, SplitTree::Leaf { pane_id: b });
    }

    #[test]
    fn all_panes_order_matches_layout_order() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        let _c = tree.split(b, Direction::Vertical).unwrap();

        let pane_ids = tree.all_panes();
        let layout_ids: Vec<Uuid> = tree
            .layout(800.0, 600.0)
            .iter()
            .map(|(id, _)| *id)
            .collect();

        assert_eq!(pane_ids, layout_ids);
    }

    #[test]
    fn resize_leaf_without_parent_split_returns_error() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        assert_eq!(
            tree.resize(a, 0.5).unwrap_err(),
            SplitTreeError::PaneNotFound(a),
        );
    }

    #[test]
    fn resize_nested_pane() {
        let a = Uuid::new_v4();
        let mut tree = SplitTree::new(a);
        let b = tree.split(a, Direction::Horizontal).unwrap();
        let c = tree.split(b, Direction::Vertical).unwrap();

        // Resize c's parent split (the V-split containing b and c).
        tree.resize(c, 0.7).unwrap();

        let rects = tree.layout(1000.0, 600.0);
        // b should get 70% of the right half's height, c gets 30%.
        assert_rect_eq(
            &rects[1].1,
            &Rect {
                x: 500.0,
                y: 0.0,
                width: 500.0,
                height: 420.0, // 600 * 0.7
            },
        );
        assert_rect_eq(
            &rects[2].1,
            &Rect {
                x: 500.0,
                y: 420.0,
                width: 500.0,
                height: 180.0, // 600 * 0.3
            },
        );
    }

    #[test]
    fn clamp_ratio_boundaries() {
        assert!(approx_eq(clamp_ratio(0.0), 0.1));
        assert!(approx_eq(clamp_ratio(-1.0), 0.1));
        assert!(approx_eq(clamp_ratio(0.1), 0.1));
        assert!(approx_eq(clamp_ratio(0.5), 0.5));
        assert!(approx_eq(clamp_ratio(0.9), 0.9));
        assert!(approx_eq(clamp_ratio(1.0), 0.9));
        assert!(approx_eq(clamp_ratio(99.0), 0.9));
    }

    #[test]
    fn error_display_messages() {
        let id = Uuid::new_v4();
        assert_eq!(
            SplitTreeError::PaneNotFound(id).to_string(),
            format!("pane not found: {id}"),
        );
        assert_eq!(
            SplitTreeError::CannotCloseLastPane.to_string(),
            "cannot close the last pane",
        );
    }
}
