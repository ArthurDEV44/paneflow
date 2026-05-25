//! Affordance handlers for the Agents-mode sidebar (US-011 of
//! `tasks/prd-agents-view.md`): create / rename / delete /
//! duplicate, plus the cross-cutting reveal-in-file-manager and
//! open-in-editor entry points.
//!
//! Every handler mutates `PaneFlowApp` state in-place, calls
//! `cx.notify()` and `save_session(cx)` so the change is persisted
//! across restarts, and -- when a deletion involves a row that has
//! been written to `threads.db` -- cascades the SQL DELETE so the
//! durable store does not accumulate orphan rows (AC #9).

use gpui::{App, ClickEvent, Context, MouseButton, PathPromptOptions, Pixels, Point};
use paneflow_acp::AgentKind;

use super::state::{AgentsContextMenu, AgentsDeleteTarget, AgentsRenameTarget};
use crate::PaneFlowApp;
use crate::app::workspace_ops::{resolve_editor_binary, reveal_in_file_manager};

impl PaneFlowApp {
    // ------------------------------------------------------------------
    // Open / close context menus + confirmation dialog
    // ------------------------------------------------------------------

    /// Open the project-row context menu at the click position.
    /// Closes any prior menu and cancels any pending rename so the
    /// menu opens on a clean slate.
    pub(crate) fn open_agents_project_menu(
        &mut self,
        project_idx: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.cancel_agents_rename(cx);
        self.agents_menu_open = Some(AgentsContextMenu::Project {
            project_idx,
            position,
        });
        cx.notify();
    }

    /// Open the thread-row context menu at the click position.
    pub(crate) fn open_agents_thread_menu(
        &mut self,
        project_idx: usize,
        thread_idx: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.cancel_agents_rename(cx);
        self.agents_menu_open = Some(AgentsContextMenu::Thread {
            project_idx,
            thread_idx,
            position,
        });
        cx.notify();
    }

    pub(crate) fn close_agents_menu(&mut self, cx: &mut Context<Self>) {
        if self.agents_menu_open.take().is_some() {
            cx.notify();
        }
    }

    /// Queue a confirmation dialog for `target`. Idempotent: replacing
    /// a pending confirm with a fresh one is fine.
    pub(crate) fn request_agents_confirm_delete(
        &mut self,
        target: AgentsDeleteTarget,
        cx: &mut Context<Self>,
    ) {
        self.agents_menu_open = None;
        self.agents_confirm_delete = Some(target);
        cx.notify();
    }

