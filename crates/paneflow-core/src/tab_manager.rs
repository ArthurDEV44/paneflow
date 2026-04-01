use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur when mutating the tab list.
#[derive(Debug, thiserror::Error)]
pub enum TabManagerError {
    #[error("workspace {0} not found")]
    WorkspaceNotFound(Uuid),

    #[error("cannot close the last workspace")]
    CannotCloseLastWorkspace,

    #[error("index {index} is out of bounds (len {len})")]
    IndexOutOfBounds { index: usize, len: usize },
}

// ---------------------------------------------------------------------------
// TabManager
// ---------------------------------------------------------------------------

/// Manages an ordered list of [`Workspace`]s and tracks which one is selected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabManager {
    /// Ordered list of workspaces (tab order).
    workspaces: Vec<Workspace>,
    /// The `id` of the currently selected workspace, or `None` when the list
    /// is empty (should not happen during normal operation).
    pub selected_id: Option<Uuid>,
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TabManager {
    /// Create an empty `TabManager` with no workspaces.
    pub fn new() -> Self {
        Self {
            workspaces: Vec::new(),
            selected_id: None,
        }
    }

    // -- Accessors ----------------------------------------------------------

    /// Returns a slice over all workspaces in tab order.
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    /// Returns the number of open workspaces.
    pub fn len(&self) -> usize {
        self.workspaces.len()
    }

    /// Returns `true` when there are no workspaces.
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }

    /// Find a workspace by its `id`.
    pub fn get(&self, id: Uuid) -> Option<&Workspace> {
        self.workspaces.iter().find(|ws| ws.id == id)
    }

    /// Returns the currently selected workspace, if any.
    pub fn selected(&self) -> Option<&Workspace> {
        self.selected_id
            .and_then(|id| self.workspaces.iter().find(|ws| ws.id == id))
    }

    // -- Mutations ----------------------------------------------------------

    /// Add a new workspace at the end of the tab list and select it.
    ///
    /// Returns the `Uuid` of the newly created workspace.
    pub fn add_workspace(&mut self, workspace: Workspace) -> Uuid {
        let id = workspace.id;
        self.workspaces.push(workspace);
        self.selected_id = Some(id);
        id
    }

    /// Close (remove) the workspace identified by `id`.
    ///
    /// After removal the selection moves to the workspace that now occupies
    /// the same index, or — if the removed workspace was the last in the
    /// list — to the new last workspace. Closing the **only** remaining
    /// workspace returns [`TabManagerError::CannotCloseLastWorkspace`].
    pub fn close_workspace(&mut self, id: Uuid) -> Result<(), TabManagerError> {
        let index = self
            .workspaces
            .iter()
            .position(|ws| ws.id == id)
            .ok_or(TabManagerError::WorkspaceNotFound(id))?;

        if self.workspaces.len() == 1 {
            return Err(TabManagerError::CannotCloseLastWorkspace);
        }

        self.workspaces.remove(index);

        // Pick the new selection: same index if it still exists, otherwise
        // the previous one (i.e. the new last element).
        let new_index = if index < self.workspaces.len() {
            index
        } else {
            self.workspaces.len() - 1
        };
        self.selected_id = Some(self.workspaces[new_index].id);

        Ok(())
    }

    /// Select the workspace identified by `id`.
    pub fn select_workspace(&mut self, id: Uuid) -> Result<(), TabManagerError> {
        if !self.workspaces.iter().any(|ws| ws.id == id) {
            return Err(TabManagerError::WorkspaceNotFound(id));
        }
        self.selected_id = Some(id);
        Ok(())
    }

    /// Move the workspace identified by `id` to `new_index` in the tab order.
    ///
    /// The index is clamped to `0..len`. Other workspaces shift to
    /// accommodate the move. The selection is **not** changed.
    pub fn reorder_workspace(&mut self, id: Uuid, new_index: usize) -> Result<(), TabManagerError> {
        let old_index = self
            .workspaces
            .iter()
            .position(|ws| ws.id == id)
            .ok_or(TabManagerError::WorkspaceNotFound(id))?;

        // Clamp target to valid range.
        let clamped = new_index.min(self.workspaces.len() - 1);

        if old_index != clamped {
            let ws = self.workspaces.remove(old_index);
            self.workspaces.insert(clamped, ws);
        }

        Ok(())
    }

    // -- Helpers (private) --------------------------------------------------

    /// Returns the positional index of a workspace, or `None`.
    #[allow(dead_code)]
    fn index_of(&self, id: Uuid) -> Option<usize> {
        self.workspaces.iter().position(|ws| ws.id == id)
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    /// Helper: create a workspace with a simple title.
    fn make_ws(title: &str) -> Workspace {
        Workspace::new(title, "/tmp")
    }

    // -- add ----------------------------------------------------------------

    #[test]
    fn add_workspace_sets_selected() {
        let mut mgr = TabManager::new();
        let id = mgr.add_workspace(make_ws("first"));
        assert_eq!(mgr.selected_id, Some(id));
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn add_multiple_selects_latest() {
        let mut mgr = TabManager::new();
        let _a = mgr.add_workspace(make_ws("a"));
        let b = mgr.add_workspace(make_ws("b"));
        assert_eq!(mgr.selected_id, Some(b));
        assert_eq!(mgr.len(), 2);
    }

    // -- close (middle) -----------------------------------------------------

    #[test]
    fn close_middle_selects_same_index() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let b = mgr.add_workspace(make_ws("b"));
        let c = mgr.add_workspace(make_ws("c"));

        // Select A so we can verify selection changes on close.
        mgr.select_workspace(a).unwrap();

        // Close B (index 1). C should slide into index 1 and be selected.
        mgr.close_workspace(b).unwrap();
        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.selected_id, Some(c));
    }

    // -- close (last in list) -----------------------------------------------

    #[test]
    fn close_last_selects_previous() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let b = mgr.add_workspace(make_ws("b"));

        mgr.close_workspace(b).unwrap();
        assert_eq!(mgr.selected_id, Some(a));
        assert_eq!(mgr.len(), 1);
    }

    // -- close (only workspace) ---------------------------------------------

    #[test]
    fn close_only_workspace_returns_error() {
        let mut mgr = TabManager::new();
        let id = mgr.add_workspace(make_ws("only"));

        let err = mgr.close_workspace(id).unwrap_err();
        assert!(matches!(err, TabManagerError::CannotCloseLastWorkspace));
        // Workspace must still be present.
        assert_eq!(mgr.len(), 1);
    }

    // -- close (not found) --------------------------------------------------

    #[test]
    fn close_nonexistent_returns_error() {
        let mut mgr = TabManager::new();
        let _id = mgr.add_workspace(make_ws("ws"));

        let err = mgr.close_workspace(Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, TabManagerError::WorkspaceNotFound(_)));
    }

    // -- select -------------------------------------------------------------

    #[test]
    fn select_workspace_updates_selected_id() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let _b = mgr.add_workspace(make_ws("b"));

        mgr.select_workspace(a).unwrap();
        assert_eq!(mgr.selected_id, Some(a));
    }

    #[test]
    fn select_nonexistent_returns_error() {
        let mut mgr = TabManager::new();
        let _a = mgr.add_workspace(make_ws("a"));

        let err = mgr.select_workspace(Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, TabManagerError::WorkspaceNotFound(_)));
    }

    // -- reorder ------------------------------------------------------------

    #[test]
    fn reorder_moves_workspace() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let _b = mgr.add_workspace(make_ws("b"));
        let c = mgr.add_workspace(make_ws("c"));

        // Move A from index 0 to index 2 (end).
        mgr.reorder_workspace(a, 2).unwrap();

        let ids: Vec<Uuid> = mgr.workspaces().iter().map(|ws| ws.id).collect();
        assert_eq!(ids, vec![_b, c, a]);
    }

    #[test]
    fn reorder_clamps_out_of_bounds_index() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let b = mgr.add_workspace(make_ws("b"));

        // Request index 100 — should clamp to last valid position (1).
        mgr.reorder_workspace(a, 100).unwrap();
        let ids: Vec<Uuid> = mgr.workspaces().iter().map(|ws| ws.id).collect();
        assert_eq!(ids, vec![b, a]);
    }

    #[test]
    fn reorder_same_position_is_noop() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let b = mgr.add_workspace(make_ws("b"));

        mgr.reorder_workspace(a, 0).unwrap();
        let ids: Vec<Uuid> = mgr.workspaces().iter().map(|ws| ws.id).collect();
        assert_eq!(ids, vec![a, b]);
    }

    #[test]
    fn reorder_nonexistent_returns_error() {
        let mut mgr = TabManager::new();
        let _a = mgr.add_workspace(make_ws("a"));

        let err = mgr.reorder_workspace(Uuid::new_v4(), 0).unwrap_err();
        assert!(matches!(err, TabManagerError::WorkspaceNotFound(_)));
    }

    // -- selected accessor --------------------------------------------------

    #[test]
    fn selected_returns_correct_workspace() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let _b = mgr.add_workspace(make_ws("b"));

        mgr.select_workspace(a).unwrap();
        let sel = mgr.selected().unwrap();
        assert_eq!(sel.id, a);
        assert_eq!(sel.title, "a");
    }

    // -- close first workspace, selection goes to next ----------------------

    #[test]
    fn close_first_selects_new_first() {
        let mut mgr = TabManager::new();
        let a = mgr.add_workspace(make_ws("a"));
        let b = mgr.add_workspace(make_ws("b"));
        let _c = mgr.add_workspace(make_ws("c"));

        mgr.close_workspace(a).unwrap();
        // Index 0 now holds B.
        assert_eq!(mgr.selected_id, Some(b));
    }
}
