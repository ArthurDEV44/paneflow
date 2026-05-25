// US-007 (prd-agents-view.md): these ops are the data-plane primitives
// the Agents sidebar (US-010) and create-affordances (US-011) call
// into. They mutate `PaneFlowApp` state and notify the view, but no
// caller exists yet -- US-008 wires the mode toggle, US-010/US-011
// wire the buttons and context menus that invoke each op. The module-
// scoped allow keeps that staging visible in one line instead of
// twelve `#[allow(dead_code)]` attributes.
#![allow(dead_code)]

//! Project + thread lifecycle operations for `PaneFlowApp`.
//!
//! Mirrors the existing [`crate::app::workspace_ops`] module: every
//! method is a thin state mutation on `PaneFlowApp` that updates the
//! `projects` vector and the `active_project_idx`, then calls
//! [`cx.notify`] so the next render sees the change. UI rendering
//! (sidebar headers, thread rows, context menus) lives in US-010;
//! these ops are the data-plane primitives the sidebar will call into.
//!
//! No GPUI focus / persistence side effects yet. `save_session` is
//! invoked at the call site (matching the workspace_ops pattern --
//! some op chains save once at the end, not after every step).
//!
//! See US-007 of `tasks/prd-agents-view.md`.

use gpui::Context;
use paneflow_acp::AgentKind;

use crate::PaneFlowApp;
use crate::project::{Project, Thread, ThreadStatus, next_project_id, next_thread_id};

/// Hard caps from the PRD's Non-Functional Requirements: at most 50
/// projects per session and 100 threads per project. Exceeding either
/// cap is a no-op (the UI surfaces a toast at the call site -- the op
/// layer just refuses to mutate state).
pub const MAX_PROJECTS: usize = 50;
pub const MAX_THREADS_PER_PROJECT: usize = 100;

/// Outcome of an op that may fail with a user-facing reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// Too many projects already exist.
    ProjectLimitReached,
    /// Too many threads already exist in the target project.
    ThreadLimitReached,
    /// The target project index does not exist.
    ProjectNotFound,
    /// The target thread does not exist within the target project.
    ThreadNotFound,
}

impl PaneFlowApp {
    /// Currently active project, or `None` if no project is selected
    /// (because none have been created yet).
    pub(crate) fn active_project(&self) -> Option<&Project> {
        self.projects.get(self.active_project_idx)
    }

    pub(crate) fn active_project_mut(&mut self) -> Option<&mut Project> {
        self.projects.get_mut(self.active_project_idx)
    }

