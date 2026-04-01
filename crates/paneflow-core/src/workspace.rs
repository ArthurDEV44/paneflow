use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A workspace represents a single tab in the terminal multiplexer.
///
/// Each workspace has its own working directory and an ordered list of panel
/// references. Panels themselves are owned externally (e.g. in a `HashMap`);
/// the workspace only stores lightweight `Uuid` references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// Unique identifier for this workspace.
    pub id: Uuid,
    /// Auto-generated title (e.g. based on CWD or running process).
    pub title: String,
    /// User-provided override title. When `Some`, displayed instead of `title`.
    pub custom_title: Option<String>,
    /// The working directory associated with this workspace.
    pub working_directory: PathBuf,
    /// Whether this workspace is pinned (pinned tabs resist accidental close).
    pub is_pinned: bool,
    /// Ordered list of panel IDs that belong to this workspace.
    /// Panels are owned externally; this is a lightweight reference list for
    /// the split-tree integration that comes in US-002.
    pub panel_ids: Vec<Uuid>,
}

impl Workspace {
    /// Create a new workspace with the given title and working directory.
    pub fn new(title: impl Into<String>, working_directory: impl Into<PathBuf>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            custom_title: None,
            working_directory: working_directory.into(),
            is_pinned: false,
            panel_ids: Vec::new(),
        }
    }

    /// Returns the display title: `custom_title` if set, otherwise `title`.
    pub fn display_title(&self) -> &str {
        self.custom_title.as_deref().unwrap_or(&self.title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_workspace_has_unique_id() {
        let a = Workspace::new("ws-a", "/tmp/a");
        let b = Workspace::new("ws-b", "/tmp/b");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn display_title_falls_back_to_title() {
        let ws = Workspace::new("default", "/tmp");
        assert_eq!(ws.display_title(), "default");
    }

    #[test]
    fn display_title_prefers_custom() {
        let mut ws = Workspace::new("default", "/tmp");
        ws.custom_title = Some("my tab".into());
        assert_eq!(ws.display_title(), "my tab");
    }

    #[test]
    fn defaults_are_sensible() {
        let ws = Workspace::new("ws", "/home/user");
        assert!(!ws.is_pinned);
        assert!(ws.panel_ids.is_empty());
        assert!(ws.custom_title.is_none());
    }
}
