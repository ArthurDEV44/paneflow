//! Splittable arrangement tree over diff column indices (inc 5 - drag-and-drop
//! diff panes).
//!
//! The diff view keeps its `Vec<Column>` (branch diffs) unchanged - all the
//! index-based logic (selected column, scroll sync, sidebar file lists,
//! jump-to-file) stays intact. This tree only describes how those columns are
//! *arranged* on screen: instead of a fixed side-by-side flex row, columns can
//! be split beside (`Row`) or under (`Col`) one another and freely rearranged
//! by drag-and-drop, mirroring the CLI pane layout.
//!
//! Structural only - equal splits, no ratios (interactive divider resize is a
//! follow-up). Leaves reference `columns` by index; indices are stable (columns
//! are never reordered, only appended/hidden), so the tree survives reloads.
//! [`Arrange::reconcile`] reconciles the stored arrangement with the currently
//! visible columns at render time (prune hidden/removed leaves, append newly
//! visible ones), so hide/show/reload paths need no special hooks.

/// Split axis. `Row` = children laid out side by side (left↔right); `Col` =
/// stacked (top↔bottom, i.e. one pane *under* another).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Row,
    Col,
}

/// A node of the arrangement tree: a single column, or a split of children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arrange {
    Leaf(usize),
    Split { axis: Axis, children: Vec<Arrange> },
}

impl Arrange {
    /// A side-by-side row of the given column ids (a bare `Leaf` for one id).
    /// Empty input yields an empty `Row` split (reconciled away on next render).
    pub fn row(ids: &[usize]) -> Arrange {
        if ids.len() == 1 {
            Arrange::Leaf(ids[0])
        } else {
            Arrange::Split {
                axis: Axis::Row,
                children: ids.iter().map(|&i| Arrange::Leaf(i)).collect(),
            }
        }
    }

    /// Collect every leaf column id, left-to-right / top-to-bottom.
    pub fn leaves(&self, out: &mut Vec<usize>) {
        match self {
            Arrange::Leaf(i) => out.push(*i),
            Arrange::Split { children, .. } => {
                for c in children {
                    c.leaves(out);
                }
            }
        }
    }

    /// Drop leaves for which `keep(id)` is false, then collapse the tree. A bare
    /// `Leaf(id)` with `!keep(id)` is left as-is (the caller, `reconcile`,
    /// rebuilds the root when it empties).
    fn retain(&mut self, keep: &impl Fn(usize) -> bool) {
        if let Arrange::Split { children, .. } = self {
            let mut i = 0;
            while i < children.len() {
                match &mut children[i] {
                    Arrange::Leaf(id) if !keep(*id) => {
                        children.remove(i);
                    }
                    Arrange::Leaf(_) => i += 1,
                    node @ Arrange::Split { .. } => {
                        node.retain(keep);
                        if node.leaf_count() == 0 {
                            children.remove(i);
                        } else {
                            i += 1;
                        }
                    }
                }
            }
        }
    }

    fn leaf_count(&self) -> usize {
        match self {
            Arrange::Leaf(_) => 1,
            Arrange::Split { children, .. } => children.iter().map(|c| c.leaf_count()).sum(),
        }
    }

    /// Collapse single-child splits into their child and flatten a split nested
    /// directly inside a same-axis parent, so the tree stays minimal.
    fn normalize(&mut self) {
        if let Arrange::Split { axis, children } = self {
            let axis = *axis;
            for c in children.iter_mut() {
                c.normalize();
            }
            // Flatten same-axis child splits into this one.
            let mut flat: Vec<Arrange> = Vec::with_capacity(children.len());
            for c in children.drain(..) {
                match c {
                    Arrange::Split {
                        axis: ca,
                        children: cc,
                    } if ca == axis => flat.extend(cc),
                    other => flat.push(other),
                }
            }
            *children = flat;
            // Collapse a single-child split into the child.
            if children.len() == 1 {
                let only = children.remove(0);
                *self = only;
            }
        }
    }

    /// Remove leaf `id`, collapsing empties. Returns true if found. A bare
    /// `Leaf(id)` returns false (a root leaf can't remove itself - the diff view
    /// guards against closing the last pane).
    pub fn remove(&mut self, id: usize) -> bool {
        let found = self.remove_inner(id);
        if found {
            self.normalize();
        }
        found
    }

    fn remove_inner(&mut self, id: usize) -> bool {
        let Arrange::Split { children, .. } = self else {
            return false;
        };
        let mut found = false;
        let mut i = 0;
        while i < children.len() {
            match &mut children[i] {
                Arrange::Leaf(x) if *x == id => {
                    children.remove(i);
                    found = true;
                }
                node => {
                    if node.remove_inner(id) {
                        found = true;
                        if node.leaf_count() == 0 {
                            children.remove(i);
                            continue;
                        }
                    }
                    i += 1;
                }
            }
        }
        found
    }

    /// Split `target` along `axis`, placing `new` on the `before` (left/top) or
    /// after (right/bottom) side. If `target`'s parent already runs along
    /// `axis`, `new` lands as an adjacent sibling; otherwise `target` is wrapped
    /// in a fresh split. Returns true if `target` was found. `new` must already
    /// have been [`remove`]d from the tree (move = remove then split).
    pub fn split(&mut self, target: usize, axis: Axis, new: usize, before: bool) -> bool {
        let found = self.split_inner(target, axis, new, before);
        if found {
            self.normalize();
        }
        found
    }

