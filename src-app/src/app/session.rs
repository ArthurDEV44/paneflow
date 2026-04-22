//! Session persistence for `PaneFlowApp` — save/restore workspace layouts
//! and their per-pane CWD + scrollback so relaunching rebuilds exactly
//! what the user had open.
//!
//! Extracted from `main.rs` per US-017 of the src-app refactor PRD.

use std::collections::VecDeque;

use gpui::{App, AppContext, Context, Entity};
use paneflow_config::schema::LayoutNode;

use crate::PaneFlowApp;
use crate::layout::LayoutTree;
use crate::pane::Pane;
use crate::terminal::TerminalView;
use crate::workspace::{Workspace, next_workspace_id};

impl PaneFlowApp {
    pub(crate) fn save_session(&self, cx: &App) {
        let state = paneflow_config::schema::SessionState {
            version: 1,
            active_workspace: self.active_idx,
            workspaces: self
                .workspaces
                .iter()
                .map(|ws| paneflow_config::schema::WorkspaceSession {
                    title: ws.title.clone(),
                    cwd: ws.cwd.clone(),
                    layout: ws.serialize_layout(cx),
                    custom_buttons: ws.custom_buttons.clone(),
                })
                .collect(),
        };
        let Some(path) = paneflow_config::loader::session_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&state) {
            Ok(json) => {
                // Atomic write: write to temp file, then rename. This prevents
                // corruption if the process is killed mid-write (US-013).
                let tmp_path = path.with_extension("json.tmp");
                match std::fs::write(&tmp_path, &json) {
                    Ok(()) => {
                        if let Err(e) = std::fs::rename(&tmp_path, &path) {
                            log::warn!("session save rename failed: {e}");
                            let _ = std::fs::remove_file(&tmp_path);
                        }
                    }
                    Err(e) => {
                        log::warn!("session save failed: {e}");
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                }
            }
            Err(e) => log::warn!("session serialize failed: {e}"),
        }
    }

    pub(crate) fn load_session() -> Option<paneflow_config::schema::SessionState> {
        let path = paneflow_config::loader::session_path()?;
        let data = std::fs::read_to_string(&path).ok()?;
        let state: paneflow_config::schema::SessionState = serde_json::from_str(&data).ok()?;
        Some(state)
    }

    /// Rebuild workspaces from a saved session. Each workspace's layout tree
    /// is reconstructed via `LayoutTree::from_layout_node` with CWD-aware
    /// terminal spawning. Returns the workspace list and active index.
    pub(crate) fn restore_workspaces(
        session: &paneflow_config::schema::SessionState,
        cx: &mut Context<Self>,
    ) -> (Vec<Workspace>, usize) {
        use std::path::PathBuf;

        let mut workspaces = Vec::new();

        for ws_session in &session.workspaces {
            let cwd = PathBuf::from(&ws_session.cwd);
            let ws_id = next_workspace_id();

            let mut workspace = if let Some(mut layout) = ws_session.layout.clone() {
                paneflow_config::loader::validate_layout(&mut layout);
                let mut pane_deque: VecDeque<Entity<Pane>> = VecDeque::new();
                let ws_cwd = cwd.clone();
                let tree = LayoutTree::from_layout_node(&layout, &mut pane_deque, &mut |node| {
                    let surfaces = match node {
                        LayoutNode::Pane { surfaces } => surfaces.as_slice(),
                        _ => &[],
                    };
                    Self::spawn_pane_from_surfaces(ws_id, surfaces, &ws_cwd, cx)
                });
                Workspace::with_layout_and_id(ws_id, ws_session.title.clone(), cwd, tree)
            } else {
                // No saved layout — single terminal in the workspace CWD
                let terminal =
                    cx.new(|cx| TerminalView::with_cwd(ws_id, Some(cwd.clone()), None, cx));
                cx.subscribe(&terminal, Self::handle_terminal_event)
                    .detach();
                let pane = cx.new(|cx| Pane::new(terminal, ws_id, cx));
                cx.subscribe(&pane, Self::handle_pane_event).detach();
                Workspace::with_cwd_and_id(ws_id, ws_session.title.clone(), cwd, pane)
            };

            // Restore persisted custom buttons and push them to every pane
            // in the workspace so the tab bar reflects them on startup.
            workspace.custom_buttons = ws_session.custom_buttons.clone();
            workspace.propagate_custom_buttons(cx);
            workspaces.push(workspace);
        }

        let active_idx = session
            .active_workspace
            .min(workspaces.len().saturating_sub(1));
        (workspaces, active_idx)
    }

    /// Create a `Pane` (with one tab per surface) from serialized surface
    /// definitions. Falls back to a single terminal in `fallback_cwd` when
    /// the surface list is empty.
    pub(crate) fn spawn_pane_from_surfaces(
        workspace_id: u64,
        surfaces: &[paneflow_config::schema::SurfaceDefinition],
        fallback_cwd: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> Entity<Pane> {
        use std::path::PathBuf;

        let mut focus_idx: usize = 0;
        let terminals: Vec<Entity<TerminalView>> = if surfaces.is_empty() {
            let t = cx.new(|cx| {
                TerminalView::with_cwd(workspace_id, Some(fallback_cwd.to_path_buf()), None, cx)
            });
            cx.subscribe(&t, Self::handle_terminal_event).detach();
            vec![t]
        } else {
            surfaces
                .iter()
                .enumerate()
                .map(|(i, surface)| {
                    let cwd = surface
                        .cwd
                        .as_ref()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| fallback_cwd.to_path_buf());
                    let t = cx.new(|cx| TerminalView::with_cwd(workspace_id, Some(cwd), None, cx));
                    // Restore saved scrollback into the terminal grid
                    if let Some(ref scrollback) = surface.scrollback {
                        t.read(cx).terminal.restore_scrollback(scrollback);
                    }
                    cx.subscribe(&t, Self::handle_terminal_event).detach();
                    if surface.focus == Some(true) {
                        focus_idx = i;
                    }
                    t
                })
                .collect()
        };

        let first = terminals[0].clone();
        let pane = cx.new(|cx| {
            let mut p = Pane::new(first, workspace_id, cx);
            for tab in &terminals[1..] {
                p.add_tab(tab.clone(), cx);
            }
            p.selected_idx = focus_idx.min(terminals.len() - 1);
            p
        });
        cx.subscribe(&pane, Self::handle_pane_event).detach();
        pane
    }
}