    pub(crate) fn cancel_agents_confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.agents_confirm_delete.take().is_some() {
            cx.notify();
        }
    }

    // ------------------------------------------------------------------
    // Rename machinery
    // ------------------------------------------------------------------

    /// Enter inline-rename mode for `target`. Cancels any prior rename
    /// before starting -- only one row can be renaming at a time.
    pub(crate) fn begin_agents_rename(
        &mut self,
        target: AgentsRenameTarget,
        cx: &mut Context<Self>,
    ) {
        self.commit_agents_rename(cx);
        let AgentsRenameTarget::Project { project_idx } = target;
        let current = self
            .projects
            .get(project_idx)
            .map(|p| p.title.clone())
            .unwrap_or_default();
        self.agents_rename_text = current;
        self.agents_renaming = Some(target);
        cx.notify();
    }

    /// Apply the in-progress rename if any, then exit rename mode.
    /// An empty rename text is treated as "user gave up" and rolls
    /// back to the previous title.
    pub(crate) fn commit_agents_rename(&mut self, cx: &App) {
        let Some(target) = self.agents_renaming.take() else {
            return;
        };
        let text = std::mem::take(&mut self.agents_rename_text);
        if text.is_empty() {
            return;
        }
        let AgentsRenameTarget::Project { project_idx } = target;
        if let Some(project) = self.projects.get_mut(project_idx) {
            project.title = text;
            self.save_session(cx);
        }
    }

    /// Drop the in-progress rename without applying. Used when the
    /// user presses Escape, opens a context menu, or clicks elsewhere.
    pub(crate) fn cancel_agents_rename(&mut self, cx: &mut Context<Self>) {
        if self.agents_renaming.take().is_some() {
            self.agents_rename_text.clear();
            cx.notify();
        }
    }

    /// Append a single character to the in-progress rename. Public so
    /// the sidebar's `on_key_down` listener stays compact.
    pub(crate) fn push_agents_rename_char(&mut self, ch: &str, cx: &mut Context<Self>) {
        self.agents_rename_text.push_str(ch);
        cx.notify();
    }

    pub(crate) fn pop_agents_rename_char(&mut self, cx: &mut Context<Self>) {
        self.agents_rename_text.pop();
        cx.notify();
    }

    // ------------------------------------------------------------------
    // Create / Duplicate
    // ------------------------------------------------------------------

    /// Create a new project by prompting the user for one or more
    /// directories via the OS folder picker. Mirrors the CLI sidebar's
    /// `create_workspace_with_picker`: each chosen folder becomes a
    /// fresh project, with the folder's basename as the title. The
    /// most recently created project is selected so the threads list
    /// opens onto it.
    pub(crate) fn create_agents_project_with_picker(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: None,
        });
        cx.spawn(
            async |this: gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                if let Ok(Ok(Some(paths))) = receiver.await {
                    let _ = cx.update(|cx| {
                        this.update(cx, |app, cx| {
                            let mut last_created: Option<usize> = None;
                            for path in paths {
                                let title = path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "New project".to_string());
                                let cwd = path.to_string_lossy().into_owned();
                                match app.create_project(title, cwd.as_str(), cx) {
                                    Ok(_id) => {
                                        last_created = Some(app.projects.len() - 1);
                                    }
                                    Err(err) => {
                                        app.show_toast(
                                            format!("Could not create project: {err:?}"),
                                            cx,
                                        );
                                    }
                                }
                            }
                            if let Some(idx) = last_created {
                                app.active_project_idx = idx;
                                app.active_thread_idx = None;
                            }
                            app.save_session(cx);
                            cx.notify();
                        })
                    });
                }
            },
        )
        .detach();
    }

    /// Create a fresh thread in `project_idx`, defaulting to Claude
    /// Code (the agent picker pill in US-016 lets the user switch
    /// mid-thread). Selects + enters rename so the user can rename
    /// before the first prompt.
    pub(crate) fn create_agents_thread_in(&mut self, project_idx: usize, cx: &mut Context<Self>) {
        // Default agent: prefer ClaudeCode if discovery sees it, fall
        // back to Codex, else ClaudeCode (the user can override in the
        // composer; AC #2 only requires "creates a fresh thread").
        let default_agent = AgentKind::ClaudeCode;
        let (project_id_str, _project_cwd) = match self.projects.get(project_idx) {
            Some(p) => (p.id.to_string(), p.cwd.clone()),
            None => return,
        };

        let new_thread_id = match self.add_thread(project_idx, "New thread", default_agent, cx) {
            Ok(id) => id,
            Err(err) => {
                self.show_toast(format!("Could not create thread: {err:?}"), cx);
                return;
            }
        };

        // Wire the durable store insert + capture the row id. Best-
        // effort: if the store is unavailable, the thread still lives
        // in memory for this session (graceful degradation per US-006
        // AC: corrupted/missing DB recovers without blocking the UI).
        if let Some(store) = self.thread_store.as_ref() {
            let agent_tag = crate::project::agent_kind_to_str(default_agent);
            match store.create_thread(Some(&project_id_str), agent_tag) {
                Ok(store_id) => {
                    if let Some(project) = self.projects.get_mut(project_idx)
                        && let Some(thread) =
                            project.threads.iter_mut().find(|t| t.id == new_thread_id)
                    {
                        thread.store_id = Some(store_id.as_str().to_string());
                    }
                }
                Err(err) => {
                    log::warn!("agents-sidebar: threads.db create failed: {err}");
                }
            }
        }

        // Select the just-created thread so the main area opens onto
        // it (US-013's ThreadView will pick it up once landed).
        if let Some(project) = self.projects.get(project_idx)
            && let Some(thread_idx) = project.threads.iter().position(|t| t.id == new_thread_id)
        {
            let _ = self.select_thread(project_idx, thread_idx, cx);
        }
        self.save_session(cx);
    }

    /// Duplicate a thread: same agent + cwd, empty messages, fresh ID.
    /// (AC #6 -- "Duplicate creates a new thread with the same agent +
    /// cwd, empty messages".)
    pub(crate) fn duplicate_agents_thread(
        &mut self,
        project_idx: usize,
        thread_idx: usize,
        cx: &mut Context<Self>,
    ) {
        let (agent, base_title) = match self
            .projects
            .get(project_idx)
            .and_then(|p| p.threads.get(thread_idx))
        {
            Some(t) => (t.agent, t.title.clone()),
            None => return,
        };
        let project_id_str = match self.projects.get(project_idx) {
            Some(p) => p.id.to_string(),
            None => return,
        };

        let new_title = format!("{base_title} (copy)");
        let new_thread_id = match self.add_thread(project_idx, new_title.clone(), agent, cx) {
            Ok(id) => id,
            Err(err) => {
                self.show_toast(format!("Could not duplicate thread: {err:?}"), cx);
                return;
            }
        };

        if let Some(store) = self.thread_store.as_ref() {
            let agent_tag = crate::project::agent_kind_to_str(agent);
            match store.create_thread(Some(&project_id_str), agent_tag) {
                Ok(store_id) => {
                    if let Some(project) = self.projects.get_mut(project_idx)
                        && let Some(thread) =
                            project.threads.iter_mut().find(|t| t.id == new_thread_id)
                    {
                        thread.store_id = Some(store_id.as_str().to_string());
                        // Sync the summary so the row's `summary` column
                        // doesn't lag behind the in-memory `title`.
                        if let Some(ref id_str) = thread.store_id {
                            let typed = paneflow_threads::store::ThreadId::from_string(id_str);
                            let _ = store.set_summary(&typed, &new_title);
                        }
                    }
                }
                Err(err) => {
                    log::warn!("agents-sidebar: threads.db duplicate failed: {err}");
                }
            }
        }
        self.save_session(cx);
    }

    // ------------------------------------------------------------------
    // Execute confirmed delete
    // ------------------------------------------------------------------

    /// Apply the pending delete (project or thread). Cascades thread
    /// rows when a project is deleted (AC #9 -- "cascade in code, not
    /// in SQL").
    pub(crate) fn execute_agents_confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.agents_confirm_delete.take() else {
            return;
        };
        match target {
            AgentsDeleteTarget::Project { project_idx } => {
                let Ok(removed) = self.close_project(project_idx, cx) else {
                    return;
                };
                let count = removed.threads.len();
                if let Some(store) = self.thread_store.as_ref() {
                    for thread in &removed.threads {
                        if let Some(ref id_str) = thread.store_id {
                            let typed = paneflow_threads::store::ThreadId::from_string(id_str);
                            if let Err(err) = store.delete_thread(&typed) {
                                log::warn!(
                                    "agents-sidebar: cascade delete failed for {id_str}: {err}"
                                );
                            }
                        }
                    }
                }
                self.show_toast(
                    if count == 0 {
                        "Project deleted".to_string()
                    } else if count == 1 {
                        "Project + 1 thread deleted".to_string()
                    } else {
                        format!("Project + {count} threads deleted")
                    },
                    cx,
                );
            }
            AgentsDeleteTarget::Thread {
                project_idx,
                thread_idx,
            } => {
                let Ok(removed) = self.remove_thread(project_idx, thread_idx, cx) else {
                    return;
                };
                if let Some(store) = self.thread_store.as_ref()
                    && let Some(ref id_str) = removed.store_id
                {
                    let typed = paneflow_threads::store::ThreadId::from_string(id_str);
                    if let Err(err) = store.delete_thread(&typed) {
                        log::warn!("agents-sidebar: thread delete failed for {id_str}: {err}");
                    }
                }
                self.show_toast("Thread deleted", cx);
            }
        }
        self.save_session(cx);
    }

    // ------------------------------------------------------------------
    // Reveal / Open in editor (project rows)
    // ------------------------------------------------------------------

    pub(crate) fn reveal_agents_project_in_file_manager(
        &mut self,
        project_idx: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.get(project_idx) else {
            return;
        };
        let cwd = project.cwd.clone();
        self.agents_menu_open = None;
        if let Err(msg) = reveal_in_file_manager(std::path::Path::new(&cwd)) {
            log::warn!("agents-sidebar: reveal failed: {msg}");
            self.show_toast(msg, cx);
        }
        cx.notify();
    }

    pub(crate) fn open_agents_project_in_editor(
        &mut self,
        project_idx: usize,
        command: &str,
        label: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.get(project_idx) else {
            return;
        };
        let cwd = project.cwd.clone();
        let bin = resolve_editor_binary(command);
        if let Err(err) = std::process::Command::new(&bin)
            .current_dir(&cwd)
            .arg(".")
            .spawn()
        {
            log::warn!("agents-sidebar: open in {label} failed: {err}");
            self.show_toast(format!("Couldn't open in {label}: {err}"), cx);
        }
        self.agents_menu_open = None;
        cx.notify();
    }
}

/// Convenience for the sidebar `on_aux_click` listener: a single line
/// that opens the right context menu given the right-click hit. Lives
/// here (not inline at the call site) so the sidebar render stays
/// readable and the menu dispatch logic has one home.
pub(crate) fn handle_right_click_for_project(
    this: &mut PaneFlowApp,
    project_idx: usize,
    e: &ClickEvent,
    cx: &mut Context<PaneFlowApp>,
) {
    if let Some(position) = e.mouse_position()
        && matches!(e, ClickEvent::Mouse(m) if m.down.button == MouseButton::Right)
    {
        this.open_agents_project_menu(project_idx, position, cx);
    }
}

pub(crate) fn handle_right_click_for_thread(
    this: &mut PaneFlowApp,
    project_idx: usize,
    thread_idx: usize,
    e: &ClickEvent,
    cx: &mut Context<PaneFlowApp>,
) {
    if let Some(position) = e.mouse_position()
        && matches!(e, ClickEvent::Mouse(m) if m.down.button == MouseButton::Right)
    {
        this.open_agents_thread_menu(project_idx, thread_idx, position, cx);
    }
}