    fn split_inner(&mut self, target: usize, axis: Axis, new: usize, before: bool) -> bool {
        // A bare leaf that *is* the target wraps itself in a new split.
        if let Arrange::Leaf(t) = self {
            if *t == target {
                let pair = if before {
                    vec![Arrange::Leaf(new), Arrange::Leaf(target)]
                } else {
                    vec![Arrange::Leaf(target), Arrange::Leaf(new)]
                };
                *self = Arrange::Split {
                    axis,
                    children: pair,
                };
                return true;
            }
            return false;
        }
        let Arrange::Split {
            axis: self_axis,
            children,
        } = self
        else {
            return false;
        };
        let self_axis = *self_axis;
        // Direct-leaf target in this split.
        if let Some(p) = children
            .iter()
            .position(|c| matches!(c, Arrange::Leaf(t) if *t == target))
        {
            if self_axis == axis {
                let at = if before { p } else { p + 1 };
                children.insert(at, Arrange::Leaf(new));
            } else {
                let pair = if before {
                    vec![Arrange::Leaf(new), Arrange::Leaf(target)]
                } else {
                    vec![Arrange::Leaf(target), Arrange::Leaf(new)]
                };
                children[p] = Arrange::Split {
                    axis,
                    children: pair,
                };
            }
            return true;
        }
        // Recurse.
        for c in children.iter_mut() {
            if c.split_inner(target, axis, new, before) {
                return true;
            }
        }
        false
    }

    /// Reconcile the arrangement with the live columns: drop leaves for hidden
    /// or out-of-range columns, then append any visible column missing from the
    /// tree to the root row. `visible[i]` is whether column `i` should show.
    pub fn reconcile(&mut self, visible: &[bool]) {
        let keep = |id: usize| visible.get(id).copied().unwrap_or(false);
        // `retain` only prunes inside splits; a hidden *root* leaf must be
        // replaced here so the visible columns can take its place.
        if let Arrange::Leaf(id) = self
            && !keep(*id)
        {
            *self = Arrange::Split {
                axis: Axis::Row,
                children: Vec::new(),
            };
        }
        self.retain(&keep);
        self.normalize();
        let mut present = Vec::new();
        self.leaves(&mut present);
        let missing: Vec<usize> = (0..visible.len())
            .filter(|&i| visible[i] && !present.contains(&i))
            .collect();
        for id in missing {
            self.append_leaf(id);
        }
        self.normalize();
    }

    /// Append `id` as a new trailing column at the root (root becomes a `Row`
    /// split if it was a single leaf or a `Col`).
    fn append_leaf(&mut self, id: usize) {
        match self {
            Arrange::Split {
                axis: Axis::Row,
                children,
            } => children.push(Arrange::Leaf(id)),
            other => {
                let prev = std::mem::replace(other, Arrange::Leaf(id));
                *other = Arrange::Split {
                    axis: Axis::Row,
                    children: vec![prev, Arrange::Leaf(id)],
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(a: &Arrange) -> Vec<usize> {
        let mut v = Vec::new();
        a.leaves(&mut v);
        v
    }

    #[test]
    fn row_of_one_is_a_leaf() {
        assert_eq!(Arrange::row(&[2]), Arrange::Leaf(2));
    }

    #[test]
    fn split_same_axis_inserts_sibling() {
        let mut a = Arrange::row(&[0, 1]); // Row[0,1]
        assert!(a.split(1, Axis::Row, 2, false)); // 2 after 1
        assert_eq!(leaves(&a), vec![0, 1, 2]);
        assert!(matches!(
            a,
            Arrange::Split {
                axis: Axis::Row,
                ..
            }
        ));
    }

    #[test]
    fn split_cross_axis_nests() {
        let mut a = Arrange::row(&[0, 1]);
        assert!(a.split(0, Axis::Col, 2, false)); // stack 2 under 0
        // 0 becomes Col[0,2]; order overall is [0,2,1].
        assert_eq!(leaves(&a), vec![0, 2, 1]);
    }

    #[test]
    fn split_before_places_new_first() {
        let mut a = Arrange::Leaf(0);
        assert!(a.split(0, Axis::Row, 1, true));
        assert_eq!(leaves(&a), vec![1, 0]);
    }

    #[test]
    fn remove_collapses_to_leaf() {
        let mut a = Arrange::row(&[0, 1]);
        assert!(a.remove(0));
        assert_eq!(a, Arrange::Leaf(1));
    }

    #[test]
    fn remove_nested_collapses() {
        let mut a = Arrange::row(&[0, 1]);
        a.split(1, Axis::Col, 2, false); // Row[0, Col[1,2]]
        assert!(a.remove(2));
        assert_eq!(a, Arrange::row(&[0, 1])); // Col[1] collapses → 1
    }

    #[test]
    fn reconcile_prunes_hidden_and_appends_visible() {
        let mut a = Arrange::row(&[0, 1, 2]);
        // Hide column 1, add a (new) visible column 3.
        a.reconcile(&[true, false, true, true]);
        assert_eq!(leaves(&a), vec![0, 2, 3]);
    }

    #[test]
    fn reconcile_rebuilds_when_root_leaf_hidden() {
        let mut a = Arrange::Leaf(0);
        a.reconcile(&[false, true]); // 0 gone, 1 appears
        assert_eq!(leaves(&a), vec![1]);
    }

    #[test]
    fn move_is_remove_then_split() {
        let mut a = Arrange::row(&[0, 1, 2]);
        // Move 2 to sit under 0.
        assert!(a.remove(2));
        assert!(a.split(0, Axis::Col, 2, false));
        assert_eq!(leaves(&a), vec![0, 2, 1]);
    }
}
