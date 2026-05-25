//! State types for the Agents-mode sidebar affordances (US-011).
//!
//! Lives in its own submodule so the public surface visible from
//! `PaneFlowApp` is narrow and discoverable: three small enums that
//! discriminate "is this a project or a thread row?" for each
//! transient interaction. Cloning all three is free (small tags + two
//! `usize`s + an optional `Point<Pixels>`), so we pass them by value.

use gpui::{Pixels, Point};

/// Identifies a sidebar row that is currently in inline-rename mode.
/// Threads are intentionally not renameable — only projects expose
/// the rename affordance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentsRenameTarget {
    Project { project_idx: usize },
}

/// Open right-click context menu, with the anchor position so the
/// deferred renderer can clamp to the window bounds.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AgentsContextMenu {
    Project {
        project_idx: usize,
        position: Point<Pixels>,
    },
    Thread {
        project_idx: usize,
        thread_idx: usize,
        position: Point<Pixels>,
    },
}

/// Pending delete confirmation. The mutation runs only after the user
/// clicks "Delete" in the confirmation dialog -- "Cancel" or
/// click-outside leaves the row intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentsDeleteTarget {
    Project {
        project_idx: usize,
    },
    Thread {
        project_idx: usize,
        thread_idx: usize,
    },
}
