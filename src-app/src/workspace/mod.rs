//! Workspace — a named collection of terminal panes with a split layout.
//!
//! Module layout (US-030 of the src-app refactor PRD):
//! - [`git`] — git metadata probing (branch, diff stats, `.git` dir lookup)
//! - [`ports`] — cross-platform TCP listening-port detection (Linux/macOS/stub)
//!
//! The [`Workspace`] struct and its constructors live in this `mod.rs`; git
//! and port helpers are re-exported so external callers keep the flat
//! `crate::workspace::*` API.

mod git;
mod ports;

pub use git::{GitDiffStats, detect_branch, find_git_dir};
pub use ports::detect_ports;

use std::cell::Cell;
use std::rc::Rc;

use gpui::{App, Entity, Window};
use paneflow_config::schema::{ButtonCommand, LayoutNode};

use crate::ai_types::AiToolState;
use crate::layout::LayoutTree;
use crate::pane::Pane;

use self::git::parse_head;

/// Monotonic workspace ID counter. Each workspace gets a unique ID at construction.
static NEXT_WORKSPACE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn next_workspace_id() -> u64 {
    NEXT_WORKSPACE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub struct Workspace {
    /// Unique workspace identifier, assigned at construction.
    pub id: u64,
    pub title: String,
    /// Working directory at creation time. Does not update when the shell `cd`s.
    pub cwd: String,
    pub root: Option<LayoutTree>,
    /// Saved layout tree when zoomed. `Some(tree)` means the workspace is zoomed
    /// and `root` contains only the zoomed pane as a single Leaf.
    pub saved_layout: Option<LayoutTree>,
    /// Cached git diff stats, refreshed by a background poller.
    pub git_stats: GitDiffStats,
    /// Current git branch name. Empty string when not a git repo or branch unknown.
    pub git_branch: String,
    /// Whether this workspace's CWD is inside a git repository.
    pub is_git_repo: bool,
    /// Resolved `.git` directory path (for file watcher). `None` if not a git repo.
    pub git_dir: Option<std::path::PathBuf>,
    /// Active TCP listening ports from workspace terminal processes.
    pub active_ports: Vec<u16>,
    /// Generation counter for debouncing event-driven port scans.
    /// Incremented on each `ActivityBurst` event; superseded scans check this
    /// to abort if a newer scan was triggered.
    pub port_scan_generation: u64,
    /// Service metadata detected from PTY output (enrichment for `active_ports`).
    /// Keyed by port number; cleaned up when ports are removed from `active_ports`.
    pub service_labels: std::collections::HashMap<u16, crate::terminal::ServiceInfo>,
    /// AI tool detection state for this workspace's terminals (Claude Code / Codex).
    pub ai_state: AiToolState,
    /// Animation angle for the Claude thinking spinner (radians, 0..TAU).
    pub loader_angle: Rc<Cell<f32>>,
    /// Registered AI agent PIDs, keyed by tool name ("claude", "codex").
    /// Used by the stale PID sweep to detect crashed processes.
    pub agent_pids: std::collections::HashMap<String, u32>,
    /// Name of the Claude tool currently being used (e.g., "Edit", "Bash").
    /// Set by `ai.tool_use`, cleared on state transitions. For future verbose display.
    pub active_tool_name: Option<String>,
    /// User-defined tab-bar buttons for this workspace.
    /// Rendered after the 2 built-in defaults (Claude / Codex).
    pub custom_buttons: Vec<ButtonCommand>,
}

impl Workspace {
    /// Create a workspace with a pre-allocated ID (use `next_workspace_id()` to obtain one).
    pub fn with_id(id: u64, title: impl Into<String>, pane: Entity<Pane>) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".into());
        let git_stats = GitDiffStats::from_cwd(&cwd);
        let git_dir = find_git_dir(&cwd);
        let (git_branch, is_git_repo) = match &git_dir {
            Some(dir) => parse_head(dir),
            None => (String::new(), false),
        };
        Self {
            id,
            title: title.into(),
            cwd,
            root: Some(LayoutTree::Leaf(pane)),
            saved_layout: None,
            git_stats,
            git_branch,
            is_git_repo,
            git_dir,
            active_ports: vec![],
            port_scan_generation: 0,
            service_labels: std::collections::HashMap::new(),
            ai_state: AiToolState::Inactive,
            loader_angle: Rc::new(Cell::new(0.0)),
            agent_pids: std::collections::HashMap::new(),
            active_tool_name: None,
            custom_buttons: Vec::new(),
        }
    }

    /// Create a workspace with a pre-allocated ID and explicit CWD.
    pub fn with_cwd_and_id(
        id: u64,
        title: impl Into<String>,
        cwd: std::path::PathBuf,
        pane: Entity<Pane>,
    ) -> Self {
        let cwd_str = cwd.display().to_string();
        let git_stats = GitDiffStats::from_cwd(&cwd_str);
        let git_dir = find_git_dir(&cwd_str);
        let (git_branch, is_git_repo) = match &git_dir {
            Some(dir) => parse_head(dir),
            None => (String::new(), false),
        };
        Self {
            id,
            title: title.into(),
            cwd: cwd_str,
            root: Some(LayoutTree::Leaf(pane)),
            saved_layout: None,
            git_stats,
            git_branch,
            is_git_repo,
            git_dir,
            active_ports: vec![],
            port_scan_generation: 0,
            service_labels: std::collections::HashMap::new(),
            ai_state: AiToolState::Inactive,
            loader_angle: Rc::new(Cell::new(0.0)),
            agent_pids: std::collections::HashMap::new(),
            active_tool_name: None,
            custom_buttons: Vec::new(),
        }
    }

    /// Create a workspace with a pre-allocated ID and layout tree.
    pub fn with_layout_and_id(
        id: u64,
        title: impl Into<String>,
        cwd: std::path::PathBuf,
        root: LayoutTree,
    ) -> Self {
        let cwd_str = cwd.display().to_string();
        let git_stats = GitDiffStats::from_cwd(&cwd_str);
        let git_dir = find_git_dir(&cwd_str);
        let (git_branch, is_git_repo) = match &git_dir {
            Some(dir) => parse_head(dir),
            None => (String::new(), false),
        };
        Self {
            id,
            title: title.into(),
            cwd: cwd_str,
            root: Some(root),
            saved_layout: None,
            git_stats,
            git_branch,
            is_git_repo,
            git_dir,
            active_ports: vec![],
            port_scan_generation: 0,
            service_labels: std::collections::HashMap::new(),
            ai_state: AiToolState::Inactive,
            loader_angle: Rc::new(Cell::new(0.0)),
            agent_pids: std::collections::HashMap::new(),
            active_tool_name: None,
            custom_buttons: Vec::new(),
        }
    }

    pub fn is_zoomed(&self) -> bool {
        self.saved_layout.is_some()
    }

    pub fn pane_count(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.leaf_count())
    }

    /// Total number of terminal tabs across every pane in the layout.
    /// A pane holds 1..N tabbed terminals, so this is ≥ `pane_count()`.
    pub fn terminal_count(&self, cx: &App) -> usize {
        let Some(root) = self.root.as_ref() else {
            return 0;
        };
        root.collect_leaves()
            .iter()
            .map(|pane| pane.read(cx).tabs.len())
            .sum()
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        if let Some(root) = &self.root {
            root.focus_first(window, cx);
        }
    }

    /// Serialize the workspace layout to a `LayoutNode`.
    ///
    /// When zoomed, serializes the saved (un-zoomed) layout so that the full
    /// pane arrangement is captured rather than just the single zoomed pane.
    pub fn serialize_layout(&self, cx: &App) -> Option<LayoutNode> {
        let tree = self.saved_layout.as_ref().or(self.root.as_ref())?;
        Some(tree.serialize(cx))
    }

    /// Push the current `custom_buttons` list to every `Pane` in the
    /// workspace's layout tree so the tab bar re-renders with the new set.
    /// Call after mutating `self.custom_buttons` (add / edit / delete).
    pub fn propagate_custom_buttons(&self, cx: &mut App) {
        if let Some(root) = &self.root {
            walk_and_push_buttons(root, &self.custom_buttons, cx);
        }
        if let Some(saved) = &self.saved_layout {
            walk_and_push_buttons(saved, &self.custom_buttons, cx);
        }
    }
}

fn walk_and_push_buttons(node: &LayoutTree, buttons: &[ButtonCommand], cx: &mut App) {
    match node {
        LayoutTree::Leaf(pane) => {
            pane.update(cx, |p, cx| {
                p.custom_buttons = buttons.to_vec();
                cx.notify();
            });
        }
        LayoutTree::Container { children, .. } => {
            for child in children {
                walk_and_push_buttons(&child.node, buttons, cx);
            }
        }
    }
}