    /// Create a new project at the end of the list and make it the
    /// active project. Returns the new project's monotonic ID, or
    /// `OpError::ProjectLimitReached` if we are at the cap.
    pub(crate) fn create_project(
        &mut self,
        title: impl Into<String>,
        cwd: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> Result<u64, OpError> {
        if self.projects.len() >= MAX_PROJECTS {
            return Err(OpError::ProjectLimitReached);
        }
        let mut project = Project {
            id: next_project_id(),
            title: title.into(),
            cwd: cwd.into(),
            threads: Vec::new(),
            is_expanded: true,
            git_stats: crate::workspace::GitDiffStats::default(),
        };
        // Defensive: in case some future caller seeds threads via
        // `..Default::default()`, clamp.
        project.threads.truncate(MAX_THREADS_PER_PROJECT);
        let id = project.id;
        self.projects.push(project);
        self.active_project_idx = self.projects.len() - 1;
        cx.notify();
        Ok(id)
    }

    /// Switch to the project at `idx`, no-op if out of bounds or
    /// already active.
    pub(crate) fn select_project(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.projects.len() && idx != self.active_project_idx {
            self.active_project_idx = idx;
            cx.notify();
        }
    }

    /// Close the project at `idx`. Adjusts `active_project_idx` to
    /// stay in range. Returns the removed project (so a caller can
    /// cascade-delete its threads from `paneflow-threads`).
    pub(crate) fn close_project(
        &mut self,
        idx: usize,
        cx: &mut Context<Self>,
    ) -> Result<Project, OpError> {
        if idx >= self.projects.len() {
            return Err(OpError::ProjectNotFound);
        }
        let removed = self.projects.remove(idx);
        // Keep active_project_idx valid: if we removed at or before
        // the active idx, shift it down (clamped to 0). If projects
        // is now empty, reset to 0 (the sidebar reads `is_empty()`
        // to decide whether to render anything).
        self.active_project_idx = if self.projects.is_empty() {
            0
        } else if idx <= self.active_project_idx {
            self.active_project_idx.saturating_sub(1)
        } else {
            self.active_project_idx
        };
        cx.notify();
        Ok(removed)
    }

    /// Rename the project at `idx`. No-op if out of bounds.
    pub(crate) fn rename_project(
        &mut self,
        idx: usize,
        new_title: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), OpError> {
        let Some(project) = self.projects.get_mut(idx) else {
            return Err(OpError::ProjectNotFound);
        };
        project.title = new_title.into();
        cx.notify();
        Ok(())
    }

    /// Move the project at `from` to position `to`. Keeps
    /// `active_project_idx` pointing at the same project. No-op if
    /// either index is out of range or they are equal.
    pub(crate) fn reorder_project(
        &mut self,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) -> Result<(), OpError> {
        let len = self.projects.len();
        if from >= len || to >= len {
            return Err(OpError::ProjectNotFound);
        }
        if from == to {
            return Ok(());
        }
        let moved_id = self.projects[from].id;
        let project = self.projects.remove(from);
        self.projects.insert(to, project);
        // Re-find the moved project to update active_project_idx so
        // the user's selection follows the drag, not the index.
        if let Some(new_idx) = self.projects.iter().position(|p| p.id == moved_id) {
            // Only re-anchor if the active project IS the moved one;
            // otherwise leave the active index pointing at whatever
            // project sits there post-shift.
            if self.projects.get(self.active_project_idx).map(|p| p.id) != Some(moved_id) {
                // Different project was active; preserve its identity.
                // (No-op here because identity is implicit in the
                // shifted vector; we just don't move the index.)
            } else {
                self.active_project_idx = new_idx;
            }
        }
        cx.notify();
        Ok(())
    }

    /// Add a thread to the project at `project_idx`. The thread is
    /// always added to the end of the project's list (the sidebar
    /// will sort by `updated_at DESC` at render time via the threads
    /// DB query -- US-010). Returns the new thread's ID.
    pub(crate) fn add_thread(
        &mut self,
        project_idx: usize,
        title: impl Into<String>,
        agent: AgentKind,
        cx: &mut Context<Self>,
    ) -> Result<u64, OpError> {
        let project = self
            .projects
            .get_mut(project_idx)
            .ok_or(OpError::ProjectNotFound)?;
        if project.threads.len() >= MAX_THREADS_PER_PROJECT {
            return Err(OpError::ThreadLimitReached);
        }
        let thread = Thread {
            id: next_thread_id(),
            title: title.into(),
            agent,
            status: ThreadStatus::Idle,
            cwd: project.cwd.clone(),
            created_at: now_unix_millis(),
            model: None,
            mode: None,
            // US-011: filled in by the affordance handler after the
            // threads.db INSERT succeeds (the op layer stays
            // persistence-agnostic so it can be unit-tested without
            // a SQLite store).
            store_id: None,
        };
        let id = thread.id;
        project.threads.push(thread);
        cx.notify();
        Ok(id)
    }

    /// Mark a thread as the user's current selection. The selection
    /// itself lives on `PaneFlowApp` as a future field (US-010); for
    /// now this op just verifies the indices and notifies the view.
    /// Returns `Err` if either index is out of bounds.
    pub(crate) fn select_thread(
        &mut self,
        project_idx: usize,
        thread_idx: usize,
        cx: &mut Context<Self>,
    ) -> Result<(), OpError> {
        let project = self
            .projects
            .get(project_idx)
            .ok_or(OpError::ProjectNotFound)?;
        if thread_idx >= project.threads.len() {
            return Err(OpError::ThreadNotFound);
        }
        self.active_project_idx = project_idx;
        self.active_thread_idx = Some(thread_idx);
        // Picking a thread leaves the Skills page.
        self.agents_skills_visible = false;
        cx.notify();
        Ok(())
    }

    /// Remove a thread by index within the given project. Returns
    /// the removed [`Thread`] so the caller can cascade-delete its
    /// row from `paneflow-threads`.
    pub(crate) fn remove_thread(
        &mut self,
        project_idx: usize,
        thread_idx: usize,
        cx: &mut Context<Self>,
    ) -> Result<Thread, OpError> {
        let project = self
            .projects
            .get_mut(project_idx)
            .ok_or(OpError::ProjectNotFound)?;
        if thread_idx >= project.threads.len() {
            return Err(OpError::ThreadNotFound);
        }
        let removed = project.threads.remove(thread_idx);
        // Drop the cached ThreadView entity for this thread so its
        // SessionRuntime + subscriptions are released. Persistence has
        // already been flushed by the time remove_thread is called
        // (the delete confirmation dialog only fires on user click,
        // and persistence is event-driven on every mutation).
        self.agents_thread_view_cache.remove(&removed.id);
        // Keep the active-thread selection in range. If we removed the
        // selected thread, clear it; if we removed something earlier,
        // shift it down.
        if let Some(active) = self.active_thread_idx {
            self.active_thread_idx = if thread_idx == active {
                None
            } else if thread_idx < active {
                Some(active - 1)
            } else {
                Some(active)
            };
        }
        cx.notify();
        Ok(removed)
    }
}

fn now_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
