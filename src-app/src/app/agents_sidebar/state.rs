//! State types for the Agents-mode sidebar affordances (US-011).
//!
//! Lives in its own submodule so the public surface visible from
//! `PaneFlowApp` is narrow and discoverable: three small enums that
//! discriminate "is this a project or a thread row?" for each
//! transient interaction. Cloning all three is free (small tags + two
//! `usize`s + an optional `Point<Pixels>`), so we pass them by value.

use gpui::{Pixels, Point};

/// Identifies a sidebar row that is currently in inline-rename mode.
/// Both projects and threads are renameable -- threads were originally
/// not (rely on agent-pushed `SessionInfoUpdate.title` / client auto-
/// derive) but Codex doesn't push titles and the background summarizer
/// is best-effort, so the user gets the always-works escape hatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentsRenameTarget {
    Project {
        project_idx: usize,
    },
    Thread {
        project_idx: usize,
        thread_idx: usize,
    },
    /// US-008: a free chat row (project-less). Renames `chats[chat_idx]`.
    Chat {
        chat_idx: usize,
    },
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
    /// US-008: context menu for a free chat row.
    Chat {
        chat_idx: usize,
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
    /// US-008: delete a free chat (`chats[chat_idx]`).
    Chat {
        chat_idx: usize,
    },
}
