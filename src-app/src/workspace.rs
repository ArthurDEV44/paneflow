//! Workspace — a named collection of terminal panes with a split layout.

use crate::split::SplitNode;
use gpui::{App, Entity, Window};

use crate::terminal::TerminalView;

pub struct Workspace {
    pub title: String,
    /// Working directory at creation time. Does not update when the shell `cd`s.
    pub cwd: String,
    pub root: Option<SplitNode>,
}

impl Workspace {
    pub fn new(title: impl Into<String>, terminal: Entity<TerminalView>) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".into());
        Self {
            title: title.into(),
            cwd,
            root: Some(SplitNode::Leaf(terminal)),
        }
    }

    pub fn pane_count(&self) -> usize {
        self.root.as_ref().map_or(0, |r| r.leaf_count())
    }

    pub fn focus_first(&self, window: &mut Window, cx: &mut App) {
        if let Some(root) = &self.root {
            root.focus_first(window, cx);
        }
    }
}
